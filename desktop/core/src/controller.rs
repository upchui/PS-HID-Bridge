//! Glue between the engine (decisions), the link (board) and the input
//! devices (grab). Shared by the CLI and the GUI.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::engine::{Effect, Engine, Event, Notice};
use crate::input::GrabGuard;
use crate::transport::{KeyState, Link};
use crate::trigger::Action;

pub struct Controller {
    pub engine: Engine,
    link: Arc<Link>,
    grab_request: Option<bool>,
    last_state: Option<KeyState>,
}

impl Controller {
    pub fn new(engine: Engine, link: Arc<Link>) -> Self {
        Self {
            engine,
            link,
            grab_request: None,
            last_state: None,
        }
    }

    /// The key state most recently forwarded to the board (None while inactive).
    pub fn last_state(&self) -> Option<KeyState> {
        if self.engine.is_active() {
            self.last_state
        } else {
            None
        }
    }

    pub fn link(&self) -> &Arc<Link> {
        &self.link
    }

    /// The devices should be grabbed (true) or released: the owner of the
    /// devices applies this after every call to `handle`/`command`.
    pub fn take_grab_request(&mut self) -> Option<bool> {
        self.grab_request.take()
    }

    /// `ts_us`: kernel timestamp of the event (for the latency statistics).
    pub fn handle(&mut self, ev: Event, now: Instant, ts_us: Option<u64>) -> Vec<Notice> {
        let fx = self.engine.handle(ev, now);
        self.apply(fx, ts_us)
    }

    pub fn command(&mut self, action: Action) -> Vec<Notice> {
        let fx = self.engine.command(action);
        self.apply(fx, None)
    }

    fn apply(&mut self, fx: Vec<Effect>, ts_us: Option<u64>) -> Vec<Notice> {
        let mut notices = Vec::new();
        for e in fx {
            match e {
                Effect::Grab(on) => self.grab_request = Some(on),
                Effect::SetActive(a) => self.link.set_active(a),
                Effect::State(s) => {
                    self.last_state = Some(s);
                    self.link.set_state(s, ts_us);
                }
                Effect::Motion(dx, dy, w, p) => self.link.add_motion(dx, dy, w, p, ts_us),
                Effect::ReleaseAll => self.link.release_all(),
                Effect::Notice(n) => notices.push(n),
            }
        }
        notices
    }

    /// Leave a clean state: everything released, devices free (the caller
    /// releases the grab, see [`take_grab_request`](Self::take_grab_request)).
    pub fn shutdown(&mut self) {
        self.command(Action::Off);
        self.link.set_active(false);
        self.grab_request = Some(false);
    }
}

/// Releases the devices and the board if the main loop stops responding
/// (a hung process must never keep keyboard and mouse grabbed).
pub struct Watchdog {
    alive_ms: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    epoch: Instant,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    pub fn start(link: Arc<Link>, grab: GrabGuard, timeout: Duration) -> Self {
        let alive_ms = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let epoch = Instant::now();
        let thread = {
            let (alive_ms, stop, fired) = (alive_ms.clone(), stop.clone(), fired.clone());
            thread::Builder::new()
                .name("pshb-watchdog".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        // Woken early by Drop.
                        thread::park_timeout(Duration::from_millis(100));
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let now = epoch.elapsed().as_millis() as u64;
                        let last = alive_ms.load(Ordering::Acquire);
                        if now.saturating_sub(last) > timeout.as_millis() as u64 && grab.is_grabbed() {
                            grab.release();
                            link.set_active(false);
                            fired.store(true, Ordering::Release);
                        }
                    }
                })
                .expect("spawn watchdog")
        };
        let w = Self {
            alive_ms,
            stop,
            fired,
            epoch,
            thread: Some(thread),
        };
        w.feed();
        w
    }

    pub fn feed(&self) {
        self.alive_ms
            .store(self.epoch.elapsed().as_millis() as u64, Ordering::Release);
    }

    /// True once if the watchdog had to intervene since the last call.
    pub fn take_fired(&self) -> bool {
        self.fired.swap(false, Ordering::AcqRel)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}
