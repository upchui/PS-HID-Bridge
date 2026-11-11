//! Command line front end of the PS HID Bridge.

use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand, ValueEnum};
use pshb_core::config::{self, Config};
use pshb_core::controller::{Controller, Watchdog};
use pshb_core::crypto::parse_key_hex;
use pshb_core::engine::{Engine, Notice};
use pshb_core::input::{self, GrabGuard, InputMsg, InputSet, Waker};
use pshb_core::protocol::{self, Pong};
use pshb_core::rt;
use pshb_core::script;
use pshb_core::transport::{KeyState, Link, LinkConfig};
use pshb_core::trigger::{Action, Trigger};
use pshb_core::update::{self, Version};
use pshb_core::usbflash::{self, Image};

#[derive(Parser)]
#[command(name = "pshb-cli", version = pshb_core::VERSION, about = "PS HID Bridge – command line")]
struct Cli {
    /// Settings file (default: ~/.config/ps-hid-bridge/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Args, Clone, Default)]
struct BoardArgs {
    /// Board address, e.g. 192.168.1.50 or 192.168.1.50:45045
    #[arg(long)]
    board: Option<String>,
    /// Pre-shared key as 64 hex digits
    #[arg(long, conflicts_with = "psk_file")]
    psk: Option<String>,
    /// File with the pre-shared key (default: ~/.config/ps-hid-bridge/psk)
    #[arg(long)]
    psk_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// List input devices
    ListDevices,
    /// Forward mouse and keyboard to the board
    Run {
        #[command(flatten)]
        board: BoardArgs,
        /// Device (path, vvvv:pppp or part of the name); can be repeated
        #[arg(long = "device")]
        devices: Vec<String>,
        /// Key combination that toggles the bridge, e.g. "Ctrl+Alt+P"
        #[arg(long)]
        hotkey: Option<String>,
        /// Print the input latency (event -> packet sent) every 5 seconds
        #[arg(long)]
        stats: bool,
    },
    /// Play input from a script file (test without real devices)
    Replay {
        #[command(flatten)]
        board: BoardArgs,
        /// Script file (format: see README)
        file: PathBuf,
        #[arg(long)]
        hotkey: Option<String>,
    },
    /// Check the connection and show the board status
    Ping {
        #[command(flatten)]
        board: BoardArgs,
        #[arg(long, default_value_t = 5)]
        count: u32,
        /// How long to wait for the first answer
        #[arg(long, default_value_t = 2000)]
        timeout_ms: u64,
    },
    /// Send keys or mouse motion once (for testing)
    Send {
        #[command(flatten)]
        board: BoardArgs,
        /// HID key codes (hex), e.g. 04,1d
        #[arg(long, value_delimiter = ',')]
        keys: Vec<String>,
        /// Modifier byte (hex), e.g. 02 = left Shift
        #[arg(long, default_value = "0")]
        mods: String,
        /// Mouse button bits (hex)
        #[arg(long, default_value = "0")]
        buttons: String,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        dx: i32,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        dy: i32,
        /// Hold this long, then release everything
        #[arg(long, default_value_t = 100)]
        hold_ms: u64,
    },
    /// Show and change settings
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Install the firmware (or a recovery image) over USB: hold BOOTSEL
    /// while connecting the board to this PC
    Flash {
        /// What to install
        #[arg(long, value_enum, default_value_t = FlashImage::FirstInstall)]
        image: FlashImage,
        /// UF2 file instead of the newest release's (for first-install it
        /// replaces the firmware; the partition table comes from the release)
        #[arg(long)]
        file: Option<PathBuf>,
        /// Do not ask before erasing the board
        #[arg(long)]
        yes: bool,
    },
    /// Look for a newer release and install it: this program, or with
    /// --firmware the board's firmware (signed with the key)
    Update {
        #[command(flatten)]
        board: BoardArgs,
        /// Only show the installed and the newest versions
        #[arg(long)]
        check: bool,
        /// Update the board's firmware instead of this program
        #[arg(long)]
        firmware: bool,
        /// Firmware file to install instead of the newest release
        #[arg(long, requires = "firmware")]
        file: Option<PathBuf>,
        /// Port of the board's web interface
        #[arg(long, default_value_t = 80)]
        http_port: u16,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum FlashImage {
    /// Partition table, then the firmware
    FirstInstall,
    /// Firmware only
    Firmware,
    /// HID test image (no network, the mouse moves in a square)
    Hidtest,
    /// HID test image with 8-bit mouse axes
    HidtestMouse8,
    /// Erase the whole flash
    Erase,
}

impl FlashImage {
    fn image(self) -> Image {
        match self {
            FlashImage::FirstInstall => Image::FirstInstall,
            FlashImage::Firmware => Image::Firmware,
            FlashImage::Hidtest => Image::HidTest,
            FlashImage::HidtestMouse8 => Image::HidTestMouse8,
            FlashImage::Erase => Image::Erase,
        }
    }
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show the current settings
    Show,
    /// Change settings
    Set {
        #[arg(long)]
        board: Option<String>,
        /// Replace the device list (can be repeated)
        #[arg(long = "device")]
        devices: Vec<String>,
        /// Replace the toggle combination, e.g. "Ctrl+Alt+P"
        #[arg(long)]
        hotkey: Option<String>,
        /// Window rule on or off (desktop app, KDE Plasma): forward while the
        /// matching window has the focus
        #[arg(long, value_enum)]
        window_rule_enabled: Option<OnOff>,
        /// Window rule pattern: part of the window class, title or desktop file
        /// (default: "chiaki-ng")
        #[arg(long, value_name = "TEXT")]
        window_rule: Option<String>,
        /// Physical keyboard type: iso (e.g. German, UK) or ansi (e.g. US)
        #[arg(long, value_enum)]
        keyboard: Option<KeyboardType>,
        #[arg(long)]
        sensitivity: Option<f64>,
        #[arg(long)]
        accel: Option<f64>,
        /// Look for updates automatically (desktop app)
        #[arg(long, value_enum)]
        update_check: Option<OnOff>,
    },
    /// Store the pre-shared key ("-" reads it from standard input)
    SetPsk { key: String },
    /// Create, store and print a new random pre-shared key
    GenPsk,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum KeyboardType {
    /// ISO keyboard (105 keys, e.g. German, UK): extra key next to Enter
    Iso,
    /// ANSI keyboard (104 keys, e.g. US)
    Ansi,
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
        return Err("No board address: pass --board or run \"pshb-cli config set --board …\".".into());
    }
    let with_port = if board.contains(':') {
        board.to_string()
    } else {
        format!("{board}:{}", protocol::UDP_PORT_DEFAULT)
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("Cannot resolve address {board}: {e}"))?
        .next()
        .ok_or_else(|| format!("Cannot resolve address {board}"))
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
    start_link_rt(ctx, args, ping, false)
}

fn start_link_rt(ctx: &Ctx, args: &BoardArgs, ping: Duration, realtime: bool) -> Result<Link, String> {
    let mut cfg = LinkConfig::new(board_addr(ctx, args)?, load_key(args)?);
    cfg.ping_interval = ping;
    cfg.realtime = realtime;
    cfg.counter_store = Some(config::counter_path());
    Link::start(cfg).map_err(|e| format!("UDP: {e}"))
}

fn no_answer_reason(link: &Link) -> String {
    if link.status().bad_replies > 0 {
        "The board answers, but the answers are not validly signed: is the key correct?".into()
    } else {
        "No answer from the board (check address, port, key and sender filter).".into()
    }
}

fn connect(ctx: &Ctx, args: &BoardArgs, ping: Duration) -> Result<Link, String> {
    connect_within(ctx, args, ping, Duration::from_secs(2))
}

fn connect_within(ctx: &Ctx, args: &BoardArgs, ping: Duration, timeout: Duration) -> Result<Link, String> {
    let link = start_link(ctx, args, ping)?;
    if !link.wait_connected(timeout) {
        return Err(no_answer_reason(&link));
    }
    Ok(link)
}

fn parse_hex_u8(s: &str) -> Result<u8, String> {
    u8::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("not a hex number: {s}"))
}

fn describe(p: &Pong) -> String {
    let mut flags = vec![if p.status & protocol::STATUS_USB_MOUNTED != 0 {
        "USB connected"
    } else {
        "USB disconnected"
    }];
    if p.status & protocol::STATUS_USB_SUSPENDED != 0 {
        flags.push("USB suspended");
    }
    if p.status & protocol::STATUS_FAILSAFE != 0 {
        flags.push("failsafe active");
    }
    format!(
        "Firmware {}, up {:.1} s, session {:016x}, {}\n  Packets: ok {}, bad signature {}, replay {}, filtered {}, malformed {}",
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
        Notice::Activated => "Bridge ON – mouse and keyboard go to the PlayStation.",
        Notice::Deactivated => "Bridge OFF – mouse and keyboard belong to the desktop again.",
        Notice::WaitingForRelease => "Please release all keys …",
        Notice::Paused => "Paused.",
        Notice::Resumed => "Resumed.",
        Notice::Emergency => "Emergency stop: devices released.",
        Notice::ReleasedAll => "All keys released.",
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
    println!("{:<20} {:<14} {:<10} {:<8} Name", "Path", "Kind", "ID", "Access");
    for d in &list.devices {
        if d.keyboard || d.mouse {
            println!(
                "{:<20} {:<14} {:<10} {:<8} {}",
                d.path.display(),
                d.kind(),
                d.id_string(),
                if d.accessible { "yes" } else { "no" },
                d.name
            );
        }
    }
    if list.inaccessible > 0 {
        println!(
            "\n{} devices without read access. To set up access see the README, section \"Device access\".",
            list.inaccessible
        );
    }
    Ok(())
}

fn cmd_run(
    ctx: &Ctx,
    board: &BoardArgs,
    devices: &[String],
    hotkey: Option<&str>,
    stats: bool,
) -> Result<(), String> {
    let specs = if devices.is_empty() {
        ctx.config.devices.clone()
    } else {
        devices.to_vec()
    };
    if specs.is_empty() {
        return Err(
            "No devices selected: pass --device or run \"pshb-cli config set --device …\" (see list-devices)."
                .into(),
        );
    }
    let ecfg = engine_config(ctx, hotkey)?;
    let list = input::list_devices().map_err(|e| format!("/dev/input: {e}"))?;
    let selected = input::select(&list.devices, &specs);
    let blocked: Vec<_> = selected.iter().filter(|d| !d.accessible).collect();
    if !blocked.is_empty() {
        let names: Vec<_> = blocked.iter().map(|d| d.name.as_str()).collect();
        return Err(format!(
            "No read access to {names:?} (see the README, section \"Device access\")."
        ));
    }
    if selected.is_empty() {
        let mut msg = format!("No device matches {specs:?}.");
        if list.inaccessible > 0 {
            msg += &format!(
                " {} devices have no read access (see the README, section \"Device access\").",
                list.inaccessible
            );
        }
        return Err(msg);
    }
    for d in &selected {
        println!("Device: {} ({}, {})", d.name, d.kind(), d.path.display());
    }

    let link = Arc::new(start_link_rt(ctx, board, Duration::from_secs(1), true)?);
    if link.wait_connected(Duration::from_secs(2)) {
        if let Some(p) = link.status().board {
            println!("Board connected: {}", describe(&p));
        }
    } else {
        eprintln!("Warning: {} Still trying.", no_answer_reason(&link));
    }

    let waker = Arc::new(Waker::new().map_err(|e| e.to_string())?);
    let guard = GrabGuard::default();
    let mut inputs =
        InputSet::open(&selected, waker, guard.clone()).map_err(|e| format!("Opening device: {e}"))?;
    let watchdog = Watchdog::start(link.clone(), guard, Duration::from_secs(1));
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone());
    install_signal_handlers();
    let realtime = rt::make_current_thread_realtime(rt::PRIORITY);

    println!(
        "Ready. Toggle: {}. Emergency stop: Ctrl+Alt+Shift+Esc. Quit: Ctrl+C.",
        trigger_summary(ctx, hotkey)
    );
    if stats {
        match &realtime {
            Ok(how) => println!("Priority: real-time ({how})."),
            Err(e) => println!("Priority: normal (real-time {e})."),
        }
    }

    // Grab or release as the engine asks; a device that cannot be grabbed
    // switches the bridge off again.
    let apply_grab = |ctl: &mut Controller, inputs: &mut InputSet| {
        while let Some(on) = ctl.take_grab_request() {
            for f in inputs.set_grab(on) {
                if let InputMsg::GrabFailed { path, error } = f {
                    eprintln!(
                        "Cannot grab device {} exclusively ({error}). The bridge stays off.",
                        path.display()
                    );
                }
                for n in ctl.command(Action::Off) {
                    println!("{}", notice_text(n));
                }
            }
        }
    };

    let mut connected = link.is_connected();
    let mut result = Ok(());
    let mut msgs = Vec::new();
    let mut next_stats = Instant::now() + Duration::from_secs(5);
    let mut stats_total = 0;
    while !STOP.load(Ordering::Acquire) {
        watchdog.feed();
        inputs.wait(Duration::from_millis(100), &mut msgs);
        for m in msgs.drain(..) {
            match m {
                InputMsg::Event { event, ts_us, .. } => {
                    for n in ctl.handle(event, Instant::now(), ts_us) {
                        println!("{}", notice_text(n));
                    }
                    apply_grab(&mut ctl, &mut inputs);
                }
                // Its keys were released just before (InputSet sends key-ups).
                InputMsg::Gone { path, error } => {
                    eprintln!("Device {} disconnected: {error}", path.display());
                }
                InputMsg::GrabFailed { .. } | InputMsg::OpenFailed { .. } | InputMsg::Hotplug => {}
            }
        }
        if inputs.is_empty() {
            result = Err("All devices disconnected.".into());
            break;
        }
        if watchdog.take_fired() {
            eprintln!("Watchdog: the main loop hung, devices were released.");
            for n in ctl.command(Action::Off) {
                println!("{}", notice_text(n));
            }
            apply_grab(&mut ctl, &mut inputs);
        }
        let now_connected = link.is_connected();
        if now_connected != connected {
            connected = now_connected;
            println!(
                "{}",
                if connected {
                    "Board reachable again."
                } else {
                    "Board not reachable – input is lost, still trying to connect."
                }
            );
        }
        if stats && Instant::now() >= next_stats {
            next_stats += Duration::from_secs(5);
            if let Some(l) = link.input_latency().filter(|l| l.total != stats_total) {
                stats_total = l.total;
                println!(
                    "Latency input -> network: median {} µs, 95 % below {} µs, 99 % below {} µs, max {} µs ({} packets)",
                    l.p50_us, l.p95_us, l.p99_us, l.max_us, l.samples
                );
            }
        }
    }

    ctl.shutdown();
    apply_grab(&mut ctl, &mut inputs);
    drop(watchdog);
    drop(inputs);
    thread::sleep(Duration::from_millis(30));
    println!("Stopped.");
    result
}

fn cmd_replay(ctx: &Ctx, board: &BoardArgs, file: &Path, hotkey: Option<&str>) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let steps = script::parse(&text)?;
    let ecfg = engine_config(ctx, hotkey)?;
    let link = Arc::new(connect(ctx, board, Duration::from_secs(1))?);
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone());
    for step in steps {
        thread::sleep(step.delay);
        for n in ctl.handle(step.event, Instant::now(), None) {
            println!("{}", notice_text(n));
        }
    }
    thread::sleep(Duration::from_millis(50));
    ctl.shutdown();
    thread::sleep(Duration::from_millis(30));
    Ok(())
}

