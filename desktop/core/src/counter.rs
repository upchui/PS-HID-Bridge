//! Packet counter of the app (doubles as AEAD nonce, so it must never repeat
//! under the same key).
//!
//! counter = max(last + 1, unix time in µs, persisted floor)
//!
//! The floor is written to disk ahead of time (counter + 60 s). After a
//! restart with a clock that went backwards the counter still continues
//! above every value used before.
//!
//! The regular refresh (when the counter comes within 30 s of the floor) is
//! written by a background thread, so the packet being sent does not wait
//! for the disk. Only a value that is not yet covered by the floor on disk
//! (right after a start, or when the clock went backwards) waits for the
//! write before it is used.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FLOOR_AHEAD_US: u64 = 60_000_000;
const FLOOR_REFRESH_US: u64 = 30_000_000;

pub struct TxCounter {
    last: u64,
    floor: u64,
    /// Floor known to be on disk.
    stored: Arc<AtomicU64>,
    store: Option<PathBuf>,
    writer: Option<(Sender<u64>, JoinHandle<()>)>,
}

pub fn unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn persist(path: &Path, floor: u64) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, format!("{floor}\n"))?;
    fs::rename(&tmp, path)
}

fn warn(e: io::Error) {
    // Not fatal: the clock-based counter keeps working; only a clock going
    // backwards across restarts is no longer covered.
    eprintln!("Warning: could not save the packet counter: {e}");
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
        let stored = Arc::new(AtomicU64::new(floor));
        let writer = match &store {
            Some(path) => {
                let (tx, rx) = mpsc::channel::<u64>();
                let (path, stored) = (path.clone(), stored.clone());
                let handle = thread::Builder::new()
                    .name("pshb-counter".into())
                    .spawn(move || {
                        while let Ok(mut floor) = rx.recv() {
                            while let Ok(newer) = rx.try_recv() {
                                floor = newer;
                            }
                            match persist(&path, floor) {
                                Ok(()) => {
                                    stored.fetch_max(floor, Ordering::AcqRel);
                                }
                                Err(e) => warn(e),
                            }
                        }
                    })?;
                Some((tx, handle))
            }
            None => None,
        };
        Ok(Self {
            last: 0,
            floor,
            stored,
            store,
            writer,
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
            if let Some(path) = &self.store {
                if c >= self.stored.load(Ordering::Acquire) {
                    // Not covered by the disk yet: a crash after sending
                    // could hand out this value again. Write first.
                    match persist(path, self.floor) {
                        Ok(()) => {
                            self.stored.fetch_max(self.floor, Ordering::AcqRel);
                        }
                        Err(e) => warn(e),
                    }
                } else if let Some((tx, _)) = &self.writer {
                    let _ = tx.send(self.floor);
                }
            }
        }
        c
    }

    /// Wait until the current floor is on disk (tests; at most `timeout`).
    pub fn flush(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while self.store.is_some() && self.stored.load(Ordering::Acquire) < self.floor {
            if Instant::now() > end {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

impl Drop for TxCounter {
    fn drop(&mut self) {
        // Let the writer finish what is queued.
        if let Some((tx, handle)) = self.writer.take() {
            drop(tx);
            let _ = handle.join();
        }
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
            assert!(c.flush(Duration::from_secs(2)));
            let stored: u64 = fs::read_to_string(&path).unwrap().trim().parse().unwrap();
            assert!(stored > v + FLOOR_REFRESH_US - 1_000_000);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn uncovered_value_is_written_before_it_is_used() {
        let path = temp_file("sync");
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let v = c.next_at(5_000_000_000);
        // No flush: the first value is never covered by an old floor.
        let stored: u64 = fs::read_to_string(&path).unwrap().trim().parse().unwrap();
        assert!(stored > v);
        drop(c);
        // Restart with the clock 1 h behind: the value is the stored floor
        // itself, so the next floor must be on disk before it is used.
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let w = c.next_at(5_000_000_000 - 3_600_000_000);
        assert_eq!(w, stored);
        let again: u64 = fs::read_to_string(&path).unwrap().trim().parse().unwrap();
        assert!(again > w);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
