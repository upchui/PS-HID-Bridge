//! Connection to the board.
//!
//! Input goes out from the thread that produced it: [`Link::set_state`] and
//! [`Link::add_motion`] encrypt and send right away when the rate limit
//! allows it, so an event costs no hand-over to another thread. The rate
//! limit allows on average one INPUT per millisecond with bursts of two
//! (GCRA), so a 1000 Hz mouse whose reports jitter a little is never held
//! back, while faster mice are combined into one packet per millisecond.
//!
//! A timer thread sends what has to wait: input held back by the rate limit,
//! a HEARTBEAT with the full key state when nothing else was sent for 50 ms
//! while active, and a PING every second. A receiver thread verifies PONGs,
//! learns the board's session id and measures the round trip time. Only a
//! PONG that answers one of our recent pings counts: a recorded old one
//! must neither switch the session nor fake a connection.

use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::clock;
use crate::counter::TxCounter;
use crate::crypto::PacketKey;
use crate::protocol::{Direction, FLAG_RELEASE_ALL, InputPayload, KEY_LEN, PacketType, Ping, Pong};
use crate::rt;

/// Upper bound for key states waiting to be sent (one per millisecond).
const MAX_QUEUED_STATES: usize = 64;

/// How many latency samples (input event -> packet sent) are kept.
pub const LATENCY_HISTORY: usize = 1000;

/// DSCP "Expedited Forwarding": low-latency class; Wi-Fi sends it in the
/// voice queue (WMM), Linux qdiscs in their priority band.
const TOS_EF: libc::c_int = 0xB8;
/// SO_PRIORITY 6 is the highest value allowed without CAP_NET_ADMIN.
const SOCKET_PRIORITY: libc::c_int = 6;
/// A PONG must answer a ping sent at most this long ago.
const PONG_MAX_AGE_US: u64 = 2_000_000;
/// Earliest next ping after a session hint (hints come per input packet).
const HINT_PING_GAP: Duration = Duration::from_millis(5);

pub struct LinkConfig {
    pub board: SocketAddr,
    pub key: [u8; KEY_LEN],
    /// File that keeps the packet counter floor across restarts.
    pub counter_store: Option<PathBuf>,
    pub heartbeat: Duration,
    pub ping_interval: Duration,
    /// Average spacing of INPUT packets (bursts of two are allowed).
    pub min_send_interval: Duration,
    /// Run the timer thread with real-time priority (see [`crate::rt`]).
    pub realtime: bool,
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
            realtime: false,
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

/// How many round-trip samples the status keeps (for the latency view).
pub const RTT_HISTORY: usize = 120;

#[derive(Clone, Debug, Default)]
pub struct LinkStatus {
    pub session_id: Option<u64>,
    pub last_pong: Option<Instant>,
    pub rtt: Option<Duration>,
    /// Most recent round trips in milliseconds, oldest first.
    pub rtt_history: VecDeque<f64>,
    pub board: Option<Pong>,
    pub packets_sent: u64,
    /// Packets the network stack did not take (send buffer full).
    pub send_failures: u64,
    /// Replies that failed verification (wrong key on one side?).
    pub bad_replies: u64,
    /// Authentic replies that did not answer a recent ping (replayed?).
    pub stale_replies: u64,
}

impl LinkStatus {
    pub fn is_connected(&self) -> bool {
        self.last_pong
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }
}

/// Time from an input event (kernel timestamp) until its packet was handed
/// to the network stack, over the last [`LATENCY_HISTORY`] packets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatencyStats {
    pub p50_us: u32,
    pub p95_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
    pub samples: u32,
    /// Samples recorded since the link started (to tell new ones apart).
    pub total: u64,
}