fn cmd_ping(ctx: &Ctx, board: &BoardArgs, count: u32, timeout: Duration) -> Result<(), String> {
    let link = connect_within(ctx, board, Duration::from_millis(200), timeout)?;
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
                println!("Answer: round trip {:.2} ms", rtt.as_secs_f64() * 1000.0);
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
        return Err("At most 6 keys.".into());
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
    link.set_state(state, None);
    link.add_motion(dx, dy, 0, 0, None);
    thread::sleep(Duration::from_millis(hold_ms));
    link.set_active(false);
    thread::sleep(Duration::from_millis(20));
    Ok(())
}

fn cmd_flash(image: Image, file: Option<&Path>, yes: bool) -> Result<(), String> {
    if image == Image::Erase && !yes {
        eprint!("This erases the board's firmware and settings. Type \"erase\" to continue: ");
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|e| e.to_string())?;
        if answer.trim() != "erase" {
            return Err("Nothing was erased.".into());
        }
    }
    let own = match file {
        Some(f) => {
            let data = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "firmware.uf2".into());
            Some((name, data))
        }
        None => None,
    };
    println!("{}", image.description());
    let mut say = |msg: &str| println!("{msg}");
    let files = usbflash::files_for(image, own, &mut say)?;
    let cancel = AtomicBool::new(false);
    usbflash::flash(&files, &mut say, &cancel)?;
    println!("{}", image.done_message());
    Ok(())
}

