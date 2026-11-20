//! QML-facing controller (singleton `AppController`): settings, the bridge
//! service, tray icon, window-focus rule and notifications.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use pshb_core::config::{self, Config, TriggerConfig};
use pshb_core::crypto::parse_key_hex;
use pshb_core::diagnostics::{BoardHealth, HealthWarning};
use pshb_core::discover::{self, FoundBoard, board_ipv4};
use pshb_core::engine::Notice;
use pshb_core::input::{self, DeviceInfo};
use pshb_core::install::{self, Found};
use pshb_core::protocol::{self, Pong};
use pshb_core::service::{Service, ServiceCmd, ServiceConfig, ServiceEvent, ServiceStatus, TypingStop};
use pshb_core::transport::KeyState;
use pshb_core::trigger::Action;
use pshb_core::typing;
use pshb_core::update::{self, Release, Version};
use pshb_core::usbflash::{self, Image};
use pshb_core::window::{self, WindowInfo};
use serde::Serialize;

use crate::i18n::{self, tr, trf};
use crate::notify;
use crate::tray::{self, BridgeTray};

/// The app's requests to the board's web API (latency page, start over,
/// clock speeds) go one at a time.
static BOARD_API: Mutex<()> = Mutex::new(());

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, connected)]
        #[qproperty(bool, active)]
        #[qproperty(bool, pending)]
        #[qproperty(bool, paused)]
        #[qproperty(QString, status_text)]
        #[qproperty(f64, rtt_ms)]
        #[qproperty(f64, rtt_p50)]
        #[qproperty(f64, rtt_p95)]
        #[qproperty(QString, rtt_history_json)]
        /// One-way time PC -> board in ms (-1 = not known: older board
        /// firmware or no answers yet) and its recent samples.
        /// The board answers timed pings (firmware that knows PING_TIMED).
        #[qproperty(bool, board_timed_ping)]
        #[qproperty(f64, up_p50)]
        #[qproperty(f64, up_p95)]
        #[qproperty(QString, up_history_json)]
        #[qproperty(f64, input_p50)]
        #[qproperty(f64, input_p95)]
        #[qproperty(f64, input_max)]
        #[qproperty(bool, realtime)]
        #[qproperty(i32, packets_per_second)]
        #[qproperty(QString, board_firmware)]
        #[qproperty(QString, board_usb)]
        #[qproperty(bool, board_no_partition_table)]
        #[qproperty(QString, board_address)]
        #[qproperty(bool, psk_set)]
        #[qproperty(QString, devices_json)]
        #[qproperty(i32, devices_blocked)]
        /// Keyboards that are not selected and may not be read: triggers
        /// and the emergency stop do not work on them yet.
        #[qproperty(i32, other_keyboards_blocked)]
        #[qproperty(i32, devices_open)]
        #[qproperty(i32, input_resyncs)]
        #[qproperty(QString, triggers_json)]
        #[qproperty(bool, window_rule_enabled)]
        #[qproperty(QString, window_rule)]
        #[qproperty(bool, window_rule_available)]
        /// Where the focused window is learned from ("KDE Plasma", "Hyprland"
        /// …), or why it cannot be (window_rule_available false).
        #[qproperty(QString, window_backend)]
        #[qproperty(bool, iso_keyboard)]
        #[qproperty(f64, sensitivity)]
        #[qproperty(f64, accel)]
        /// Touchpads and touchscreens: speed factor, tap to click, natural
        /// scrolling (config [touchpad]).
        #[qproperty(f64, touchpad_speed)]
        #[qproperty(bool, tap_to_click)]
        #[qproperty(bool, natural_scroll)]
        #[qproperty(bool, setup_done)]
        #[qproperty(bool, recording)]
        #[qproperty(QString, recorded)]
        #[qproperty(QString, identified_path)]
        #[qproperty(QString, live_text)]
        /// The same as chips: JSON array of texts ("Shift", "A", "mouse: left").
        #[qproperty(QString, live_chips_json)]
        /// The mouse buttons the PlayStation receives right now (bits:
        /// left 1, right 2, middle 4, back 8, forward 16; 0 = none).
        #[qproperty(i32, live_mouse_buttons)]
        #[qproperty(QString, live_device)]
        /// The service is starting (connecting to the board).
        #[qproperty(bool, connecting)]
        /// "en" or "de"; the texts for QML in translations_json (I18n.qml).
        #[qproperty(QString, language)]
        #[qproperty(QString, translations_json)]
        /// Profiles, "Standard" first: JSON array of {name, window,
        /// sensitivity, accel, invert_y, y_ratio, wheel_speed, remap: [{from, to}]}.
        #[qproperty(QString, profiles_json)]
        /// The profile chosen by hand or by a trigger, and the one in effect
        /// (a window's profile may override the first).
        #[qproperty(QString, active_profile)]
        #[qproperty(QString, effective_profile)]
        /// Keyboard language of the PlayStation for typing text ("de", "us",
        /// "uk"), and the choices: JSON array of {id, name}.
        #[qproperty(QString, ps_layout)]
        #[qproperty(QString, layouts_json)]
        /// Typing text: busy, share done (0..1), outcome ("" while busy).
        #[qproperty(bool, typing_busy)]
        #[qproperty(f64, typing_progress)]
        #[qproperty(QString, typing_message)]
        #[qproperty(QString, typing_level)]
        /// Pairing with the board's password: busy, outcome, and whether the
        /// board asks for a new password first (factory password).
        #[qproperty(bool, pair_busy)]
        #[qproperty(QString, pair_message)]
        #[qproperty(QString, pair_level)]
        #[qproperty(bool, pair_needs_new_password)]
        /// Saving the diagnostics report: busy, outcome.
        #[qproperty(bool, diag_busy)]
        #[qproperty(QString, diag_message)]
        #[qproperty(QString, diag_level)]
        /// The board refuses packets from somewhere else ("" = all fine).
        #[qproperty(QString, board_warning)]
        /// Installing the AppImage with Shelly: what the app offers ("",
        /// "install", "start-installed", "restart-installed"), the folder
        /// it runs from, the installed copy's version, and the outcome.
        #[qproperty(QString, install_offer)]
        #[qproperty(QString, install_folder)]
        #[qproperty(QString, installed_version)]
        #[qproperty(bool, install_busy)]
        #[qproperty(QString, install_message)]
        #[qproperty(QString, install_level)]
        /// Removing what the app set up as root (before uninstalling):
        /// busy, outcome.
        #[qproperty(bool, cleanup_busy)]
        #[qproperty(QString, cleanup_message)]
        #[qproperty(QString, cleanup_level)]
        #[qproperty(QString, last_error)]
        /// Settings that could not be written; stays until a save works.
        #[qproperty(QString, save_error)]
        /// The packet counter cannot be saved (from the service status).
        #[qproperty(QString, counter_error)]
        #[qproperty(bool, access_busy)]
        #[qproperty(QString, access_message)]
        #[qproperty(QString, version)]
        #[qproperty(bool, update_check)]
        #[qproperty(QString, appearance)]
        #[qproperty(QString, latest_version)]
        #[qproperty(bool, app_update_available)]
        #[qproperty(bool, board_update_available)]
        #[qproperty(bool, can_update_app)]
        #[qproperty(bool, app_updated)]
        #[qproperty(bool, update_busy)]
        #[qproperty(QString, update_status)]
        /// How `update_status` reads: "info" (working, a note), "ok" (done)
        /// or "warn" (it failed).
        #[qproperty(QString, update_level)]
        /// The board's status, latency diagnostics and clock speeds as JSON
        /// {"status": …, "diag": …, "clock": …} (signed requests, see
        /// refresh_board_info); "" until the first answer.
        #[qproperty(QString, board_info_json)]
        #[qproperty(QString, board_info_error)]
        #[qproperty(bool, board_info_busy)]
        /// Boards that answered the last search in the network (see
        /// discover_boards), JSON [{name, address, ip, mac, firmware,
        /// key_set, current}]; `current`: the board in the settings.
        #[qproperty(QString, found_boards_json)]
        #[qproperty(bool, discover_busy)]
        /// The connected board's name ("" = not known yet), from its
        /// discovery answer; rename_board changes it.
        #[qproperty(QString, board_name)]
        /// MAC address of the board in the settings ("" = not known yet).
        #[qproperty(QString, board_mac)]
        #[qproperty(bool, rename_busy)]
        /// Outcome of the last rename_board: the text and "ok" or "warn".
        #[qproperty(QString, rename_message)]
        #[qproperty(QString, rename_level)]
        #[qproperty(bool, clock_busy)]
        #[qproperty(QString, clock_message)]
        #[qproperty(bool, usb_board_found)]
        #[qproperty(bool, usb_busy)]
        #[qproperty(QString, usb_status)]
        /// USB network adapter towards the board that can bundle packets
        /// ("" = none), how long it holds a packet back at most when it does
        /// (ms), and who decides: "managed" (the app, while it runs), "delaying" (bundles, the app may not switch it) or
        /// "fixed" (sends at once, set outside the app).
        #[qproperty(QString, adapter_iface)]
        #[qproperty(f64, adapter_delay_ms)]
        #[qproperty(QString, adapter_mode)]
        #[qproperty(bool, adapter_at_once_now)]
        /// "disabled", "held", "ready", "not_permitted" or "unavailable"
        /// (see pshb_core::lowlat::CpuWakeState).
        #[qproperty(QString, cpu_wake_state)]
        #[qproperty(bool, keep_cpu_awake)]
        #[qproperty(bool, latency_setup_busy)]
        #[qproperty(QString, latency_setup_message)]
        /// The udev rules the app installed as root (see pshb_core::access).
        #[qproperty(bool, access_rule_installed)]
        #[qproperty(bool, latency_rule_installed)]
        type AppController = super::AppControllerRust;

        /// Switch the bridge on/off (same as the toggle trigger).
        #[qinvokable]
        fn toggle(self: Pin<&mut AppController>);
        #[qinvokable]
        fn set_on(self: Pin<&mut AppController>, on: bool);
        #[qinvokable]
        fn pause(self: Pin<&mut AppController>);
        #[qinvokable]
        fn release_all(self: Pin<&mut AppController>);

        /// Store board address and key; returns an error text or "".
        #[qinvokable]
        fn save_board(self: Pin<&mut AppController>, address: &QString, psk: &QString) -> QString;
        /// (Re)connect with the stored settings.
        #[qinvokable]
        fn reconnect(self: Pin<&mut AppController>);
        /// Look for boards in the network (`discover_busy` meanwhile, the
        /// result in `found_boards_json`).
        #[qinvokable]
        fn discover_boards(self: Pin<&mut AppController>);
        /// Give the board a name (signed with the key; "" = its default
        /// name); `rename_busy` meanwhile, then `rename_message`.
        #[qinvokable]
        fn rename_board(self: Pin<&mut AppController>, name: &QString);
        /// The network name the board takes for `name` (router, browser), as
        /// the firmware derives it; `fallback` if nothing is left of it.
        #[qinvokable]
        fn network_name_for(self: &AppController, name: &QString, fallback: &QString) -> QString;

        #[qinvokable]
        fn refresh_devices(self: Pin<&mut AppController>);
        /// Select a device group by its "vvvv:pppp" id.
        #[qinvokable]
        fn set_device_selected(self: Pin<&mut AppController>, id: &QString, selected: bool);
        /// Install the udev rule for the selected devices and every keyboard
        /// (asks for the administrator password); `access_busy` meanwhile,
        /// then the outcome in `access_message` ("" = done).
        #[qinvokable]
        fn install_access(self: Pin<&mut AppController>);
        #[qinvokable]
        fn set_identify(self: Pin<&mut AppController>, on: bool);

        /// Record the next key combination into `recorded`.
        #[qinvokable]
        fn record_trigger(self: Pin<&mut AppController>, on: bool);
        /// Replace all triggers (JSON array of {kind, keys, action, window_ms}); error text or "".
        #[qinvokable]
        fn set_triggers(self: Pin<&mut AppController>, json: &QString) -> QString;
        #[qinvokable]
        fn set_window_rule_text(self: Pin<&mut AppController>, text: &QString);
        /// Switch the window rule on or off.
        #[qinvokable]
        fn set_window_rule_on(self: Pin<&mut AppController>, on: bool);
        /// Physical keyboard type: ISO (true) or ANSI (false).
        #[qinvokable]
        fn set_keyboard_type(self: Pin<&mut AppController>, iso: bool);
        #[qinvokable]
        fn set_mouse(self: Pin<&mut AppController>, sensitivity: f64, accel: f64);
        /// Mouse settings of a profile (0 = "Standard").
        #[qinvokable]
        fn set_profile_mouse(
            self: Pin<&mut AppController>,
            index: i32,
            sensitivity: f64,
            accel: f64,
            invert_y: bool,
            y_ratio: f64,
            wheel_speed: f64,
        );
        /// Remapped keys of a profile (JSON array of {from, to}); error text or "".
        #[qinvokable]
        fn set_profile_remap(self: Pin<&mut AppController>, index: i32, json: &QString) -> QString;
        /// Window pattern of a profile ("" = by hand only).
        #[qinvokable]
        fn set_profile_window(self: Pin<&mut AppController>, index: i32, pattern: &QString);
        /// New profile with the settings of `copy_of`; error text or "".
        #[qinvokable]
        fn add_profile(self: Pin<&mut AppController>, name: &QString, copy_of: i32) -> QString;
        #[qinvokable]
        fn rename_profile(self: Pin<&mut AppController>, index: i32, name: &QString) -> QString;
        #[qinvokable]
        fn delete_profile(self: Pin<&mut AppController>, index: i32);
        /// Use this profile (by hand).
        #[qinvokable]
        fn choose_profile(self: Pin<&mut AppController>, index: i32);
        /// Gain of a profile's acceleration curve at a given speed.
        #[qinvokable]
        fn profile_curve_gain(self: &AppController, index: i32, speed: f64) -> f64;
        /// Language of the app: "en" or "de".
        #[qinvokable]
        fn set_app_language(self: Pin<&mut AppController>, code: &QString);
        /// Keyboard language of the PlayStation ("de", "us", "uk").
        #[qinvokable]
        fn choose_ps_layout(self: Pin<&mut AppController>, id: &QString);
        /// Characters of `text` the PlayStation's keyboard language cannot type.
        #[qinvokable]
        fn unsupported_chars(self: &AppController, text: &QString) -> QString;
        /// Type `text` on the PlayStation (the bridge must be off);
        /// `typing_busy` meanwhile, then `typing_message`.
        #[qinvokable]
        fn type_text(self: Pin<&mut AppController>, text: &QString);
        #[qinvokable]
        fn cancel_typing(self: Pin<&mut AppController>);
        /// Take a new key from the board at `address` with its web password
        /// (and set `new_password` first while it has the factory one);
        /// `pair_busy` meanwhile, then `pair_message`.
        #[qinvokable]
        fn pair_board(
            self: Pin<&mut AppController>,
            address: &QString,
            password: &QString,
            new_password: &QString,
        );
        /// Write the diagnostics report to the file at `url` (file dialog);
        /// `diag_busy` meanwhile, then `diag_message`.
        #[qinvokable]
        fn save_diagnostics(self: Pin<&mut AppController>, url: &QString);
        #[qinvokable]
        fn dismiss_board_warning(self: Pin<&mut AppController>);
        /// Install this AppImage with Shelly; `install_busy` meanwhile.
        #[qinvokable]
        fn install_with_shelly(self: Pin<&mut AppController>);
        /// Start the installed copy instead of this file.
        #[qinvokable]
        fn start_installed(self: Pin<&mut AppController>);
        /// Hide the offer (until the next start).
        #[qinvokable]
        fn dismiss_install(self: Pin<&mut AppController>);
        /// Remove every rule the app installed as root and the access they
        /// gave (asks for the administrator password once); the settings
        /// stay. `cleanup_busy` meanwhile, then `cleanup_message`.
        #[qinvokable]
        fn remove_system_changes(self: Pin<&mut AppController>);
        /// Touchpads and touchscreens: speed factor, tap to click, natural scrolling.
        #[qinvokable]
        fn set_touchpad(self: Pin<&mut AppController>, speed: f64, tap: bool, natural: bool);
        /// Gain of the acceleration curve at a given speed (for the curve plot).
        #[qinvokable]
        fn curve_gain(self: &AppController, speed: f64) -> f64;

        /// Ping faster while the latency view is visible.
        #[qinvokable]
        fn set_latency_view(self: Pin<&mut AppController>, visible: bool);
        /// Keep the CPU out of deep sleep states while the bridge is on.
        #[qinvokable]
        fn enable_keep_cpu_awake(self: Pin<&mut AppController>, on: bool);
        /// Network adapters send every packet at once, and the CPU may be
        /// kept awake (udev rule, asks for the administrator password);
        /// `latency_setup_busy` meanwhile, then `latency_setup_message`.
        #[qinvokable]
        fn setup_low_latency(self: Pin<&mut AppController>);
        /// Undo setup_low_latency (asks for the administrator password).
        #[qinvokable]
        fn remove_low_latency(self: Pin<&mut AppController>);
        /// Undo install_access: the app can no longer read keyboard and
        /// mouse (asks for the administrator password).
        #[qinvokable]
        fn remove_access(self: Pin<&mut AppController>);

        #[qinvokable]
        fn finish_setup(self: Pin<&mut AppController>);

        /// Look for a new release now (also when the automatic check is off).
        #[qinvokable]
        fn check_updates(self: Pin<&mut AppController>);
        /// Look of the app: "system", "light" or "dark".
        #[qinvokable]
        fn choose_appearance(self: Pin<&mut AppController>, value: &QString);
        /// Automatic check at start and every six hours.
        #[qinvokable]
        fn enable_update_check(self: Pin<&mut AppController>, on: bool);
        /// Download the newest AppImage and replace this one (see restart_app).
        #[qinvokable]
        fn update_app(self: Pin<&mut AppController>);
        /// Start the newly installed version and quit this one.
        #[qinvokable]
        fn restart_app(self: Pin<&mut AppController>);
        /// Install the newest firmware on the board (the bridge must be off).
        #[qinvokable]
        fn update_board(self: Pin<&mut AppController>);

        /// Fetch the board's status, diagnostics and clock speeds (in the
        /// background; ignored while a fetch is running).
        #[qinvokable]
        fn refresh_board_info(self: Pin<&mut AppController>);
        /// Start the board's latency statistics over.
        #[qinvokable]
        fn reset_board_diag(self: Pin<&mut AppController>);
        /// Set the board's clock speeds (CPU 150 or 200 MHz); it restarts.
        #[qinvokable]
        fn set_board_clock(self: Pin<&mut AppController>, cpu_mhz: i32, spi_fast: bool);

        /// Look for the board's USB boot drive (BOOTSEL) every second while on.
        #[qinvokable]
        fn usb_watch(self: Pin<&mut AppController>, on: bool);
        /// Flash over USB: image "first-install", "firmware", "hidtest",
        /// "hidtest-mouse8" or "erase"; `file` (URL) instead of the newest release.
        #[qinvokable]
        fn usb_flash(self: Pin<&mut AppController>, image: &QString, file: &QString);
        #[qinvokable]
        fn usb_cancel(self: Pin<&mut AppController>);
        #[qinvokable]
        fn quit_app(self: Pin<&mut AppController>);

        /// The window should come to the front (tray menu "Show window").
        #[qsignal]
        fn show_window_requested(self: Pin<&mut AppController>);
    }

    impl cxx_qt::Threading for AppController {}
    impl cxx_qt::Initialize for AppController {}
}

