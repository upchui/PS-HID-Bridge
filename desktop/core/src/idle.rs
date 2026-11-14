//! Keep the screen on while the bridge is on.
//!
//! While the devices are grabbed the desktop sees no input at all and takes
//! the user for away: it dims the screen, turns it off and locks it. As long
//! as the grab lasts we ask it not to, over D-Bus (session bus):
//! * `org.freedesktop.ScreenSaver.Inhibit` (Plasma, Xfce, LXQt, Cinnamon,
//!   MATE; Plasma lists it in the power applet), else
//! * the desktop portal's `Inhibit` with the "idle" flag (GNOME).
//!
//! Both end with our D-Bus connection, so a crash cannot keep the screen on
//! for good. The calls run on a helper thread: the input path never waits
//! for D-Bus.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[cfg_attr(not(feature = "idle"), allow(dead_code))]
const APP: &str = "PS HID Bridge";
#[cfg_attr(not(feature = "idle"), allow(dead_code))]
const REASON: &str = "Mouse and keyboard go to the PlayStation";

/// What the desktop handed out for one inhibition, to end it again.
#[cfg_attr(not(feature = "idle"), allow(dead_code))]
enum Token {
    /// Cookie of org.freedesktop.ScreenSaver.Inhibit.
    ScreenSaver(u32),
    /// Object path of the portal's request.
    Portal(String),
}

trait Backend: Send {
    fn inhibit(&mut self) -> Result<Token, String>;
    fn release(&mut self, token: Token);
}

/// Asks the desktop to keep the screen on while `set(true)` is in force.
pub struct IdleInhibitor {
    on: bool,
    /// The helper thread, started at the first `set(true)`; it ends (and
    /// releases) when this sender is dropped.
    worker: Option<Sender<bool>>,
    backend: Option<Box<dyn Backend>>,
}

impl IdleInhibitor {
    pub fn new() -> Self {
        Self::with_backend(default_backend())
    }

    fn with_backend(backend: Option<Box<dyn Backend>>) -> Self {
        Self {
            on: false,
            worker: None,
            backend,
        }
    }

    /// Keep the screen on (true) or let the desktop handle it again (false).
    /// Never blocks.
    pub fn set(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        if self.worker.is_none() {
            let Some(backend) = self.backend.take().filter(|_| on) else {
                return;
            };
            let (tx, rx) = mpsc::channel();
            let spawned = thread::Builder::new()
                .name("pshb-idle".into())
                .spawn(move || run(backend, rx));
            if let Err(e) = spawned {
                eprintln!("PS HID Bridge: the screen may turn off while the bridge is on ({e}).");
                return;
            }
            self.worker = Some(tx);
        }
        if let Some(tx) = &self.worker {
            let _ = tx.send(on);
        }
    }
}

impl Default for IdleInhibitor {
    fn default() -> Self {
        Self::new()
    }
}

fn run(mut backend: Box<dyn Backend>, rx: Receiver<bool>) {
    let mut held = None;
    let mut warned = false;
    for on in rx {
        if !on {
            if let Some(token) = held.take() {
                backend.release(token);
            }
        } else if held.is_none() {
            match backend.inhibit() {
                Ok(token) => held = Some(token),
                // Tried again at the next switch-on; said once.
                Err(e) if !warned => {
                    warned = true;
                    eprintln!("PS HID Bridge: the screen may turn off while the bridge is on ({e}).");
                }
                Err(_) => {}
            }
        }
    }
    // The inhibitor is gone (bridge stopped, app quits).
    if let Some(token) = held {
        backend.release(token);
    }
}

#[cfg(feature = "idle")]
fn default_backend() -> Option<Box<dyn Backend>> {
    Some(Box::new(dbus::DBus::default()))
}

#[cfg(not(feature = "idle"))]
fn default_backend() -> Option<Box<dyn Backend>> {
    None
}

#[cfg(feature = "idle")]
mod dbus {
    use std::collections::HashMap;

    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::{APP, Backend, REASON, Token};

    const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
    const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const PORTAL_INHIBIT: &str = "org.freedesktop.portal.Inhibit";
    const PORTAL_REQUEST: &str = "org.freedesktop.portal.Request";
    /// Portal Inhibit flag: the session going idle (screen off, lock).
    const PORTAL_IDLE: u32 = 8;

    /// The connection stays open while an inhibition is held: the desktop
    /// ends it when the connection closes.
    #[derive(Default)]
    pub struct DBus {
        conn: Option<Connection>,
    }

    impl DBus {
        fn conn(&mut self) -> Result<&Connection, String> {
            let conn = match self.conn.take() {
                Some(c) => c,
                None => Connection::session().map_err(|e| format!("no D-Bus session: {e}"))?,
            };
            Ok(self.conn.insert(conn))
        }
    }

