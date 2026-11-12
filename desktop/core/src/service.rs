//! Long-running bridge service for the GUI: devices, engine, link, watchdog
//! and the extras the GUI needs (hot-plug rescans, window focus, recording a
//! trigger, identifying a device, live view of the forwarded input).
//!
//! The GUI talks to it through [`ServiceCmd`] and receives [`ServiceEvent`]s
//! from the service thread via a callback.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::controller::{Controller, Watchdog};
use crate::engine::{Engine, Event, Notice};
use crate::input::{self, DeviceInfo, GrabGuard, InputMsg, InputSet, Waker};
use crate::keymap;
use crate::mouse::MouseCurve;
use crate::protocol::{KEY_LEN, Pong};
use crate::rt;
use crate::transport::{KeyState, LatencyStats, Link, LinkConfig};
use crate::trigger::{Action, Combo, KeyAlt};

/// Device list check: after hot-plug events (settled), else rarely as a
/// fallback; regularly only where /dev/input cannot be watched.
const HOTPLUG_SETTLE: Duration = Duration::from_millis(200);
const RESCAN_FALLBACK: Duration = Duration::from_secs(30);
const RESCAN_POLL: Duration = Duration::from_secs(2);
const STATUS: Duration = Duration::from_millis(250);
/// At most one log line (journal) about input corrections per this time.
const RESYNC_LOG_GAP: Duration = Duration::from_secs(10);