/// Download progress on standard error, in steps of 10 %.
fn progress_printer(what: &str) -> impl FnMut(u64, u64) + '_ {
    let mut shown = 0;
    move |done, total| {
        let pct = (done * 100).checked_div(total).unwrap_or(0).min(100);
        if pct >= shown + 10 || (pct == 100 && shown < 100) {
            shown = pct - pct % 10;
            eprint!("\r{what}: {pct} %");
            if pct == 100 {
                eprintln!();
            }
        }
    }
}

/// Firmware version the board reports (a short ping).
fn board_firmware(ctx: &Ctx, board: &BoardArgs) -> Result<Version, String> {
    let link = connect_within(ctx, board, Duration::from_millis(200), Duration::from_secs(3))?;
    link.status()
        .board
        .map(|p| Version::of_firmware(p.fw_version))
        .ok_or_else(|| "No answer from the board.".into())
}

fn cmd_update(
    ctx: &Ctx,
    board: &BoardArgs,
    check: bool,
    firmware: bool,
    file: Option<&Path>,
    http_port: u16,
) -> Result<(), String> {
    if firmware {
        return update_firmware(ctx, board, file, http_port);
    }
    let latest = update::latest_release()?;
    let this = Version::of_this_program();
    if check {
        println!("Newest release: {} ({})", latest.version, latest.page);
        match this {
            Some(v) if latest.version > v => println!("pshb-cli {v}: update available (pshb-cli update)"),
            Some(v) => println!("pshb-cli {v}: up to date"),
            None => println!("pshb-cli {}: development build", pshb_core::VERSION),
        }
        if board_addr(ctx, board).is_ok() {
            match board_firmware(ctx, board) {
                Ok(v) if latest.version > v => {
                    println!("Board firmware {v}: update available (pshb-cli update --firmware)")
                }
                Ok(v) => println!("Board firmware {v}: up to date"),
                Err(e) => println!("Board firmware: unknown ({e})"),
            }
        }
        return Ok(());
    }
    let Some(v) = this else {
        return Err("This is a development build; it is not replaced by a release.".into());
    };
    if latest.version <= v {
        println!("pshb-cli {v} is up to date.");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("Own path: {e}"))?;
    let data = update::download_verified(&latest, update::CLI_ASSET, &mut progress_printer("Download"))?;
    update::replace_file(&exe, &data).map_err(|e| format!("{}: {e}", exe.display()))?;
    println!("Updated {} from {v} to {}.", exe.display(), latest.version);
    Ok(())
}

