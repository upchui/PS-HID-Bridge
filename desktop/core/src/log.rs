//! Log lines: written to stderr (the journal, for an app started from the
//! desktop) and kept in memory, the most recent [`CAPACITY`] of them, for
//! the diagnostics report (crate::diagnostics).
//!
//! Use [`logln!`](crate::logln) like `eprintln!`. Nothing typed by the user
//! (text sent to the PlayStation, passwords) is ever logged.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Lines kept in memory.
pub const CAPACITY: usize = 300;

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Write `text` to stderr and keep it, with the time (UTC), for the report.
pub fn line(text: &str) {
    eprintln!("{text}");
    keep(text);
}

/// Keep `text` for the report only (frequent, unremarkable events such as
/// the bridge switching on and off: not worth a line in the journal).
pub fn note(text: &str) {
    keep(text);
}

fn keep(text: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = format!("{} {text}", utc_time(secs));
    let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    if ring.len() >= CAPACITY {
        ring.pop_front();
    }
    ring.push_back(entry);
}

/// The kept lines, oldest first.
pub fn recent() -> Vec<String> {
    RING.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

/// "HH:MM:SSZ" of a Unix time.
fn utc_time(secs: u64) -> String {
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// `eprintln!` that also keeps the line for the diagnostics report.
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_most_recent_lines() {
        for i in 0..CAPACITY + 5 {
            keep(&format!("ring test {i}"));
        }
        // (Other tests may log meanwhile.)
        let lines = recent();
        assert!(lines.len() <= CAPACITY);
        let newest = format!("ring test {}", CAPACITY + 4);
        assert!(lines.iter().any(|l| l.ends_with(&newest)));
        assert!(!lines.iter().any(|l| l.ends_with("ring test 4")));
    }

    #[test]
    fn time_of_day() {
        assert_eq!(utc_time(0), "00:00:00Z");
        assert_eq!(utc_time(86_400 + 3600 + 61), "01:01:01Z");
    }
}
