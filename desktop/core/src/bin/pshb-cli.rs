//! Command line front end of the PS5 HID Bridge.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use pshb_core::crypto::parse_key_hex;
use pshb_core::protocol::{self, Pong};
use pshb_core::transport::{KeyState, Link, LinkConfig};

#[derive(Parser)]
#[command(name = "pshb-cli", version, about = "PS5 HID Bridge – Kommandozeile")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Args, Clone)]
struct BoardArgs {
    /// Adresse des Boards, z. B. 192.168.1.50 oder 192.168.1.50:45045
    #[arg(long)]
    board: String,
    /// Pre-Shared Key als 64 Hex-Zeichen
    #[arg(long, conflicts_with = "psk_file")]
    psk: Option<String>,
    /// Datei mit dem Pre-Shared Key
    #[arg(long)]
    psk_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Verbindung prüfen und Board-Status anzeigen
    Ping {
        #[command(flatten)]
        board: BoardArgs,
        /// Anzahl der Pings
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
}

fn resolve(board: &str) -> Result<SocketAddr, String> {
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

fn load_key(args: &BoardArgs) -> Result<[u8; 32], String> {
    let text = match (&args.psk, &args.psk_file) {
        (Some(k), _) => k.clone(),
        (None, Some(path)) => {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?
        }
        (None, None) => return Err("Bitte --psk oder --psk-file angeben.".into()),
    };
    parse_key_hex(&text)
}

fn connect(args: &BoardArgs, ping_interval: Duration) -> Result<Link, String> {
    let mut cfg = LinkConfig::new(resolve(&args.board)?, load_key(args)?);
    cfg.ping_interval = ping_interval;
    let link = Link::start(cfg).map_err(|e| format!("UDP: {e}"))?;
    if !link.wait_connected(Duration::from_secs(2)) {
        let bad = link.status().bad_replies;
        return Err(if bad > 0 {
            "Das Board antwortet, aber die Antworten sind nicht gültig signiert: Stimmt der Schlüssel?".into()
        } else {
            "Keine Antwort vom Board (Adresse, Port, Schlüssel und Absenderfilter prüfen).".into()
        });
    }
    Ok(link)
}

fn parse_hex_u8(s: &str) -> Result<u8, String> {
    u8::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("keine Hex-Zahl: {s}"))
}

fn describe(p: &Pong) -> String {
    let mut flags = Vec::new();
    if p.status & protocol::STATUS_USB_MOUNTED != 0 {
        flags.push("USB verbunden");
    } else {
        flags.push("USB getrennt");
    }
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

fn cmd_ping(board: &BoardArgs, count: u32) -> Result<(), String> {
    let link = connect(board, Duration::from_millis(200))?;
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
    let link = connect(board, Duration::from_secs(1))?;
    link.set_active(true);
    link.set_state(state);
    link.add_motion(dx, dy, 0, 0);
    thread::sleep(Duration::from_millis(hold_ms));
    link.set_active(false);
    thread::sleep(Duration::from_millis(20));
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let res = match &cli.cmd {
        Command::Ping { board, count } => cmd_ping(board, *count),
        Command::Send {
            board,
            keys,
            mods,
            buttons,
            dx,
            dy,
            hold_ms,
        } => cmd_send(board, keys, mods, buttons, *dx, *dy, *hold_ms),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