impl LatencyStats {
    pub fn from_samples(samples: &[u32], total: u64) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut v = samples.to_vec();
        v.sort_unstable();
        let at = |q: f64| v[((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
        Some(Self {
            p50_us: at(0.50),
            p95_us: at(0.95),
            p99_us: at(0.99),
            max_us: *v.last().unwrap(),
            samples: v.len() as u32,
            total,
        })
    }
}

struct Shared {
    tx: Mutex<Tx>,
    /// Wakes the timer thread when something is due earlier than it planned.
    timer: Condvar,
    status: Mutex<LinkStatus>,
    session: AtomicU64,
    session_known: AtomicBool,
    stop: AtomicBool,
    epoch: Instant,
    /// Random start of our ping clock, so pings of an earlier run (and
    /// recorded answers to them) never match ours.
    epoch_offset_us: u64,
    packets_sent: AtomicU64,
    send_failures: AtomicU64,
    latency: Mutex<VecDeque<u32>>,
    latency_total: AtomicU64,
    /// Statistics for `latency_total` samples (computed once per change).
    latency_cache: Mutex<Option<LatencyStats>>,
    timer_realtime: AtomicBool,
}

impl Shared {
    fn session(&self) -> Option<u64> {
        self.session_known
            .load(Ordering::Acquire)
            .then(|| self.session.load(Ordering::Relaxed))
    }

    fn micros(&self) -> u64 {
        self.epoch_offset_us + self.epoch.elapsed().as_micros() as u64
    }

    fn record_latency(&self, event_us: u64) {
        let us = clock::now_us().saturating_sub(event_us).min(u32::MAX as u64) as u32;
        let mut l = self.latency.lock().unwrap();
        if l.len() == LATENCY_HISTORY {
            l.pop_front();
        }
        l.push_back(us);
        self.latency_total.fetch_add(1, Ordering::Release);
    }
}

pub struct Link {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

fn set_int_opt(
    sock: &UdpSocket,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    // SAFETY: valid socket and a pointer to an int of the given size.
    let r = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            level,
            name,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Random 64-bit value from the kernel (0 if that fails).
fn random_u64() -> u64 {
    let mut v = 0u64;
    // SAFETY: writes at most 8 bytes into a valid u64.
    unsafe { libc::getrandom((&mut v as *mut u64).cast(), 8, 0) };
    v
}

/// Mark our packets as low-latency traffic (failures are harmless).
fn mark_low_latency(sock: &UdpSocket, ipv4: bool) {
    let _ = if ipv4 {
        set_int_opt(sock, libc::IPPROTO_IP, libc::IP_TOS, TOS_EF)
    } else {
        set_int_opt(sock, libc::IPPROTO_IPV6, libc::IPV6_TCLASS, TOS_EF)
    };
    let _ = set_int_opt(sock, libc::SOL_SOCKET, libc::SO_PRIORITY, SOCKET_PRIORITY);
}

impl Link {
    pub fn start(cfg: LinkConfig) -> io::Result<Self> {
        let bind = if cfg.board.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let sock = UdpSocket::bind(bind)?;
        mark_low_latency(&sock, cfg.board.is_ipv4());
        sock.connect(cfg.board)?;
        // Only a safety net: Drop wakes the receiver by shutting the socket down.
        sock.set_read_timeout(Some(Duration::from_secs(1)))?;
        let sock = Arc::new(sock);
        let now = Instant::now();
        let tx = Tx {
            sock: sock.clone(),
            key: PacketKey::new(&cfg.key),
            counter: TxCounter::new(cfg.counter_store.clone())?,
            heartbeat: cfg.heartbeat,
            interval: cfg.min_send_interval,
            current: KeyState::default(),
            queue: VecDeque::new(),
            motion: Motion::default(),
            motion_ts: None,
            active: false,
            release_pending: false,
            board_state: None,
            sent_session: None,
            last_send: now - Duration::from_secs(1),
            tat: now,
            ping_interval: cfg.ping_interval,
            next_ping: now,
            ping_seq: 0,
            last_ping: now - Duration::from_secs(1),
            timer_due: now,
        };
        let shared = Arc::new(Shared {
            tx: Mutex::new(tx),
            timer: Condvar::new(),
            status: Mutex::new(LinkStatus::default()),
            session: AtomicU64::new(0),
            session_known: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            epoch: now,
            // Below 2^62: room for centuries of microseconds on top.
            epoch_offset_us: (random_u64() >> 2).max(1 << 40),
            packets_sent: AtomicU64::new(0),
            send_failures: AtomicU64::new(0),
            latency: Mutex::new(VecDeque::new()),
            latency_total: AtomicU64::new(0),
            latency_cache: Mutex::new(None),
            timer_realtime: AtomicBool::new(false),
        });

        let rx_thread = {
            let shared = shared.clone();
            let key = PacketKey::new(&cfg.key);
            thread::Builder::new()
                .name("pshb-rx".into())
                .spawn(move || receiver(sock, key, shared))?
        };
        let timer_thread = {
            let shared = shared.clone();
            let realtime = cfg.realtime;
            thread::Builder::new().name("pshb-tx".into()).spawn(move || {
                if realtime {
                    let ok = rt::make_current_thread_realtime(rt::PRIORITY).is_ok();
                    shared.timer_realtime.store(ok, Ordering::Release);
                }
                timer(&shared)
            })?
        };
        Ok(Self {
            shared,
            threads: vec![timer_thread, rx_thread],
        })
    }

    /// Run `f` on the sending state, then send whatever became due.
    fn with_tx(&self, f: impl FnOnce(&mut Tx)) {
        let mut tx = self.shared.tx.lock().unwrap();
        f(&mut tx);
        tx.pump_and_plan(&self.shared, false);
    }

    /// New key/button state (ignored while inactive). `ts_us`: kernel
    /// timestamp of the event that caused it (for the latency statistics).
    pub fn set_state(&self, state: KeyState, ts_us: Option<u64>) {
        self.with_tx(|tx| {
            if tx.active && state != tx.current {
                tx.current = state;
                tx.queue.push_back((state, ts_us));
                if tx.queue.len() > MAX_QUEUED_STATES {
                    // Far behind: keep the order of the newest states.
                    tx.queue.pop_front();
                }
            }
        });
    }

    /// Relative motion to add (ignored while inactive).
    pub fn add_motion(&self, dx: i32, dy: i32, wheel: i32, pan: i32, ts_us: Option<u64>) {
        self.with_tx(|tx| {
            if tx.active {
                let m = &mut tx.motion;
                m.dx = m.dx.saturating_add(dx.into());
                m.dy = m.dy.saturating_add(dy.into());
                m.wheel = m.wheel.saturating_add(wheel.into());
                m.pan = m.pan.saturating_add(pan.into());
                if tx.motion_ts.is_none() {
                    tx.motion_ts = ts_us;
                }
            }
        });
    }

    /// Active: input is forwarded and kept alive by heartbeats.
    /// Inactive: everything is released once, then only pings are sent.
    pub fn set_active(&self, active: bool) {
        self.with_tx(|tx| {
            if tx.active && !active {
                tx.clear_input();
            }
            tx.active = active;
        });
    }

    pub fn release_all(&self) {
        self.with_tx(Tx::clear_input);
    }

    pub fn set_ping_interval(&self, interval: Duration) {
        self.with_tx(|tx| {
            tx.ping_interval = interval;
            tx.next_ping = tx.next_ping.min(Instant::now() + interval);
        });
    }

    pub fn status(&self) -> LinkStatus {
        let mut st = self.shared.status.lock().unwrap();
        if !st.is_connected() && (st.board.is_some() || st.rtt.is_some()) {
            // The board went away: don't keep showing what it last said.
            st.board = None;
            st.rtt = None;
            st.rtt_history.clear();
        }
        let mut st = st.clone();
        st.packets_sent = self.shared.packets_sent.load(Ordering::Relaxed);
        st.send_failures = self.shared.send_failures.load(Ordering::Relaxed);
        st
    }

    /// The board answered a ping within the last 3 s (cheaper than `status`).
    pub fn is_connected(&self) -> bool {
        self.shared.status.lock().unwrap().is_connected()
    }

    /// Input latency over the most recent packets (None: no samples yet).
    pub fn input_latency(&self) -> Option<LatencyStats> {
        let total = self.shared.latency_total.load(Ordering::Acquire);
        let mut cache = self.shared.latency_cache.lock().unwrap();
        if let Some(c) = *cache
            && c.total == total
        {
            return Some(c);
        }
        // Samples and total change together under the latency lock.
        let (samples, total) = {
            let l = self.shared.latency.lock().unwrap();
            let samples: Vec<u32> = l.iter().copied().collect();
            (samples, self.shared.latency_total.load(Ordering::Acquire))
        };
        *cache = LatencyStats::from_samples(&samples, total);
        *cache
    }

    /// The timer thread got real-time priority (only with `realtime`).
    pub fn timer_is_realtime(&self) -> bool {
        self.shared.timer_realtime.load(Ordering::Acquire)
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
        // Release everything on the board before going away, rate limit or not.
        {
            let mut tx = self.shared.tx.lock().unwrap();
            tx.clear_input();
            tx.pump_and_plan(&self.shared, true);
            // Under the lock: the timer thread checks `stop` holding it, so
            // it cannot miss the wake-up below.
            self.shared.stop.store(true, Ordering::Release);
            // Wake the receiver from recv() at once.
            // SAFETY: plain syscall on our socket.
            unsafe { libc::shutdown(tx.sock.as_raw_fd(), libc::SHUT_RD) };
        }
        self.shared.timer.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn receiver(sock: Arc<UdpSocket>, key: PacketKey, shared: Arc<Shared>) {
    let mut buf = [0u8; 512];
    while !shared.stop.load(Ordering::Acquire) {
        let n = match sock.recv(&mut buf) {
            Ok(0) => continue, // shut down by Drop (or an empty datagram)
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
        if pong.is_session_hint() {
            // The board does not know our session (it restarted?). A hint
            // answers nothing we sent and could be replayed, so ask the
            // board directly: the answer to that ping switches the session.
            if changed {
                let mut tx = shared.tx.lock().unwrap();
                tx.ping_soon();
                tx.pump_and_plan(&shared, false);
            }
            continue;
        }
        if pong.t_app_us > now_us || now_us - pong.t_app_us > PONG_MAX_AGE_US {
            shared.status.lock().unwrap().stale_replies += 1;
            continue;
        }
        {
            let mut st = shared.status.lock().unwrap();
            st.session_id = Some(pong.session_id);
            st.last_pong = Some(Instant::now());
            let rtt = Duration::from_micros(now_us - pong.t_app_us);
            st.rtt = Some(rtt);
            if st.rtt_history.len() == RTT_HISTORY {
                st.rtt_history.pop_front();
            }
            st.rtt_history.push_back(rtt.as_secs_f64() * 1000.0);
            st.board = Some(pong);
        }
        if changed {
            shared.session.store(pong.session_id, Ordering::Relaxed);
            shared.session_known.store(true, Ordering::Release);
            // The new session knows nothing of our state: send it now.
            shared.tx.lock().unwrap().pump_and_plan(&shared, false);
        }
    }
}

/// Sends everything that cannot go out right when it is produced.
fn timer(shared: &Shared) {
    let mut tx = shared.tx.lock().unwrap();
    while !shared.stop.load(Ordering::Acquire) {
        tx.pump_and_plan(shared, false);
        let wait = tx.timer_due.saturating_duration_since(Instant::now());
        tx = shared.timer.wait_timeout(tx, wait).unwrap().0;
    }
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

/// Everything about sending, behind one mutex.
///
/// Key/button states are absolute, but every distinct state is sent in
/// order: a key that goes down and up within one interval must still reach
/// the board.
struct Tx {
    sock: Arc<UdpSocket>,
    key: PacketKey,
    counter: TxCounter,
    heartbeat: Duration,
    interval: Duration,
    current: KeyState,                        // latest requested state
    queue: VecDeque<(KeyState, Option<u64>)>, // not yet sent, oldest first, with event time
    motion: Motion,
    motion_ts: Option<u64>, // event time of the oldest motion not yet sent
    active: bool,
    release_pending: bool,
    board_state: Option<KeyState>, // what we last sent
    sent_session: Option<u64>,
    last_send: Instant,
    /// Rate limit (GCRA): theoretical arrival time of the next packet.
    tat: Instant,
    ping_interval: Duration,
    next_ping: Instant,
    ping_seq: u32,
    last_ping: Instant,
    /// When the timer thread wakes up next.
    timer_due: Instant,
}

impl Tx {
    /// Ping as soon as possible (but not more often than every few ms).
    fn ping_soon(&mut self) {
        let at = (self.last_ping + HINT_PING_GAP).max(Instant::now());
        self.next_ping = self.next_ping.min(at);
    }

    fn clear_input(&mut self) {
        self.release_pending = true;
        self.current = KeyState::default();
        self.queue.clear();
        self.motion = Motion::default();
        self.motion_ts = None;
    }

    fn wants_input(&self) -> bool {
        self.release_pending
            || (self.active
                && (!self.queue.is_empty()
                    || !self.motion.is_zero()
                    || self.board_state != Some(self.current)))
    }

    /// Bursts of two: a packet may go out up to one interval early.
    fn may_send(&self, now: Instant) -> bool {
        now + self.interval >= self.tat
    }

    fn send(&mut self, shared: &Shared, ptype: PacketType, payload: &[u8], now: Instant) {
        let counter = self.counter.next_value();
        let pkt = self.key.seal(Direction::AppToBoard, ptype, counter, payload);
        // Never block while holding the Tx lock (the input thread waits on
        // it): a full send buffer drops the packet, the next heartbeat
        // repeats the state.
        // SAFETY: valid buffer for the duration of the call.
        let r = unsafe {
            libc::send(
                self.sock.as_raw_fd(),
                pkt.as_ptr().cast(),
                pkt.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if r == pkt.len() as isize {
            shared.packets_sent.fetch_add(1, Ordering::Relaxed);
        } else if r < 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
            shared.send_failures.fetch_add(1, Ordering::Relaxed);
        }
        if ptype != PacketType::Ping {
            self.last_send = now;
            self.tat = self.tat.max(now) + self.interval;
        }
    }

    fn send_input(&mut self, shared: &Shared, sid: u64, now: Instant) {
        let mut event_ts = None;
        let payload = if self.release_pending {
            self.release_pending = false;
            self.board_state = Some(KeyState::default());
            InputPayload {
                session_id: sid,
                flags: FLAG_RELEASE_ALL,
                ..Default::default()
            }
        } else {
            let state = match self.queue.pop_front() {
                Some((s, ts)) => {
                    event_ts = ts;
                    s
                }
                None => self.current,
            };
            if !self.motion.is_zero() {
                event_ts = match (event_ts, self.motion_ts) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            let (dx, dy, wheel, pan) = self.motion.take();
            if self.motion.is_zero() {
                self.motion_ts = None;
            }
            self.board_state = Some(state);
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
        self.send(shared, PacketType::Input, &payload.encode(), now);
        if let Some(ts) = event_ts {
            shared.record_latency(ts);
        }
    }

    /// Send what is due now (`force`: ignore the rate limit, for shutdown),
    /// then make sure the timer thread wakes up in time for the rest.
    fn pump_and_plan(&mut self, shared: &Shared, force: bool) {
        let now = Instant::now();
        let session = shared.session();
        if session != self.sent_session {
            // New board session: it knows nothing of our state.
            self.board_state = None;
            self.sent_session = session;
        }
        if let Some(sid) = session {
            let mut sent = false;
            while self.wants_input() && (force || self.may_send(now)) {
                self.send_input(shared, sid, now);
                sent = true;
                if force {
                    break;
                }
            }
            if !sent && self.active && now >= self.last_send + self.heartbeat {
                let payload = InputPayload {
                    session_id: sid,
                    mods: self.current.mods,
                    keys: self.current.keys,
                    buttons: self.current.buttons,
                    ..Default::default()
                };
                self.send(shared, PacketType::Heartbeat, &payload.encode(), now);
            }
        } else {
            // Without a session the board would drop motion anyway.
            self.motion = Motion::default();
            self.motion_ts = None;
        }

        if now >= self.next_ping {
            self.ping_seq = self.ping_seq.wrapping_add(1).max(1);
            let ping = Ping {
                t_app_us: shared.micros().max(1),
                seq: self.ping_seq,
            };
            self.send(shared, PacketType::Ping, &ping.encode(), now);
            self.last_ping = now;
            // Ping faster while we are still looking for the board.
            let interval = if session.is_none() {
                self.ping_interval.min(Duration::from_millis(250))
            } else {
                self.ping_interval
            };
            self.next_ping = now + interval;
        }

        let mut due = self.next_ping;
        if session.is_some() {
            if self.wants_input() {
                due = due.min(self.tat.checked_sub(self.interval).unwrap_or(now));
            }
            if self.active {
                due = due.min(self.last_send + self.heartbeat);
            }
        }
        if due < self.timer_due || self.timer_due <= now {
            self.timer_due = due;
            shared.timer.notify_one();
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
        /// Current session; input for another one gets a session hint.
        session: u64,
        /// Answer pings with this instead of their own timestamp.
        fake_t_app_us: Option<u64>,
    }

    impl FakeBoard {
        fn new() -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            Self {
                sock,
                key: PacketKey::new(&KEY),
                counter: 1000,
                session: SESSION,
                fake_t_app_us: None,
            }
        }

        fn reply(&mut self, pong: Pong, to: SocketAddr) {
            self.counter += 1;
            let pkt = self.key.seal(
                Direction::BoardToApp,
                PacketType::Pong,
                self.counter,
                &pong.encode(),
            );
            assert_eq!(pkt.len(), 10 + PONG_LEN + 16);
            self.sock.send_to(&pkt, to).unwrap();
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
                            t_app_us: self.fake_t_app_us.unwrap_or(ping.t_app_us),
                            seq: ping.seq,
                            session_id: self.session,
                            status: STATUS_USB_MOUNTED,
                            ..Default::default()
                        };
                        self.reply(pong, from);
                    }
                    t => {
                        let p = InputPayload::decode(&payload);
                        if p.session_id != self.session {
                            // Like the board: drop it and tell the app.
                            let hint = Pong {
                                session_id: self.session,
                                ..Default::default()
                            };
                            self.reply(hint, from);
                            continue;
                        }
                        got.push((t, p, h.counter));
                    }
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
        link.set_state(
            KeyState {
                mods: 0x02,
                keys: [0x04, 0, 0, 0, 0, 0],
                buttons: 0,
            },
            None,
        );
        for _ in 0..10 {
            link.add_motion(10, -5, 0, 0, None);
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
        link.add_motion(100_000, 0, 300, 0, None);
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
            link.set_state(a, None);
            link.set_state(KeyState::default(), None);
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
        link.set_state(
            KeyState {
                keys: [0x04, 0, 0, 0, 0, 0],
                ..Default::default()
            },
            None,
        );
        link.add_motion(5, 5, 0, 0, None);
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

    #[test]
    fn input_is_sent_by_the_calling_thread() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        board.run_for(Duration::from_millis(20));
        // Idle long enough for the rate limit: the packet must already be in
        // the board's socket when set_state returns.
        thread::sleep(Duration::from_millis(5));
        link.set_state(
            KeyState {
                keys: [0x05, 0, 0, 0, 0, 0],
                ..Default::default()
            },
            Some(clock::now_us()),
        );
        board.sock.set_nonblocking(true).unwrap();
        let mut buf = [0u8; 256];
        let n = board.sock.recv(&mut buf).expect("packet sent synchronously");
        let (_, payload) = board.key.open(Direction::AppToBoard, &buf[..n]).unwrap();
        assert_eq!(InputPayload::decode(&payload).keys[0], 0x05);
        let lat = link.input_latency().expect("latency sample");
        assert_eq!(lat.samples, 1);
        assert!(lat.max_us < 1_000_000);
    }

    #[test]
    fn rate_limit_allows_bursts_of_two() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        board.run_for(Duration::from_millis(20));
        thread::sleep(Duration::from_millis(5));
        let start = Instant::now();
        for i in 0..6u8 {
            link.set_state(
                KeyState {
                    keys: [0x04 + i, 0, 0, 0, 0, 0],
                    ..Default::default()
                },
                None,
            );
        }
        let got = board.run_for(Duration::from_millis(30));
        let keys: Vec<u8> = got
            .iter()
            .map(|(_, p, _)| p.keys[0])
            .filter(|k| *k != 0)
            .collect();
        assert_eq!(keys, vec![0x04, 0x05, 0x06, 0x07, 0x08, 0x09]);
        // Six states: two at once, then one per millisecond.
        assert!(start.elapsed() >= Duration::from_millis(4));
    }

    #[test]
    fn packets_are_marked_low_latency() {
        let board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        let tx = link.shared.tx.lock().unwrap();
        let mut tos: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: valid socket, int buffer and length.
        let r = unsafe {
            libc::getsockopt(
                tx.sock.as_raw_fd(),
                libc::IPPROTO_IP,
                libc::IP_TOS,
                (&mut tos as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        assert_eq!(r, 0);
        assert_eq!(tos, TOS_EF);
    }

    #[test]
    fn answers_to_foreign_pings_are_ignored() {
        // A recorded PONG answers a ping we never sent (other timestamp).
        let mut board = FakeBoard::new();
        board.fake_t_app_us = Some(1_000_000);
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(600));
        assert!(!link.wait_connected(Duration::from_millis(10)));
        let st = link.status();
        assert!(!st.is_connected());
        assert_eq!(st.session_id, None);
        assert!(st.stale_replies >= 2, "{}", st.stale_replies);
    }

    #[test]
    fn session_hint_leads_to_the_new_session_via_a_ping() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        assert!(link.wait_connected(Duration::from_millis(500)));
        link.set_active(true);
        board.run_for(Duration::from_millis(20));
        // The board restarts with a new session.
        board.session = SESSION + 1;
        let started = Instant::now();
        link.set_state(
            KeyState {
                keys: [0x04, 0, 0, 0, 0, 0],
                ..Default::default()
            },
            None,
        );
        let got = board.run_for(Duration::from_millis(60));
        // Hint -> ping -> PONG with the new session -> state sent again.
        assert!(
            got.iter()
                .any(|(t, p, _)| *t == PacketType::Input && p.keys[0] == 0x04),
            "state did not reach the new session"
        );
        assert_eq!(link.status().session_id, Some(SESSION + 1));
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn dropping_the_link_is_quick() {
        let mut board = FakeBoard::new();
        let link = Link::start(LinkConfig::new(board.addr(), KEY)).unwrap();
        board.run_for(Duration::from_millis(50));
        let t = Instant::now();
        drop(link);
        assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
    }

    #[test]
    fn latency_stats() {
        let v: Vec<u32> = (1..=100).collect();
        let s = LatencyStats::from_samples(&v, 100).unwrap();
        assert_eq!(
            (s.p50_us, s.p95_us, s.p99_us, s.max_us, s.samples),
            (50, 95, 99, 100, 100)
        );
        assert!(LatencyStats::from_samples(&[], 0).is_none());
    }
}