pub struct AppControllerRust {
    connected: bool,
    active: bool,
    pending: bool,
    paused: bool,
    status_text: QString,
    rtt_ms: f64,
    rtt_p50: f64,
    rtt_p95: f64,
    rtt_history_json: QString,
    board_timed_ping: bool,
    up_p50: f64,
    up_p95: f64,
    up_history_json: QString,
    /// Input event -> packet sent, in ms (-1 = no samples yet).
    input_p50: f64,
    input_p95: f64,
    input_max: f64,
    realtime: bool,
    packets_per_second: i32,
    board_firmware: QString,
    board_usb: QString,
    board_no_partition_table: bool,
    board_address: QString,
    psk_set: bool,
    devices_json: QString,
    devices_blocked: i32,
    other_keyboards_blocked: i32,
    devices_open: i32,
    input_resyncs: i32,
    triggers_json: QString,
    window_rule_enabled: bool,
    window_rule: QString,
    window_rule_available: bool,
    window_backend: QString,
    iso_keyboard: bool,
    sensitivity: f64,
    accel: f64,
    touchpad_speed: f64,
    tap_to_click: bool,
    natural_scroll: bool,
    setup_done: bool,
    recording: bool,
    recorded: QString,
    identified_path: QString,
    live_text: QString,
    live_chips_json: QString,
    live_mouse_buttons: i32,
    live_device: QString,
    connecting: bool,
    language: QString,
    translations_json: QString,
    profiles_json: QString,
    active_profile: QString,
    effective_profile: QString,
    ps_layout: QString,
    layouts_json: QString,
    typing_busy: bool,
    typing_progress: f64,
    typing_message: QString,
    typing_level: QString,
    pair_busy: bool,
    pair_message: QString,
    pair_level: QString,
    pair_needs_new_password: bool,
    diag_busy: bool,
    diag_message: QString,
    diag_level: QString,
    board_warning: QString,
    install_offer: QString,
    install_folder: QString,
    installed_version: QString,
    install_busy: bool,
    install_message: QString,
    install_level: QString,
    cleanup_busy: bool,
    cleanup_message: QString,
    cleanup_level: QString,
    /// This app in Shelly's record (the installed copy).
    installed: Option<install::Installed>,
    last_error: QString,
    save_error: QString,
    counter_error: QString,
    access_busy: bool,
    access_message: QString,
    version: QString,

    config: Config,
    devices: Vec<DeviceInfo>,
    slot: Arc<ServiceSlot>,
    tray: Option<ksni::blocking::Handle<BridgeTray>>,
    /// Last tray state shown (active, connected).
    tray_state: Option<(bool, bool)>,
    /// Watches the focused window while the window rule or a profile needs it.
    window_watcher: Option<window::Watcher>,
    /// What the GUI asked the service for; re-applied after a restart.
    identify_on: bool,
    latency_view: bool,
    /// The board's address the running service sends to (looked up from
    /// the setting, which may be a host name).
    board_addr: Option<std::net::SocketAddr>,
    /// The focused window as last reported (None: not watched).
    window_state: Option<Option<WindowInfo>>,
    /// The service's last status and live state (shown again in another
    /// language).
    last_status: ServiceStatus,
    last_live: Option<KeyState>,
    /// Watches the board's counters of refused packets.
    health: BoardHealth,

    update_check: bool,
    appearance: QString,
    latest_version: QString,
    app_update_available: bool,
    board_update_available: bool,
    can_update_app: bool,
    app_updated: bool,
    update_busy: bool,
    update_status: QString,
    update_level: QString,
    board_info_json: QString,
    board_info_error: QString,
    board_info_busy: bool,
    found_boards_json: QString,
    discover_busy: bool,
    board_name: QString,
    board_mac: QString,
    rename_busy: bool,
    rename_message: QString,
    rename_level: QString,
    /// The last search's boards (`found_boards_json`).
    found: Vec<FoundBoard>,
    /// Service generation whose board was asked for its MAC address.
    mac_checked: u64,
    /// Wait before the next search for the board that moved (doubles while
    /// it stays away).
    follow_wait: Duration,
    clock_busy: bool,
    clock_message: QString,
    /// Newest release found by the last check.
    latest: Option<Release>,
    /// Version a notification was shown for (once per version).
    notified: Option<Version>,
    /// Firmware the board should come back with after an update (and the
    /// number of that update, for its timeout).
    board_expected: Option<(Version, u64)>,
    board_updates: u64,

    usb_board_found: bool,
    usb_busy: bool,
    usb_status: QString,
    /// The USB watch thread runs while this is set.
    usb_watching: Arc<AtomicBool>,
    usb_cancel: Arc<AtomicBool>,

    adapter_iface: QString,
    adapter_delay_ms: f64,
    adapter_mode: QString,
    adapter_at_once_now: bool,
    cpu_wake_state: QString,
    keep_cpu_awake: bool,
    latency_setup_busy: bool,
    latency_setup_message: QString,
    access_rule_installed: bool,
    latency_rule_installed: bool,
}

/// The running service, shared with the threads that (re)start it, so that
/// name lookups and the start itself never block the UI.
#[derive(Default)]
struct ServiceSlot {
    service: Mutex<Option<Service>>,
    /// Bumped by every restart request: a start that is no longer the
    /// newest one is dropped.
    generation: AtomicU64,
    /// One start at a time: the old service (devices, packet counter) must
    /// be gone before the next one starts.
    starting: Mutex<()>,
}

impl ServiceSlot {
    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

/// Retry interval when the service could not be started (no network yet,
/// board name not resolvable, another program sending).
const RETRY: Duration = Duration::from_secs(5);
/// Board configured by name and not answering: look the name up again this
/// often (it may have a new address from DHCP).
const RENAME_CHECK: Duration = Duration::from_secs(10);
/// Board configured by address (its MAC known) and not answering: look for
/// it in the network after this long (it counts as gone after 3 s without an
/// answer), then after twice as long each time up to FOLLOW_MAX_ON while the
/// bridge is on and FOLLOW_MAX otherwise (a board that is off is not looked
/// for all the time).
const FOLLOW_FIRST: Duration = Duration::from_secs(3);
const FOLLOW_MAX_ON: Duration = Duration::from_secs(30);
const FOLLOW_MAX: Duration = Duration::from_secs(300);

impl Default for AppControllerRust {
    fn default() -> Self {
        Self {
            connected: false,
            active: false,
            pending: false,
            paused: false,
            status_text: QString::from("Not connected"),
            rtt_ms: -1.0,
            rtt_p50: -1.0,
            rtt_p95: -1.0,
            rtt_history_json: QString::from("[]"),
            board_timed_ping: false,
            up_p50: -1.0,
            up_p95: -1.0,
            up_history_json: QString::from("[]"),
            input_p50: -1.0,
            input_p95: -1.0,
            input_max: -1.0,
            realtime: false,
            packets_per_second: 0,
            board_firmware: QString::default(),
            board_usb: QString::default(),
            board_no_partition_table: false,
            board_address: QString::default(),
            psk_set: false,
            devices_json: QString::from("[]"),
            devices_blocked: 0,
            other_keyboards_blocked: 0,
            devices_open: 0,
            input_resyncs: 0,
            triggers_json: QString::from("[]"),
            window_rule_enabled: false,
            window_rule: QString::default(),
            window_rule_available: false,
            window_backend: QString::default(),
            iso_keyboard: true,
            sensitivity: 1.0,
            accel: 0.0,
            touchpad_speed: 1.0,
            tap_to_click: true,
            natural_scroll: false,
            setup_done: false,
            recording: false,
            recorded: QString::default(),
            identified_path: QString::default(),
            live_text: QString::default(),
            live_chips_json: QString::from("[]"),
            live_mouse_buttons: 0,
            live_device: QString::default(),
            connecting: false,
            language: QString::from("en"),
            translations_json: QString::from("{}"),
            profiles_json: QString::from("[]"),
            active_profile: QString::default(),
            effective_profile: QString::default(),
            ps_layout: QString::default(),
            layouts_json: QString::from(&layouts_json()),
            typing_busy: false,
            typing_progress: 0.0,
            typing_message: QString::default(),
            typing_level: QString::default(),
            pair_busy: false,
            pair_message: QString::default(),
            pair_level: QString::default(),
            pair_needs_new_password: false,
            diag_busy: false,
            diag_message: QString::default(),
            diag_level: QString::default(),
            board_warning: QString::default(),
            install_offer: QString::default(),
            install_folder: QString::default(),
            installed_version: QString::default(),
            install_busy: false,
            install_message: QString::default(),
            install_level: QString::default(),
            cleanup_busy: false,
            cleanup_message: QString::default(),
            cleanup_level: QString::default(),
            installed: None,
            last_error: QString::default(),
            save_error: QString::default(),
            counter_error: QString::default(),
            access_busy: false,
            access_message: QString::default(),
            version: QString::from(pshb_core::VERSION),
            config: Config::default(),
            devices: Vec::new(),
            slot: Arc::default(),
            tray: None,
            tray_state: None,
            window_watcher: None,
            identify_on: false,
            latency_view: false,
            board_addr: None,
            window_state: None,
            last_status: ServiceStatus::default(),
            last_live: None,
            health: BoardHealth::default(),
            update_check: true,
            appearance: QString::from("system"),
            latest_version: QString::default(),
            app_update_available: false,
            board_update_available: false,
            can_update_app: false,
            app_updated: false,
            update_busy: false,
            update_status: QString::default(),
            update_level: QString::from("info"),
            board_info_json: QString::default(),
            board_info_error: QString::default(),
            board_info_busy: false,
            found_boards_json: QString::from("[]"),
            discover_busy: false,
            board_name: QString::default(),
            board_mac: QString::default(),
            rename_busy: false,
            rename_message: QString::default(),
            rename_level: QString::default(),
            found: Vec::new(),
            mac_checked: 0,
            follow_wait: FOLLOW_FIRST,
            clock_busy: false,
            clock_message: QString::default(),
            latest: None,
            notified: None,
            board_expected: None,
            board_updates: 0,
            usb_board_found: false,
            usb_busy: false,
            usb_status: QString::default(),
            usb_watching: Arc::default(),
            usb_cancel: Arc::default(),
            adapter_iface: QString::default(),
            adapter_delay_ms: 0.0,
            adapter_mode: QString::default(),
            adapter_at_once_now: false,
            cpu_wake_state: QString::from("disabled"),
            keep_cpu_awake: true,
            latency_setup_busy: false,
            latency_setup_message: QString::default(),
            access_rule_installed: false,
            latency_rule_installed: false,
        }
    }
}

/// One physical device (all event nodes with the same vendor:product).
#[derive(Serialize)]
struct DeviceGroup {
    id: String,
    name: String,
    kind: String,
    nodes: usize,
    accessible: bool,
    selected: bool,
    paths: Vec<String>,
}

/// The kind of a device with several nodes: a keyboard, and the pointing
/// device that says the most (a touchpad's "Mouse" sibling node is the
/// touchpad).
fn merge_kinds(a: &str, b: &str) -> &'static str {
    let keyboard = a.contains("keyboard") || b.contains("keyboard");
    let pointer = ["touchscreen", "touchpad", "mouse"]
        .into_iter()
        .find(|p| a.contains(p) || b.contains(p));
    match (keyboard, pointer) {
        (true, Some("touchscreen")) => "keyboard + touchscreen",
        (true, Some("touchpad")) => "keyboard + touchpad",
        (true, Some(_)) => "keyboard + mouse",
        (true, None) => "keyboard",
        (false, Some("touchscreen")) => "touchscreen",
        (false, Some("touchpad")) => "touchpad",
        (false, Some(_)) => "mouse",
        (false, None) => "other",
    }
}

