//! QML-facing controller (singleton `AppController`): settings, the bridge
//! service, tray icon, window-focus rule and notifications.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use pshb_core::config::{self, Config, TriggerConfig};
use pshb_core::crypto::parse_key_hex;
use pshb_core::engine::Notice;
use pshb_core::input::{self, DeviceInfo};
use pshb_core::protocol::{self, Pong};
use pshb_core::service::{Service, ServiceCmd, ServiceConfig, ServiceEvent, ServiceStatus};
use pshb_core::trigger::Action;
use serde::Serialize;

use crate::focus::FocusWatcher;
use crate::notify;
use crate::tray::{self, BridgeTray};

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
        #[qproperty(f64, input_p50)]
        #[qproperty(f64, input_p95)]
        #[qproperty(f64, input_max)]
        #[qproperty(bool, realtime)]
        #[qproperty(i32, packets_per_second)]
        #[qproperty(QString, board_firmware)]
        #[qproperty(QString, board_usb)]
        #[qproperty(QString, board_address)]
        #[qproperty(bool, psk_set)]
        #[qproperty(QString, devices_json)]
        #[qproperty(i32, devices_blocked)]
        #[qproperty(i32, devices_open)]
        #[qproperty(QString, triggers_json)]
        #[qproperty(bool, window_rule_enabled)]
        #[qproperty(QString, window_rule)]
        #[qproperty(bool, window_rule_available)]
        #[qproperty(bool, iso_keyboard)]
        #[qproperty(f64, sensitivity)]
        #[qproperty(f64, accel)]
        #[qproperty(bool, setup_done)]
        #[qproperty(bool, recording)]
        #[qproperty(QString, recorded)]
        #[qproperty(QString, identified_path)]
        #[qproperty(QString, live_text)]
        #[qproperty(QString, last_error)]
        #[qproperty(bool, access_busy)]
        #[qproperty(QString, access_message)]
        #[qproperty(QString, version)]
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

        #[qinvokable]
        fn refresh_devices(self: Pin<&mut AppController>);
        /// Select a device group by its "vvvv:pppp" id.
        #[qinvokable]
        fn set_device_selected(self: Pin<&mut AppController>, id: &QString, selected: bool);
        /// Install the udev rule for the selected devices (asks for the
        /// administrator password); `access_busy` meanwhile, then the
        /// outcome in `access_message` ("" = done).
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
        /// Gain of the acceleration curve at a given speed (for the curve plot).
        #[qinvokable]
        fn curve_gain(self: &AppController, speed: f64) -> f64;

        /// Ping faster while the latency view is visible.
        #[qinvokable]
        fn set_latency_view(self: Pin<&mut AppController>, visible: bool);

        #[qinvokable]
        fn finish_setup(self: Pin<&mut AppController>);
        #[qinvokable]
        fn quit_app(self: Pin<&mut AppController>);
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
    /// Input event -> packet sent, in ms (-1 = no samples yet).
    input_p50: f64,
    input_p95: f64,
    input_max: f64,
    realtime: bool,
    packets_per_second: i32,
    board_firmware: QString,
    board_usb: QString,
    board_address: QString,
    psk_set: bool,
    devices_json: QString,
    devices_blocked: i32,
    devices_open: i32,
    triggers_json: QString,
    window_rule_enabled: bool,
    window_rule: QString,
    window_rule_available: bool,
    iso_keyboard: bool,
    sensitivity: f64,
    accel: f64,
    setup_done: bool,
    recording: bool,
    recorded: QString,
    identified_path: QString,
    live_text: QString,
    last_error: QString,
    access_busy: bool,
    access_message: QString,
    version: QString,

    config: Config,
    devices: Vec<DeviceInfo>,
    slot: Arc<ServiceSlot>,
    tray: Option<ksni::blocking::Handle<BridgeTray>>,
    /// Last tray state shown (active, connected).
    tray_state: Option<(bool, bool)>,
    focus: Option<FocusWatcher>,
    /// What the GUI asked the service for; re-applied after a restart.
    identify_on: bool,
    latency_view: bool,
    focus_state: Option<bool>,
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
            input_p50: -1.0,
            input_p95: -1.0,
            input_max: -1.0,
            realtime: false,
            packets_per_second: 0,
            board_firmware: QString::default(),
            board_usb: QString::default(),
            board_address: QString::default(),
            psk_set: false,
            devices_json: QString::from("[]"),
            devices_blocked: 0,
            devices_open: 0,
            triggers_json: QString::from("[]"),
            window_rule_enabled: false,
            window_rule: QString::default(),
            window_rule_available: false,
            iso_keyboard: true,
            sensitivity: 1.0,
            accel: 0.0,
            setup_done: false,
            recording: false,
            recorded: QString::default(),
            identified_path: QString::default(),
            live_text: QString::default(),
            last_error: QString::default(),
            access_busy: false,
            access_message: QString::default(),
            version: QString::from(pshb_core::VERSION),
            config: Config::default(),
            devices: Vec::new(),
            slot: Arc::default(),
            tray: None,
            tray_state: None,
            focus: None,
            identify_on: false,
            latency_view: false,
            focus_state: None,
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