/// Install firmware on the board (newest release or a file), then wait until
/// it runs the new version.
fn update_firmware(ctx: &Ctx, board: &BoardArgs, file: Option<&Path>, http_port: u16) -> Result<(), String> {
    let addr = board_addr(ctx, board)?;
    let key = load_key(board)?;
    let before = board_firmware(ctx, board).ok();
    let (image, expected) = match file {
        Some(f) => (
            std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?,
            None,
        ),
        None => {
            let latest = update::latest_release()?;
            if let Some(b) = before
                && latest.version <= b
            {
                println!("Board firmware {b} is up to date.");
                return Ok(());
            }
            let data = update::download_verified(
                &latest,
                update::FIRMWARE_ASSET,
                &mut progress_printer("Download"),
            )?;
            (data, Some(latest.version))
        }
    };
    println!("Installing on the board ({} KiB) …", image.len() / 1024);
    update::board_ota(&addr.ip().to_string(), http_port, &key, &image)?;
    println!("Written and checked; the board restarts into the new firmware …");
    let deadline = Instant::now() + Duration::from_secs(60);
    thread::sleep(Duration::from_secs(2));
    loop {
        let late = Instant::now() > deadline;
        match board_firmware(ctx, board) {
            Ok(v) if expected.is_none_or(|e| v == e) => {
                println!("The board runs firmware {v}.");
                return Ok(());
            }
            Ok(v) if late => {
                return Err(format!(
                    "The board runs {v}: it went back to the previous version."
                ));
            }
            Err(e) if late => return Err(format!("The board did not come back: {e}")),
            _ => thread::sleep(Duration::from_secs(1)),
        }
    }
}

