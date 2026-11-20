//! Command line front end of the PS HID Bridge.

use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// After a device came or went, look at the device list again this much
/// later (like the service: udev has set up the new node by then).
const HOTPLUG_SETTLE: Duration = Duration::from_millis(200);

use clap::{Args, Parser, Subcommand, ValueEnum};
use pshb_core::config::{self, Config};
use pshb_core::controller::{Controller, Watchdog};
use pshb_core::crypto::parse_key_hex;
use pshb_core::discover::{self, FoundBoard};
use pshb_core::engine::{Engine, Notice};
use pshb_core::follow::{self, FollowConfig, NetFinder};
use pshb_core::idle::IdleInhibitor;
use pshb_core::input::{self, GrabGuard, InputMsg, InputSet, Waker};
use pshb_core::lowlat::{self, AdapterTimer, CpuWake, CpuWakeState};
use pshb_core::protocol::{self, Pong};
use pshb_core::rt;
use pshb_core::script;
use pshb_core::transport::{KeyState, Link, LinkConfig};
use pshb_core::trigger::{Action, Trigger};
use pshb_core::typing;
use pshb_core::update::{self, Version};
use pshb_core::usbflash::{self, Image};
use pshb_core::window;

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

/// Finding the board again when it moves (DHCP) while `run` or `replay` sends.
#[derive(Args, Clone, Default)]
struct FollowArgs {
    /// Where to look for the board when it stops answering (discovery
    /// requests; default: every network of this PC); can be repeated
    #[arg(long = "discovery-host")]
    discovery_hosts: Vec<std::net::Ipv4Addr>,
    /// Discovery port of the board
    #[arg(long, default_value_t = protocol::DISCOVERY_PORT)]
    discovery_port: u16,
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
        #[command(flatten)]
        follow: FollowArgs,
    },
    /// Play input from a script file (test without real devices)
    Replay {
        #[command(flatten)]
        board: BoardArgs,
        /// Script file (format: see docs/advanced-setup.md)
        file: PathBuf,
        #[arg(long)]
        hotkey: Option<String>,
        #[command(flatten)]
        follow: FollowArgs,
    },
    /// Find boards in the network (no key needed)
    Discover {
        /// Ask only this address instead of the whole network
        #[arg(long)]
        host: Option<std::net::Ipv4Addr>,
        /// Discovery port
        #[arg(long, default_value_t = protocol::DISCOVERY_PORT)]
        port: u16,
        /// How long to wait for answers
        #[arg(long, default_value_t = 1200)]
        timeout_ms: u64,
        /// Print the boards as JSON
        #[arg(long)]
        json: bool,
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
    /// Type text on the PlayStation (the bridge must be off in the app)
    Type {
        #[command(flatten)]
        board: BoardArgs,
        /// The text; "-" reads it from standard input
        text: String,
        /// Keyboard language set on the PlayStation: us, uk or de (default:
        /// from the settings, else from the system language)
        #[arg(long)]
        layout: Option<String>,
    },
    /// Take a new key from the board with its web password and store it
    /// (replaces the key on the board: other PCs need the new one)
    Pair {
        /// Board address (default: from the settings)
        #[arg(long)]
        board: Option<String>,
        /// The board's web password; "-" reads it from standard input
        #[arg(long)]
        password: String,
        /// New web password, needed while the board has its factory password
        #[arg(long)]
        new_password: Option<String>,
        /// Port of the board's web interface
        #[arg(long, default_value_t = 80)]
        http_port: u16,
    },
    /// Write a report for troubleshooting (versions, system, settings
    /// without the key, devices, board status, recent messages)
    Diag {
        #[command(flatten)]
        board: BoardArgs,
        /// Write it to this file instead of standard output
        #[arg(long)]
        output: Option<PathBuf>,
        /// Port of the board's web interface
        #[arg(long, default_value_t = 80)]
        http_port: u16,
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
    /// --firmware the board's firmware (signed with the key); with
    /// --version a particular release, also an older one
    Update {
        #[command(flatten)]
        board: BoardArgs,
        /// Only show the installed and the newest versions
        #[arg(long, conflicts_with = "to_version")]
        check: bool,
        /// Update the board's firmware instead of this program
        #[arg(long)]
        firmware: bool,
        /// Firmware file to install instead of the newest release
        #[arg(long, requires = "firmware", conflicts_with = "to_version")]
        file: Option<PathBuf>,
        /// Install this release instead of the newest, e.g. 1.0.89 (also an
        /// older one: app from 1.0.32, firmware from 1.0.64)
        #[arg(long = "version", value_name = "VERSION")]
        to_version: Option<String>,
        /// Port of the board's web interface
        #[arg(long, default_value_t = 80)]
        http_port: u16,
    },
    /// The board's latency diagnostics, status and clock speeds (signed
    /// with the key); with --cpu/--spi it changes the clock (the board
    /// restarts)
    Board {
        #[command(flatten)]
        board: BoardArgs,
        /// Set the CPU clock: 150 (rated) or 200 MHz (overclocked)
        #[arg(long, value_parser = ["150", "200"])]
        cpu: Option<String>,
        /// Set the SPI clock to the network chip
        #[arg(long, value_enum)]
        spi: Option<SpiClock>,
        /// Start the latency statistics over
        #[arg(long)]
        reset: bool,
        /// Give the board a name (shown in the app and the search; its
        /// network name in the router follows from it); "" = the default name
        #[arg(long)]
        name: Option<String>,
        /// Print the board's answers as JSON
        #[arg(long)]
        json: bool,
        /// Port of the board's web interface
        #[arg(long, default_value_t = 80)]
        http_port: u16,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum SpiClock {
    Standard,
    Fast,
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

// (Parsed once per run: the size of `Set` does not matter.)
#[allow(clippy::large_enum_variant)]
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
        /// Replace all triggers with one toggle combination, e.g. "Ctrl+Alt+P"
        #[arg(long)]
        hotkey: Option<String>,
        /// Window rule on or off: forward while the matching window has the
        /// focus (KDE Plasma, Hyprland, Sway, i3, niri, X11)
        #[arg(long, value_enum)]
        window_rule_enabled: Option<OnOff>,
        /// Window rule pattern: part of the window class, title or desktop file
        /// (default: "chiaki-ng")
        #[arg(long, value_name = "TEXT")]
        window_rule: Option<String>,
        /// Physical keyboard type: iso (e.g. German, UK) or ansi (e.g. US)
        #[arg(long, value_enum)]
        keyboard: Option<KeyboardType>,
        /// The profile whose mouse settings the options below change
        /// (default: "Standard")
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,
        /// The profile in use (chosen by hand)
        #[arg(long, value_name = "NAME")]
        active_profile: Option<String>,
        #[arg(long)]
        sensitivity: Option<f64>,
        #[arg(long)]
        accel: Option<f64>,
        /// Up and down reversed
        #[arg(long, value_enum)]
        invert_y: Option<OnOff>,
        /// Vertical sensitivity relative to the horizontal one (1.0 = the same)
        #[arg(long, value_name = "FACTOR")]
        y_ratio: Option<f64>,
        /// Factor on the wheel's notches (0.5 = one step per two notches)
        #[arg(long, value_name = "FACTOR")]
        wheel_speed: Option<f64>,
        /// Touchpads and touchscreens: factor on the motion (1.0 = as designed)
        #[arg(long, value_name = "FACTOR")]
        touchpad_speed: Option<f64>,
        /// Touchpads and touchscreens: a short tap clicks
        #[arg(long, value_enum)]
        tap_to_click: Option<OnOff>,
        /// Touchpads and touchscreens: two-finger scrolling moves the content with the fingers
        #[arg(long, value_enum)]
        natural_scroll: Option<OnOff>,
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
    config::board_socket_addr(board).map_err(|e| format!("Cannot resolve address {board}: {e}"))
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
    start_link_rt(ctx, args, ping, false, None)
}

/// With `follow`: the link finds the board again when it moves (its notice,
/// its MAC address, its name; see pshb_core::follow).
fn start_link_rt(
    ctx: &Ctx,
    args: &BoardArgs,
    ping: Duration,
    realtime: bool,
    follow: Option<&FollowArgs>,
) -> Result<Link, String> {
    let mut cfg = LinkConfig::new(board_addr(ctx, args)?, load_key(args)?);
    cfg.ping_interval = ping;
    cfg.realtime = realtime;
    cfg.counter_store = Some(config::counter_path());
    if let Some(f) = follow {
        let board = args.board.as_deref().unwrap_or(&ctx.config.board);
        // The stored MAC address belongs to the stored board.
        let mac = (board == ctx.config.board && !ctx.config.board_mac.is_empty())
            .then(|| ctx.config.board_mac.clone());
        let mut fc = FollowConfig::new(follow::host_name(board), mac);
        fc.finder = Arc::new(NetFinder {
            targets: (!f.discovery_hosts.is_empty()).then(|| f.discovery_hosts.clone()),
            port: f.discovery_port,
        });
        cfg.follow = Some(fc);
    }
    Link::start(cfg).map_err(|e| format!("UDP: {e}"))
}

/// Say so when the board moves while the link lives.
fn report_moves(link: &Arc<Link>) {
    let link = Arc::downgrade(link);
    thread::spawn(move || {
        let mut seen = 0;
        while let Some(l) = link.upgrade() {
            if let Some(m) = l.status().last_move.filter(|m| m.seq != seen) {
                seen = m.seq;
                println!("Board moved to {} ({})", m.to, m.reason.as_str());
            }
            drop(l);
            thread::sleep(Duration::from_millis(50));
        }
    });
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
        Notice::ProfileChanged => "Profile changed.",
    }
}

/// A notice for the terminal; a profile change names the profile.
fn notice_line(ctl: &Controller, n: Notice) -> String {
    match n {
        Notice::ProfileChanged => {
            let e = &ctl.engine;
            let name = e
                .profiles()
                .get(e.profile())
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            format!("Profile: {name}.")
        }
        _ => notice_text(n).to_string(),
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
        if d.keyboard || d.pointer() {
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
    follow: &FollowArgs,
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

    let link = Arc::new(start_link_rt(
        ctx,
        board,
        Duration::from_secs(1),
        true,
        Some(follow),
    )?);
    report_moves(&link);
    // Switches the bundling of a USB network adapter towards the board off
    // while this runs (if set up, see the README); puts back what a crashed
    // run left switched.
    let mut adapter = AdapterTimer::new(Some(config::adapter_state_path()));
    let adapter_info = board_addr(ctx, board).ok().and_then(lowlat::adapter_info);
    if let Some(d) = adapter_info.as_ref().filter(|d| d.timer_us > 0 && !d.writable) {
        eprintln!(
            "Note: the network adapter {iface} holds every packet back up to about {ms:.1} ms before \
             sending it (USB adapter, cdc_ncm). Set up once (as root) so that the app may switch \
             that off while it runs: the app's Latency page (\"Set up\"), or by hand, see \
             docs/latency-pc.md (section \"USB network adapters\").",
            iface = d.iface,
            ms = 3.0 * d.timer_us as f64 / 1000.0,
        );
    }
    adapter.set_adapter(adapter_info);
    // From here on Ctrl+C must end the run in order (the adapter is put
    // back on the way out), also during the setup below.
    install_signal_handlers();
    adapter.set_active(true);
    if link.wait_connected(Duration::from_secs(2)) {
        if let Some(p) = link.status().board {
            println!("Board connected: {}", describe(&p));
        }
    } else {
        eprintln!("Warning: {} Still trying.", no_answer_reason(&link));
    }

    let waker = Arc::new(Waker::new().map_err(|e| e.to_string())?);
    let guard = GrabGuard::default();
    let mut inputs = InputSet::open(&selected, waker.clone(), guard.clone())
        .map_err(|e| format!("Opening device: {e}"))?;
    inputs.set_touch_settings(ctx.config.touchpad.settings());
    // Every other keyboard is only watched: triggers and the emergency stop
    // work there too, nothing of it is forwarded.
    let watch = input::watch_keyboards(&list.devices, &selected);
    let mut opened = Vec::new();
    inputs.update(&selected, &watch, &mut opened);
    for d in &watch {
        println!("Triggers also on: {} ({})", d.name, d.path.display());
    }
    let unwatched = input::unwatched_keyboards(&list.devices, &selected);
    if unwatched > 0 {
        println!(
            "Note: {unwatched} other keyboards may not be read; triggers do not work on them (see the README, section \"Device access\")."
        );
    }
    let grabbed = guard.clone();
    let watchdog = Watchdog::start(link.clone(), guard, Duration::from_secs(1));
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone());
    let realtime = rt::make_current_thread_realtime(rt::PRIORITY);

    // The focused window, for the window rule and profiles that follow one.
    let (window_tx, window_rx) = std::sync::mpsc::channel();
    let wants_window =
        ctx.config.window_rule_active() || ctx.config.profiles.iter().any(|p| !p.window.trim().is_empty());
    let _window = if wants_window {
        let w = waker.clone();
        match window::detect().and_then(|b| {
            window::Watcher::start(b, move |info| {
                let _ = window_tx.send(info);
                w.wake();
            })
        }) {
            Ok(watcher) => {
                println!("Windows: from {}.", watcher.backend().name());
                Some(watcher)
            }
            Err(e) => {
                eprintln!("Window rule and window profiles not available: {e}.");
                None
            }
        }
    } else {
        None
    };

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
    // switches the bridge off again. While grabbed, the screen stays on
    // (the desktop sees no input then and would turn it off and lock it)
    // and the CPU out of deep sleep states (faster reaction to input).
    let mut idle = IdleInhibitor::new();
    let mut cpu = CpuWake::new(ctx.config.keep_cpu_awake);
    if stats {
        println!(
            "CPU sleep states: {}",
            match cpu.state() {
                CpuWakeState::Disabled => "left alone (keep_cpu_awake = false)",
                CpuWakeState::Ready | CpuWakeState::Held => "kept light while the bridge is on",
                CpuWakeState::NotPermitted => "may not be limited (no write access to /dev/cpu_dma_latency)",
                CpuWakeState::Unavailable => "cannot be limited on this system",
            }
        );
        if let Some(a) = adapter.status() {
            println!(
                "Network adapter {}: {}",
                a.iface,
                if a.managed {
                    "sends at once while this runs"
                } else if a.at_once_now {
                    "sends at once (set outside the app)"
                } else {
                    "bundles packets (not set up)"
                }
            );
        }
    }
    let apply_grab =
        |ctl: &mut Controller, inputs: &mut InputSet, idle: &mut IdleInhibitor, cpu: &mut CpuWake| {
            let forgotten = ctl.take_forgotten();
            if !forgotten.is_empty() {
                inputs.forget(&forgotten);
            }
            while let Some(on) = ctl.take_grab_request() {
                let problems = inputs.set_grab(on);
                if let Some(text) = input::grab_problems(&problems) {
                    eprintln!("{text}");
                }
                if problems.iter().any(|m| matches!(m, InputMsg::GrabFailed { .. })) {
                    for n in ctl.command(Action::Off) {
                        println!("{}", notice_line(ctl, n));
                    }
                }
            }
            idle.set(grabbed.is_grabbed());
            cpu.set_active(grabbed.is_grabbed());
        };

    let mut connected = link.is_connected();
    let mut result = Ok(());
    let mut msgs = Vec::new();
    let mut next_stats = Instant::now() + Duration::from_secs(5);
    let mut stats_total = 0;
    let mut resyncs = 0;
    // A device came or went: look at the device list again shortly after
    // (like the app's service), so one that comes back (a keyboard waking
    // from Bluetooth sleep) is grabbed and watched again.
    let mut rescan_at: Option<Instant> = None;
    while !STOP.load(Ordering::Acquire) {
        watchdog.feed();
        inputs.wait(Duration::from_millis(100), &mut msgs);
        while let Ok(info) = window_rx.try_recv() {
            for n in ctl.window(info.as_ref()) {
                println!("{}", notice_line(&ctl, n));
            }
            apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
        }
        let mut grab_failed = Vec::new();
        for m in std::mem::take(&mut msgs) {
            match m {
                InputMsg::Event { event, ts_us, .. } => {
                    for n in ctl.handle(event, Instant::now(), ts_us) {
                        println!("{}", notice_line(&ctl, n));
                    }
                    apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
                }
                // Its keys were released just before (InputSet sends key-ups).
                InputMsg::Gone { path, error } => {
                    eprintln!("Device {} disconnected: {error}", path.display());
                    rescan_at = Some(Instant::now() + HOTPLUG_SETTLE);
                }
                InputMsg::Hotplug => {
                    rescan_at.get_or_insert(Instant::now() + HOTPLUG_SETTLE);
                }
                InputMsg::OpenFailed { path, error } => {
                    eprintln!("Opening {}: {error}", path.display());
                }
                // A device that came back could not be grabbed (another
                // program holds it, e.g. input-remapper or keyd).
                m @ InputMsg::GrabFailed { .. } => grab_failed.push(m),
                InputMsg::ReleaseFailed { .. } => {}
            }
        }
        // As when switching on: the bridge goes off, or that device's keys
        // would reach the PlayStation and the desktop at once.
        if !grab_failed.is_empty() {
            if let Some(text) = input::grab_problems(&grab_failed) {
                eprintln!("{text}");
            }
            for n in ctl.command(Action::Off) {
                println!("{}", notice_line(&ctl, n));
            }
            apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
        }
        if rescan_at.is_some_and(|t| Instant::now() >= t) {
            rescan_at = None;
            if let Ok(list) = input::list_devices() {
                let selected = input::select(&list.devices, &specs);
                let watch = input::watch_keyboards(&list.devices, &selected);
                let before = inputs.len();
                // Key-ups of closed devices and failures come back as
                // messages: handled with the next batch.
                inputs.update(&selected, &watch, &mut msgs);
                if inputs.len() > before {
                    println!("Device back: {} open.", inputs.len());
                }
                apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
            }
        }
        if inputs.resyncs() != resyncs {
            resyncs = inputs.resyncs();
            eprintln!(
                "Input events were lost (reader fell behind) and corrected from the key state ({resyncs} so far)."
            );
        }
        if inputs.is_empty() && rescan_at.is_none() {
            result = Err("All devices disconnected.".into());
            break;
        }
        if watchdog.take_fired() {
            eprintln!("Watchdog: the main loop hung, devices were released.");
            for n in ctl.command(Action::Off) {
                println!("{}", notice_line(&ctl, n));
            }
            apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
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
            let st = link.status();
            if let Some(o) = &st.oneway {
                println!(
                    "PC -> board: median {:.0} µs, 95 % below {:.0} µs (round trip {:.0} µs)",
                    o.p50_us,
                    o.p95_us,
                    st.rtt.map_or(0.0, |r| r.as_secs_f64() * 1e6)
                );
            }
        }
    }

    ctl.shutdown();
    apply_grab(&mut ctl, &mut inputs, &mut idle, &mut cpu);
    adapter.set_active(false);
    drop(watchdog);
    drop(inputs);
    thread::sleep(Duration::from_millis(30));
    println!("Stopped.");
    result
}

fn cmd_replay(
    ctx: &Ctx,
    board: &BoardArgs,
    file: &Path,
    hotkey: Option<&str>,
    follow: &FollowArgs,
) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let steps = script::parse(&text)?;
    let ecfg = engine_config(ctx, hotkey)?;
    let link = Arc::new(start_link_rt(
        ctx,
        board,
        Duration::from_secs(1),
        false,
        Some(follow),
    )?);
    if !link.wait_connected(Duration::from_secs(2)) {
        return Err(no_answer_reason(&link));
    }
    report_moves(&link);
    let mut ctl = Controller::new(Engine::new(ecfg), link.clone());
    for step in steps {
        thread::sleep(step.delay);
        for n in ctl.handle(step.event, Instant::now(), None) {
            println!("{}", notice_line(&ctl, n));
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
                println!("Answer: round trip {:.0} µs", rtt.as_secs_f64() * 1e6);
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

fn read_arg_or_stdin(value: &str) -> Result<String, String> {
    if value != "-" {
        return Ok(value.to_string());
    }
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    Ok(s.strip_suffix('\n').unwrap_or(&s).to_string())
}

fn cmd_type(ctx: &Ctx, board: &BoardArgs, text: &str, layout: Option<&str>) -> Result<(), String> {
    let text = read_arg_or_stdin(text)?;
    let layout = match layout {
        Some(id) => {
            typing::layout(id).ok_or_else(|| format!("Unknown keyboard language \"{id}\" (us, uk, de)"))?
        }
        None => typing::layout_or_default(&ctx.config.ps_layout),
    };
    let states = typing::plan(&text, layout).map_err(|missing| {
        format!(
            "The keyboard language \"{}\" has no key for: {}",
            layout.id,
            missing.iter().collect::<String>()
        )
    })?;
    let link = connect(ctx, board, Duration::from_secs(1))?;
    let st = link.status().board;
    if !st.is_some_and(|p| {
        p.status & protocol::STATUS_USB_MOUNTED != 0 && p.status & protocol::STATUS_USB_SUSPENDED == 0
    }) {
        return Err("The PlayStation is off, asleep or not connected to the board.".into());
    }
    link.set_active(true);
    for s in states {
        link.set_state(s, None);
        thread::sleep(typing::EDGE);
    }
    link.set_active(false);
    thread::sleep(Duration::from_millis(30));
    println!("Typed {} characters ({}).", text.chars().count(), layout.name);
    Ok(())
}

fn cmd_pair(
    ctx: &Ctx,
    board: Option<&str>,
    password: &str,
    new_password: Option<&str>,
    http_port: u16,
) -> Result<(), String> {
    let address = board.unwrap_or(&ctx.config.board).to_string();
    let host = resolve(&address)?.ip().to_string();
    let password = read_arg_or_stdin(password)?;
    let key = update::pair(&host, http_port, &password, new_password).map_err(|e| match e {
        update::PairError::WrongPassword => "Wrong password.".to_string(),
        update::PairError::LockedOut(m) => format!("Too many wrong passwords: {m}"),
        update::PairError::NeedsNewPassword => {
            "The board still has its factory password: pass a new one with --new-password.".to_string()
        }
        update::PairError::NewPasswordRefused(m) => format!("New password refused: {m}"),
        update::PairError::Other(m) => m,
    })?;
    config::save_psk(&config::psk_path(), &key)?;
    if board.is_some() {
        let mut c = ctx.config.clone();
        c.board = address.clone();
        c.save(&ctx.config_path)?;
    }
    println!(
        "Paired with {address}: the board made a new key, stored in {}.",
        config::psk_path().display()
    );
    Ok(())
}

fn cmd_diag(ctx: &Ctx, board: &BoardArgs, output: Option<&Path>, http_port: u16) -> Result<(), String> {
    let mut config = ctx.config.clone();
    if let Some(b) = &board.board {
        config.board = b.clone();
    }
    let text = pshb_core::diagnostics::report(&pshb_core::diagnostics::ReportInput {
        config: &config,
        program: "pshb-cli",
        windows: match window::detect() {
            Ok(b) => b.name().to_string(),
            Err(e) => e,
        },
        status: None,
        key: load_key(board).ok(),
        http_port,
    });
    match output {
        Some(path) => {
            std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("Written: {}", path.display());
        }
        None => print!("{text}"),
    }
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

/// The release `--version` names ("1.0.89" or "v1.0.89"), if `target` may
/// be installed in that version.
fn chosen_release(version: &str, target: update::Target) -> Result<update::Release, String> {
    let tag = format!("v{}", version.trim().trim_start_matches('v'));
    let release = update::release_for_tag(&tag).ok_or_else(|| format!("Not a version: {version}"))?;
    update::installable(target, release.version)?;
    Ok(release)
}

fn cmd_update(
    ctx: &Ctx,
    board: &BoardArgs,
    check: bool,
    firmware: bool,
    file: Option<&Path>,
    to_version: Option<&str>,
    http_port: u16,
) -> Result<(), String> {
    if firmware {
        return update_firmware(ctx, board, file, to_version, http_port);
    }
    if let Some(version) = to_version {
        let release = chosen_release(version, update::Target::App)?;
        let exe = std::env::current_exe().map_err(|e| format!("Own path: {e}"))?;
        let data =
            update::download_verified(&release, &update::cli_asset(), &mut progress_printer("Download"))?;
        update::replace_file(&exe, &data).map_err(|e| format!("{}: {e}", exe.display()))?;
        println!("Installed pshb-cli {} as {}.", release.version, exe.display());
        return Ok(());
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
            match board_firmware_http(ctx, board, http_port) {
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
    let data = update::download_verified(&latest, &update::cli_asset(), &mut progress_printer("Download"))?;
    update::replace_file(&exe, &data).map_err(|e| format!("{}: {e}", exe.display()))?;
    println!("Updated {} from {v} to {}.", exe.display(), latest.version);
    Ok(())
}

/// The board's input-path latency, status and clock speeds; or set the
/// clock speeds, or start the latency statistics over.
#[allow(clippy::too_many_arguments)]
fn cmd_board(
    ctx: &Ctx,
    board: &BoardArgs,
    cpu: Option<&str>,
    spi: Option<SpiClock>,
    reset: bool,
    name: Option<&str>,
    json: bool,
    http_port: u16,
) -> Result<(), String> {
    let addr = board_addr(ctx, board)?;
    let key = load_key(board)?;
    let api = update::BoardApi::new(&addr.ip().to_string(), http_port, &key);
    if let Some(name) = name {
        let (shown, host) = api.set_name(name.trim())?;
        println!("The board is now called \"{shown}\" (in the router and the browser: {host}).");
        return Ok(());
    }
    if cpu.is_some() || spi.is_some() {
        let now = api.clock()?;
        let cpu_mhz = cpu.map_or(now.cpu_mhz, |c| c.parse().unwrap_or(150));
        let spi_fast = spi.map_or(now.spi_fast, |s| matches!(s, SpiClock::Fast));
        if api.set_clock(cpu_mhz, spi_fast)? {
            println!("Clock speeds stored; the board restarts.");
        } else {
            println!("Clock speeds unchanged.");
        }
        return Ok(());
    }
    if reset {
        api.diag_reset()?;
        println!("Latency statistics start over.");
        return Ok(());
    }
    let clock = api.clock()?;
    let diag = api.diag()?;
    let status = api.status()?;
    if json {
        let all = serde_json::json!({ "status": status, "diag": diag, "clock": clock });
        println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
        return Ok(());
    }
    let n = |k: &str| diag.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    println!(
        "Firmware {} (slot {}), CPU {} MHz, SPI {:.1} MHz{}",
        status.get("firmware").and_then(|v| v.as_str()).unwrap_or("?"),
        status.get("partition").and_then(|v| v.as_str()).unwrap_or("?"),
        clock.cpu_hz / 1_000_000,
        f64::from(clock.spi_hz) / 1e6,
        if clock.cpu_fell_back {
            " (200 MHz did not run stably)"
        } else {
            ""
        }
    );
    println!(
        "{:<40} {:>7} {:>6} {:>7} {:>6} {:>6}  (µs)",
        "", "n", "min", "median", "p99", "max"
    );
    for (k, label) in [
        ("ready", "packet -> report ready"),
        ("ready1", "  first datagram of a round"),
        ("detect", "  noticed by the board"),
        ("bus", "  waiting for the network chip"),
        ("spi", "  reading it"),
        ("handle", "  checking and decrypting"),
        ("usb", "packet -> fetched by the PlayStation"),
    ] {
        if n(&format!("{k}_n")) > 0 {
            println!(
                "{label:<40} {:>7} {:>6} {:>7} {:>6} {:>6}",
                n(&format!("{k}_n")),
                n(&format!("{k}_min")),
                n(&format!("{k}_p50")),
                n(&format!("{k}_p99")),
                n(&format!("{k}_max"))
            );
        }
    }
    let fetches = n("mouse_fetches");
    if fetches > 0 {
        println!(
            "Missed polls {} of {fetches}, PlayStation skipped {}, armed {} µs before the poll",
            n("mouse_misses"),
            n("mouse_host_skips"),
            n("mouse_lead_us")
        );
    }
    Ok(())
}

fn cmd_discover(
    host: Option<std::net::Ipv4Addr>,
    port: u16,
    timeout: Duration,
    json: bool,
) -> Result<(), String> {
    let found: Vec<FoundBoard> = match host {
        Some(ip) => discover::probe(ip, port, timeout)
            .map_err(|e| format!("Network: {e}"))?
            .into_iter()
            .collect(),
        None => {
            let targets = discover::targets(&discover::ipv4_networks());
            discover::discover_at(&targets, port, timeout).map_err(|e| format!("Network: {e}"))?
        }
    };
    if json {
        let list: Vec<_> = found
            .iter()
            .map(|b| {
                serde_json::json!({
                    "name": b.name,
                    "address": b.address(),
                    "ip": b.ip.to_string(),
                    "mac": b.mac_string(),
                    "firmware": b.firmware().to_string(),
                    "udp_port": b.udp_port,
                    "key_set": b.key_set,
                    "static_ip": b.static_ip,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list).unwrap_or_default());
        return Ok(());
    }
    if found.is_empty() {
        println!(
            "No board answered. Boards with older firmware do not answer the search: \
             enter the address by hand (the router lists the board as ps-hid-bridge-...)."
        );
        return Ok(());
    }
    for b in &found {
        println!(
            "{:<22} {:<21} {}  firmware {:<8} {}",
            b.name,
            b.address(),
            b.mac_string(),
            b.firmware().to_string(),
            if b.key_set { "key set" } else { "no key yet" }
        );
    }
    Ok(())
}

/// The board's firmware version over its web API (signed with the key):
/// needs no packet counter, so it also works while the app is running.
/// Firmware before 1.0.25 (no signed requests): asked over UDP instead.
fn board_firmware_http(ctx: &Ctx, board: &BoardArgs, http_port: u16) -> Result<Version, String> {
    let addr = board_addr(ctx, board)?;
    let api = update::BoardApi::new(&addr.ip().to_string(), http_port, &load_key(board)?);
    match api.status() {
        Ok(st) => st
            .get("firmware")
            .and_then(|v| v.as_str())
            .and_then(Version::parse)
            .ok_or_else(|| "The board did not say its firmware version.".into()),
        Err(e) if e.contains("too old") => board_firmware(ctx, board),
        Err(e) => Err(e),
    }
}

/// The board's firmware version and how long it has been running (None:
/// firmware before 1.0.25, asked over UDP).
fn board_running(ctx: &Ctx, board: &BoardArgs, http_port: u16) -> Result<(Version, Option<u64>), String> {
    let addr = board_addr(ctx, board)?;
    let api = update::BoardApi::new(&addr.ip().to_string(), http_port, &load_key(board)?);
    match api.status() {
        Ok(st) => {
            let v = st
                .get("firmware")
                .and_then(|v| v.as_str())
                .and_then(Version::parse)
                .ok_or("The board did not say its firmware version.")?;
            Ok((v, st.get("uptime_ms").and_then(|u| u.as_u64())))
        }
        Err(e) if e.contains("too old") => Ok((board_firmware(ctx, board)?, None)),
        Err(e) => Err(e),
    }
}

/// Install firmware on the board (newest release or a file), then wait until
/// it runs the new version.
fn update_firmware(
    ctx: &Ctx,
    board: &BoardArgs,
    file: Option<&Path>,
    to_version: Option<&str>,
    http_port: u16,
) -> Result<(), String> {
    let addr = board_addr(ctx, board)?;
    let key = load_key(board)?;
    let before = board_firmware_http(ctx, board, http_port).ok();
    let (image, expected) = match (file, to_version) {
        (Some(f), _) => {
            let data = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
            usbflash::check_uf2(&data).map_err(|e| format!("{}: {e}", f.display()))?;
            (data, None)
        }
        (None, Some(version)) => {
            let release = chosen_release(version, update::Target::Firmware)?;
            if before == Some(release.version) {
                println!("The board runs firmware {} already.", release.version);
                return Ok(());
            }
            let data = update::download_verified(
                &release,
                update::FIRMWARE_ASSET,
                &mut progress_printer("Download"),
            )?;
            (data, Some(release.version))
        }
        (None, None) => {
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
    let uploaded = Instant::now();
    println!("Written and checked; the board restarts into the new firmware …");
    let deadline = uploaded + Duration::from_secs(60);
    thread::sleep(Duration::from_secs(2));
    loop {
        let late = Instant::now() > deadline;
        match board_running(ctx, board, http_port) {
            Ok((v, uptime_ms)) => {
                // A file has no version to wait for: the board must have
                // started since the upload (or, without an uptime, run
                // another version than before).
                let restarted = match uptime_ms {
                    Some(ms) => Duration::from_millis(ms) < uploaded.elapsed(),
                    None => before != Some(v),
                };
                if expected.map_or(restarted, |e| v == e) {
                    println!("The board runs firmware {v}.");
                    return Ok(());
                }
                if late {
                    return Err(if expected.is_some() {
                        format!("The board runs {v}: it went back to the previous version.")
                    } else {
                        format!("The board still runs {v}: it did not start the new firmware.")
                    });
                }
            }
            Err(e) if late => return Err(format!("The board did not come back: {e}")),
            Err(_) => {}
        }
        thread::sleep(Duration::from_secs(1));
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
            profile,
            active_profile,
            sensitivity,
            accel,
            invert_y,
            y_ratio,
            wheel_speed,
            touchpad_speed,
            tap_to_click,
            natural_scroll,
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
            let index = match profile {
                Some(name) => c
                    .profile_index(name)
                    .ok_or_else(|| format!("No profile named \"{name}\""))?,
                None => 0,
            };
            if let Some(name) = active_profile {
                let i = c
                    .profile_index(name)
                    .ok_or_else(|| format!("No profile named \"{name}\""))?;
                c.active_profile = if i == 0 {
                    String::new()
                } else {
                    c.profiles[i - 1].name.clone()
                };
            }
            let m = c.profile_mouse_mut(index);
            if let Some(s) = sensitivity {
                m.sensitivity = config::check_range("--sensitivity", *s, config::SENSITIVITY_RANGE)?;
            }
            if let Some(a) = accel {
                m.accel = config::check_range("--accel", *a, config::ACCEL_RANGE)?;
            }
            if let Some(on) = invert_y {
                m.invert_y = matches!(on, OnOff::On);
            }
            if let Some(r) = y_ratio {
                m.y_ratio = config::check_range("--y-ratio", *r, config::Y_RATIO_RANGE)?;
            }
            if let Some(w) = wheel_speed {
                m.wheel_speed = config::check_range("--wheel-speed", *w, config::WHEEL_SPEED_RANGE)?;
            }
            if let Some(s) = touchpad_speed {
                c.touchpad.speed = config::check_range("--touchpad-speed", *s, config::TOUCH_SPEED_RANGE)?;
            }
            if let Some(on) = tap_to_click {
                c.touchpad.tap_to_click = matches!(on, OnOff::On);
            }
            if let Some(on) = natural_scroll {
                c.touchpad.natural_scroll = matches!(on, OnOff::On);
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
            follow,
        } => cmd_run(&ctx, board, devices, hotkey.as_deref(), *stats, follow),
        Command::Replay {
            board,
            file,
            hotkey,
            follow,
        } => cmd_replay(&ctx, board, file, hotkey.as_deref(), follow),
        Command::Discover {
            host,
            port,
            timeout_ms,
            json,
        } => cmd_discover(*host, *port, Duration::from_millis(*timeout_ms), *json),
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
        Command::Type { board, text, layout } => cmd_type(&ctx, board, text, layout.as_deref()),
        Command::Pair {
            board,
            password,
            new_password,
            http_port,
        } => cmd_pair(
            &ctx,
            board.as_deref(),
            password,
            new_password.as_deref(),
            *http_port,
        ),
        Command::Diag {
            board,
            output,
            http_port,
        } => cmd_diag(&ctx, board, output.as_deref(), *http_port),
        Command::Config { cmd } => cmd_config(&mut ctx, cmd),
        Command::Update {
            board,
            check,
            firmware,
            file,
            to_version,
            http_port,
        } => cmd_update(
            &ctx,
            board,
            *check,
            *firmware,
            file.as_deref(),
            to_version.as_deref(),
            *http_port,
        ),
        Command::Flash { image, file, yes } => cmd_flash(image.image(), file.as_deref(), *yes),
        Command::Board {
            board,
            cpu,
            spi,
            reset,
            name,
            json,
            http_port,
        } => cmd_board(
            &ctx,
            board,
            cpu.as_deref(),
            *spi,
            *reset,
            name.as_deref(),
            *json,
            *http_port,
        ),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