fn group_devices(devices: &[DeviceInfo], config: &Config) -> Vec<DeviceGroup> {
    let mut groups: Vec<DeviceGroup> = Vec::new();
    for d in devices.iter().filter(|d| d.keyboard || d.mouse) {
        let id = d.id_string();
        if let Some(g) = groups.iter_mut().find(|g| g.id == id) {
            g.nodes += 1;
            g.accessible &= d.accessible;
            g.paths.push(d.path.display().to_string());
            if d.name.len() < g.name.len() {
                g.name = d.name.clone();
            }
            let kinds = [g.kind.as_str(), d.kind()];
            g.kind = if kinds.iter().any(|k| k.contains("keyboard"))
                && kinds.iter().any(|k| k.contains("mouse"))
            {
                "keyboard + mouse".into()
            } else {
                g.kind.clone()
            };
        } else {
            groups.push(DeviceGroup {
                selected: config.devices.iter().any(|s| s.eq_ignore_ascii_case(&id)),
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
        "PS5 not connected"
    } else if p.status & protocol::STATUS_USB_SUSPENDED != 0 {
        "PS5 in rest mode"
    } else {
        "PS5 connected"
    }
}

impl qobject::AppController {
    fn save_config(mut self: Pin<&mut Self>) {
        let path = config::config_path();
        if let Err(e) = self.config_ref().save(&path) {
            self.as_mut()
                .set_last_error(QString::from(&format!("Settings: {e}")));
        }
    }

    fn config_ref(&self) -> &Config {
        &self.rust().config
    }

    fn publish_config(mut self: Pin<&mut Self>) {
        let c = self.config_ref().clone();
        self.as_mut().set_board_address(QString::from(&c.board));
        self.as_mut().set_psk_set(config::psk_path().exists());
        self.as_mut().set_triggers_json(QString::from(
            &serde_json::to_string(&c.triggers).unwrap_or_default(),
        ));
        self.as_mut().set_window_rule_enabled(c.window_rule_enabled);
        self.as_mut().set_window_rule(QString::from(&c.window_rule));
        self.as_mut().set_iso_keyboard(c.iso_keyboard);
        self.as_mut().set_sensitivity(c.mouse.sensitivity);
        self.as_mut().set_accel(c.mouse.accel);
        self.as_mut().set_setup_done(c.setup_done);
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
            Some("Board address missing")
        } else if !config::psk_path().exists() {
            Some("Key missing")
        } else {
            None
        };
        if let Some(text) = problem {
            self.as_mut().set_status_text(QString::from(text));
            stop_service(slot);
            return;
        }
        self.as_mut().set_status_text(QString::from("Connecting …"));
        let events = self.qt_thread();
        let done = self.qt_thread();
        let spawned = thread::Builder::new().name("pshb-start".into()).spawn(move || {
            let _one_at_a_time = slot.starting.lock().unwrap();
            if !slot.is_current(generation) {
                return;
            }
            let old = slot.service.lock().unwrap().take();
            drop(old); // releases devices and board, frees the packet counter
            let result = config::load_psk(&config::psk_path())
                .and_then(|key| Ok((key, resolve(&cfg.board)?)))
                .and_then(|(key, board)| {
                    let sink = Box::new(move |ev: ServiceEvent| {
                        let _ = events.queue(move |obj| obj.on_service_event(ev));
                    });
                    let sc = ServiceConfig {
                        config: cfg,
                        key,
                        board,
                        counter_store: Some(config::counter_path()),
                    };
                    Service::start(sc, sink)
                });
            match result {
                Ok(service) if slot.is_current(generation) => {
                    *slot.service.lock().unwrap() = Some(service);
                    let _ = done.queue(move |obj| obj.service_started(generation));
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

    fn service_started(mut self: Pin<&mut Self>, generation: u64) {
        if !self.rust().slot.is_current(generation) {
            return;
        }
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
        if let Some(f) = self.rust().focus_state {
            self.command(ServiceCmd::Focus(f));
        }
    }

    fn service_failed(mut self: Pin<&mut Self>, generation: u64, error: String) {
        if !self.rust().slot.is_current(generation) {
            return;
        }
        self.as_mut().set_status_text(QString::from("Not connected"));
        self.as_mut()
            .set_last_error(QString::from(&format!("{error} (trying again)")));
        let qt = self.qt_thread();
        let _ = thread::Builder::new().name("pshb-retry".into()).spawn(move || {
            thread::sleep(RETRY);
            let _ = qt.queue(move |obj| obj.retry_service(generation));
        });
    }

    fn retry_service(self: Pin<&mut Self>, generation: u64) {
        if self.rust().slot.is_current(generation) {
            self.restart_service();
        }
    }

    /// Watch the window focus (only when the window rule is on). Restarted
    /// only when the rule changes; the service learns the focus from
    /// `focus_state`.
    fn restart_focus_watcher(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().focus = None;
        self.as_mut().rust_mut().focus_state = None;
        if !self.config_ref().window_rule_active() {
            return;
        }
        let rule = self.config_ref().window_rule.trim().to_string();
        let qt = self.qt_thread();
        match FocusWatcher::start(&rule, move |focused| {
            let _ = qt.queue(move |obj| obj.focus_changed(focused));
        }) {
            Ok(w) => self.as_mut().rust_mut().focus = Some(w),
            Err(e) => self
                .as_mut()
                .set_last_error(QString::from(&format!("Window rule not available: {e}"))),
        }
    }

    fn focus_changed(mut self: Pin<&mut Self>, focused: bool) {
        self.as_mut().rust_mut().focus_state = Some(focused);
        self.command(ServiceCmd::Focus(focused));
    }

    fn apply_status(mut self: Pin<&mut Self>, st: ServiceStatus) {
        self.as_mut().set_connected(st.connected);
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
        let ms = |us: u32| f64::from(us) / 1000.0;
        let lat = st.input_latency;
        self.as_mut().set_input_p50(lat.map_or(-1.0, |l| ms(l.p50_us)));
        self.as_mut().set_input_p95(lat.map_or(-1.0, |l| ms(l.p95_us)));
        self.as_mut().set_input_max(lat.map_or(-1.0, |l| ms(l.max_us)));
        self.as_mut().set_realtime(st.realtime);
        self.as_mut().set_packets_per_second(st.packets_per_second as i32);
        self.as_mut().set_devices_open(st.devices_open as i32);
        self.as_mut().set_devices_blocked(st.devices_blocked as i32);
        if let Some(p) = &st.board {
            self.as_mut()
                .set_board_firmware(QString::from(&p.fw_version_string()));
            self.as_mut().set_board_usb(QString::from(usb_text(p)));
        }
        let text = if !st.connected {
            "Board not reachable"
        } else if st.active {
            "Bridge on"
        } else if st.pending {
            "Please release all keys …"
        } else if st.paused {
            "Paused"
        } else {
            "Ready"
        };
        self.as_mut().set_status_text(QString::from(text));
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

    fn on_service_event(mut self: Pin<&mut Self>, ev: ServiceEvent) {
        match ev {
            ServiceEvent::Status(st) => self.as_mut().apply_status(st),
            ServiceEvent::Notice(n) => {
                let msg = match n {
                    Notice::Activated => Some(("Bridge on", "Mouse and keyboard go to the PS5.")),
                    Notice::Deactivated => {
                        Some(("Bridge off", "Mouse and keyboard belong to the desktop again."))
                    }
                    Notice::Emergency => Some(("Emergency stop", "All devices were released.")),
                    Notice::Paused => Some(("Paused", "Input is not forwarded.")),
                    _ => None,
                };
                if let Some((summary, body)) = msg {
                    notify::send(summary, body);
                }
                if matches!(n, Notice::Deactivated | Notice::Emergency) {
                    self.as_mut().set_live_text(QString::default());
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
            ServiceEvent::Live(text) => self.as_mut().set_live_text(QString::from(&text)),
            ServiceEvent::Error(e) => self.as_mut().set_last_error(QString::from(&e)),
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
        if !psk.trim().is_empty() {
            match parse_key_hex(&psk) {
                Ok(k) => {
                    if let Err(e) = config::save_psk(&config::psk_path(), &k) {
                        return QString::from(&e);
                    }
                }
                Err(e) => return QString::from(&e),
            }
        } else if !config::psk_path().exists() {
            return QString::from("Please paste the key from the board's web interface.");
        }
        self.as_mut().rust_mut().config.board = address;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn reconnect(self: Pin<&mut Self>) {
        self.restart_service();
    }

    pub fn refresh_devices(mut self: Pin<&mut Self>) {
        let list = match input::list_devices() {
            Ok(l) => l.devices,
            Err(e) => {
                self.as_mut()
                    .set_last_error(QString::from(&format!("Device list: {e}")));
                return;
            }
        };
        let groups = group_devices(&list, self.config_ref());
        self.as_mut().rust_mut().devices = list;
        self.as_mut()
            .set_devices_json(QString::from(&serde_json::to_string(&groups).unwrap_or_default()));
    }

    pub fn set_device_selected(mut self: Pin<&mut Self>, id: &QString, selected: bool) {
        let id = String::from(id).to_lowercase();
        {
            let devices = &mut self.as_mut().rust_mut().config.devices;
            devices.retain(|d| !d.eq_ignore_ascii_case(&id));
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
                .set_access_message(QString::from("Please select keyboard and mouse first."));
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

    fn access_installed(mut self: Pin<&mut Self>, result: Result<(), String>) {
        self.as_mut().set_access_busy(false);
        if let Err(e) = result {
            self.as_mut().set_access_message(QString::from(&e));
            return;
        }
        // The running service notices the new permissions by itself.
        self.as_mut().refresh_devices();
        let still = input::select(&self.rust().devices, &self.config_ref().devices)
            .iter()
            .filter(|d| !d.accessible)
            .count();
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
            Err(e) => return QString::from(&format!("Invalid triggers: {e}")),
        };
        for t in &triggers {
            if let Err(e) = t.to_trigger() {
                return QString::from(&e);
            }
        }
        self.as_mut().rust_mut().config.triggers = triggers;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_service();
        QString::default()
    }

    pub fn set_window_rule_text(mut self: Pin<&mut Self>, text: &QString) {
        self.as_mut().rust_mut().config.window_rule = String::from(text).trim().to_string();
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_focus_watcher();
        self.as_mut().restart_service();
    }

    pub fn set_window_rule_on(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().config.window_rule_enabled = on;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_focus_watcher();
        self.as_mut().restart_service();
    }

    pub fn set_keyboard_type(mut self: Pin<&mut Self>, iso: bool) {
        self.as_mut().rust_mut().config.iso_keyboard = iso;
        self.as_mut().save_config();
        self.as_mut().publish_config();
        self.as_mut().restart_service();
    }

    pub fn set_mouse(mut self: Pin<&mut Self>, sensitivity: f64, accel: f64) {
        {
            let m = &mut self.as_mut().rust_mut().config.mouse;
            m.sensitivity = sensitivity.clamp(0.1, 8.0);
            m.accel = accel.clamp(0.0, 1.0);
        }
        self.as_mut().save_config();
        self.as_mut().publish_config();
        let curve = self.config_ref().mouse_curve();
        self.command(ServiceCmd::MouseCurve(curve));
    }

    pub fn curve_gain(&self, speed: f64) -> f64 {
        self.config_ref().mouse_curve().gain(speed)
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
        // Releases devices and board; a start still in progress is dropped.
        let slot = self.rust().slot.clone();
        slot.generation.fetch_add(1, Ordering::SeqCst);
        let service = slot.service.lock().unwrap().take();
        drop(service);
        self.as_mut().rust_mut().focus = None;
        std::process::exit(0);
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
        self.as_mut()
            .set_window_rule_available(crate::focus::kwin_available());
        self.as_mut().publish_config();
        self.as_mut().refresh_devices();

        let self_test = std::env::args().any(|a| a == "--self-test" || a == "--screenshot");
        if !self_test {
            let handle = tray::spawn(self.qt_thread());
            self.as_mut().rust_mut().tray = handle;
            self.as_mut().restart_focus_watcher();
            if self.config_ref().setup_done {
                self.as_mut().restart_service();
            }
        }
    }
}

/// Ping interval while the latency view is visible.
const LATENCY_VIEW_PING: Duration = Duration::from_millis(200);

/// Drop the running service (and any start in progress) on a helper thread.
fn stop_service(slot: Arc<ServiceSlot>) {
    let _ = thread::Builder::new().name("pshb-stop".into()).spawn(move || {
        let _one_at_a_time = slot.starting.lock().unwrap();
        let service = slot.service.lock().unwrap().take();
        drop(service);
    });
}

/// Form of a board address: IP, IP:port, host name or host name:port.
fn check_address(board: &str) -> Result<(), String> {
    if board.is_empty() {
        return Err("Please enter the board address.".into());
    }
    if board.parse::<std::net::SocketAddr>().is_ok() || board.parse::<std::net::IpAddr>().is_ok() {
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
    use std::net::ToSocketAddrs;
    if board.is_empty() {
        return Err("Please enter the board address.".into());
    }
    let with_port = if board.contains(':') {
        board.to_string()
    } else {
        format!("{board}:{}", protocol::UDP_PORT_DEFAULT)
    };
    with_port
        .to_socket_addrs()
        .map_err(|_| format!("Address \"{board}\" not found."))?
        .next()
        .ok_or_else(|| format!("Address \"{board}\" not found."))
}
