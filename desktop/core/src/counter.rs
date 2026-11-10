//! Packet counter of the app (doubles as AEAD nonce, so it must never repeat
//! under the same key).
//!
//! counter = max(last + 1, unix time in µs, persisted floor)
//!
//! The floor is written to disk ahead of time (counter + 60 s). After a
//! restart with a clock that went backwards the counter still continues
//! above every value used before.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const FLOOR_AHEAD_US: u64 = 60_000_000;
const FLOOR_REFRESH_US: u64 = 30_000_000;

pub struct TxCounter {
    last: u64,
    floor: u64,
    store: Option<PathBuf>,
}

pub fn unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

impl TxCounter {
    /// `store`: file that keeps the floor across restarts (None = memory only).
    pub fn new(store: Option<PathBuf>) -> io::Result<Self> {
        let floor = match &store {
            Some(path) => match fs::read_to_string(path) {
                Ok(text) => text.trim().parse::<u64>().unwrap_or(0),
                Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
                Err(e) => return Err(e),
            },
            None => 0,
        };
        Ok(Self {
            last: 0,
            floor,
            store,
        })
    }

    pub fn next_value(&mut self) -> u64 {
        self.next_at(unix_us())
    }

    /// Same as [`next_value`](Self::next_value) with an explicit clock (for tests).
    pub fn next_at(&mut self, now_us: u64) -> u64 {
        let c = (self.last + 1).max(now_us).max(self.floor);
        self.last = c;
        if c + FLOOR_REFRESH_US > self.floor {
            self.floor = c + FLOOR_AHEAD_US;
            if let Err(e) = self.persist() {
                // Not fatal: the clock-based counter keeps working; only a
                // clock going backwards across restarts is no longer covered.
                eprintln!("Warning: could not save the packet counter: {e}");
            }
        }
        c
    }

    fn persist(&self) -> io::Result<()> {
        let Some(path) = &self.store else { return Ok(()) };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, format!("{}\n", self.floor))?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pshb-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        dir.join("counter")
    }

    #[test]
    fn strictly_increasing_even_if_clock_stalls_or_goes_back() {
        let mut c = TxCounter::new(None).unwrap();
        let a = c.next_at(1_000_000);
        let b = c.next_at(1_000_000);
        let d = c.next_at(500_000);
        assert!(a < b && b < d);
    }

    #[test]
    fn floor_survives_restart_with_clock_going_back() {
        let path = temp_file("floor");
        let used = {
            let mut c = TxCounter::new(Some(path.clone())).unwrap();
            c.next_at(10_000_000_000)
        };
        // Restart, clock is now 1 hour behind.
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let next = c.next_at(10_000_000_000 - 3_600_000_000);
        assert!(next > used);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn floor_is_refreshed_before_it_is_reached() {
        let path = temp_file("refresh");
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let mut now = 1_000_000_000;
        for _ in 0..200 {
            now += 1_000_000; // one second per step
            let v = c.next_at(now);
            let stored: u64 = fs::read_to_string(&path).unwrap().trim().parse().unwrap();
            assert!(stored > v + FLOOR_REFRESH_US - 1_000_000);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
