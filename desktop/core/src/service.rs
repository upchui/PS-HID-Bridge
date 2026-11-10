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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::controller::{Controller, Watchdog};
use crate::engine::{Engine, Event, Notice};
use crate::input::{self, DeviceInfo, InputMsg, Readers};
use crate::keymap;
use crate::mouse::MouseCurve;
use crate::protocol::{KEY_LEN, Pong};
use crate::transport::{KeyState, Link, LinkConfig};
use crate::trigger::{Action, Combo, KeyAlt};

const RESCAN: Duration = Duration::from_secs(2);
const STATUS: Duration = Duration::from_millis(250);

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
    /// Selected devices that are open / that exist but cannot be opened.
    pub devices_open: usize,
    pub devices_blocked: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServiceEvent {
    Status(ServiceStatus),
    Notice(Notice),
    /// Text form of a recorded combination, e.g. "Ctrl+Alt+P".
    Recorded(String),
    /// A selected device produced a key or button press.
    Identified(PathBuf),
    /// What the PS5 currently receives, e.g. "Shift + A · mouse: left".
    Live(String),
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
    thread: Option<JoinHandle<()>>,
}

impl Service {
    pub fn start(cfg: ServiceConfig, sink: Sink) -> Result<Self, String> {
        let engine_cfg = cfg.config.engine_config()?;
        let mut link_cfg = LinkConfig::new(cfg.board, cfg.key);
        link_cfg.counter_store = cfg.counter_store;
        let link = Arc::new(Link::start(link_cfg).map_err(|e| format!("UDP: {e}"))?);
        let (tx, rx) = mpsc::channel();
        let devices = cfg.config.devices.clone();
        let thread = thread::Builder::new()
            .name("pshb-service".into())
            .spawn(move || Runner::new(Engine::new(engine_cfg), link, devices, sink).run(rx))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: ServiceCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn sender(&self) -> Sender<ServiceCmd> {
        self.tx.clone()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(ServiceCmd::Shutdown);
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
    for &u in s.keys.iter().filter(|u| **u != 0) {
        parts.push(if u == 0x01 {
            "too many keys".into()
        } else {
            keymap::usage_name(u)
        });
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
    readers: Option<Readers>,
    open: Vec<DeviceInfo>,
    blocked: usize,
    input_tx: Sender<InputMsg>,
    input_rx: Receiver<InputMsg>,
    grab: Arc<std::sync::atomic::AtomicBool>,
    recording: Option<BTreeSet<u16>>,
    record_pressed: BTreeSet<u16>,
    identify: bool,
    last_identify: Option<Instant>,
    last_status: ServiceStatus,
    last_live: String,
    last_sent: u64,
    last_sent_at: Instant,
}

impl Runner {
    fn new(engine: Engine, link: Arc<Link>, specs: Vec<String>, sink: Sink) -> Self {
        let (input_tx, input_rx) = mpsc::channel();
        let grab = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Self {
            ctl: Controller::new(engine, link.clone(), Some(grab.clone())),
            link,
            specs,
            sink,
            readers: None,
            open: Vec::new(),
            blocked: 0,
            input_tx,
            input_rx,
            grab,
            recording: None,
            record_pressed: BTreeSet::new(),
            identify: false,
            last_identify: None,
            last_status: ServiceStatus::default(),
            last_live: String::new(),
            last_sent: 0,
            last_sent_at: Instant::now(),
        }
    }

    fn emit(&self, e: ServiceEvent) {
        (self.sink)(e);
    }

    /// (Re)open the selected devices if the set of available ones changed.
    fn rescan(&mut self) {
        let list = match input::list_devices() {
            Ok(l) => l,
            Err(e) => {
                self.emit(ServiceEvent::Error(format!("Device list: {e}")));
                return;
            }
        };
        let selected = input::select(&list.devices, &self.specs);
        let wanted: Vec<DeviceInfo> = selected.iter().filter(|d| d.accessible).cloned().collect();
        self.blocked = selected.len() - wanted.len();
        let paths = |v: &[DeviceInfo]| v.iter().map(|d| d.path.clone()).collect::<Vec<_>>();
        if paths(&wanted) == paths(&self.open) && self.readers.is_some() {
            return;
        }
        // Stop the old readers first (they release their grab).
        self.readers = None;
        while self.input_rx.try_recv().is_ok() {}
        // One grab flag for the whole service: controller, watchdog and every
        // generation of readers share it.
        match Readers::start(&wanted, self.input_tx.clone(), self.grab.clone()) {
            Ok(r) => {
                self.readers = Some(r);
                self.open = wanted;
            }
            Err(e) => {
                self.emit(ServiceEvent::Error(format!("Opening device: {e}")));
                self.readers = None;
                self.open.clear();
            }
        }
    }

    fn status(&mut self) {
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
            devices_open: self.open.len(),
            devices_blocked: self.blocked,
        };
        if status != self.last_status {
            self.last_status = status.clone();
            self.emit(ServiceEvent::Status(status));
        }
    }

    fn handle_input(&mut self, dev: usize, event: Event) {
        if let Some(rec) = &mut self.recording {
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
                    }
                }
            }
            return;
        }
        if self.identify
            && let Event::Key { down: true, .. } = event
            && self
                .last_identify
                .is_none_or(|t| t.elapsed() > Duration::from_millis(300))
            && let Some(d) = self.open.get(dev)
        {
            self.last_identify = Some(Instant::now());
            self.emit(ServiceEvent::Identified(d.path.clone()));
        }
        for n in self.ctl.handle(event, Instant::now()) {
            self.emit(ServiceEvent::Notice(n));
        }
        if let Some(s) = self.ctl.last_state() {
            let text = describe_state(&s);
            if text != self.last_live {
                self.last_live = text.clone();
                self.emit(ServiceEvent::Live(text));
            }
        }
    }

    fn run(mut self, rx: Receiver<ServiceCmd>) {
        self.rescan();
        let watchdog = Watchdog::start(self.link.clone(), self.grab.clone(), Duration::from_secs(1));
        let mut next_rescan = Instant::now() + RESCAN;
        let mut next_status = Instant::now();
        loop {
            watchdog.feed();
            // Commands from the GUI
            loop {
                match rx.try_recv() {
                    Ok(ServiceCmd::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        self.ctl.shutdown();
                        drop(watchdog);
                        self.readers = None;
                        thread::sleep(Duration::from_millis(30));
                        return;
                    }
                    Ok(ServiceCmd::Action(a)) => {
                        for n in self.ctl.command(a) {
                            self.emit(ServiceEvent::Notice(n));
                        }
                    }
                    Ok(ServiceCmd::Focus(f)) => {
                        for n in self.ctl.handle(Event::Focus(f), Instant::now()) {
                            self.emit(ServiceEvent::Notice(n));
                        }
                    }
                    Ok(ServiceCmd::Record(on)) => {
                        if on {
                            // Never record while the PS5 gets the keys.
                            for n in self.ctl.command(Action::Off) {
                                self.emit(ServiceEvent::Notice(n));
                            }
                        }
                        self.recording = on.then(BTreeSet::new);
                        self.record_pressed.clear();
                    }
                    Ok(ServiceCmd::Identify(on)) => self.identify = on,
                    Ok(ServiceCmd::MouseCurve(c)) => self.ctl.engine.set_mouse_curve(c),
                    Ok(ServiceCmd::PingInterval(d)) => self.link.set_ping_interval(d),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            // Input
            match self.input_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(InputMsg::Event { dev, event }) => {
                    self.handle_input(dev, event);
                    while let Ok(m) = self.input_rx.try_recv() {
                        if let InputMsg::Event { dev, event } = m {
                            self.handle_input(dev, event);
                        }
                    }
                }
                Ok(InputMsg::Gone { .. }) => next_rescan = Instant::now(),
                Ok(InputMsg::GrabFailed { path, error }) => {
                    self.emit(ServiceEvent::Error(format!(
                        "{} cannot be grabbed exclusively: {error}",
                        path.display()
                    )));
                    for n in self.ctl.command(Action::Off) {
                        self.emit(ServiceEvent::Notice(n));
                    }
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {}
            }
            let now = Instant::now();
            if now >= next_rescan {
                next_rescan = now + RESCAN;
                self.rescan();
            }
            if now >= next_status {
                next_status = now + STATUS;
                self.status();
            }
            if watchdog.take_fired() {
                self.emit(ServiceEvent::Error("Watchdog: devices were released.".into()));
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
}