    impl Backend for DBus {
        fn inhibit(&mut self) -> Result<Token, String> {
            let conn = self.conn()?;
            let screensaver = conn
                .call_method(
                    Some(SCREENSAVER),
                    SCREENSAVER_PATH,
                    Some(SCREENSAVER),
                    "Inhibit",
                    &(APP, REASON),
                )
                .and_then(|m| m.body().deserialize::<u32>());
            let first = match screensaver {
                Ok(cookie) => return Ok(Token::ScreenSaver(cookie)),
                Err(e) => e,
            };
            let options: HashMap<&str, Value> = HashMap::from([("reason", Value::from(REASON))]);
            conn.call_method(
                Some(PORTAL),
                PORTAL_PATH,
                Some(PORTAL_INHIBIT),
                "Inhibit",
                &("", PORTAL_IDLE, options),
            )
            .and_then(|m| m.body().deserialize::<OwnedObjectPath>())
            .map(|handle| Token::Portal(handle.as_str().to_string()))
            .map_err(|e| format!("ScreenSaver: {first}; portal: {e}"))
        }

        fn release(&mut self, token: Token) {
            let Some(conn) = &self.conn else { return };
            let result = match &token {
                Token::ScreenSaver(cookie) => conn
                    .call_method(
                        Some(SCREENSAVER),
                        SCREENSAVER_PATH,
                        Some(SCREENSAVER),
                        "UnInhibit",
                        &(*cookie,),
                    )
                    .map(drop),
                Token::Portal(handle) => conn
                    .call_method(Some(PORTAL), handle.as_str(), Some(PORTAL_REQUEST), "Close", &())
                    .map(drop),
            };
            if let Err(e) = result {
                // Closing the connection ends the inhibition as well.
                eprintln!("PS HID Bridge: releasing the screen ({e}), closing the D-Bus connection.");
                self.conn = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Reports every call; inhibit fails while `fail` says so.
    struct Fake {
        calls: Sender<String>,
        fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
        next: u32,
    }

    impl Backend for Fake {
        fn inhibit(&mut self) -> Result<Token, String> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = self.calls.send("inhibit failed".into());
                return Err("no desktop".into());
            }
            self.next += 1;
            let _ = self.calls.send(format!("inhibit {}", self.next));
            Ok(Token::ScreenSaver(self.next))
        }

        fn release(&mut self, token: Token) {
            let Token::ScreenSaver(n) = token else {
                panic!("unexpected token")
            };
            let _ = self.calls.send(format!("release {n}"));
        }
    }

    fn fake(
        fail: bool,
    ) -> (
        IdleInhibitor,
        Receiver<String>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (tx, rx) = mpsc::channel();
        let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(fail));
        let backend = Fake {
            calls: tx,
            fail: fail.clone(),
            next: 0,
        };
        (IdleInhibitor::with_backend(Some(Box::new(backend))), rx, fail)
    }

    fn expect(rx: &Receiver<String>, call: &str) {
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).as_deref(), Ok(call));
    }

    fn expect_nothing(rx: &Receiver<String>) {
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn one_inhibition_while_on() {
        let (mut idle, rx, _) = fake(false);
        idle.set(false); // nothing to do yet: no thread, no call
        idle.set(true);
        idle.set(true);
        expect(&rx, "inhibit 1");
        expect_nothing(&rx);
        idle.set(false);
        expect(&rx, "release 1");
        idle.set(true);
        expect(&rx, "inhibit 2");
        drop(idle); // bridge stopped while on
        expect(&rx, "release 2");
        expect_nothing(&rx);
    }

    #[test]
    fn failure_is_tried_again_at_the_next_switch_on() {
        let (mut idle, rx, fail) = fake(true);
        idle.set(true);
        expect(&rx, "inhibit failed");
        idle.set(false);
        expect_nothing(&rx); // nothing held, nothing to release
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        idle.set(true);
        expect(&rx, "inhibit 1");
        drop(idle);
        expect(&rx, "release 1");
    }

    #[test]
    fn without_a_backend_nothing_happens() {
        let mut idle = IdleInhibitor::with_backend(None);
        idle.set(true);
        idle.set(false);
        assert!(idle.worker.is_none());
    }

    /// Against the real desktop: holds the inhibition for
    /// PSHB_IDLE_TEST_SECS (default 5) seconds. Meanwhile Plasma lists it:
    /// busctl --user call org.kde.Solid.PowerManagement \
    ///   /org/kde/Solid/PowerManagement/PolicyAgent \
    ///   org.kde.Solid.PowerManagement.PolicyAgent ListInhibitions
    #[cfg(feature = "idle")]
    #[test]
    #[ignore = "needs a desktop session bus"]
    fn desktop_session() {
        let secs = std::env::var("PSHB_IDLE_TEST_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);
        let mut backend = dbus::DBus::default();
        let token = backend.inhibit().expect("inhibit");
        let kind = match &token {
            Token::ScreenSaver(c) => format!("org.freedesktop.ScreenSaver, cookie {c}"),
            Token::Portal(h) => format!("portal, request {h}"),
        };
        eprintln!("inhibited via {kind}");
        std::thread::sleep(Duration::from_secs(secs));
        backend.release(token);
        eprintln!("released");
    }
}