fn group_devices(devices: &[DeviceInfo], config: &Config) -> Vec<DeviceGroup> {
    let mut groups: Vec<DeviceGroup> = Vec::new();
    for d in devices.iter().filter(|d| d.keyboard || d.pointer()) {
        let id = d.id_string();
        // Selected by its id, or by a path or part of the name (pshb-cli).
        let selected = config.devices.iter().any(|s| d.matches(s));
        if let Some(g) = groups.iter_mut().find(|g| g.id == id) {
            g.selected |= selected;
            g.nodes += 1;
            g.accessible &= d.accessible;
            g.paths.push(d.path.display().to_string());
            // The shortest name, but a touchpad's own name before its
            // "… Mouse" sibling node's.
            let touch = d.touchpad || d.touchscreen;
            if touch || (d.name.len() < g.name.len() && !g.kind.contains("touch")) {
                g.name = d.name.clone();
            }
            g.kind = merge_kinds(&g.kind, d.kind()).into();
        } else {
            groups.push(DeviceGroup {
                selected,
                id,
                name: d.name.clone(),
                kind: d.kind().into(),
                nodes: 1,
                accessible: d.accessible,
                paths: vec![d.path.display().to_string()],
            });
        }
    }
    groups
}

fn usb_text(p: &Pong) -> &'static str {
    if p.status & protocol::STATUS_USB_MOUNTED == 0 {
        "PlayStation not connected"
    } else if p.status & protocol::STATUS_USB_SUSPENDED != 0 {
        "PlayStation in rest mode"
    } else {
        "PlayStation connected"
    }
}

impl qobject::AppController {
    /// Write the settings; false if that failed. The failure stays shown
    /// (`save_error`, on every page) until a save works: `last_error` is
    /// cleared by every service start, and a lost change must not look
    /// saved.
    fn save_config(mut self: Pin<&mut Self>) -> bool {
        let path = config::config_path();
        let result = self.config_ref().save(&path);
        let text = match &result {
            Ok(()) => QString::default(),
            Err(e) => QString::from(&trf(
                "Settings could not be saved: %1. Changes apply only until the app closes.",
                &[e],
            )),
        };
        self.as_mut().set_save_error(text);
        result.is_ok()
    }

    fn config_ref(&self) -> &Config {
        &self.rust().config
    }

    fn publish_config(mut self: Pin<&mut Self>) {
        let c = self.config_ref().clone();
        self.as_mut().set_board_address(QString::from(&c.board));
        self.as_mut().set_board_mac(QString::from(&c.board_mac));
        self.as_mut().set_psk_set(config::psk_path().exists());
        self.as_mut().set_triggers_json(QString::from(
            &serde_json::to_string(&c.triggers).unwrap_or_default(),
        ));
        self.as_mut().set_window_rule_enabled(c.window_rule_enabled);
        self.as_mut().set_window_rule(QString::from(&c.window_rule));
        self.as_mut().set_iso_keyboard(c.iso_keyboard);
        self.as_mut().set_sensitivity(c.mouse.sensitivity);
        self.as_mut().set_accel(c.mouse.accel);
        self.as_mut().set_touchpad_speed(c.touchpad.speed);
        self.as_mut().set_tap_to_click(c.touchpad.tap_to_click);
        self.as_mut().set_natural_scroll(c.touchpad.natural_scroll);
        self.as_mut().set_setup_done(c.setup_done);
        self.as_mut().set_update_check(c.update_check);
        self.as_mut().set_appearance(QString::from(&c.appearance));
        self.as_mut().set_keep_cpu_awake(c.keep_cpu_awake);
        self.as_mut().set_language(QString::from(i18n::language()));
        self.as_mut()
            .set_translations_json(QString::from(i18n::catalog_json()));
        self.as_mut().set_profiles_json(QString::from(&profiles_json(&c)));
        self.as_mut()
            .set_ps_layout(QString::from(typing::layout_or_default(&c.ps_layout).id));
        let active = c.profile_names()[c.active_profile_index()].to_string();
        self.as_mut().set_active_profile(QString::from(&active));
        if let Some(t) = &self.rust().tray {
            let names: Vec<String> = c.profile_names().iter().map(|n| n.to_string()).collect();
            let index = c.active_profile_index();
            t.update(move |tray| {
                tray.profiles = names;
                tray.profile = index;
            });
        }
        self.as_mut().publish_rules();
        self.as_mut().publish_found();
    }

    /// The last search's boards for QML, marked against the settings.
    fn publish_found(mut self: Pin<&mut Self>) {
        #[derive(Serialize)]
        struct Row {
            name: String,
            address: String,
            ip: String,
            mac: String,
            firmware: String,
            key_set: bool,
            current: bool,
        }
        let current = board_ipv4(&self.config_ref().board);
        if let Some(b) = self.rust().found.iter().find(|b| current == Some(b.ip)) {
            let name = QString::from(&b.name);
            self.as_mut().set_board_name(name);
        }
        let rows: Vec<Row> = self
            .rust()
            .found
            .iter()
            .map(|b| Row {
                name: b.name.clone(),
                address: b.address(),
                ip: b.ip.to_string(),
                mac: b.mac_string(),
                firmware: b.firmware().to_string(),
                key_set: b.key_set,
                current: current == Some(b.ip),
            })
            .collect();
        self.as_mut()
            .set_found_boards_json(QString::from(&serde_json::to_string(&rows).unwrap_or_default()));
    }

    fn publish_rules(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_access_rule_installed(pshb_core::access::rule_installed());
        self.as_mut()
            .set_latency_rule_installed(pshb_core::access::latency_rule_installed());
    }

    fn command(&self, cmd: ServiceCmd) {
        if let Some(s) = self.rust().slot.service.lock().unwrap().as_ref() {
            s.send(cmd);
        }
    }

    /// Start (or restart) the bridge service with the current settings. The
    /// work happens on a helper thread; the outcome arrives as
    /// `service_started` / `service_failed`.
    fn restart_service(mut self: Pin<&mut Self>) {
        let slot = self.rust().slot.clone();
        let generation = slot.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cfg = self.config_ref().clone();
        let problem = if cfg.board.is_empty() {
            Some(tr("Board address missing"))
        } else if !config::psk_path().exists() {
            Some(tr("Key missing"))
        } else {
            None
        };
        if let Some(text) = problem {
            self.as_mut().apply_status(ServiceStatus::default()); // no service: not on, not connected
            self.as_mut().set_status_text(QString::from(&text));
            self.as_mut().set_connecting(false);
            stop_service(slot);
            return;
        }
        // The old service is let go on the start thread: from now on nothing
        // is on or connected until the new one says so (a host name lookup
        // can take seconds).
        self.as_mut().apply_status(ServiceStatus::default());
        self.as_mut().set_status_text(QString::from(&tr("Connecting …")));
        self.as_mut().set_connecting(true);
        let events = self.qt_thread();
        let done = self.qt_thread();
        let spawned = thread::Builder::new().name("pshb-start".into()).spawn(move || {
            let _one_at_a_time = slot.starting.lock().unwrap();
            if !slot.is_current(generation) {
                return;
            }
            let old = slot.service.lock().unwrap().take();
            drop(old); // releases devices and board, frees the packet counter
            let mut addr = None;
            let result = config::load_psk(&config::psk_path())
                .and_then(|key| Ok((key, resolve(&cfg.board)?)))
                .and_then(|(key, board)| {
                    addr = Some(board);
                    let sink = Box::new(move |ev: ServiceEvent| {
                        let _ = events.queue(move |obj| obj.on_service_event(generation, ev));
                    });
                    let sc = ServiceConfig {
                        config: cfg,
                        key,
                        board,
                        counter_store: Some(config::counter_path()),
                        adapter_state: Some(config::adapter_state_path()),
                    };
                    Service::start(sc, sink)
                });
            match result {
                Ok(service) if slot.is_current(generation) => {
                    *slot.service.lock().unwrap() = Some(service);
                    let _ = done.queue(move |obj| obj.service_started(generation, addr));
                }
                Ok(_) => {} // superseded: dropped here
                Err(e) => {
                    let _ = done.queue(move |obj| obj.service_failed(generation, e));
                }
            }
        });
        if let Err(e) = spawned {
            self.as_mut().set_last_error(QString::from(&e.to_string()));
        }
    }

    fn service_started(mut self: Pin<&mut Self>, generation: u64, addr: Option<std::net::SocketAddr>) {
        if !self.rust().slot.is_current(generation) {
            return;
        }
        self.as_mut().rust_mut().board_addr = addr;
        self.as_mut().set_connecting(false);
        self.as_mut().rust_mut().health.reset(std::time::Instant::now());
        self.as_mut().schedule_name_check(generation); // in case it never answers
        self.as_mut().set_last_error(QString::default());
        // A new service knows nothing of what the GUI asked the old one for.
        if self.rust().identify_on {
            self.command(ServiceCmd::Identify(true));
        }
        if self.rust().latency_view {
            self.command(ServiceCmd::PingInterval(LATENCY_VIEW_PING));
        }
        if *self.recording() {
            self.command(ServiceCmd::Record(true));
        }
        if let Some(w) = self.rust().window_state.clone() {
            self.command(ServiceCmd::Window(w));
        }
    }

