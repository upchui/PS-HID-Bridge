//! Command line front end of the PS5 HID Bridge.

use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use pshb_core::config::{self, Config};
use pshb_core::controller::{Controller, Watchdog};
use pshb_core::crypto::parse_key_hex;
use pshb_core::engine::{Engine, Notice};
use pshb_core::input::{self, InputMsg, Readers};
use pshb_core::protocol::{self, Pong};
use pshb_core::script;
use pshb_core::transport::{KeyState, Link, LinkConfig};
use pshb_core::trigger::{Action, Trigger};

#[derive(Parser)]
#[command(name = "pshb-cli", version, about = "PS5 HID Bridge – Kommandozeile")]
struct Cli {
    /// Einstellungsdatei (Standard: ~/.config/ps5-hid-bridge/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Args, Clone, Default)]
struct BoardArgs {
    /// Adresse des Boards, z. B. 192.168.1.50 oder 192.168.1.50:45045
    #[arg(long)]
    board: Option<String>,
    /// Pre-Shared Key als 64 Hex-Zeichen
    #[arg(long, conflicts_with = "psk_file")]
    psk: Option<String>,
    /// Datei mit dem Pre-Shared Key (Standard: ~/.config/ps5-hid-bridge/psk)
    #[arg(long)]
    psk_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Eingabegeräte auflisten
    ListDevices,
    /// Maus und Tastatur an das Board weiterleiten
    Run {
        #[command(flatten)]
        board: BoardArgs,
        /// Gerät (Pfad, vvvv:pppp oder Teil des Namens); mehrfach möglich
        #[arg(long = "device")]
        devices: Vec<String>,
        /// Tastenkombination zum Umschalten, z. B. "Strg+Alt+P"
        #[arg(long)]
        hotkey: Option<String>,
    },
    /// Eingaben aus einer Skriptdatei abspielen (Test ohne echte Geräte)
    Replay {
        #[command(flatten)]
        board: BoardArgs,
        /// Skriptdatei (Format siehe README)
        file: PathBuf,
        #[arg(long)]
        hotkey: Option<String>,
    },
    /// Verbindung prüfen und Board-Status anzeigen
    Ping {
        #[command(flatten)]
        board: BoardArgs,
        #[arg(long, default_value_t = 5)]
        count: u32,
    },
    /// Einmalig Tasten/Mausbewegung senden (zum Testen)
    Send {
        #[command(flatten)]
        board: BoardArgs,
        /// HID-Tastencodes (hex), z. B. 04,1d
        #[arg(long, value_delimiter = ',')]
        keys: Vec<String>,
        /// Modifikator-Byte (hex), z. B. 02 = Shift links
        #[arg(long, default_value = "0")]
        mods: String,
        /// Maustasten-Bits (hex)
        #[arg(long, default_value = "0")]
        buttons: String,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        dx: i32,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        dy: i32,
        /// So lange gedrückt halten, dann alles loslassen
        #[arg(long, default_value_t = 100)]
        hold_ms: u64,
    },
    /// Einstellungen anzeigen und ändern
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Aktuelle Einstellungen anzeigen
    Show,
    /// Einstellungen setzen
    Set {
        #[arg(long)]
        board: Option<String>,
        /// Geräteliste ersetzen (mehrfach möglich)
        #[arg(long = "device")]
        devices: Vec<String>,
        /// Umschalt-Kombination ersetzen, z. B. "Strg+Alt+P"
        #[arg(long)]
        hotkey: Option<String>,
        /// Fensterregel: aktiv, solange dieses Fenster den Fokus hat (leer = aus)
        #[arg(long)]
        window_rule: Option<String>,
        #[arg(long)]
        sensitivity: Option<f64>,
        #[arg(long)]
        accel: Option<f64>,
    },
    /// Pre-Shared Key speichern ("-" liest von der Standardeingabe)
    SetPsk { key: String },
    /// Neuen zufälligen Pre-Shared Key erzeugen, speichern und anzeigen
    GenPsk,
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Release);
}

fn install_signal_handlers() {
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }
}

struct Ctx {
    config_path: PathBuf,
    config: Config,
}

fn resolve(board: &str) -> Result<SocketAddr, String> {
    if board.is_empty() {
        return Err("Keine Board-Adresse: --board angeben oder „pshb-cli config set --board …“.".into());
    }
    let with_port = if board.contains(':') {
        board.to_string()
    } else {
        format!("{board}:{}", protocol::UDP_PORT_DEFAULT)
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("Adresse {board} nicht auflösbar: {e}"))?
        .next()
        .ok_or_else(|| format!("Adresse {board} nicht auflösbar"))
}

