//! Desktop notifications via org.freedesktop.Notifications (Plasma shows
//! them natively). Sent from a helper thread so the UI never waits on D-Bus;
//! each new message replaces the previous one instead of stacking up.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::blocking::Connection;
use zbus::zvariant::Value;

static LAST_ID: AtomicU32 = AtomicU32::new(0);
static CONN: Mutex<Option<Connection>> = Mutex::new(None);

pub fn send(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    std::thread::spawn(move || send_now(&summary, &body));
}

/// Show the notification before returning (e.g. right before exiting).
pub fn send_now(summary: &str, body: &str) {
    let mut guard = CONN.lock().unwrap();
    if guard.is_none() {
        *guard = Connection::session().ok();
    }
    let Some(conn) = guard.as_ref() else { return };
    let hints: HashMap<&str, Value> = HashMap::from([("transient", Value::from(true))]);
    let reply = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "PS HID Bridge",
            LAST_ID.load(Ordering::Relaxed),
            "io.github.thealhu.pshidbridge",
            summary,
            body,
            Vec::<&str>::new(),
            hints,
            3000i32,
        ),
    );
    if let Ok(msg) = reply
        && let Ok(id) = msg.body().deserialize::<u32>()
    {
        LAST_ID.store(id, Ordering::Relaxed);
    }
}