    fn service_failed(mut self: Pin<&mut Self>, generation: u64, error: String) {
        if !self.rust().slot.is_current(generation) {
            return;
        }
        pshb_core::logln!("PS HID Bridge: starting the bridge: {error}");
        // The old service is gone (dropped before the start): switch and
        // tray must not keep showing it as on and connected.
        self.as_mut().apply_status(ServiceStatus::default());
        self.as_mut().set_status_text(QString::from(&tr("Not connected")));
        self.as_mut().set_connecting(false);
        self.as_mut()
            .set_last_error(QString::from(&trf("%1 (trying again)", &[&error])));
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-retry".into()).spawn(move || {
            thread::sleep(RETRY);
            let _ = qt.queue(move |obj| obj.retry_service(generation));
        });
    }

    /// While the board does not answer: if its address is a host name,
    /// look it up again every RENAME_CHECK and reconnect when it changed.
    fn schedule_name_check(self: Pin<&mut Self>, generation: u64) {
        let board = self.config_ref().board.clone();
        let bare = board
            .strip_prefix('[')
            .and_then(|b| b.strip_suffix(']'))
            .unwrap_or(&board);
        if board.parse::<std::net::SocketAddr>().is_ok() || bare.parse::<std::net::IpAddr>().is_ok() {
            // An address: nothing to look up, but the board may have got
            // another one.
            self.schedule_follow(generation);
            return;
        }
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-name".into()).spawn(move || {
            thread::sleep(RENAME_CHECK);
            let found = resolve(&board).ok();
            let _ = qt.queue(move |obj| obj.name_checked(generation, found));
        });
    }

    fn name_checked(mut self: Pin<&mut Self>, generation: u64, found: Option<std::net::SocketAddr>) {
        if !self.rust().slot.is_current(generation) || *self.connected() {
            return;
        }
        match found {
            Some(a) if self.rust().board_addr.is_some_and(|old| old != a) => self.restart_service(),
            _ => self.as_mut().schedule_name_check(generation),
        }
    }

    /// While the board does not answer at its address: look for its MAC
    /// address in the network (after `follow_wait`) and take its new address.
    fn schedule_follow(mut self: Pin<&mut Self>, generation: u64) {
        let mac = self.config_ref().board_mac.clone();
        if mac.is_empty() {
            return; // not known (yet): learned once it answers
        }
        let max = if *self.active() { FOLLOW_MAX_ON } else { FOLLOW_MAX };
        let wait = self.rust().follow_wait.min(max);
        self.as_mut().rust_mut().follow_wait = (wait * 2).min(max);
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-follow".into()).spawn(move || {
            thread::sleep(wait);
            let found = discover::discover(discover::DISCOVER_TIMEOUT)
                .ok()
                .and_then(|list| list.into_iter().find(|b| b.mac_string() == mac));
            let _ = qt.queue(move |obj| obj.follow_checked(generation, found));
        });
    }

    fn follow_checked(mut self: Pin<&mut Self>, generation: u64, found: Option<FoundBoard>) {
        if !self.rust().slot.is_current(generation) || *self.connected() {
            return;
        }
        let board = self.config_ref().board.clone();
        let Some(b) = found.filter(|b| board_ipv4(&board) != Some(b.ip)) else {
            self.schedule_follow(generation);
            return;
        };
        let address = b.moved_from(&board);
        self.as_mut().rust_mut().config.board = address.clone();
        // Also when the file cannot be written: the board is here now
        // (save_error tells).
        self.as_mut().save_config();
        self.as_mut().rust_mut().follow_wait = FOLLOW_FIRST;
        self.as_mut().set_board_info_json(QString::default());
        self.as_mut().set_board_info_error(QString::default());
        notify::send(
            &tr("Board found again"),
            &trf("%1 has a new address in the network: %2.", &[&b.name, &address]),
        );
        self.as_mut().publish_config();
        // A running service only sends elsewhere from now on: the bridge
        // stays as it is (on stays on), nothing is let go of or grabbed anew.
        let running = self.rust().slot.service.lock().unwrap().is_some();
        match config::board_socket_addr(&address) {
            Ok(addr) if running => {
                self.as_mut().rust_mut().board_addr = Some(addr);
                self.command(ServiceCmd::Board(addr));
            }
            _ => self.restart_service(),
        }
    }

    /// Once per service start, when the board answers: its MAC address from
    /// its discovery answer, so it can be found again under another address.
    fn learn_board_mac(mut self: Pin<&mut Self>) {
        let generation = self.rust().slot.generation.load(Ordering::SeqCst);
        if self.rust().mac_checked == generation {
            return;
        }
        self.as_mut().rust_mut().mac_checked = generation;
        let Some(std::net::IpAddr::V4(ip)) = self.rust().board_addr.map(|a| a.ip()) else {
            return;
        };
        let qt = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-board-mac".into())
            .spawn(move || {
                let found = discover::probe(ip, protocol::DISCOVERY_PORT, discover::DISCOVER_TIMEOUT)
                    .ok()
                    .flatten();
                let _ = qt.queue(move |obj| obj.board_mac_learned(generation, found));
            });
    }

    fn board_mac_learned(mut self: Pin<&mut Self>, generation: u64, found: Option<FoundBoard>) {
        // Older firmware does not answer: nothing to learn.
        let Some(b) = found else { return };
        if !self.rust().slot.is_current(generation) {
            return;
        }
        self.as_mut().set_board_name(QString::from(&b.name));
        // It answered with the key: this is the board, whatever was stored.
        let mac = b.mac_string();
        if self.config_ref().board_mac != mac {
            self.as_mut().rust_mut().config.board_mac = mac.clone();
            self.as_mut().save_config();
            self.as_mut().set_board_mac(QString::from(&mac));
        }
    }

    fn retry_service(self: Pin<&mut Self>, generation: u64) {
        if self.rust().slot.is_current(generation) {
            self.restart_service();
        }
    }

    /// The window rule or a profile follows the focused window.
    fn wants_window(&self) -> bool {
        let c = self.config_ref();
        c.window_rule_active() || c.profiles.iter().any(|p| !p.window.trim().is_empty())
    }

    /// Watch the focused window while the window rule or a profile needs
    /// it (started and stopped as that changes); the service learns the
    /// window from `window_state`.
    fn update_window_watcher(mut self: Pin<&mut Self>) {
        let wanted = self.wants_window();
        if wanted == self.rust().window_watcher.is_some() {
            return;
        }
        self.as_mut().rust_mut().window_watcher = None;
        self.as_mut().rust_mut().window_state = None;
        if !wanted {
            self.command(ServiceCmd::Window(None));
            return;
        }
        let qt = self.qt_thread();
        let started = window::detect().and_then(|b| {
            window::Watcher::start(b, move |info| {
                let _ = qt.queue(move |obj| obj.window_changed(info));
            })
        });
        match started {
            Ok(w) => self.as_mut().rust_mut().window_watcher = Some(w),
            Err(e) => self.as_mut().set_last_error(QString::from(&trf(
                "Window rule and window profiles not available: %1",
                &[&e],
            ))),
        }
    }

    fn window_changed(mut self: Pin<&mut Self>, info: Option<WindowInfo>) {
        if self.rust().window_watcher.is_none() {
            return; // stopped meanwhile
        }
        self.as_mut().rust_mut().window_state = Some(info.clone());
        self.command(ServiceCmd::Window(info));
    }

    /// What the PlayStation receives, as text and chips in the app's
    /// language (None: nothing, the bridge is off).
    fn show_live(mut self: Pin<&mut Self>, state: Option<KeyState>) {
        self.as_mut().rust_mut().last_live = state;
        // The buttons first: whoever reacts to the text sees them new too.
        self.as_mut()
            .set_live_mouse_buttons(state.map_or(0, |s| i32::from(s.buttons)));
        let Some(s) = state else {
            self.as_mut().set_live_text(QString::default());
            self.as_mut().set_live_chips_json(QString::from("[]"));
            return;
        };
        let (keys, mouse) = live_parts(&s);
        let mut text = if keys.is_empty() {
            tr("no key")
        } else {
            keys.join(" + ")
        };
        let mut chips = keys;
        if let Some(m) = mouse {
            text += &format!(" · {m}");
            chips.push(m);
        }
        self.as_mut().set_live_text(QString::from(&text));
        self.as_mut()
            .set_live_chips_json(QString::from(&serde_json::to_string(&chips).unwrap_or_default()));
    }

    fn apply_status(mut self: Pin<&mut Self>, st: ServiceStatus) {
        if *self.connected() && !st.connected {
            let generation = self.rust().slot.generation.load(Ordering::SeqCst);
            self.as_mut().schedule_name_check(generation);
        } else if !*self.connected() && st.connected {
            self.as_mut().rust_mut().follow_wait = FOLLOW_FIRST;
            self.as_mut().learn_board_mac();
        }
        self.as_mut().set_connected(st.connected);
        self.as_mut()
            .set_counter_error(QString::from(st.counter_error.as_deref().unwrap_or("")));
        self.as_mut().set_active(st.active);
        self.as_mut().set_pending(st.pending);
        self.as_mut().set_paused(st.paused);
        self.as_mut().set_rtt_ms(st.rtt_ms.unwrap_or(-1.0));
        self.as_mut().set_rtt_p50(st.rtt_p50_ms.unwrap_or(-1.0));
        self.as_mut().set_rtt_p95(st.rtt_p95_ms.unwrap_or(-1.0));
        let rounded: Vec<f64> = st
            .rtt_history
            .iter()
            .map(|v| (v * 100.0).round() / 100.0)
            .collect();
        self.as_mut().set_rtt_history_json(QString::from(
            &serde_json::to_string(&rounded).unwrap_or_default(),
        ));
        self.as_mut().set_up_p50(st.up_p50_ms.unwrap_or(-1.0));
        self.as_mut().set_up_p95(st.up_p95_ms.unwrap_or(-1.0));
        // Microsecond steps: the one-way time is often below 0.1 ms.
        let up: Vec<f64> = st
            .up_history
            .iter()
            .map(|v| (v * 1000.0).round() / 1000.0)
            .collect();
        self.as_mut()
            .set_up_history_json(QString::from(&serde_json::to_string(&up).unwrap_or_default()));
        let ms = |us: u32| f64::from(us) / 1000.0;
        let lat = st.input_latency;
        self.as_mut().set_input_p50(lat.map_or(-1.0, |l| ms(l.p50_us)));
        self.as_mut().set_input_p95(lat.map_or(-1.0, |l| ms(l.p95_us)));
        self.as_mut().set_input_max(lat.map_or(-1.0, |l| ms(l.max_us)));
        self.as_mut().set_realtime(st.realtime);
        self.as_mut().set_packets_per_second(st.packets_per_second as i32);
        self.as_mut().set_devices_open(st.devices_open as i32);
        self.as_mut().set_devices_blocked(st.devices_blocked as i32);
        self.as_mut()
            .set_input_resyncs(st.input_resyncs.min(i32::MAX as u64) as i32);
        self.as_mut()
            .set_cpu_wake_state(QString::from(st.cpu_wake.as_str()));
        let a = st.adapter.as_ref();
        let mode = match a {
            None => "",
            Some(a) if a.managed => "managed",
            Some(a) if a.at_once_now => "fixed",
            Some(_) => "delaying",
        };
        self.as_mut()
            .set_adapter_iface(QString::from(a.map_or("", |a| a.iface.as_str())));
        // A lone packet waits up to three rounds of the driver's timer.
        self.as_mut()
            .set_adapter_delay_ms(a.map_or(0.0, |a| 3.0 * f64::from(a.timer_us) / 1000.0));
        self.as_mut().set_adapter_mode(QString::from(mode));
        self.as_mut()
            .set_adapter_at_once_now(a.is_some_and(|a| a.at_once_now));
        if let Some(p) = &st.board {
            let warning = self
                .as_mut()
                .rust_mut()
                .health
                .update(p, std::time::Instant::now());
            if let Some(w) = warning {
                let text = match w {
                    HealthWarning::WrongKey => tr(
                        "The board refuses packets with a wrong key: another PC (or program) sends with an old key. Pair it again, or switch it off.",
                    ),
                    HealthWarning::Replayed => tr(
                        "The board refuses repeated packets: a second PC sends with the same key. Use the bridge on one PC at a time.",
                    ),
                };
                pshb_core::logln!("PS HID Bridge: {w:?} (board counters rose)");
                self.as_mut().set_board_warning(QString::from(&text));
            }
            self.as_mut()
                .set_board_firmware(QString::from(&p.fw_version_string()));
            self.as_mut().set_board_usb(QString::from(usb_text(p)));
            self.as_mut()
                .set_board_no_partition_table(p.status & protocol::STATUS_NO_PARTITION_TABLE != 0);
            self.as_mut()
                .set_board_timed_ping(p.status & protocol::STATUS_TIMED_PING != 0);
            if let Some((want, _)) = self.rust().board_expected
                && Version::of_firmware(p.fw_version) == want
            {
                self.as_mut().rust_mut().board_expected = None;
                self.as_mut().set_update_busy(false);
                self.as_mut()
                    .update_message("ok", &trf("The board runs firmware %1.", &[&want.to_string()]));
            }
        } else if !self.board_firmware().is_empty() || !self.board_usb().is_empty() {
            // Not reachable: what it said last no longer holds.
            self.as_mut().set_board_firmware(QString::default());
            self.as_mut().set_board_usb(QString::default());
            self.as_mut().set_board_no_partition_table(false);
            self.as_mut().set_board_timed_ping(false);
        }
        self.as_mut().refresh_board_update(st.connected);
        let text = if !st.connected {
            tr("Board not reachable")
        } else if st.active {
            tr("Bridge on")
        } else if st.pending {
            tr("Please release all keys …")
        } else if st.paused {
            tr("Paused")
        } else {
            tr("Ready")
        };
        self.as_mut().set_status_text(QString::from(&text));
        // Profiles: the one in effect, and a choice by a trigger is kept.
        if !st.profile.is_empty() {
            let before = self.effective_profile().to_string();
            if !before.is_empty() && before != st.profile {
                notify::send(&tr("Profile"), &st.profile);
            }
            self.as_mut().set_effective_profile(QString::from(&st.profile));
            if *self.active_profile() != QString::from(&st.manual_profile)
                && let Some(i) = self.config_ref().profile_index(&st.manual_profile)
            {
                self.as_mut().rust_mut().config.active_profile = if i == 0 {
                    String::new()
                } else {
                    st.manual_profile.clone()
                };
                self.as_mut().save_config();
                self.as_mut().publish_config(); // also the tray's menu
            }
        } else {
            self.as_mut().set_effective_profile(QString::default());
        }
        self.as_mut().rust_mut().last_status = st.clone();
        let tray_state = Some((st.active, st.connected));
        if self.rust().tray_state != tray_state {
            self.as_mut().rust_mut().tray_state = tray_state;
            if let Some(t) = &self.rust().tray {
                t.update(|tray| {
                    tray.active = st.active;
                    tray.connected = st.connected;
                });
            }
        }
    }

    fn on_service_event(mut self: Pin<&mut Self>, generation: u64, ev: ServiceEvent) {
        // From a service already replaced or stopped (still queued, or sent
        // while it was let go): it would show its state as the current one.
        if !self.rust().slot.is_current(generation) {
            return;
        }
        match ev {
            ServiceEvent::Status(st) => self.as_mut().apply_status(*st),
            ServiceEvent::Notice(n) => {
                let msg = match n {
                    Notice::Activated => {
                        Some((tr("Bridge on"), tr("Mouse and keyboard go to the PlayStation.")))
                    }
                    Notice::Deactivated => Some((
                        tr("Bridge off"),
                        tr("Mouse and keyboard belong to the desktop again."),
                    )),
                    Notice::Emergency => Some((tr("Emergency stop"), tr("All devices were released."))),
                    Notice::Paused => Some((tr("Paused"), tr("Input is not forwarded."))),
                    _ => None,
                };
                if let Some((summary, body)) = msg {
                    notify::send(&summary, &body);
                }
                if matches!(n, Notice::Deactivated | Notice::Emergency) {
                    self.as_mut().show_live(None);
                }
            }
            ServiceEvent::Recorded(text) => {
                self.as_mut().set_recorded(QString::from(&text));
                self.as_mut().set_recording(false);
            }
            ServiceEvent::Identified(path) => {
                self.as_mut()
                    .set_identified_path(QString::from(&path.display().to_string()));
            }
            ServiceEvent::Live { state, device, .. } => {
                self.as_mut().show_live(Some(state));
                if !device.is_empty() {
                    self.as_mut().set_live_device(QString::from(&device));
                }
            }
            ServiceEvent::Error(e) => self.as_mut().set_last_error(QString::from(&e)),
            ServiceEvent::Typing {
                done,
                total,
                finished,
                error,
            } => {
                self.as_mut().set_typing_progress(if total > 0 {
                    done as f64 / total as f64
                } else {
                    1.0
                });
                if finished {
                    self.as_mut().set_typing_busy(false);
                    let (level, text) = match error {
                        None => ("ok", tr("Typed on the PlayStation.")),
                        Some(TypingStop::BridgeOn) => ("warn", tr("Switch the bridge off first.")),
                        Some(TypingStop::NotReady) => (
                            "warn",
                            tr("The PlayStation is off, asleep or the board is not reachable."),
                        ),
                        Some(TypingStop::Cancelled) => ("warn", tr("Stopped before the end.")),
                    };
                    self.as_mut().set_typing_level(QString::from(level));
                    self.as_mut().set_typing_message(QString::from(&text));
                }
            }
        }
    }

    pub fn toggle(self: Pin<&mut Self>) {
        self.command(ServiceCmd::Action(Action::Toggle));
    }

    pub fn set_on(self: Pin<&mut Self>, on: bool) {
        self.command(ServiceCmd::Action(if on { Action::On } else { Action::Off }));
    }

    pub fn pause(self: Pin<&mut Self>) {
        self.command(ServiceCmd::Action(Action::Pause));
    }

    pub fn release_all(self: Pin<&mut Self>) {
        self.command(ServiceCmd::Action(Action::ReleaseAll));
    }

    pub fn save_board(mut self: Pin<&mut Self>, address: &QString, psk: &QString) -> QString {
        let address = String::from(address).trim().to_string();
        // Only the form here: looking up a host name could block the UI
        // (the service start does that on its own thread).
        if let Err(e) = check_address(&address) {
            return QString::from(&e);
        }
        let psk = String::from(psk);
        let key_path = config::psk_path();
        let new_key = if !psk.trim().is_empty() {
            match parse_key_hex(&psk) {
                Ok(k) => Some(k),
                Err(e) => return QString::from(&e),
            }
        } else if !key_path.exists() {
            return QString::from(&tr("Please paste the key from the board's web interface."));
        } else {
            None
        };
        // Address and key are saved together or not at all: a new key with
        // the old address (or the other way round) would not connect at the
        // next start.
        let old_key = new_key.and_then(|_| config::load_psk(&key_path).ok());
        if let Some(k) = &new_key {
            if let Err(e) = config::save_psk(&key_path, k) {
                return QString::from(&e);
            }
        }
        // The MAC address of a board picked from the search; else learned
        // when it answers (learn_board_mac).
        let found_mac = board_ipv4(&address)
            .and_then(|ip| self.rust().found.iter().find(|b| b.ip == ip))
            .map(FoundBoard::mac_string);
        let old_mac = self.config_ref().board_mac.clone();
        let new_mac = match found_mac {
            Some(m) => m,
            None if self.config_ref().board == address => old_mac.clone(),
            None => String::new(),
        };
        let old_board = std::mem::replace(&mut self.as_mut().rust_mut().config.board, address.clone());
        self.as_mut().rust_mut().config.board_mac = new_mac;
        if !self.as_mut().save_config() {
            self.as_mut().rust_mut().config.board = old_board;
            self.as_mut().rust_mut().config.board_mac = old_mac;
            if new_key.is_some() {
                let _ = match &old_key {
                    Some(k) => config::save_psk(&key_path, k),
                    None => std::fs::remove_file(&key_path).map_err(|e| e.to_string()),
                };
            }
            return self.save_error().clone();
        }
        // An error of the old settings would read as the new ones' result.
        self.as_mut().set_last_error(QString::default());
        if old_board != address {
            // The latency page shows the new board's figures only.
            self.as_mut().set_board_info_json(QString::default());
            self.as_mut().set_board_info_error(QString::default());
            self.as_mut().set_board_name(QString::default());
            self.as_mut().set_rename_message(QString::default());
        }
        self.as_mut().publish_config();
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn reconnect(self: Pin<&mut Self>) {
        self.restart_service();
    }

    pub fn rename_board(mut self: Pin<&mut Self>, name: &QString) {
        if *self.rename_busy() {
            return;
        }
        let name = String::from(name).trim().to_string();
        self.as_mut().set_rename_busy(true);
        self.as_mut().set_rename_message(QString::default());
        let done = self.qt_thread();
        let api = self.board_api();
        let spawned = thread::Builder::new().name("pshb-rename".into()).spawn(move || {
            let result = api().and_then(|(api, _one_at_a_time)| api.set_name(&name));
            let _ = done.queue(move |mut obj| {
                obj.as_mut().set_rename_busy(false);
                match result {
                    Ok((shown, host)) => {
                        obj.as_mut().set_board_name(QString::from(&shown));
                        obj.as_mut().set_rename_level(QString::from("ok"));
                        obj.as_mut().set_rename_message(QString::from(&trf(
                            "Renamed. In your router and the browser the board is now called %1.",
                            &[&host],
                        )));
                        // The list shows the new name.
                        obj.as_mut().discover_boards();
                    }
                    Err(e) => {
                        obj.as_mut().set_rename_level(QString::from("warn"));
                        obj.as_mut().set_rename_message(QString::from(&e));
                    }
                }
            });
        });
        if spawned.is_err() {
            self.as_mut().set_rename_busy(false);
        }
    }

    pub fn network_name_for(&self, name: &QString, fallback: &QString) -> QString {
        QString::from(&discover::hostname_from_name(
            String::from(name).trim(),
            &String::from(fallback),
        ))
    }

    pub fn discover_boards(mut self: Pin<&mut Self>) {
        if *self.discover_busy() {
            return;
        }
        self.as_mut().set_discover_busy(true);
        let done = self.qt_thread();
        let spawned = thread::Builder::new()
            .name("pshb-discover".into())
            .spawn(move || {
                // No network at all: an empty list says the same.
                let found = discover::discover(discover::DISCOVER_TIMEOUT).unwrap_or_default();
                let _ = done.queue(move |mut obj| {
                    obj.as_mut().set_discover_busy(false);
                    obj.as_mut().rust_mut().found = found;
                    obj.as_mut().publish_found();
                });
            });
        if spawned.is_err() {
            self.as_mut().set_discover_busy(false);
        }
    }

    pub fn refresh_devices(mut self: Pin<&mut Self>) {
        let list = match input::list_devices() {
            Ok(l) => l.devices,
            Err(e) => {
                self.as_mut()
                    .set_last_error(QString::from(&trf("Device list: %1", &[&e.to_string()])));
                return;
            }
        };
        let groups = group_devices(&list, self.config_ref());
        let selected = input::select(&list, &self.config_ref().devices);
        let unwatched = input::unwatched_keyboards(&list, &selected);
        self.as_mut().set_other_keyboards_blocked(unwatched as i32);
        self.as_mut().rust_mut().devices = list;
        self.as_mut()
            .set_devices_json(QString::from(&serde_json::to_string(&groups).unwrap_or_default()));
    }

    pub fn set_device_selected(mut self: Pin<&mut Self>, id: &QString, selected: bool) {
        // "Please select keyboard and mouse first." and the like: answered
        // by this very change, or stale.
        self.as_mut().set_access_message(QString::default());
        let id = String::from(id).to_lowercase();
        let list = self.rust().devices.clone();
        {
            let devices = &mut self.as_mut().rust_mut().config.devices;
            let in_group = |s: &String| list.iter().any(|d| d.id_string() == id && d.matches(s));
            // Off: also a selector by path or name that takes this device
            // (pshb-cli); other devices it took stay selected by their id.
            let broad: Vec<String> = devices
                .iter()
                .filter(|s| !selected && !s.eq_ignore_ascii_case(&id) && in_group(s))
                .cloned()
                .collect();
            devices.retain(|d| !d.eq_ignore_ascii_case(&id) && !broad.contains(d));
            for d in list
                .iter()
                .filter(|d| d.id_string() != id && broad.iter().any(|s| d.matches(s)))
            {
                let other = d.id_string();
                if !devices.iter().any(|s| s.eq_ignore_ascii_case(&other)) {
                    devices.push(other);
                }
            }
            if selected {
                devices.push(id);
            }
        }
        self.as_mut().save_config();
        self.as_mut().refresh_devices();
        self.as_mut().restart_service();
    }

    pub fn install_access(mut self: Pin<&mut Self>) {
        if *self.access_busy() {
            return;
        }
        let selected = input::select(&self.rust().devices, &self.config_ref().devices);
        if selected.is_empty() {
            self.as_mut()
                .set_access_message(QString::from(&tr("Please select keyboard and mouse first.")));
            return;
        }
        let rule = pshb_core::access::udev_rule(&selected);
        self.as_mut().set_access_busy(true);
        self.as_mut().set_access_message(QString::default());
        let qt = self.qt_thread();
        // pkexec waits for the password dialog: never on the UI thread.
        let spawned = thread::Builder::new().name("pshb-access".into()).spawn(move || {
            let result = pshb_core::access::install_rule(&rule);
            if result.is_ok() {
                // udev applies the ACLs asynchronously.
                thread::sleep(Duration::from_millis(500));
            }
            let _ = qt.queue(move |obj| obj.access_installed(result));
        });
        if let Err(e) = spawned {
            self.as_mut().set_access_busy(false);
            self.as_mut().set_access_message(QString::from(&e.to_string()));
        }
    }

    pub fn enable_keep_cpu_awake(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().config.keep_cpu_awake = on;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        // Takes effect at once, the bridge stays as it is.
        self.command(ServiceCmd::KeepCpuAwake(on));
    }

    pub fn setup_low_latency(self: Pin<&mut Self>) {
        self.change_low_latency(true);
    }

    pub fn remove_low_latency(self: Pin<&mut Self>) {
        self.change_low_latency(false);
    }

    /// Install (`on`) or remove the low-latency rule.
    fn change_low_latency(mut self: Pin<&mut Self>, on: bool) {
        if *self.latency_setup_busy() {
            return;
        }
        self.as_mut().set_latency_setup_busy(true);
        self.as_mut().set_latency_setup_message(QString::default());
        let qt = self.qt_thread();
        // pkexec waits for the password dialog: never on the UI thread.
        let spawned = thread::Builder::new().name("pshb-lowlat".into()).spawn(move || {
            let result = if on {
                pshb_core::access::install_latency_rule()
            } else {
                pshb_core::access::remove_latency_rule()
            };
            if result.is_ok() {
                // udev applies the ACLs asynchronously.
                thread::sleep(Duration::from_millis(500));
            }
            let _ = qt.queue(move |obj| obj.low_latency_changed(on, result));
        });
        if let Err(e) = spawned {
            self.as_mut().set_latency_setup_busy(false);
            self.as_mut()
                .set_latency_setup_message(QString::from(&e.to_string()));
        }
    }

    fn low_latency_changed(mut self: Pin<&mut Self>, on: bool, result: Result<(), String>) {
        self.as_mut().set_latency_setup_busy(false);
        self.as_mut().publish_rules();
        let text = match result {
            Ok(()) => {
                // The service looks at the adapter and the permissions again.
                self.command(ServiceCmd::CheckLowLatency);
                if on {
                    "Set up: from now on the network adapter sends at once while the app runs \
                     and bundles again when it is closed, without asking for a password."
                        .to_string()
                } else {
                    "Undone: the rule is gone, the network adapter bundles packets again (up to 3 × 400 µs), \
                     and the app no longer switches it."
                        .to_string()
                }
            }
            Err(e) => e,
        };
        self.as_mut().set_latency_setup_message(QString::from(&text));
    }

    pub fn remove_access(mut self: Pin<&mut Self>) {
        if *self.access_busy() {
            return;
        }
        if *self.active() || *self.pending() {
            self.as_mut()
                .set_access_message(QString::from(&tr("Switch the bridge off first.")));
            return;
        }
        self.as_mut().set_access_busy(true);
        self.as_mut().set_access_message(QString::default());
        let qt = self.qt_thread();
        let spawned = thread::Builder::new().name("pshb-access".into()).spawn(move || {
            let result = pshb_core::access::remove_rule();
            if result.is_ok() {
                thread::sleep(Duration::from_millis(500));
            }
            let _ = qt.queue(move |obj| obj.access_removed(result));
        });
        if let Err(e) = spawned {
            self.as_mut().set_access_busy(false);
            self.as_mut().set_access_message(QString::from(&e.to_string()));
        }
    }

    pub fn remove_system_changes(mut self: Pin<&mut Self>) {
        if *self.cleanup_busy() {
            return;
        }
        if *self.active() || *self.pending() {
            self.as_mut().set_cleanup_level(QString::from("warn"));
            self.as_mut()
                .set_cleanup_message(QString::from(&tr("Switch the bridge off first.")));
            return;
        }
        self.as_mut().set_cleanup_busy(true);
        self.as_mut().set_cleanup_message(QString::default());
        let qt = self.qt_thread();
        let spawned = thread::Builder::new().name("pshb-cleanup".into()).spawn(move || {
            let result = pshb_core::access::remove_all();
            if result.is_ok() {
                thread::sleep(Duration::from_millis(500));
            }
            let _ = qt.queue(move |obj| obj.system_changes_removed(result));
        });
        if let Err(e) = spawned {
            self.as_mut().set_cleanup_busy(false);
            self.as_mut().set_cleanup_level(QString::from("bad"));
            self.as_mut().set_cleanup_message(QString::from(&e.to_string()));
        }
    }

    fn system_changes_removed(mut self: Pin<&mut Self>, result: Result<(), String>) {
        self.as_mut().set_cleanup_busy(false);
        self.as_mut().publish_rules();
        match result {
            Ok(()) => {
                pshb_core::logln!("PS HID Bridge: the rules installed as root were removed");
                self.as_mut().set_cleanup_level(QString::from("ok"));
                self.as_mut().set_cleanup_message(QString::from(&trf(
                    "Removed: device access and low-latency rules. Settings and key are still there (%1).",
                    &[&config::config_dir().display().to_string()],
                )));
                self.as_mut().refresh_devices();
                // Devices still open keep working until they are closed: start over.
                self.restart_service();
            }
            Err(e) => {
                self.as_mut().set_cleanup_level(QString::from("bad"));
                self.as_mut().set_cleanup_message(QString::from(&e));
            }
        }
    }

    fn access_removed(mut self: Pin<&mut Self>, result: Result<(), String>) {
        self.as_mut().set_access_busy(false);
        self.as_mut().publish_rules();
        if let Err(e) = result {
            self.as_mut().set_access_message(QString::from(&e));
            return;
        }
        self.as_mut().refresh_devices();
        // Devices still open keep working until they are closed: start over.
        self.restart_service();
    }

    fn access_installed(mut self: Pin<&mut Self>, result: Result<(), String>) {
        self.as_mut().set_access_busy(false);
        self.as_mut().publish_rules();
        if let Err(e) = result {
            self.as_mut().set_access_message(QString::from(&e));
            return;
        }
        // The running service notices the new permissions by itself.
        self.as_mut().refresh_devices();
        let still = input::select(&self.rust().devices, &self.config_ref().devices)
            .iter()
            .filter(|d| !d.accessible)
            .count()
            + *self.other_keyboards_blocked() as usize;
        self.as_mut().set_access_message(if still > 0 {
            QString::from(
                "Rule installed, but no access yet. Please unplug and replug the devices once, \
                 or log in again.",
            )
        } else {
            QString::default()
        });
    }

    pub fn set_identify(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().identify_on = on;
        if !on {
            // The last device found stays marked otherwise.
            self.as_mut().set_identified_path(QString::default());
        }
        self.command(ServiceCmd::Identify(on));
    }

    pub fn record_trigger(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().set_recorded(QString::default());
        self.as_mut().set_recording(on);
        self.command(ServiceCmd::Record(on));
    }

    pub fn set_triggers(mut self: Pin<&mut Self>, json: &QString) -> QString {
        let triggers: Vec<TriggerConfig> = match serde_json::from_str(&String::from(json)) {
            Ok(t) => t,
            Err(e) => return QString::from(&trf("Invalid triggers: %1", &[&e.to_string()])),
        };
        let names: Vec<String> = self
            .config_ref()
            .profile_names()
            .iter()
            .map(|n| n.to_string())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        for t in &triggers {
            if let Err(e) = t.to_trigger(&names) {
                return QString::from(&e);
            }
        }
        let old = std::mem::replace(&mut self.as_mut().rust_mut().config.triggers, triggers);
        if !self.as_mut().save_config() {
            // Not saved, not used: the page keeps the edit and shows why.
            self.as_mut().rust_mut().config.triggers = old;
            return self.save_error().clone();
        }
        self.as_mut().publish_config();
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn set_window_rule_text(mut self: Pin<&mut Self>, text: &QString) {
        self.as_mut().rust_mut().config.window_rule = String::from(text).trim().to_string();
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().update_window_watcher();
        self.as_mut().restart_service();
    }

    pub fn set_window_rule_on(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().config.window_rule_enabled = on;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().update_window_watcher();
        self.as_mut().restart_service();
    }

    pub fn set_keyboard_type(mut self: Pin<&mut Self>, iso: bool) {
        self.as_mut().rust_mut().config.iso_keyboard = iso;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_service();
    }

    /// Sensitivity and acceleration of the profile "Standard" (wizard).
    pub fn set_mouse(self: Pin<&mut Self>, sensitivity: f64, accel: f64) {
        let m = self.config_ref().mouse.clone();
        self.set_profile_mouse(0, sensitivity, accel, m.invert_y, m.y_ratio, m.wheel_speed);
    }

    pub fn set_profile_mouse(
        mut self: Pin<&mut Self>,
        index: i32,
        sensitivity: f64,
        accel: f64,
        invert_y: bool,
        y_ratio: f64,
        wheel_speed: f64,
    ) {
        {
            // On the sliders' steps: "1.00x" is exactly 1 (raw input), not
            // 1.0000000000000002 from the slider's arithmetic.
            let (s, a) = (config::SENSITIVITY_RANGE, config::ACCEL_RANGE);
            let (y, w) = (config::Y_RATIO_RANGE, config::WHEEL_SPEED_RANGE);
            let mut r = self.as_mut().rust_mut();
            let m = r.config.profile_mouse_mut(index.max(0) as usize);
            m.sensitivity = round_to(sensitivity, 0.05).clamp(s.0, s.1);
            m.accel = round_to(accel, 0.01).clamp(a.0, a.1);
            m.invert_y = invert_y;
            m.y_ratio = round_to(y_ratio, 0.05).clamp(y.0, y.1);
            m.wheel_speed = round_to(wheel_speed, 0.05).clamp(w.0, w.1);
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().send_profiles();
    }

    /// The profiles' settings to the running service (the bridge stays as it
    /// is); a profile that cannot be used shows why.
    fn send_profiles(mut self: Pin<&mut Self>) {
        match self.config_ref().engine_profiles() {
            Ok(p) => self.command(ServiceCmd::Profiles(p)),
            Err(e) => self.as_mut().set_last_error(QString::from(&e)),
        }
    }

    pub fn set_profile_remap(mut self: Pin<&mut Self>, index: i32, json: &QString) -> QString {
        let list: Vec<config::RemapConfig> = match serde_json::from_str(&String::from(json)) {
            Ok(l) => l,
            Err(e) => return QString::from(&format!("{e}")),
        };
        let keymap = self.config_ref().keymap();
        for r in &list {
            if let Err(e) = r.parse(keymap) {
                return QString::from(&e);
            }
        }
        let index = index.max(0) as usize;
        let old = std::mem::replace(self.as_mut().rust_mut().config.profile_remap_mut(index), list);
        if !self.as_mut().save_config() {
            *self.as_mut().rust_mut().config.profile_remap_mut(index) = old;
            return self.save_error().clone();
        }
        self.as_mut().publish_config();
        self.as_mut().send_profiles();
        QString::default()
    }

    pub fn set_profile_window(mut self: Pin<&mut Self>, index: i32, pattern: &QString) {
        let Some(i) = (index as usize).checked_sub(1) else {
            return; // "Standard" applies when no other one does
        };
        {
            let mut r = self.as_mut().rust_mut();
            let Some(p) = r.config.profiles.get_mut(i) else {
                return;
            };
            p.window = String::from(pattern).trim().to_string();
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().update_window_watcher();
        self.as_mut().send_profiles();
    }

    pub fn add_profile(mut self: Pin<&mut Self>, name: &QString, copy_of: i32) -> QString {
        let name = String::from(name).trim().to_string();
        if let Err(e) = self.config_ref().check_profile_name(&name, None) {
            return QString::from(&e);
        }
        let from = copy_of.max(0) as usize;
        let profile = config::ProfileConfig {
            name,
            mouse: self.config_ref().profile_mouse(from).clone(),
            remap: self.config_ref().profile_remap(from).to_vec(),
            ..Default::default()
        };
        self.as_mut().rust_mut().config.profiles.push(profile);
        if !self.as_mut().save_config() {
            self.as_mut().rust_mut().config.profiles.pop();
            return self.save_error().clone();
        }
        self.as_mut().publish_config();
        // A new profile: the service learns it with a restart (the bridge
        // goes off), as with changed triggers.
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn rename_profile(mut self: Pin<&mut Self>, index: i32, name: &QString) -> QString {
        let name = String::from(name).trim().to_string();
        let index = index.max(0) as usize;
        if index == 0 {
            return QString::from(&tr("The profile \"Standard\" keeps its name."));
        }
        if let Err(e) = self.config_ref().check_profile_name(&name, Some(index)) {
            return QString::from(&e);
        }
        {
            let mut r = self.as_mut().rust_mut();
            let c = &mut r.config;
            let Some(p) = c.profiles.get_mut(index - 1) else {
                return QString::default();
            };
            let old = std::mem::replace(&mut p.name, name.clone());
            // Triggers and the choice name the profile.
            for t in &mut c.triggers {
                let named = t.action.trim().strip_prefix("profile:").map(str::trim);
                if named.is_some_and(|n| n.to_lowercase() == old.to_lowercase()) {
                    t.action = format!("profile:{name}");
                }
            }
            if c.active_profile.to_lowercase() == old.to_lowercase() {
                c.active_profile = name;
            }
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn delete_profile(mut self: Pin<&mut Self>, index: i32) {
        let index = index.max(0) as usize;
        if index == 0 || index > self.config_ref().profiles.len() {
            return;
        }
        {
            let mut r = self.as_mut().rust_mut();
            let c = &mut r.config;
            let gone = c.profiles.remove(index - 1).name.to_lowercase();
            // Triggers that chose it now go to the next profile.
            for t in &mut c.triggers {
                let named = t.action.trim().strip_prefix("profile:").map(str::trim);
                if named.is_some_and(|n| n.to_lowercase() == gone) {
                    t.action = "next-profile".into();
                }
            }
            if c.active_profile.to_lowercase() == gone {
                c.active_profile.clear();
            }
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().update_window_watcher();
        self.as_mut().restart_service();
    }

    pub fn choose_profile(mut self: Pin<&mut Self>, index: i32) {
        let index = index.max(0) as usize;
        let Some(name) = self
            .config_ref()
            .profile_names()
            .get(index)
            .map(|n| n.to_string())
        else {
            return;
        };
        let stored = if index == 0 { String::new() } else { name };
        if self.config_ref().active_profile != stored {
            self.as_mut().rust_mut().config.active_profile = stored;
            self.as_mut().save_config();
            self.as_mut().publish_config();
        }
        self.command(ServiceCmd::Action(Action::Profile(index as u16)));
    }

    pub fn profile_curve_gain(&self, index: i32, speed: f64) -> f64 {
        self.config_ref()
            .profile_mouse(index.max(0) as usize)
            .curve()
            .gain(speed)
    }

    pub fn choose_ps_layout(mut self: Pin<&mut Self>, id: &QString) {
        let id = String::from(id);
        if typing::layout(&id).is_none() {
            return;
        }
        self.as_mut().rust_mut().config.ps_layout = id;
        self.as_mut().save_config();
        self.as_mut().publish_config();
    }

    pub fn unsupported_chars(&self, text: &QString) -> QString {
        let layout = typing::layout_or_default(&self.config_ref().ps_layout);
        let missing: String = layout.unsupported(&String::from(text)).into_iter().collect();
        QString::from(&missing)
    }

    pub fn type_text(mut self: Pin<&mut Self>, text: &QString) {
        let layout = typing::layout_or_default(&self.config_ref().ps_layout);
        let text = String::from(text);
        let states = match typing::plan(&text, layout) {
            Ok(s) => s,
            Err(missing) => {
                let chars: String = missing.into_iter().collect();
                self.as_mut().set_typing_level(QString::from("bad"));
                self.as_mut().set_typing_message(QString::from(&trf(
                    "The PlayStation's keyboard language (%1) has no key for: %2",
                    &[layout.name, &chars],
                )));
                return;
            }
        };
        if self.rust().slot.service.lock().unwrap().is_none() {
            self.as_mut().set_typing_level(QString::from("warn"));
            self.as_mut()
                .set_typing_message(QString::from(&tr("The board is not connected.")));
            return;
        }
        self.as_mut().set_typing_message(QString::default());
        self.as_mut().set_typing_progress(0.0);
        self.as_mut().set_typing_busy(true);
        self.command(ServiceCmd::Type(states));
    }

    pub fn cancel_typing(self: Pin<&mut Self>) {
        self.command(ServiceCmd::CancelTyping);
    }

    pub fn pair_board(
        mut self: Pin<&mut Self>,
        address: &QString,
        password: &QString,
        new_password: &QString,
    ) {
        if *self.pair_busy() {
            return;
        }
        let address = String::from(address).trim().to_string();
        if let Err(e) = check_address(&address) {
            self.as_mut().set_pair_level(QString::from("bad"));
            self.as_mut().set_pair_message(QString::from(&e));
            return;
        }
        let password = String::from(password);
        let new_password = String::from(new_password);
        self.as_mut().set_pair_busy(true);
        self.as_mut().set_pair_message(QString::default());
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-pair".into()).spawn(move || {
            let result = resolve(&address).and_then(|a| {
                update::pair(
                    &a.ip().to_string(),
                    80,
                    &password,
                    Some(new_password.as_str()).filter(|p| !p.is_empty()),
                )
                .map_err(|e| match e {
                    update::PairError::WrongPassword => tr("Wrong password."),
                    update::PairError::LockedOut(m) => trf("Too many wrong passwords: %1", &[&m]),
                    update::PairError::NeedsNewPassword => NEEDS_NEW_PASSWORD.to_string(),
                    update::PairError::NewPasswordRefused(m) => {
                        trf("The new password was refused: %1", &[&m])
                    }
                    update::PairError::Other(m) => m,
                })
            });
            let _ = qt.queue(move |obj| obj.paired(address, result));
        });
    }

    fn paired(mut self: Pin<&mut Self>, address: String, result: Result<[u8; 32], String>) {
        self.as_mut().set_pair_busy(false);
        match result {
            Ok(key) => {
                let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
                let error = self
                    .as_mut()
                    .save_board(&QString::from(&address), &QString::from(&hex));
                self.as_mut().set_pair_needs_new_password(false);
                if error.is_empty() {
                    self.as_mut().set_pair_level(QString::from("ok"));
                    self.as_mut().set_pair_message(QString::from(&tr(
                        "Paired: the board made a new key and the app stored it.",
                    )));
                } else {
                    self.as_mut().set_pair_level(QString::from("bad"));
                    self.as_mut().set_pair_message(error);
                }
            }
            Err(e) if e == NEEDS_NEW_PASSWORD => {
                self.as_mut().set_pair_needs_new_password(true);
                self.as_mut().set_pair_level(QString::from("warn"));
                self.as_mut().set_pair_message(QString::from(&tr(
                    "The board still has its factory password: choose your own (8 to 64 characters).",
                )));
            }
            Err(e) => {
                self.as_mut().set_pair_level(QString::from("bad"));
                self.as_mut().set_pair_message(QString::from(&e));
            }
        }
    }

    pub fn save_diagnostics(mut self: Pin<&mut Self>, url: &QString) {
        if *self.diag_busy() {
            return;
        }
        let Some(path) = local_path(&String::from(url)) else {
            return;
        };
        let config = self.config_ref().clone();
        let status = format!("{:#?}", self.rust().last_status);
        let windows = self.window_backend().to_string();
        self.as_mut().set_diag_busy(true);
        self.as_mut().set_diag_message(QString::default());
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-diag".into()).spawn(move || {
            let text = {
                // One request to the board at a time (see BOARD_API).
                let _one = BOARD_API.lock().unwrap_or_else(|e| e.into_inner());
                pshb_core::diagnostics::report(&pshb_core::diagnostics::ReportInput {
                    config: &config,
                    program: "desktop app",
                    windows,
                    status: Some(status),
                    key: config::load_psk(&config::psk_path()).ok(),
                    http_port: 80,
                })
            };
            let result = std::fs::write(&path, text)
                .map(|_| path.display().to_string())
                .map_err(|e| format!("{}: {e}", path.display()));
            let _ = qt.queue(move |obj| obj.diagnostics_saved(result));
        });
    }

    fn diagnostics_saved(mut self: Pin<&mut Self>, result: Result<String, String>) {
        self.as_mut().set_diag_busy(false);
        match result {
            Ok(path) => {
                self.as_mut().set_diag_level(QString::from("ok"));
                self.as_mut()
                    .set_diag_message(QString::from(&trf("Saved: %1", &[&path])));
            }
            Err(e) => {
                self.as_mut().set_diag_level(QString::from("bad"));
                self.as_mut().set_diag_message(QString::from(&e));
            }
        }
    }

    pub fn dismiss_board_warning(self: Pin<&mut Self>) {
        self.set_board_warning(QString::default());
    }

    /// Running as an AppImage on a system with Shelly: is the app
    /// installed (offer it if not, or the installed copy), and does
    /// Shelly's record show the version that runs (tell it if not)?
    fn check_installation(self: Pin<&mut Self>) {
        let Some(running) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from) else {
            return;
        };
        if install::shelly().is_none() {
            return;
        }
        let qt = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-install-check".into())
            .spawn(move || {
                let found = install::list().map(|list| install::find(&list, &running));
                let _ = qt.queue(move |obj| obj.installation_checked(running, found));
            });
    }

    fn installation_checked(
        mut self: Pin<&mut Self>,
        running: std::path::PathBuf,
        found: Result<Found, String>,
    ) {
        let found = match found {
            Ok(f) => f,
            Err(e) => {
                pshb_core::logln!("PS HID Bridge: {e}");
                return;
            }
        };
        pshb_core::logln!(
            "PS HID Bridge: Shelly: {}",
            match &found {
                Found::ThisFile(i) => format!("installed as {} (version {})", i.name, i.version),
                Found::Elsewhere(i) => format!(
                    "installed at {} (version {}), this is another file",
                    i.path.display(),
                    i.version
                ),
                Found::NotInstalled => "not installed".into(),
            }
        );
        let folder = running
            .parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        self.as_mut().set_install_folder(QString::from(&folder));
        let offer = match &found {
            Found::ThisFile(i) => {
                // Installed and running: Shelly's record follows the
                // version the app updated itself to.
                if install::record_outdated(&i.version, pshb_core::VERSION) {
                    let name = i.name.clone();
                    let _ = thread::Builder::new()
                        .name("pshb-shelly-sync".into())
                        .spawn(move || {
                            if let Err(e) = install::sync(&name) {
                                pshb_core::logln!("PS HID Bridge: {e}");
                            }
                        });
                } else if install::menu_outdated() {
                    // Installed by hand: KDE's menu did not see the entry yet.
                    let _ = thread::Builder::new()
                        .name("pshb-menu".into())
                        .spawn(install::refresh_menu);
                }
                ""
            }
            Found::Elsewhere(i) => {
                self.as_mut().set_installed_version(QString::from(&i.version));
                "start-installed"
            }
            Found::NotInstalled => "install",
        };
        self.as_mut().rust_mut().installed = match found {
            Found::ThisFile(i) | Found::Elsewhere(i) => Some(i),
            Found::NotInstalled => None,
        };
        self.as_mut().set_install_offer(QString::from(offer));
    }

    pub fn install_with_shelly(mut self: Pin<&mut Self>) {
        if *self.install_busy() {
            return;
        }
        let Some(running) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from) else {
            return;
        };
        self.as_mut().set_install_busy(true);
        self.as_mut().set_install_message(QString::default());
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-install".into()).spawn(move || {
            let result = install::install(&running);
            let _ = qt.queue(move |obj| obj.installed_with_shelly(result));
        });
    }

    fn installed_with_shelly(mut self: Pin<&mut Self>, result: Result<install::Installed, String>) {
        self.as_mut().set_install_busy(false);
        match result {
            Ok(i) => {
                self.as_mut().set_install_level(QString::from("ok"));
                self.as_mut().set_install_message(QString::from(&trf(
                    "Installed: PS HID Bridge is in the application menu now (%1). The file you started can be deleted.",
                    &[&i.path.display().to_string()],
                )));
                self.as_mut().set_installed_version(QString::from(&i.version));
                self.as_mut().rust_mut().installed = Some(i);
                self.as_mut()
                    .set_install_offer(QString::from("restart-installed"));
            }
            Err(e) => {
                self.as_mut().set_install_level(QString::from("bad"));
                self.as_mut().set_install_message(QString::from(&e));
            }
        }
    }

    pub fn start_installed(self: Pin<&mut Self>) {
        let Some(path) = self.rust().installed.as_ref().map(|i| i.path.clone()) else {
            return;
        };
        self.restart_from(path);
    }

    pub fn dismiss_install(self: Pin<&mut Self>) {
        self.set_install_offer(QString::default());
    }

    pub fn set_app_language(mut self: Pin<&mut Self>, code: &QString) {
        let code = String::from(code);
        let code = if code == "de" { "de" } else { "en" };
        self.as_mut().rust_mut().config.language = code.into();
        self.as_mut().save_config();
        i18n::set_language(code);
        self.as_mut().publish_config();
        // Texts made in Rust follow at once too.
        if let Some(t) = &self.rust().tray {
            t.update(|_| {});
        }
        let st = self.rust().last_status.clone();
        self.as_mut().apply_status(st);
        let live = self.rust().last_live;
        self.as_mut().show_live(live);
    }

    pub fn set_touchpad(mut self: Pin<&mut Self>, speed: f64, tap: bool, natural: bool) {
        {
            let t = &mut self.as_mut().rust_mut().config.touchpad;
            let r = config::TOUCH_SPEED_RANGE;
            t.speed = round_to(speed, 0.05).clamp(r.0, r.1);
            t.tap_to_click = tap;
            t.natural_scroll = natural;
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        let settings = self.config_ref().touchpad.settings();
        self.command(ServiceCmd::Touchpad(settings));
    }

    pub fn curve_gain(&self, speed: f64) -> f64 {
        self.profile_curve_gain(0, speed)
    }

    pub fn set_latency_view(mut self: Pin<&mut Self>, visible: bool) {
        self.as_mut().rust_mut().latency_view = visible;
        let interval = if visible {
            LATENCY_VIEW_PING
        } else {
            Duration::from_secs(1)
        };
        self.command(ServiceCmd::PingInterval(interval));
    }

    pub fn finish_setup(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().config.setup_done = true;
        self.as_mut().save_config();
        self.as_mut().publish_config();
    }

    pub fn quit_app(mut self: Pin<&mut Self>) {
        self.as_mut().stop_everything();
        std::process::exit(0);
    }

    /// Release devices and board; a start still in progress is dropped.
    fn stop_everything(mut self: Pin<&mut Self>) {
        let slot = self.rust().slot.clone();
        slot.generation.fetch_add(1, Ordering::SeqCst);
        let service = slot.service.lock().unwrap().take();
        drop(service);
        self.as_mut().rust_mut().window_watcher = None;
    }

    // ------------------------------------------------------------ updates

    fn start_update_check(self: Pin<&mut Self>, manual: bool) {
        let qt = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-update-check".into())
            .spawn(move || {
                let result = update::latest_release();
                let _ = qt.queue(move |obj| obj.updates_found(result, manual));
            });
    }

    /// Timer tick (start and every six hours).
    fn periodic_update_check(self: Pin<&mut Self>) {
        if self.config_ref().update_check {
            self.start_update_check(false);
        }
    }

    pub fn check_updates(mut self: Pin<&mut Self>) {
        if !*self.update_busy() {
            self.as_mut().update_message("info", &tr("Looking for updates …"));
        }
        self.start_update_check(true);
    }

    pub fn choose_appearance(mut self: Pin<&mut Self>, value: &QString) {
        let value = String::from(value);
        if !["system", "light", "dark"].contains(&value.as_str()) {
            return;
        }
        self.as_mut().rust_mut().config.appearance = value;
        self.as_mut().save_config();
        self.as_mut().publish_config();
    }

    pub fn enable_update_check(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().config.update_check = on;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        if on {
            self.start_update_check(false);
        }
    }

    fn updates_found(mut self: Pin<&mut Self>, result: Result<Release, String>, manual: bool) {
        let release = match result {
            Ok(r) => r,
            Err(e) => {
                // An automatic check stays quiet (offline, rate limit).
                if manual && !*self.update_busy() {
                    self.as_mut()
                        .update_message("warn", &trf("Could not look for updates: %1", &[&e]));
                }
                return;
            }
        };
        let v = release.version;
        let newer_app = !*self.app_updated() && Version::of_this_program().is_some_and(|mine| v > mine);
        self.as_mut().set_latest_version(QString::from(&v.to_string()));
        self.as_mut().set_app_update_available(newer_app);
        self.as_mut().rust_mut().latest = Some(release);
        let connected = *self.connected();
        self.as_mut().refresh_board_update(connected);
        let any = newer_app || *self.board_update_available();
        if manual && !*self.update_busy() {
            if any {
                self.as_mut()
                    .update_message("info", &trf("Version %1 is available.", &[&v.to_string()]));
            } else {
                self.as_mut()
                    .update_message("ok", &tr("Everything is up to date."));
            }
        } else if any && !*self.update_busy() {
            // An earlier "Everything is up to date." (or an error) would stand
            // in the banner that now offers the update.
            self.as_mut().update_message("info", "");
        }
        if any && self.rust().notified != Some(v) {
            self.as_mut().rust_mut().notified = Some(v);
            notify::send(
                &tr("Update available"),
                &trf(
                    "PS HID Bridge %1 is available. Install it from the app's status page.",
                    &[&v.to_string()],
                ),
            );
        }
    }

    /// Board update available: the board answers and runs an older firmware.
    fn refresh_board_update(mut self: Pin<&mut Self>, connected: bool) {
        let latest = self.rust().latest.as_ref().map(|r| r.version);
        let board = Version::parse(&self.board_firmware().to_string());
        let available = connected && matches!((latest, board), (Some(l), Some(b)) if l > b);
        if *self.board_update_available() != available {
            self.as_mut().set_board_update_available(available);
        }
    }

    /// Set `update_status` and how it reads (`update_level`).
    fn update_message(mut self: Pin<&mut Self>, level: &str, text: &str) {
        self.as_mut().set_update_level(QString::from(level));
        self.as_mut().set_update_status(QString::from(text));
    }

    fn update_progress(mut self: Pin<&mut Self>, text: String) {
        if *self.update_busy() {
            self.as_mut().update_message("info", &text);
        }
    }

    pub fn update_app(mut self: Pin<&mut Self>) {
        if *self.update_busy() {
            return;
        }
        let Some(path) = appimage_path() else {
            self.as_mut().update_message(
                "warn",
                &tr("This app does not run as an AppImage in a writable folder: download the new version from the release page."),
            );
            return;
        };
        let Some(release) = self.rust().latest.clone() else {
            return;
        };
        self.as_mut().set_update_busy(true);
        self.as_mut().update_message("info", &tr("Downloading the app …"));
        let progress = self.qt_thread();
        let done = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-update-app".into())
            .spawn(move || {
                let mut shown = 0;
                let result =
                    update::download_verified(&release, &update::appimage_asset(), &mut |got, total| {
                        let pct = (got * 100).checked_div(total).unwrap_or(0);
                        if pct >= shown + 5 {
                            shown = pct;
                            let text = trf("Downloading the app: %1 %", &[&pct.to_string()]);
                            let _ = progress.queue(move |obj| obj.update_progress(text));
                        }
                    })
                    .and_then(|data| {
                        update::replace_file(&path, &data).map_err(|e| format!("{}: {e}", path.display()))
                    })
                    .map(|()| release.version);
                // Shelly's record follows (version, menu entry, icon).
                if result.is_ok() {
                    tell_shelly(&path);
                }
                let _ = done.queue(move |obj| obj.app_update_done(result));
            });
    }

    fn app_update_done(mut self: Pin<&mut Self>, result: Result<Version, String>) {
        self.as_mut().set_update_busy(false);
        match result {
            Ok(v) => {
                self.as_mut().set_app_updated(true);
                self.as_mut().set_app_update_available(false);
                self.as_mut().update_message(
                    "ok",
                    &trf(
                        "Version %1 is installed. Restart the app to use it.",
                        &[&v.to_string()],
                    ),
                );
            }
            Err(e) => self
                .as_mut()
                .update_message("warn", &trf("App update failed: %1", &[&e])),
        }
    }

    pub fn restart_app(self: Pin<&mut Self>) {
        let Some(path) = std::env::var_os("APPIMAGE") else {
            return;
        };
        self.restart_from(std::path::PathBuf::from(path));
    }

    /// Quit and start the AppImage at `path` (this one after an update, or
    /// the installed copy).
    fn restart_from(mut self: Pin<&mut Self>, path: std::path::PathBuf) {
        // The new instance needs the devices, the board's attention, the
        // packet counter and the instance lock.
        self.as_mut().stop_everything();
        crate::release_instance_lock();
        match install::host_command(&path).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                self.as_mut()
                    .update_message("warn", &trf("Restart failed: %1", &[&e.to_string()]));
                self.as_mut().restart_service();
            }
        }
    }

    /// The board's web API, for a background thread: it looks up the
    /// address there (a host name could block the UI) and waits for the
    /// app's other requests to the board to finish first.
    fn board_api(
        &self,
    ) -> impl FnOnce() -> Result<(update::BoardApi, MutexGuard<'static, ()>), String> + Send + 'static {
        let key = config::load_psk(&config::psk_path());
        let board = self.config_ref().board.clone();
        move || {
            let key = key?;
            let addr = resolve(&board)?;
            let api = update::BoardApi::new(&addr.ip().to_string(), BOARD_HTTP_PORT, &key);
            Ok((api, BOARD_API.lock().unwrap_or_else(|e| e.into_inner())))
        }
    }

    pub fn refresh_board_info(mut self: Pin<&mut Self>) {
        // Not while the board restarts into new firmware: it refuses or does
        // not answer until it is back, which its UDP answers show
        // (board_expected).
        if *self.board_info_busy() || self.rust().board_expected.is_some() {
            return;
        }
        let board = self.config_ref().board.clone();
        if board.is_empty() {
            return;
        }
        self.as_mut().set_board_info_busy(true);
        let done = self.qt_thread();
        let api = self.board_api();
        let _ = thread::Builder::new()
            .name("pshb-board-info".into())
            .spawn(move || {
                let result = api().and_then(|(api, _one_at_a_time)| {
                    let clock = api.clock()?;
                    let diag = api.diag()?;
                    let status = api.status()?;
                    Ok(serde_json::json!({ "status": status, "diag": diag, "clock": clock }).to_string())
                });
                let _ = done.queue(move |mut obj| {
                    obj.as_mut().set_board_info_busy(false);
                    if obj.config_ref().board != board {
                        return; // asked the board before the address changed
                    }
                    match result {
                        Ok(json) => {
                            obj.as_mut().set_board_info_json(QString::from(&json));
                            obj.as_mut().set_board_info_error(QString::default());
                        }
                        // Waited for an update and ran into the restart.
                        Err(_) if obj.rust().board_expected.is_some() => {}
                        Err(e) => obj.as_mut().set_board_info_error(QString::from(&e)),
                    }
                });
            });
    }

    pub fn reset_board_diag(self: Pin<&mut Self>) {
        let done = self.qt_thread();
        let api = self.board_api();
        let _ = thread::Builder::new()
            .name("pshb-board-reset".into())
            .spawn(move || {
                let result = api().and_then(|(api, _one_at_a_time)| api.diag_reset());
                let _ = done.queue(move |mut obj| {
                    if let Err(e) = result {
                        obj.as_mut().set_board_info_error(QString::from(&e));
                    }
                    obj.as_mut().refresh_board_info();
                });
            });
    }

    pub fn set_board_clock(mut self: Pin<&mut Self>, cpu_mhz: i32, spi_fast: bool) {
        if *self.clock_busy() {
            return;
        }
        if cpu_mhz != 150 && cpu_mhz != 200 {
            return;
        }
        self.as_mut().set_clock_busy(true);
        self.as_mut()
            .set_clock_message(QString::from(&tr("Storing the clock speeds …")));
        let done = self.qt_thread();
        let api = self.board_api();
        let _ = thread::Builder::new()
            .name("pshb-board-clock".into())
            .spawn(move || {
                let result = api().and_then(|(api, _one_at_a_time)| api.set_clock(cpu_mhz as u32, spi_fast));
                let _ = done.queue(move |mut obj| {
                    obj.as_mut().set_clock_busy(false);
                    let text = match result {
                        Ok(true) => tr("Stored. The board restarts and reconnects in a few seconds."),
                        Ok(false) => tr("Nothing changed."),
                        Err(e) => e,
                    };
                    obj.as_mut().set_clock_message(QString::from(&text));
                });
            });
    }

    pub fn update_board(mut self: Pin<&mut Self>) {
        if *self.update_busy() {
            return;
        }
        if *self.active() {
            self.as_mut().update_message(
                "warn",
                &tr("Switch the bridge off first: the board restarts during the update."),
            );
            return;
        }
        let Some(release) = self.rust().latest.clone() else {
            return;
        };
        let key = match config::load_psk(&config::psk_path()) {
            Ok(k) => k,
            Err(e) => {
                self.as_mut().update_message("warn", &e);
                return;
            }
        };
        let board = self.config_ref().board.clone();
        self.as_mut().set_update_busy(true);
        self.as_mut()
            .update_message("info", &tr("Downloading the firmware …"));
        let progress = self.qt_thread();
        let done = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-update-board".into())
            .spawn(move || {
                let result = update::download_verified(&release, update::FIRMWARE_ASSET, &mut |_, _| {})
                    .and_then(|image| {
                        let addr = resolve(&board)?;
                        let text = "Installing on the board …".to_string();
                        let _ = progress.queue(move |obj| obj.update_progress(text));
                        // The app's status requests wait meanwhile: the board
                        // serves two connections at a time.
                        let _one_at_a_time = BOARD_API.lock().unwrap_or_else(|e| e.into_inner());
                        update::board_ota(&addr.ip().to_string(), BOARD_HTTP_PORT, &key, &image)
                    })
                    .map(|()| release.version);
                let _ = done.queue(move |obj| obj.board_update_sent(result));
            });
    }

    fn board_update_sent(mut self: Pin<&mut Self>, result: Result<Version, String>) {
        let v = match result {
            Ok(v) => v,
            Err(e) => {
                self.as_mut().set_update_busy(false);
                self.as_mut()
                    .update_message("warn", &trf("Board update failed: %1", &[&e]));
                return;
            }
        };
        let n = self.rust().board_updates + 1;
        self.as_mut().rust_mut().board_updates = n;
        self.as_mut().rust_mut().board_expected = Some((v, n));
        // Not asked over HTTP while it restarts (refresh_board_info).
        self.as_mut().set_board_info_error(QString::default());
        self.as_mut().update_message(
            "info",
            &format!("Written and checked. The board restarts into firmware {v} …"),
        );
        let qt = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-update-wait".into())
            .spawn(move || {
                thread::sleep(BOARD_UPDATE_WAIT);
                let _ = qt.queue(move |obj| obj.board_update_timeout(n));
            });
    }

    // ------------------------------------------------------------ USB

    pub fn usb_watch(self: Pin<&mut Self>, on: bool) {
        let flag = self.rust().usb_watching.clone();
        if !on {
            flag.store(false, Ordering::Relaxed);
            return;
        }
        if flag.swap(true, Ordering::Relaxed) {
            return; // already watching
        }
        let qt = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-usb-watch".into())
            .spawn(move || {
                let mut last = None;
                while flag.load(Ordering::Relaxed) {
                    let found = matches!(usbflash::find_drive(), Ok(Some(_)));
                    if last != Some(found) {
                        last = Some(found);
                        if qt
                            .queue(move |mut obj| obj.as_mut().set_usb_board_found(found))
                            .is_err()
                        {
                            break;
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            });
    }

    pub fn usb_flash(mut self: Pin<&mut Self>, image: &QString, file: &QString) {
        if *self.usb_busy() {
            return;
        }
        let Some(image) = Image::from_id(&String::from(image)) else {
            return;
        };
        let own = match local_path(&String::from(file)) {
            None => None,
            Some(path) => match std::fs::read(&path) {
                Ok(data) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "firmware.uf2".into());
                    Some((name, data))
                }
                Err(e) => {
                    self.as_mut()
                        .set_usb_status(QString::from(&format!("{}: {e}", path.display())));
                    return;
                }
            },
        };
        let cancel = self.rust().usb_cancel.clone();
        cancel.store(false, Ordering::Relaxed);
        self.as_mut().set_usb_busy(true);
        self.as_mut().set_usb_status(QString::from(image.description()));
        let progress = self.qt_thread();
        let done = self.qt_thread();
        let _ = thread::Builder::new()
            .name("pshb-usb-flash".into())
            .spawn(move || {
                let mut say = |msg: &str| {
                    let text = msg.to_string();
                    let _ = progress.queue(move |mut obj| obj.as_mut().set_usb_status(QString::from(&text)));
                };
                let result = usbflash::files_for(image, own, &mut say)
                    .and_then(|files| usbflash::flash(&files, &mut say, &cancel))
                    .map(|()| image.done_message().to_string());
                let _ = done.queue(move |mut obj| {
                    obj.as_mut().set_usb_busy(false);
                    let text = result.unwrap_or_else(|e| format!("Not installed: {e}"));
                    obj.as_mut().set_usb_status(QString::from(&text));
                });
            });
    }

    pub fn usb_cancel(self: Pin<&mut Self>) {
        self.rust().usb_cancel.store(true, Ordering::Relaxed);
    }

    fn board_update_timeout(mut self: Pin<&mut Self>, n: u64) {
        let Some((v, m)) = self.rust().board_expected else {
            return;
        };
        if m != n {
            return;
        }
        self.as_mut().rust_mut().board_expected = None;
        self.as_mut().set_update_busy(false);
        let now = self.board_firmware().to_string();
        self.as_mut().update_message(
            "warn",
            &trf(
                "The board did not come back with firmware %1 (it reports %2): it probably went back to the previous version.",
                &[&v.to_string(), &now],
            ),
        );
    }
}

impl cxx_qt::Initialize for qobject::AppController {
    fn initialize(mut self: Pin<&mut Self>) {
        let path = config::config_path();
        let cfg = match Config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                // Keep the broken file: the next save would replace it.
                let backup = path.with_extension("toml.broken");
                let kept = std::fs::copy(&path, &backup).is_ok();
                self.as_mut().set_last_error(QString::from(&if kept {
                    format!("{e} (kept as {}; using defaults)", backup.display())
                } else {
                    format!("{e} (using defaults)")
                }));
                Config::default()
            }
        };
        self.as_mut().rust_mut().config = cfg;
        i18n::set_language(&self.config_ref().language);
        match window::detect() {
            Ok(b) => {
                self.as_mut().set_window_rule_available(true);
                self.as_mut().set_window_backend(QString::from(b.name()));
            }
            Err(e) => {
                self.as_mut().set_window_rule_available(false);
                self.as_mut().set_window_backend(QString::from(&e));
            }
        }
        self.as_mut().publish_config();
        self.as_mut().refresh_devices();

        let self_test = std::env::args().any(|a| a == "--self-test" || a == "--screenshot");
        // Screenshots with live values: the service runs (no tray, no window
        // rule, no update check). Only with --screenshot, as in main.rs,
        // which takes the instance lock for exactly this case.
        let live = std::env::args().any(|a| a == "--screenshot")
            && std::env::args().any(|a| a == "--screenshot-live");
        if self_test && live && self.config_ref().setup_done {
            self.as_mut().restart_service();
        }
        if !self_test {
            let handle = tray::spawn(self.qt_thread());
            self.as_mut().rust_mut().tray = handle;
            self.as_mut().update_window_watcher();
            // Also with a board set but the setup not finished (set with
            // pshb-cli, or the wizard closed early): the wizard's board step
            // shows whether it answers, as after "Connect".
            if self.config_ref().setup_done || !self.config_ref().board.is_empty() {
                self.as_mut().restart_service();
            }
            self.as_mut().set_can_update_app(appimage_path().is_some());
            self.as_mut().check_installation();
            // Look for updates shortly after the start, then every six hours.
            let qt = self.qt_thread();
            let _ = thread::Builder::new()
                .name("pshb-update-timer".into())
                .spawn(move || {
                    thread::sleep(Duration::from_secs(10));
                    while qt.queue(|obj| obj.periodic_update_check()).is_ok() {
                        thread::sleep(update::CHECK_INTERVAL);
                    }
                });
        }
    }
}

