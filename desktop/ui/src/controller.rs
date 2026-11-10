//! QML-facing controller (singleton `AppController`): settings, the bridge
//! service, tray icon, window-focus rule and notifications.

use std::pin::Pin;
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
        /// Install the udev rule for the selected devices; error text or "".
        #[qinvokable]
        fn install_access(self: Pin<&mut AppController>) -> QString;
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
    version: QString,

    config: Config,
    devices: Vec<DeviceInfo>,
    service: Option<Service>,
    tray: Option<ksni::blocking::Handle<BridgeTray>>,
    focus: Option<FocusWatcher>,
}

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
            version: QString::from(pshb_core::VERSION),
            config: Config::default(),
            devices: Vec::new(),
            service: None,
            tray: None,
            focus: None,
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
        if let Some(s) = &self.rust().service {
            s.send(cmd);
        }
    }

    /// Start (or restart) the bridge service with the current settings.
    fn restart_service(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().service = None;
        let cfg = self.config_ref().clone();
        if cfg.board.is_empty() {
            self.as_mut()
                .set_status_text(QString::from("Board address missing"));
            return;
        }
        let key = match config::load_psk(&config::psk_path()) {
            Ok(k) => k,
            Err(_) => {
                self.as_mut().set_status_text(QString::from("Key missing"));
                return;
            }
        };
        let addr = match resolve(&cfg.board) {
            Ok(a) => a,
            Err(e) => {
                self.as_mut().set_status_text(QString::from(&e));
                return;
            }
        };
        let qt = self.qt_thread();
        let sink = Box::new(move |ev: ServiceEvent| {
            let _ = qt.queue(move |obj| obj.on_service_event(ev));
        });
        let sc = ServiceConfig {
            config: cfg,
            key,
            board: addr,
            counter_store: Some(config::counter_path()),
        };
        match Service::start(sc, sink) {
            Ok(s) => {
                self.as_mut().rust_mut().service = Some(s);
                self.as_mut().set_status_text(QString::from("Connecting …"));
                self.as_mut().set_last_error(QString::default());
            }
            Err(e) => self.as_mut().set_last_error(QString::from(&e)),
        }
        self.as_mut().restart_focus_watcher();
    }

    fn restart_focus_watcher(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().focus = None;
        if !self.config_ref().window_rule_active() {
            return;
        }
        let rule = self.config_ref().window_rule.trim().to_string();
        let qt = self.qt_thread();
        match FocusWatcher::start(&rule, move |focused| {
            let _ = qt.queue(move |obj| obj.command(ServiceCmd::Focus(focused)));
        }) {
            Ok(w) => self.as_mut().rust_mut().focus = Some(w),
            Err(e) => self
                .as_mut()
                .set_last_error(QString::from(&format!("Window rule not available: {e}"))),
        }
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
        if let Some(t) = &self.rust().tray {
            t.update(|tray| {
                tray.active = st.active;
                tray.connected = st.connected;
            });
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
        if let Err(e) = resolve(&address) {
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

    pub fn install_access(mut self: Pin<&mut Self>) -> QString {
        let selected = input::select(&self.rust().devices, &self.config_ref().devices);
        if selected.is_empty() {
            return QString::from("Please select keyboard and mouse first.");
        }
        let rule = pshb_core::access::udev_rule(&selected);
        if let Err(e) = pshb_core::access::install_rule(&rule) {
            return QString::from(&e);
        }
        // udev applies the ACLs asynchronously.
        std::thread::sleep(Duration::from_millis(500));
        self.as_mut().refresh_devices();
        self.as_mut().restart_service();
        let still = input::select(&self.rust().devices, &self.config_ref().devices)
            .iter()
            .filter(|d| !d.accessible)
            .count();
        if still > 0 {
            QString::from(
                "Rule installed, but no access yet. Please unplug and replug the devices once, \
                 or log in again.",
            )
        } else {
            QString::default()
        }
    }

    pub fn set_identify(self: Pin<&mut Self>, on: bool) {
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
        self.as_mut().restart_service();
    }

    pub fn set_window_rule_on(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().config.window_rule_enabled = on;
        self.as_mut().save_config();
        self.as_mut().publish_config();
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

    pub fn set_latency_view(self: Pin<&mut Self>, visible: bool) {
        let interval = if visible {
            Duration::from_millis(200)
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
        self.as_mut().rust_mut().service = None; // releases everything
        self.as_mut().rust_mut().focus = None;
        std::process::exit(0);
    }
}

impl cxx_qt::Initialize for qobject::AppController {
    fn initialize(mut self: Pin<&mut Self>) {
        let cfg = match Config::load(&config::config_path()) {
            Ok(c) => c,
            Err(e) => {
                self.as_mut().set_last_error(QString::from(&e));
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
            if self.config_ref().setup_done {
                self.as_mut().restart_service();
            }
        }
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