fn cmd_config(ctx: &mut Ctx, cmd: &ConfigCmd) -> Result<(), String> {
    match cmd {
        ConfigCmd::Show => {
            println!("# {}", ctx.config_path.display());
            print!(
                "{}",
                toml::to_string_pretty(&ctx.config).map_err(|e| e.to_string())?
            );
            let c = &ctx.config;
            println!("# Keyboard type: {}", if c.iso_keyboard { "ISO" } else { "ANSI" });
            println!(
                "# Window rule: {} (pattern \"{}\")",
                if c.window_rule_active() { "on" } else { "off" },
                c.window_rule
            );
            let psk = config::psk_path();
            println!(
                "# Key: {} ({})",
                psk.display(),
                if psk.exists() { "present" } else { "missing" }
            );
        }
        ConfigCmd::Set {
            board,
            devices,
            hotkey,
            window_rule_enabled,
            window_rule,
            keyboard,
            sensitivity,
            accel,
            update_check,
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
            if let Some(on) = window_rule_enabled {
                c.window_rule_enabled = matches!(on, OnOff::On);
            }
            if let Some(w) = window_rule {
                c.window_rule = w.trim().to_string();
            }
            if let Some(k) = keyboard {
                c.iso_keyboard = matches!(k, KeyboardType::Iso);
            }
            if let Some(s) = sensitivity {
                c.mouse.sensitivity = *s;
            }
            if let Some(a) = accel {
                c.mouse.accel = *a;
            }
            if let Some(on) = update_check {
                c.update_check = matches!(on, OnOff::On);
            }
            c.engine_config()?;
            c.save(&ctx.config_path)?;
            println!("Saved: {}", ctx.config_path.display());
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
            println!("Key saved: {}", config::psk_path().display());
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
                "Saved to {}. The same key must be set on the board.",
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
            eprintln!("Error in the settings: {e}");
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
            stats,
        } => cmd_run(&ctx, board, devices, hotkey.as_deref(), *stats),
        Command::Replay { board, file, hotkey } => cmd_replay(&ctx, board, file, hotkey.as_deref()),
        Command::Ping {
            board,
            count,
            timeout_ms,
        } => cmd_ping(&ctx, board, *count, Duration::from_millis(*timeout_ms)),
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
        Command::Update {
            board,
            check,
            firmware,
            file,
            http_port,
        } => cmd_update(&ctx, board, *check, *firmware, file.as_deref(), *http_port),
        Command::Flash { image, file, yes } => cmd_flash(image.image(), file.as_deref(), *yes),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