/// Ping interval while the latency view is visible.
const LATENCY_VIEW_PING: Duration = Duration::from_millis(200);

/// The board's web interface (firmware updates go there).
const BOARD_HTTP_PORT: u16 = 80;
/// How long the board may take to come back after a firmware update.
const BOARD_UPDATE_WAIT: Duration = Duration::from_secs(60);

/// Local path of a file chosen in QML ("file:///home/me/x.uf2", percent
/// encoded), or None for "".
fn local_path(url: &str) -> Option<std::path::PathBuf> {
    if url.is_empty() {
        return None;
    }
    let raw = url.strip_prefix("file://").unwrap_or(url).as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && let Some(b) = raw
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(std::ffi::OsString::from_vec(out).into())
}

/// The running AppImage, if this is one and its folder is writable (the
/// update writes the new file next to it).
fn appimage_path() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("APPIMAGE")?);
    let probe = path
        .parent()?
        .join(format!(".ps-hid-bridge-write-test-{}", std::process::id()));
    std::fs::File::create(&probe).ok()?;
    let _ = std::fs::remove_file(&probe);
    Some(path)
}

/// Drop the running service (and any start in progress) on a helper thread.
fn stop_service(slot: Arc<ServiceSlot>) {
    let _ = thread::Builder::new().name("pshb-stop".into()).spawn(move || {
        let _one_at_a_time = slot.starting.lock().unwrap();
        let service = slot.service.lock().unwrap().take();
        drop(service);
    });
}

