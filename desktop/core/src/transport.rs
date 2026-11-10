//! Connection to the board.
//!
//! A sender thread turns state updates into packets: at most one INPUT per
//! millisecond (motion in between is summed up), a HEARTBEAT with the full
//! key state when nothing else was sent for 50 ms while active, and a PING
//! every second. A receiver thread verifies PONGs, learns the board's
//! session id and measures the round trip time.

use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::counter::TxCounter;
use crate::crypto::PacketKey;
use crate::protocol::{Direction, FLAG_RELEASE_ALL, InputPayload, KEY_LEN, PacketType, Ping, Pong};

/// Upper bound for key states waiting to be sent (one per millisecond).
const MAX_QUEUED_STATES: usize = 64;

pub struct LinkConfig {
    pub board: SocketAddr,
    pub key: [u8; KEY_LEN],
    /// File that keeps the packet counter floor across restarts.
    pub counter_store: Option<PathBuf>,
    pub heartbeat: Duration,
    pub ping_interval: Duration,
    pub min_send_interval: Duration,
}

impl LinkConfig {
    pub fn new(board: SocketAddr, key: [u8; KEY_LEN]) -> Self {
        Self {
            board,
            key,
            counter_store: None,
            heartbeat: Duration::from_millis(50),
            ping_interval: Duration::from_secs(1),
            min_send_interval: Duration::from_millis(1),
        }
    }
}

/// Absolute part of the input: modifiers, up to six keys, mouse buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    pub mods: u8,
    pub keys: [u8; 6],
    pub buttons: u8,
}

#[derive(Clone, Debug, Default)]
pub struct LinkStatus {
    pub session_id: Option<u64>,
    pub last_pong: Option<Instant>,
    pub rtt: Option<Duration>,
    pub board: Option<Pong>,
    pub packets_sent: u64,
    /// Replies that failed verification (wrong key on one side?).
    pub bad_replies: u64,
}

impl LinkStatus {
    pub fn is_connected(&self) -> bool {
        self.last_pong
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }
}

enum Cmd {
    State(KeyState),
    Motion(i32, i32, i32, i32),
    Active(bool),
    ReleaseAll,
    PingInterval(Duration),
    SessionChanged,
    Shutdown,
}

struct Shared {
    status: Mutex<LinkStatus>,
    session: AtomicU64,
    session_known: AtomicBool,
    stop: AtomicBool,
    epoch: Instant,
}

impl Shared {
    fn session(&self) -> Option<u64> {
        self.session_known
            .load(Ordering::Acquire)
            .then(|| self.session.load(Ordering::Relaxed))
    }

    fn micros(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }
}

pub struct Link {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Link {
    pub fn start(cfg: LinkConfig) -> io::Result<Self> {
        let bind = if cfg.board.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let sock = UdpSocket::bind(bind)?;
        sock.connect(cfg.board)?;
        sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        let sock = Arc::new(sock);
        let key = Arc::new(PacketKey::new(&cfg.key));
        let counter = TxCounter::new(cfg.counter_store.clone())?;
        let shared = Arc::new(Shared {
            status: Mutex::new(LinkStatus::default()),
            session: AtomicU64::new(0),
            session_known: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            epoch: Instant::now(),
        });
        let (tx, rx) = mpsc::channel();

        let rx_thread = {
            let (sock, key, shared, wake) = (sock.clone(), key.clone(), shared.clone(), tx.clone());
            thread::Builder::new()
                .name("pshb-rx".into())
                .spawn(move || receiver(sock, key, shared, wake))?
        };
        let tx_thread = {
            let shared = shared.clone();
            thread::Builder::new().name("pshb-tx".into()).spawn(move || {
                TxWorker {
                    cfg,
                    sock,
                    key,
                    counter,
                    shared,
                }
                .run(rx)
            })?
        };
        Ok(Self {
            tx,
            shared,
            threads: vec![tx_thread, rx_thread],
        })
    }

    /// New key/button state (ignored while inactive).
    pub fn set_state(&self, state: KeyState) {
        let _ = self.tx.send(Cmd::State(state));
    }

    /// Relative motion to add (ignored while inactive).
    pub fn add_motion(&self, dx: i32, dy: i32, wheel: i32, pan: i32) {
        let _ = self.tx.send(Cmd::Motion(dx, dy, wheel, pan));
    }

    /// Active: input is forwarded and kept alive by heartbeats.
    /// Inactive: everything is released once, then only pings are sent.
    pub fn set_active(&self, active: bool) {
        let _ = self.tx.send(Cmd::Active(active));
    }