fn board_addr(ctx: &Ctx, args: &BoardArgs) -> Result<SocketAddr, String> {
    resolve(args.board.as_deref().unwrap_or(&ctx.config.board))
}

fn load_key(args: &BoardArgs) -> Result<[u8; 32], String> {
    match (&args.psk, &args.psk_file) {
        (Some(k), _) => parse_key_hex(k),
        (None, Some(path)) => config::load_psk(path),
        (None, None) => config::load_psk(&config::psk_path()),
    }
}

fn start_link(ctx: &Ctx, args: &BoardArgs, ping: Duration) -> Result<Link, String> {
    let mut cfg = LinkConfig::new(board_addr(ctx, args)?, load_key(args)?);
    cfg.ping_interval = ping;
    cfg.counter_store = Some(config::counter_path());
    Link::start(cfg).map_err(|e| format!("UDP: {e}"))
}

fn no_answer_reason(link: &Link) -> String {
    if link.status().bad_replies > 0 {
        "Das Board antwortet, aber die Antworten sind nicht gültig signiert: Stimmt der Schlüssel?".into()
    } else {
        "Keine Antwort vom Board (Adresse, Port, Schlüssel und Absenderfilter prüfen).".into()
    }
}

fn connect(ctx: &Ctx, args: &BoardArgs, ping: Duration) -> Result<Link, String> {
    let link = start_link(ctx, args, ping)?;
    if !link.wait_connected(Duration::from_secs(2)) {
        return Err(no_answer_reason(&link));
    }
    Ok(link)
}

fn parse_hex_u8(s: &str) -> Result<u8, String> {
    u8::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("keine Hex-Zahl: {s}"))
}

fn describe(p: &Pong) -> String {
    let mut flags = vec![if p.status & protocol::STATUS_USB_MOUNTED != 0 {
        "USB verbunden"
    } else {
        "USB getrennt"
    }];
    if p.status & protocol::STATUS_USB_SUSPENDED != 0 {
        flags.push("USB im Ruhemodus");
    }
    if p.status & protocol::STATUS_FAILSAFE != 0 {
        flags.push("Failsafe aktiv");
    }
    format!(
        "Firmware {}, Laufzeit {:.1} s, Session {:016x}, {}\n  Pakete: ok {}, Signatur falsch {}, Replay {}, gefiltert {}, fehlerhaft {}",
        p.fw_version_string(),
        p.uptime_ms as f64 / 1000.0,
        p.session_id,
        flags.join(", "),
        p.rx_ok,
        p.rx_auth_fail,
        p.rx_replay,
        p.rx_filtered,
        p.rx_malformed
    )
}

fn notice_text(n: Notice) -> &'static str {
    match n {
        Notice::Activated => "Steuerung AN – Maus und Tastatur gehen an die PS5.",
        Notice::Deactivated => "Steuerung AUS – Maus und Tastatur gehören wieder dem Desktop.",
        Notice::WaitingForRelease => "Bitte alle Tasten loslassen …",
        Notice::Paused => "Pausiert.",
        Notice::Resumed => "Fortgesetzt.",
        Notice::Emergency => "Not-Aus: Geräte freigegeben.",
        Notice::ReleasedAll => "Alle Tasten losgelassen.",
    }
}

fn engine_config(ctx: &Ctx, hotkey: Option<&str>) -> Result<pshb_core::engine::EngineConfig, String> {
    let mut ec = ctx.config.engine_config()?;
    if let Some(h) = hotkey {
        ec.triggers = vec![Trigger::combo(h, Action::Toggle)?];
    }
    Ok(ec)
}