pub enum ServiceCmd {
    Action(Action),
    /// Window rule: the configured window gained (true) or lost the focus.
    Focus(bool),
    /// Record the next key combination (bridge input is paused meanwhile).
    Record(bool),
    /// Report which selected device produces input.
    Identify(bool),
    MouseCurve(MouseCurve),
    /// Ping more often while the latency view is visible.
    PingInterval(Duration),
    Shutdown,
    /// Result of a device scan (internal: the scan runs on its own thread,
    /// the input thread must not stall on sysfs).
    #[doc(hidden)]
    Devices(Result<input::DeviceList, String>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceStatus {
    pub connected: bool,
    pub rtt_ms: Option<f64>,
    /// Median / 95th percentile of the recent round trips.
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    /// Recent round trips (ms), oldest first.
    pub rtt_history: Vec<f64>,
    pub board: Option<Pong>,
    pub active: bool,
    pub pending: bool,
    pub paused: bool,
    pub packets_per_second: u32,
    /// Input event -> packet sent, over the most recent packets.
    pub input_latency: Option<LatencyStats>,
    /// The input path runs with real-time priority.
    pub realtime: bool,
    /// Selected devices that are open / that exist but cannot be opened.
    pub devices_open: usize,
    pub devices_blocked: usize,
    /// How often lost input events (e.g. a button release) were corrected
    /// from the kernel's key state.
    pub input_resyncs: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServiceEvent {
    Status(ServiceStatus),
    Notice(Notice),
    /// Text form of a recorded combination, e.g. "Ctrl+Alt+P".
    Recorded(String),
    /// A selected device produced a key or button press.
    Identified(PathBuf),
    /// What the PlayStation currently receives, e.g. "Shift + A · mouse: left",
    /// and the device whose event last changed it ("" for a command).
    Live {
        text: String,
        device: String,
    },
    Error(String),
}

pub struct ServiceConfig {
    pub config: Config,
    pub key: [u8; KEY_LEN],
    pub board: SocketAddr,
    pub counter_store: Option<PathBuf>,
}

pub type Sink = Box<dyn Fn(ServiceEvent) + Send>;

pub struct Service {
    tx: Sender<ServiceCmd>,
    waker: Arc<Waker>,
    thread: Option<JoinHandle<()>>,
}

impl Service {
    pub fn start(cfg: ServiceConfig, sink: Sink) -> Result<Self, String> {
        let engine_cfg = cfg.config.engine_config()?;
        let mut link_cfg = LinkConfig::new(cfg.board, cfg.key);
        link_cfg.counter_store = cfg.counter_store;
        link_cfg.realtime = true;
        let link = Arc::new(Link::start(link_cfg).map_err(|e| format!("UDP: {e}"))?);
        let waker = Arc::new(Waker::new().map_err(|e| e.to_string())?);
        let (tx, rx) = mpsc::channel();
        let devices = cfg.config.devices.clone();
        let w = waker.clone();
        let cmds = tx.clone();
        let thread = thread::Builder::new()
            .name("pshb-service".into())
            .spawn(move || Runner::new(Engine::new(engine_cfg), link, devices, sink, w, cmds).run(rx))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx,
            waker,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: ServiceCmd) {
        let _ = self.tx.send(cmd);
        self.waker.wake();
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.send(ServiceCmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Human readable form of a key state for the live view.
pub fn describe_state(s: &KeyState) -> String {
    const MOD_NAMES: [(u8, &str); 8] = [
        (0x01, "Ctrl"),
        (0x02, "Shift"),
        (0x04, "Alt"),
        (0x08, "Super"),
        (0x10, "right Ctrl"),
        (0x20, "right Shift"),
        (0x40, "AltGr"),
        (0x80, "right Super"),
    ];
    let mut parts: Vec<String> = MOD_NAMES
        .iter()
        .filter(|(bit, _)| s.mods & bit != 0)
        .map(|(_, n)| n.to_string())
        .collect();
    if s.keys.contains(&0x01) {
        // Rollover: the keyboard report has 0x01 in every slot.
        parts.push("too many keys".into());
    } else {
        parts.extend(s.keys.iter().filter(|u| **u != 0).map(|u| keymap::usage_name(*u)));
    }
    let buttons: Vec<&str> = [
        (0x01, "left"),
        (0x02, "right"),
        (0x04, "middle"),
        (0x08, "back"),
        (0x10, "forward"),
    ]
    .iter()
    .filter(|(b, _)| s.buttons & b != 0)
    .map(|(_, n)| *n)
    .collect();
    let mut text = if parts.is_empty() {
        "no key".to_string()
    } else {
        parts.join(" + ")
    };
    if !buttons.is_empty() {
        text += &format!(" · mouse: {}", buttons.join(", "));
    }
    text
}

struct Runner {
    ctl: Controller,
    link: Arc<Link>,
    specs: Vec<String>,
    sink: Sink,
    guard: GrabGuard,
    inputs: InputSet,
    /// Back to our own queue: the device scan thread reports through it.
    cmds: Sender<ServiceCmd>,
    waker: Arc<Waker>,
    scan_running: bool,
    next_rescan: Instant,
    realtime: bool,
    blocked: usize,
    recording: Option<BTreeSet<u16>>,
    record_pressed: BTreeSet<u16>,
    /// Window focus that changed while recording (applied afterwards).
    pending_focus: Option<bool>,
    identify: bool,
    last_identify: Option<Instant>,
    last_status: ServiceStatus,
    last_live: Option<KeyState>,
    last_sent: u64,
    last_sent_at: Instant,
    /// Input corrections already written to the log, and when.
    logged_resyncs: u64,
    logged_at: Option<Instant>,
}

impl Runner {
    fn new(
        engine: Engine,
        link: Arc<Link>,
        specs: Vec<String>,
        sink: Sink,
        waker: Arc<Waker>,
        cmds: Sender<ServiceCmd>,
    ) -> Self {
        let guard = GrabGuard::default();
        Self {
            ctl: Controller::new(engine, link.clone()),
            link,
            specs,
            sink,
            inputs: InputSet::new(waker.clone(), guard.clone()),
            guard,
            cmds,
            waker,
            scan_running: false,
            next_rescan: Instant::now(),
            realtime: false,
            blocked: 0,
            recording: None,
            record_pressed: BTreeSet::new(),
            pending_focus: None,
            identify: false,
            last_identify: None,
            last_status: ServiceStatus::default(),
            last_live: None,
            last_sent: 0,
            last_sent_at: Instant::now(),
            logged_resyncs: 0,
            logged_at: None,
        }
    }

    fn emit(&self, e: ServiceEvent) {
        (self.sink)(e);
    }

    fn notices(&self, notices: Vec<Notice>) {
        for n in notices {
            self.emit(ServiceEvent::Notice(n));
        }
    }

    /// Apply what the engine asked for: keys forgotten by "release all",
    /// and grab or release the devices. A device that cannot be grabbed
    /// switches the bridge off again.
    fn apply_requests(&mut self) {
        let forgotten = self.ctl.take_forgotten();
        if !forgotten.is_empty() {
            self.inputs.forget(&forgotten);
        }
        while let Some(on) = self.ctl.take_grab_request() {
            let problems = self.inputs.set_grab(on);
            self.report_grab_problems(&problems);
            if problems.iter().any(|m| matches!(m, InputMsg::GrabFailed { .. })) {
                let n = self.ctl.command(Action::Off);
                self.notices(n);
            }
        }
    }

    /// Devices that could not be grabbed or released: one message for the
    /// GUI, and a line in the log (journal).
    fn report_grab_problems(&self, msgs: &[InputMsg]) {
        if let Some(text) = input::grab_problems(msgs) {
            eprintln!("PS HID Bridge: {text}");
            self.emit(ServiceEvent::Error(text));
        }
    }

    fn command(&mut self, action: Action) {
        let n = self.ctl.command(action);
        self.notices(n);
        self.apply_requests();
        self.update_live("");
    }

    /// Tell the GUI when what the PlayStation receives has changed;
    /// `device`: the device whose event caused it ("" for a command).
    fn update_live(&mut self, device: &str) {
        let live = self.ctl.last_state();
        if live != self.last_live {
            self.last_live = live;
            if let Some(s) = live {
                self.emit(ServiceEvent::Live {
                    text: describe_state(&s),
                    device: device.to_string(),
                });
            }
        }
    }

    /// Look at the device list again (on its own thread: walking sysfs
    /// takes a few milliseconds, long enough for the kernel to drop input
    /// events of a fast mouse); [`Self::devices_listed`] applies the result.
    fn rescan(&mut self) {
        self.next_rescan = Instant::now()
            + if self.inputs.watches_hotplug() {
                RESCAN_FALLBACK
            } else {
                RESCAN_POLL
            };
        if self.scan_running {
            return;
        }
        let cmds = self.cmds.clone();
        let waker = self.waker.clone();
        let spawned = thread::Builder::new().name("pshb-devices".into()).spawn(move || {
            let list = input::list_devices().map_err(|e| e.to_string());
            let _ = cmds.send(ServiceCmd::Devices(list));
            waker.wake();
        });
        self.scan_running = spawned.is_ok();
    }

    /// Open newly selected devices and close vanished ones; devices that
    /// stay open keep their grab.
    fn devices_listed(&mut self, list: Result<input::DeviceList, String>) {
        self.scan_running = false;
        let list = match list {
            Ok(l) => l,
            Err(e) => {
                self.emit(ServiceEvent::Error(format!("Device list: {e}")));
                return;
            }
        };
        let selected = input::select(&list.devices, &self.specs);
        let wanted: Vec<DeviceInfo> = selected.iter().filter(|d| d.accessible).cloned().collect();
        self.blocked = selected.len() - wanted.len();
        let mut msgs = Vec::new();
        self.inputs.update(&wanted, &mut msgs);
        self.handle_msgs(&mut msgs);
    }

    fn handle_msgs(&mut self, msgs: &mut Vec<InputMsg>) {
        self.report_grab_problems(msgs);
        let mut grab_failed = false;
        for m in msgs.drain(..) {
            match m {
                InputMsg::Event { dev, event, ts_us } => self.handle_input(dev, event, ts_us),
                InputMsg::Gone { .. } | InputMsg::Hotplug => {
                    self.next_rescan = self.next_rescan.min(Instant::now() + HOTPLUG_SETTLE);
                }
                InputMsg::OpenFailed { path, error } => {
                    self.emit(ServiceEvent::Error(format!(
                        "Opening {}: {error}",
                        path.display()
                    )));
                }
                InputMsg::GrabFailed { .. } => grab_failed = true,
                InputMsg::ReleaseFailed { .. } => {}
            }
        }
        if grab_failed {
            self.command(Action::Off);
        }
    }

    fn status(&mut self) {
        let resyncs = self.inputs.resyncs();
        if resyncs != self.logged_resyncs && self.logged_at.is_none_or(|t| t.elapsed() >= RESYNC_LOG_GAP) {
            self.logged_at = Some(Instant::now());
            eprintln!(
                "PS HID Bridge: input corrected {} time(s) from the kernel's key state ({resyncs} in total); \
                 the kernel had dropped events because the input thread fell behind",
                resyncs - self.logged_resyncs
            );
            self.logged_resyncs = resyncs;
        }
        let st = self.link.status();
        let now = Instant::now();
        let dt = now.duration_since(self.last_sent_at).as_secs_f64();
        let pps = if dt > 0.0 {
            ((st.packets_sent - self.last_sent) as f64 / dt).round() as u32
        } else {
            0
        };
        self.last_sent = st.packets_sent;
        self.last_sent_at = now;
        let history: Vec<f64> = st.rtt_history.iter().copied().collect();
        let status = ServiceStatus {
            connected: st.is_connected(),
            rtt_ms: st.rtt.map(|d| d.as_secs_f64() * 1000.0),
            rtt_p50_ms: percentile(&history, 0.50),
            rtt_p95_ms: percentile(&history, 0.95),
            rtt_history: history,
            board: st.board,
            active: self.ctl.engine.is_active(),
            pending: self.ctl.engine.is_pending(),
            paused: self.ctl.engine.is_paused(),
            packets_per_second: pps,
            input_latency: self.link.input_latency(),
            realtime: self.realtime && self.link.timer_is_realtime(),
            devices_open: self.inputs.len(),
            devices_blocked: self.blocked,
            input_resyncs: self.inputs.resyncs(),
        };
        if status != self.last_status {
            self.last_status = status.clone();
            self.emit(ServiceEvent::Status(status));
        }
    }

    fn handle_input(&mut self, dev: usize, event: Event, ts_us: Option<u64>) {
        // Releases of keys held since before the recording still belong to
        // the engine (it would keep them pressed otherwise).
        let engine_release =
            matches!(event, Event::Key { code, down: false } if !self.record_pressed.contains(&code));
        if let Some(rec) = &mut self.recording
            && !engine_release
        {
            if let Event::Key { code, down } = event {
                if down {
                    rec.insert(code);
                    self.record_pressed.insert(code);
                } else {
                    self.record_pressed.remove(&code);
                    if self.record_pressed.is_empty() && !rec.is_empty() {
                        // Modifiers first, then the rest, as the user expects to read it.
                        let mut codes: Vec<u16> = rec.iter().copied().collect();
                        codes.sort_by_key(|c| (keymap::modifier_bit(*c).is_none(), *c));
                        let combo = Combo(codes.into_iter().map(|c| KeyAlt(vec![c])).collect());
                        let text = normalize_modifiers(&combo.to_text());
                        self.recording = None;
                        self.emit(ServiceEvent::Recorded(text));
                        self.apply_pending_focus();
                    }
                }
            }
            return;
        }
        // Forward first; everything below is for the GUI and may wait.
        let notices = self.ctl.handle(event, Instant::now(), ts_us);
        self.apply_requests();
        self.notices(notices);
        if self.identify
            && let Event::Key { down: true, .. } = event
            && self
                .last_identify
                .is_none_or(|t| t.elapsed() > Duration::from_millis(300))
            && let Some(d) = self.inputs.info(dev)
        {
            self.last_identify = Some(Instant::now());
            self.emit(ServiceEvent::Identified(d.path.clone()));
        }
        let device = self.inputs.info(dev).map(|d| d.name.clone()).unwrap_or_default();
        self.update_live(&device);
    }

    fn focus(&mut self, focused: bool) {
        if self.recording.is_some() {
            // Never let the window rule grab while a trigger is recorded.
            self.pending_focus = Some(focused);
            return;
        }
        let n = self.ctl.handle(Event::Focus(focused), Instant::now(), None);
        self.apply_requests();
        self.notices(n);
    }

    fn apply_pending_focus(&mut self) {
        if let Some(f) = self.pending_focus.take() {
            self.focus(f);
        }
    }

    fn shutdown(&mut self) {
        self.ctl.shutdown();
        self.apply_requests();
        let mut msgs = Vec::new();
        self.inputs.update(&[], &mut msgs);
    }

    fn run(mut self, rx: Receiver<ServiceCmd>) {
        self.realtime = rt::make_current_thread_realtime(rt::PRIORITY).is_ok();
        self.rescan();
        let watchdog = Watchdog::start(self.link.clone(), self.guard.clone(), Duration::from_secs(1));
        let mut next_status = Instant::now();
        let mut msgs = Vec::new();
        loop {
            watchdog.feed();
            // Commands from the GUI
            loop {
                match rx.try_recv() {
                    Ok(ServiceCmd::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        self.shutdown();
                        drop(watchdog);
                        // Dropping the last Link reference sends the
                        // release to the board, rate limit or not.
                        return;
                    }
                    Ok(ServiceCmd::Action(a)) => self.command(a),
                    Ok(ServiceCmd::Focus(f)) => self.focus(f),
                    Ok(ServiceCmd::Record(on)) => {
                        if on {
                            // Never record while the PlayStation gets the keys.
                            self.command(Action::Off);
                        }
                        self.recording = on.then(BTreeSet::new);
                        self.record_pressed.clear();
                        if !on {
                            self.apply_pending_focus();
                        }
                    }
                    Ok(ServiceCmd::Identify(on)) => self.identify = on,
                    Ok(ServiceCmd::MouseCurve(c)) => self.ctl.engine.set_mouse_curve(c),
                    Ok(ServiceCmd::PingInterval(d)) => self.link.set_ping_interval(d),
                    Ok(ServiceCmd::Devices(list)) => self.devices_listed(list),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            // Input (or a wake-up for a command), until the next periodic job
            let now = Instant::now();
            let timeout = self.next_rescan.min(next_status).saturating_duration_since(now);
            self.inputs.wait(timeout, &mut msgs);
            self.handle_msgs(&mut msgs);
            let now = Instant::now();
            if now >= self.next_rescan {
                self.rescan();
            }
            if now >= next_status {
                next_status = now + STATUS;
                self.status();
            }
            if watchdog.take_fired() {
                self.emit(ServiceEvent::Error("Watchdog: devices were released.".into()));
                // Bring the engine in line with what the watchdog did.
                self.command(Action::Off);
            }
        }
    }
}

/// Nearest-rank percentile (q in 0..=1) of the samples.
pub fn percentile(samples: &[f64], q: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len());
    Some(v[rank - 1])
}

/// "LEFTCTRL+LEFTALT+P" style names from recording -> "Ctrl+Alt+P".
fn normalize_modifiers(text: &str) -> String {
    text.split('+')
        .map(|p| match p {
            "LEFTCTRL" | "RIGHTCTRL" => "Ctrl",
            "LEFTSHIFT" | "RIGHTSHIFT" => "Shift",
            "LEFTMETA" | "RIGHTMETA" => "Super",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_text() {
        let s = KeyState {
            mods: 0x02,
            keys: [0x04, 0, 0, 0, 0, 0],
            buttons: 0x01,
        };
        assert_eq!(describe_state(&s), "Shift + A · mouse: left");
        assert_eq!(describe_state(&KeyState::default()), "no key");
    }

    #[test]
    fn percentiles() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.5), Some(50.0));
        assert_eq!(percentile(&v, 0.95), Some(95.0));
        assert_eq!(percentile(&[3.0], 0.95), Some(3.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn recorded_combo_text() {
        assert_eq!(normalize_modifiers("LEFTCTRL+Alt+P"), "Ctrl+Alt+P");
        assert!(Combo::parse(&normalize_modifiers("LEFTCTRL+Alt+P")).is_ok());
    }

    /// Answers pings; counts INPUT packets with RELEASE_ALL and with keys.
    fn fake_board(key: [u8; KEY_LEN]) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicBool>) {
        use crate::crypto::PacketKey;
        use crate::protocol::{Direction, PacketType, Ping, STATUS_USB_MOUNTED};
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let addr = sock.local_addr().unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        thread::spawn(move || {
            let key = PacketKey::new(&key);
            let mut buf = [0u8; 256];
            let mut counter = 1;
            while !s2.load(std::sync::atomic::Ordering::Acquire) {
                let Ok((n, from)) = sock.recv_from(&mut buf) else {
                    continue;
                };
                let Ok((h, payload)) = key.open(Direction::AppToBoard, &buf[..n]) else {
                    continue;
                };
                if h.ptype == PacketType::Ping as u8 {
                    let ping = Ping::decode(&payload);
                    let pong = Pong {
                        t_app_us: ping.t_app_us,
                        seq: ping.seq,
                        session_id: 77,
                        status: STATUS_USB_MOUNTED,
                        ..Default::default()
                    };
                    counter += 1;
                    let pkt = key.seal(Direction::BoardToApp, PacketType::Pong, counter, &pong.encode());
                    let _ = sock.send_to(&pkt, from);
                }
            }
        });
        (addr, stop)
    }

    #[test]
    fn service_reacts_at_once_and_stops_quickly() {
        let key = [3u8; KEY_LEN];
        let (board, stop) = fake_board(key);
        let (tx, rx) = mpsc::channel();
        let sink: Sink = Box::new(move |e| {
            let _ = tx.send(e);
        });
        let svc = Service::start(
            ServiceConfig {
                config: Config::default(),
                key,
                board,
                counter_store: None,
            },
            sink,
        )
        .unwrap();
        let wait_for = |want: &dyn Fn(&ServiceEvent) -> bool, within: Duration| -> bool {
            let end = Instant::now() + within;
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(e) if want(&e) => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
            false
        };
        assert!(wait_for(
            &|e| matches!(e, ServiceEvent::Status(s) if s.connected),
            Duration::from_secs(2)
        ));
        // Commands wake the service thread: no waiting for its 250 ms timer.
        let t = Instant::now();
        svc.send(ServiceCmd::Action(Action::On));
        assert!(wait_for(
            &|e| *e == ServiceEvent::Notice(Notice::Activated),
            Duration::from_millis(500)
        ));
        assert!(t.elapsed() < Duration::from_millis(100), "{:?}", t.elapsed());
        svc.send(ServiceCmd::Action(Action::Off));
        assert!(wait_for(
            &|e| *e == ServiceEvent::Notice(Notice::Deactivated),
            Duration::from_millis(500)
        ));
        let t = Instant::now();
        drop(svc);
        assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
        stop.store(true, std::sync::atomic::Ordering::Release);
    }
}