/// `v` on a grid of `step` (a slider's), without the float residue of the
/// slider's own arithmetic.
fn round_to(v: f64, step: f64) -> f64 {
    let steps = 1.0 / step;
    (v * steps).round() / steps
}

/// Form of a board address: IP, IP:port, host name or host name:port.
fn check_address(board: &str) -> Result<(), String> {
    if board.is_empty() {
        return Err(tr("Please enter the board address."));
    }
    let bare = board
        .strip_prefix('[')
        .and_then(|b| b.strip_suffix(']'))
        .unwrap_or(board);
    if board.parse::<std::net::SocketAddr>().is_ok() || bare.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    let (host, port) = match board.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (board, None),
    };
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let port_ok = port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p != 0));
    if host_ok && port_ok {
        Ok(())
    } else {
        Err(format!("\"{board}\" is not a valid address."))
    }
}

fn resolve(board: &str) -> Result<std::net::SocketAddr, String> {
    if board.is_empty() {
        return Err(tr("Please enter the board address."));
    }
    config::board_socket_addr(board).map_err(|_| format!("Address \"{board}\" not found."))
}

/// Names of what is held, in the app's language: the modifiers and keys,
/// and the mouse buttons as one part ("mouse: left, back").
fn live_parts(s: &KeyState) -> (Vec<String>, Option<String>) {
    let mods = [
        (0x01, tr("Ctrl")),
        (0x02, tr("Shift")),
        (0x04, tr("Alt")),
        (0x08, tr("Super")),
        (0x10, tr("right Ctrl")),
        (0x20, tr("right Shift")),
        (0x40, tr("AltGr")),
        (0x80, tr("right Super")),
    ];
    let buttons = [
        (0x01, tr("left")),
        (0x02, tr("right")),
        (0x04, tr("middle")),
        (0x08, tr("back")),
        (0x10, tr("forward")),
    ];
    let mut parts: Vec<String> = mods
        .into_iter()
        .filter(|(bit, _)| s.mods & bit != 0)
        .map(|(_, n)| n)
        .collect();
    if s.keys.contains(&0x01) {
        // Rollover: the keyboard report has 0x01 in every slot.
        parts.push(tr("too many keys"));
    } else {
        parts.extend(
            s.keys
                .iter()
                .filter(|u| **u != 0)
                .map(|u| pshb_core::keymap::usage_name(*u)),
        );
    }
    let buttons: Vec<String> = buttons
        .into_iter()
        .filter(|(b, _)| s.buttons & b != 0)
        .map(|(_, n)| n)
        .collect();
    let mouse = (!buttons.is_empty()).then(|| trf("mouse: %1", &[&buttons.join(", ")]));
    (parts, mouse)
}