fn trigger_summary(ctx: &Ctx, hotkey: Option<&str>) -> String {
    match hotkey {
        Some(h) => h.to_string(),
        None => ctx
            .config
            .triggers
            .iter()
            .map(|t| format!("{} ({})", t.keys, t.action))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

// ---------------------------------------------------------------- commands

fn cmd_list_devices() -> Result<(), String> {
    let list = input::list_devices().map_err(|e| format!("/dev/input: {e}"))?;
    println!("{:<20} {:<14} {:<10} Name", "Pfad", "Art", "ID");
    for d in &list.devices {
        if d.keyboard || d.mouse {
            println!(
                "{:<20} {:<14} {:04x}:{:04x}  {}",
                d.path.display(),
                d.kind(),
                d.vendor,
                d.product,
                d.name
            );
        }
    }
    if list.inaccessible > 0 {
        println!(
            "\n{} Geräte ohne Leserecht. Zugriff einrichten: siehe README, Abschnitt „Gerätezugriff“.",
            list.inaccessible
        );
    }
    Ok(())
}

fn cmd_run(ctx: &Ctx, board: &BoardArgs, devices: &[String], hotkey: Option<&str>) -> Result<(), String> {
    let specs = if devices.is_empty() {
        ctx.config.devices.clone()
    } else {
        devices.to_vec()
    };
    if specs.is_empty() {
        return Err(
            "Keine Geräte gewählt: --device angeben oder „pshb-cli config set --device …“ (siehe list-devices)."
                .into(),
        );
    }
    let ecfg = engine_config(ctx, hotkey)?;
    let list = input::list_devices().map_err(|e| format!("/dev/input: {e}"))?;
    let selected = input::select(&list.devices, &specs);
    if selected.is_empty() {
        let mut msg = format!("Kein Gerät passt zu {specs:?}.");
        if list.inaccessible > 0 {
            msg += &format!(
                " {} Geräte sind ohne Leserecht (siehe README, „Gerätezugriff“).",
                list.inaccessible
            );
        }
        return Err(msg);
    }
    for d in &selected {
        println!("Gerät: {} ({}, {})", d.name, d.kind(), d.path.display());
    }

    let link = Arc::new(start_link(ctx, board, Duration::from_secs(1))?);
    if link.wait_connected(Duration::from_secs(2)) {
        if let Some(p) = link.status().board {
            println!("Board verbunden: {}", describe(&p));
        }
    } else {
        eprintln!("Warnung: {} Es wird weiter versucht.", no_answer_reason(&link));
    }

    let (tx, rx) = mpsc::channel();
    let readers = Readers::start(&selected, tx).map_err(|e| format!("Gerät öffnen: {e}"))?;
    let grab = readers.grab_flag();
    let watchdog = Watchdog::start(link.clone(), grab.clone(), Duration::from_secs(1));
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone(), Some(grab));
    install_signal_handlers();

    println!(
        "Bereit. Umschalten: {}. Not-Aus: Strg+Alt+Shift+Esc. Beenden: Strg+C.",
        trigger_summary(ctx, hotkey)
    );

    let mut remaining = selected.len();
    let mut connected = link.status().is_connected();
    let mut result = Ok(());
    while !STOP.load(Ordering::Acquire) {
        watchdog.feed();
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(InputMsg::Event(ev)) => {
                for n in ctl.handle(ev, Instant::now()) {
                    println!("{}", notice_text(n));
                }
            }
            Ok(InputMsg::Gone { path, error }) => {
                eprintln!("Gerät {} getrennt: {error}", path.display());
                remaining -= 1;
                if remaining == 0 {
                    result = Err("Alle Geräte getrennt.".into());
                    break;
                }
            }
            Ok(InputMsg::GrabFailed { path, error }) => {
                eprintln!(
                    "Gerät {} kann nicht exklusiv übernommen werden ({error}). Steuerung bleibt aus.",
                    path.display()
                );
                for n in ctl.command(Action::Off) {
                    println!("{}", notice_text(n));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if watchdog.take_fired() {
            eprintln!("Watchdog: Hauptschleife hing, Geräte wurden freigegeben.");
        }
        let now_connected = link.status().is_connected();
        if now_connected != connected {
            connected = now_connected;
            println!(
                "{}",
                if connected {
                    "Board wieder erreichbar."
                } else {
                    "Board nicht erreichbar – Eingaben gehen verloren, Verbindung wird weiter versucht."
                }
            );
        }
    }

    ctl.shutdown();
    drop(watchdog);
    drop(readers);
    thread::sleep(Duration::from_millis(30));
    println!("Beendet.");
    result
}

fn cmd_replay(ctx: &Ctx, board: &BoardArgs, file: &Path, hotkey: Option<&str>) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let steps = script::parse(&text)?;
    let ecfg = engine_config(ctx, hotkey)?;
    let link = Arc::new(connect(ctx, board, Duration::from_secs(1))?);
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone(), None);
    for step in steps {
        thread::sleep(step.delay);
        for n in ctl.handle(step.event, Instant::now()) {
            println!("{}", notice_text(n));
        }
    }
    thread::sleep(Duration::from_millis(50));
    ctl.shutdown();
    thread::sleep(Duration::from_millis(30));
    Ok(())
}

fn cmd_ping(ctx: &Ctx, board: &BoardArgs, count: u32) -> Result<(), String> {
    let link = connect(ctx, board, Duration::from_millis(200))?;
    let mut last = None;
    let mut answered = 0;
    let start = Instant::now();
    while answered < count && start.elapsed() < Duration::from_millis(500 * count as u64 + 1000) {
        thread::sleep(Duration::from_millis(20));
        let st = link.status();
        if st.last_pong != last {
            last = st.last_pong;
            answered += 1;
            if let Some(rtt) = st.rtt {
                println!("Antwort: Laufzeit {:.2} ms", rtt.as_secs_f64() * 1000.0);
            }
        }
    }
    if let Some(p) = link.status().board {
        println!("{}", describe(&p));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_send(
    ctx: &Ctx,
    board: &BoardArgs,
    keys: &[String],
    mods: &str,
    buttons: &str,
    dx: i32,
    dy: i32,
    hold_ms: u64,
) -> Result<(), String> {
    if keys.len() > 6 {
        return Err("Höchstens 6 Tasten.".into());
    }
    let mut state = KeyState {
        mods: parse_hex_u8(mods)?,
        buttons: parse_hex_u8(buttons)?,
        ..Default::default()
    };
    for (slot, k) in state.keys.iter_mut().zip(keys) {
        *slot = parse_hex_u8(k)?;
    }
    let link = connect(ctx, board, Duration::from_secs(1))?;
    link.set_active(true);
    link.set_state(state);
    link.add_motion(dx, dy, 0, 0);
    thread::sleep(Duration::from_millis(hold_ms));
    link.set_active(false);
    thread::sleep(Duration::from_millis(20));
    Ok(())
}

fn cmd_config(ctx: &mut Ctx, cmd: &ConfigCmd) -> Result<(), String> {
    match cmd {
        ConfigCmd::Show => {
            println!("# {}", ctx.config_path.display());
            print!(
                "{}",
                toml::to_string_pretty(&ctx.config).map_err(|e| e.to_string())?
            );
            let psk = config::psk_path();
            println!(
                "# Schlüssel: {} ({})",
                psk.display(),
                if psk.exists() { "vorhanden" } else { "fehlt" }
            );
        }
        ConfigCmd::Set {
            board,
            devices,
            hotkey,
            window_rule,
            sensitivity,
            accel,
        } => {
            let c = &mut ctx.config;
            if let Some(b) = board {
                resolve(b)?;
                c.board = b.clone();
            }
            if !devices.is_empty() {
                c.devices = devices.clone();
            }
            if let Some(h) = hotkey {
                Trigger::combo(h, Action::Toggle)?;
                c.triggers = vec![config::TriggerConfig {
                    keys: h.clone(),
                    ..Default::default()
                }];
            }
            if let Some(w) = window_rule {
                c.window_rule = w.clone();
            }
            if let Some(s) = sensitivity {
                c.mouse.sensitivity = *s;
            }
            if let Some(a) = accel {
                c.mouse.accel = *a;
            }
            c.engine_config()?;
            c.save(&ctx.config_path)?;
            println!("Gespeichert: {}", ctx.config_path.display());
        }
        ConfigCmd::SetPsk { key } => {
            let text = if key == "-" {
                let mut s = String::new();
                std::io::stdin()
                    .read_to_string(&mut s)
                    .map_err(|e| e.to_string())?;
                s
            } else {
                key.clone()
            };
            let k = parse_key_hex(&text)?;
            config::save_psk(&config::psk_path(), &k)?;
            println!("Schlüssel gespeichert: {}", config::psk_path().display());
        }
        ConfigCmd::GenPsk => {
            let mut k = [0u8; 32];
            std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut k))
                .map_err(|e| format!("/dev/urandom: {e}"))?;
            config::save_psk(&config::psk_path(), &k)?;
            let hex: String = k.iter().map(|b| format!("{b:02x}")).collect();
            println!("{hex}");
            eprintln!(
                "Gespeichert in {}. Derselbe Schlüssel muss auf dem Board eingetragen sein.",
                config::psk_path().display()
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(config::config_path);
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Fehler in den Einstellungen: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut ctx = Ctx { config_path, config };
    let res = match &cli.cmd {
        Command::ListDevices => cmd_list_devices(),
        Command::Run {
            board,
            devices,
            hotkey,
        } => cmd_run(&ctx, board, devices, hotkey.as_deref()),
        Command::Replay { board, file, hotkey } => cmd_replay(&ctx, board, file, hotkey.as_deref()),
        Command::Ping { board, count } => cmd_ping(&ctx, board, *count),
        Command::Send {
            board,
            keys,
            mods,
            buttons,
            dx,
            dy,
            hold_ms,
        } => cmd_send(&ctx, board, keys, mods, buttons, *dx, *dy, *hold_ms),
        Command::Config { cmd } => cmd_config(&mut ctx, cmd),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