    pub fn release_all(&self) {
        let _ = self.tx.send(Cmd::ReleaseAll);
    }

    pub fn set_ping_interval(&self, interval: Duration) {
        let _ = self.tx.send(Cmd::PingInterval(interval));
    }

    pub fn status(&self) -> LinkStatus {
        self.shared.status.lock().unwrap().clone()
    }

    /// Wait until the board answered a ping (session known).
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.shared.session().is_some() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // Release everything on the board before going away.
        let _ = self.tx.send(Cmd::ReleaseAll);
        let _ = self.tx.send(Cmd::Shutdown);
        self.shared.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn receiver(sock: Arc<UdpSocket>, key: Arc<PacketKey>, shared: Arc<Shared>, wake: Sender<Cmd>) {
    let mut buf = [0u8; 512];
    while !shared.stop.load(Ordering::Acquire) {
        let n = match sock.recv(&mut buf) {
            Ok(n) => n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
            Err(_) => {
                // e.g. ECONNREFUSED from an ICMP "port unreachable"
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        let pong = match key.open(Direction::BoardToApp, &buf[..n]) {
            Ok((h, payload)) if h.ptype == PacketType::Pong as u8 => Pong::decode(&payload),
            _ => {
                shared.status.lock().unwrap().bad_replies += 1;
                continue;
            }
        };
        let now_us = shared.micros();
        let changed = shared.session() != Some(pong.session_id);
        {
            let mut st = shared.status.lock().unwrap();
            st.session_id = Some(pong.session_id);
            st.last_pong = Some(Instant::now());
            if !pong.is_session_hint() && pong.t_app_us <= now_us {
                st.rtt = Some(Duration::from_micros(now_us - pong.t_app_us));
            }
            st.board = Some(pong);
        }
        if changed {
            shared.session.store(pong.session_id, Ordering::Relaxed);
            shared.session_known.store(true, Ordering::Release);
            let _ = wake.send(Cmd::SessionChanged);
        }
    }
}

struct TxWorker {
    cfg: LinkConfig,
    sock: Arc<UdpSocket>,
    key: Arc<PacketKey>,
    counter: TxCounter,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Motion {
    dx: i64,
    dy: i64,
    wheel: i64,
    pan: i64,
}

impl Motion {
    fn is_zero(&self) -> bool {
        self.dx == 0 && self.dy == 0 && self.wheel == 0 && self.pan == 0
    }

    /// Take what fits into one packet; the rest stays for the next one.
    fn take(&mut self) -> (i16, i16, i8, i8) {
        fn part(v: &mut i64, lo: i64, hi: i64) -> i64 {
            let p = (*v).clamp(lo, hi);
            *v -= p;
            p
        }
        (
            part(&mut self.dx, i16::MIN.into(), i16::MAX.into()) as i16,
            part(&mut self.dy, i16::MIN.into(), i16::MAX.into()) as i16,
            part(&mut self.wheel, i8::MIN.into(), i8::MAX.into()) as i8,
            part(&mut self.pan, i8::MIN.into(), i8::MAX.into()) as i8,
        )
    }
}

impl TxWorker {
    fn send(&mut self, ptype: PacketType, payload: &[u8]) {
        let counter = self.counter.next_value();
        let pkt = self.key.seal(Direction::AppToBoard, ptype, counter, payload);
        if self.sock.send(&pkt).is_ok() {
            self.shared.status.lock().unwrap().packets_sent += 1;
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        // Key/button states are absolute, but every distinct state is sent in
        // order (at most one packet per min_send_interval): a key that goes
        // down and up within one interval must still reach the board.
        let mut current = KeyState::default(); // latest requested state
        let mut queue: VecDeque<KeyState> = VecDeque::new(); // not yet sent, oldest first
        let mut motion = Motion::default();
        let mut active = false;
        let mut release_pending = false;
        let mut board_state: Option<KeyState> = None; // what we last sent
        let mut sent_session: Option<u64> = None;
        let mut last_send = Instant::now() - Duration::from_secs(1);
        let mut ping_interval = self.cfg.ping_interval;
        let mut next_ping = Instant::now();
        let mut ping_seq: u32 = 0;
        let mut shutdown = false;

        loop {
            let session = self.shared.session();
            let wants_send =
                |queue: &VecDeque<KeyState>, motion: &Motion, board: Option<KeyState>, cur: KeyState| {
                    !queue.is_empty() || !motion.is_zero() || board != Some(cur)
                };
            let pending = session.is_some()
                && (release_pending || (active && wants_send(&queue, &motion, board_state, current)));

            // Sleep until something is due or a command arrives.
            let mut due = next_ping;
            if pending {
                due = due.min(last_send + self.cfg.min_send_interval);
            }
            if active && session.is_some() {
                due = due.min(last_send + self.cfg.heartbeat);
            }
            let mut cmd = match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Cmd::Shutdown),
            };
            while let Some(c) = cmd.take().or_else(|| rx.try_recv().ok()) {
                match c {
                    Cmd::State(s) if active => {
                        if s != current {
                            current = s;
                            queue.push_back(s);
                            if queue.len() > MAX_QUEUED_STATES {
                                // Far behind: keep the order of the newest states.
                                queue.pop_front();
                            }
                        }
                    }
                    Cmd::Motion(dx, dy, w, p) if active => {
                        motion.dx = motion.dx.saturating_add(dx.into());
                        motion.dy = motion.dy.saturating_add(dy.into());
                        motion.wheel = motion.wheel.saturating_add(w.into());
                        motion.pan = motion.pan.saturating_add(p.into());
                    }
                    Cmd::State(_) | Cmd::Motion(..) => {}
                    Cmd::Active(a) => {
                        if active && !a {
                            release_pending = true;
                            current = KeyState::default();
                            queue.clear();
                            motion = Motion::default();
                        }
                        active = a;
                    }
                    Cmd::ReleaseAll => {
                        release_pending = true;
                        current = KeyState::default();
                        queue.clear();
                        motion = Motion::default();
                    }
                    Cmd::PingInterval(d) => {
                        ping_interval = d;
                        next_ping = next_ping.min(Instant::now() + d);
                    }
                    Cmd::SessionChanged => {}
                    Cmd::Shutdown => shutdown = true,
                }
            }

            let session = self.shared.session();
            if session != sent_session {
                // New board session: it knows nothing of our state.
                board_state = None;
                sent_session = session;
            }
            let now = Instant::now();

            if let Some(sid) = session {
                let changed =
                    release_pending || (active && wants_send(&queue, &motion, board_state, current));
                if changed && (shutdown || now >= last_send + self.cfg.min_send_interval) {
                    let payload = if release_pending {
                        release_pending = false;
                        board_state = Some(KeyState::default());
                        InputPayload {
                            session_id: sid,
                            flags: FLAG_RELEASE_ALL,
                            ..Default::default()
                        }
                    } else {
                        let state = queue.pop_front().unwrap_or(current);
                        let (dx, dy, wheel, pan) = motion.take();
                        board_state = Some(state);
                        InputPayload {
                            session_id: sid,
                            flags: 0,
                            mods: state.mods,
                            keys: state.keys,
                            buttons: state.buttons,
                            dx,
                            dy,
                            wheel,
                            pan,
                        }
                    };
                    self.send(PacketType::Input, &payload.encode());
                    last_send = now;
                } else if active && now >= last_send + self.cfg.heartbeat {
                    let payload = InputPayload {
                        session_id: sid,
                        mods: current.mods,
                        keys: current.keys,
                        buttons: current.buttons,
                        ..Default::default()
                    };
                    self.send(PacketType::Heartbeat, &payload.encode());
                    last_send = now;
                }
            } else {
                // Without a session the board would drop motion anyway.
                motion = Motion::default();
            }

            if shutdown {
                break;
            }

            if now >= next_ping {
                ping_seq = ping_seq.wrapping_add(1).max(1);
                let ping = Ping {
                    t_app_us: self.shared.micros().max(1),
                    seq: ping_seq,
                };
                self.send(PacketType::Ping, &ping.encode());
                // Ping faster while we are still looking for the board.
                let interval = if session.is_none() {
                    ping_interval.min(Duration::from_millis(250))
                } else {
                    ping_interval
                };
                next_ping = now + interval;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PONG_LEN, STATUS_USB_MOUNTED};

    const KEY: [u8; 32] = [9; 32];
    const SESSION: u64 = 0x5E55_1011_0000_0001;

    /// Minimal board: answers pings, records everything else.
    struct FakeBoard {
        sock: UdpSocket,
        key: PacketKey,
        counter: u64,
    }

    impl FakeBoard {
        fn new() -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            Self {
                sock,
                key: PacketKey::new(&KEY),
                counter: 1000,
            }
        }

        fn addr(&self) -> SocketAddr {
            self.sock.local_addr().unwrap()
        }

        /// Collect input/heartbeat packets for `dur`, answering pings.
        fn run_for(&mut self, dur: Duration) -> Vec<(PacketType, InputPayload, u64)> {
            let mut got = Vec::new();
            let end = Instant::now() + dur;
            let mut buf = [0u8; 256];
            while Instant::now() < end {
                let Ok((n, from)) = self.sock.recv_from(&mut buf) else {
                    continue;
                };
                let (h, payload) = self.key.open(Direction::AppToBoard, &buf[..n]).unwrap();
                match PacketType::from_u8(h.ptype).unwrap() {
                    PacketType::Ping => {
                        let ping = Ping::decode(&payload);
                        let pong = Pong {
                            t_app_us: ping.t_app_us,
                            seq: ping.seq,
                            session_id: SESSION,
                            status: STATUS_USB_MOUNTED,
                            ..Default::default()
                        };
                        self.counter += 1;
                        let pkt = self.key.seal(
                            Direction::BoardToApp,
                            PacketType::Pong,
                            self.counter,
                            &pong.encode(),
                        );
                        assert_eq!(pkt.len(), 10 + PONG_LEN + 16);
                        self.sock.send_to(&pkt, from).unwrap();
                    }
                    t => got.push((t, InputPayload::decode(&payload), h.counter)),
                }
            }
            got
        }
    }

    #[test]
    fn connects_sends_input_heartbeats_and_releases() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(100));
        assert!(link.wait_connected(Duration::from_millis(500)));
        let st = link.status();
        assert_eq!(st.session_id, Some(SESSION));
        assert!(st.rtt.is_some());

        link.set_active(true);
        link.set_state(KeyState {
            mods: 0x02,
            keys: [0x04, 0, 0, 0, 0, 0],
            buttons: 0,
        });
        for _ in 0..10 {
            link.add_motion(10, -5, 0, 0);
        }
        let got = board.run_for(Duration::from_millis(220));
        let inputs: Vec<_> = got.iter().filter(|(t, ..)| *t == PacketType::Input).collect();
        assert!(!inputs.is_empty());
        assert!(inputs.iter().all(|(_, p, _)| p.session_id == SESSION));
        // Activation may first send the (empty) full state; the latest has the key.
        assert_eq!(inputs.last().unwrap().1.keys[0], 0x04);
        assert_eq!(inputs.last().unwrap().1.mods, 0x02);
        let sum_dx: i32 = inputs.iter().map(|(_, p, _)| p.dx as i32).sum();
        let sum_dy: i32 = inputs.iter().map(|(_, p, _)| p.dy as i32).sum();
        assert_eq!((sum_dx, sum_dy), (100, -50));
        let heartbeats = got.iter().filter(|(t, ..)| *t == PacketType::Heartbeat).count();
        assert!((2..=6).contains(&heartbeats), "heartbeats: {heartbeats}");
        // counters strictly increasing
        assert!(got.windows(2).all(|w| w[0].2 < w[1].2));

        link.set_active(false);
        let got = board.run_for(Duration::from_millis(150));
        assert!(got.iter().any(|(_, p, _)| p.flags & FLAG_RELEASE_ALL != 0));
        assert!(got.iter().all(|(t, ..)| *t != PacketType::Heartbeat));
    }

    #[test]
    fn large_motion_is_split() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        link.add_motion(100_000, 0, 300, 0);
        let got = board.run_for(Duration::from_millis(100));
        let sum: i64 = got.iter().map(|(_, p, _)| p.dx as i64).sum();
        let wheel: i64 = got.iter().map(|(_, p, _)| p.wheel as i64).sum();
        assert_eq!(sum, 100_000);
        assert_eq!(wheel, 300);
    }

    #[test]
    fn short_taps_are_not_merged() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        let a = KeyState {
            keys: [0x04, 0, 0, 0, 0, 0],
            ..Default::default()
        };
        for _ in 0..3 {
            link.set_state(a);
            link.set_state(KeyState::default());
        }
        let got = board.run_for(Duration::from_millis(100));
        let presses = got.iter().filter(|(_, p, _)| p.keys[0] == 0x04).count();
        assert_eq!(presses, 3);
    }

    #[test]
    fn input_ignored_while_inactive() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_state(KeyState {
            keys: [0x04, 0, 0, 0, 0, 0],
            ..Default::default()
        });
        link.add_motion(5, 5, 0, 0);
        let got = board.run_for(Duration::from_millis(100));
        assert!(got.is_empty());
    }

    #[test]
    fn drop_sends_release_all() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        board.run_for(Duration::from_millis(20));
        drop(link);
        let got = board.run_for(Duration::from_millis(50));
        assert!(got.iter().any(|(_, p, _)| p.flags & FLAG_RELEASE_ALL != 0));
    }
}