/// The profiles for QML, "Standard" first.
fn profiles_json(c: &Config) -> String {
    #[derive(Serialize)]
    struct Row<'a> {
        name: &'a str,
        window: &'a str,
        sensitivity: f64,
        accel: f64,
        invert_y: bool,
        y_ratio: f64,
        wheel_speed: f64,
        remap: &'a [config::RemapConfig],
    }
    let rows: Vec<Row> = c
        .profile_names()
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let m = c.profile_mouse(i);
            Row {
                name,
                window: i
                    .checked_sub(1)
                    .and_then(|j| c.profiles.get(j))
                    .map_or("", |p| p.window.as_str()),
                sensitivity: m.sensitivity,
                accel: m.accel,
                invert_y: m.invert_y,
                y_ratio: m.y_ratio,
                wheel_speed: m.wheel_speed,
                remap: c.profile_remap(i),
            }
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
}

/// The keyboard languages text can be typed for, for QML.
fn layouts_json() -> String {
    let rows: Vec<serde_json::Value> = typing::LAYOUTS
        .iter()
        .map(|l| serde_json::json!({ "id": l.id, "name": l.name }))
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
}

/// Marks a pairing that needs a new password first (internal, never shown).
const NEEDS_NEW_PASSWORD: &str = "\u{0}new-password";

/// The app replaced its AppImage at `path` with a new version: if Shelly
/// installed it, Shelly reads it again (its record shows the new version).
fn tell_shelly(path: &std::path::Path) {
    if install::shelly().is_none() {
        return;
    }
    let result = install::list().and_then(|list| match install::find(&list, path) {
        Found::ThisFile(i) => install::sync(&i.name),
        _ => Ok(()),
    });
    if let Err(e) = result {
        pshb_core::logln!("PS HID Bridge: {e}");
    }
}
