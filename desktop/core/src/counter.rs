//! Packet counter of the app (doubles as AEAD nonce, so it must never repeat
//! under the same key).
//!
//! counter = max(last + 1, unix time in µs, floor loaded at start)
//!
//! A floor (counter + 60 s) is written to disk ahead of time. After a
//! restart with a clock that went backwards the counter still continues
//! above every value used before.
//!
//! The regular refresh (when the counter comes within 30 s of the floor) is
//! written by a background thread, so the packet being sent does not wait
//! for the disk. Only a value that is not yet covered by the floor on disk
//! (the first one after a start, or when the clock went backwards) waits for
//! the write before it is used.
//!
//! The counter file is locked for as long as the counter exists: two
//! processes with the same file (GUI and CLI, two GUIs) would otherwise use
//! the same values.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::lockfile;

const FLOOR_AHEAD_US: u64 = 60_000_000;
const FLOOR_REFRESH_US: u64 = 30_000_000;
/// Stored floors above this are treated as damage (unix time in µs is
/// about 2^50; this leaves room for centuries and rules out overflow).
const FLOOR_MAX: u64 = 1 << 62;

pub struct TxCounter {
    last: u64,
    /// Floor loaded at start: values below it may have been used before.
    start_floor: u64,
    /// Floor requested for the disk (always above every value handed out).
    floor: u64,
    disk: Option<Disk>,
}

/// The counter file, its lock and the thread that writes it.
struct Disk {
    shared: Arc<DiskShared>,
    writer: Option<JoinHandle<()>>,
    /// Held (flock) for the lifetime of the counter.
    _lock: File,
}

struct DiskShared {
    path: PathBuf,
    state: Mutex<DiskState>,
    cond: Condvar,
}

#[derive(Default)]
struct DiskState {
    /// Newest floor requested.
    want: u64,
    /// Newest floor the writer has tried to store (successfully or not).
    attempted: u64,
    /// Floor known to be on disk.
    stored: u64,
    stop: bool,
}

pub fn unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn persist(path: &Path, floor: u64) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp)?;
    writeln!(f, "{floor}")?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

fn warn(e: io::Error) {
    // Not fatal: the clock-based counter keeps working; only a clock going
    // backwards across restarts is no longer covered.
    eprintln!("Warning: could not save the packet counter: {e}");
}

/// Exclusive lock next to the counter file.
fn lock(path: &Path) -> io::Result<File> {
    let lock_path = path.with_extension("lock");
    lockfile::exclusive(&lock_path).map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "another PS HID Bridge program is already sending to the board \
                     ({} is locked); close it first",
                    lock_path.display()
                ),
            )
        } else {
            e
        }
    })
}

fn load(path: &Path) -> io::Result<u64> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    match text.trim().parse::<u64>() {
        Ok(v) if v < FLOOR_MAX => Ok(v),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the packet counter file {} is damaged; removing it is only safe \
                 together with a new key",
                path.display()
            ),
        )),
    }
}

impl Disk {
    fn open(path: PathBuf) -> io::Result<(Self, u64)> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let lock = lock(&path)?;
        let floor = load(&path)?;
        let shared = Arc::new(DiskShared {
            path,
            state: Mutex::new(DiskState {
                want: floor,
                attempted: floor,
                stored: floor,
                stop: false,
            }),
            cond: Condvar::new(),
        });
        let s = shared.clone();
        let writer = thread::Builder::new()
            .name("pshb-counter".into())
            .spawn(move || s.run())?;
        Ok((
            Self {
                shared,
                writer: Some(writer),
                _lock: lock,
            },
            floor,
        ))
    }

    fn stored(&self) -> u64 {
        self.shared.state.lock().unwrap().stored
    }

    /// Ask the writer to store `floor`; with `wait`, return only once it has
    /// tried (a failed write has printed a warning by then).
    fn request(&self, floor: u64, wait: bool) {
        let mut st = self.shared.state.lock().unwrap();
        st.want = st.want.max(floor);
        self.shared.cond.notify_all();
        while wait && st.attempted < floor {
            st = self.shared.cond.wait(st).unwrap();
        }
    }
}

impl DiskShared {
    fn run(&self) {
        let mut st = self.state.lock().unwrap();
        loop {
            while st.want <= st.attempted && !st.stop {
                st = self.cond.wait(st).unwrap();
            }
            if st.want <= st.attempted {
                return; // stop requested and nothing left to write
            }
            let floor = st.want;
            drop(st);
            let result = persist(&self.path, floor);
            st = self.state.lock().unwrap();
            st.attempted = floor;
            match result {
                Ok(()) => st.stored = st.stored.max(floor),
                Err(e) => warn(e),
            }
            self.cond.notify_all();
        }
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        // Let the writer finish what is queued, then release the lock.
        self.shared.state.lock().unwrap().stop = true;
        self.shared.cond.notify_all();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}

impl TxCounter {
    /// `store`: file that keeps the floor across restarts (None = memory
    /// only). Fails if another process uses the same file.
    pub fn new(store: Option<PathBuf>) -> io::Result<Self> {
        let (disk, floor) = match store {
            Some(path) => {
                let (disk, floor) = Disk::open(path)?;
                (Some(disk), floor)
            }
            None => (None, 0),
        };
        Ok(Self {
            last: 0,
            start_floor: floor,
            floor,
            disk,
        })
    }

    pub fn next_value(&mut self) -> u64 {
        self.next_at(unix_us())
    }

    /// Same as [`next_value`](Self::next_value) with an explicit clock (for tests).
    pub fn next_at(&mut self, now_us: u64) -> u64 {
        let c = (self.last + 1).max(now_us).max(self.start_floor);
        self.last = c;
        if c + FLOOR_REFRESH_US > self.floor {
            self.floor = c + FLOOR_AHEAD_US;
            if let Some(disk) = &self.disk {
                // Not covered by the disk yet: a crash after sending could
                // hand out this value again. Wait for the write first.
                let uncovered = c >= disk.stored();
                disk.request(self.floor, uncovered);
            }
        }
        c
    }

    /// Wait until the current floor is on disk (tests; at most `timeout`).
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(disk) = &self.disk else {
            return true;
        };
        let end = Instant::now() + timeout;
        let mut st = disk.shared.state.lock().unwrap();
        while st.stored < self.floor {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            st = disk.shared.cond.wait_timeout(st, left).unwrap().0;
        }
        true
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

    fn stored(path: &Path) -> u64 {
        fs::read_to_string(path).unwrap().trim().parse().unwrap()
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
    fn follows_the_clock_instead_of_jumping_to_the_floor() {
        let path = temp_file("steps");
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let start = 7_000_000_000;
        let first = c.next_at(start);
        assert_eq!(first, start);
        // 1000 packets within the same microsecond: +1 each.
        for i in 1..=1000 {
            assert_eq!(c.next_at(start), start + i);
        }
        // The clock moves on: the counter follows it, nothing more.
        assert_eq!(c.next_at(start + 5_000), start + 5_000);
        assert!(c.flush(Duration::from_secs(2)));
        assert_eq!(stored(&path), start + FLOOR_AHEAD_US);
        let _ = fs::remove_dir_all(path.parent().unwrap());
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
            assert_eq!(v, now);
            assert!(c.flush(Duration::from_secs(2)));
            assert!(stored(&path) > v + FLOOR_REFRESH_US - 1_000_000);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn uncovered_value_is_written_before_it_is_used() {
        let path = temp_file("sync");
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let v = c.next_at(5_000_000_000);
        // No flush: the first value is never covered by an old floor.
        let first_floor = stored(&path);
        assert!(first_floor > v);
        drop(c);
        // Restart with the clock 1 h behind: the value is the stored floor
        // itself, so the next floor must be on disk before it is used.
        let mut c = TxCounter::new(Some(path.clone())).unwrap();
        let w = c.next_at(5_000_000_000 - 3_600_000_000);
        assert_eq!(w, first_floor);
        assert!(stored(&path) > w);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn second_user_of_the_same_file_is_refused() {
        let path = temp_file("lock");
        let first = TxCounter::new(Some(path.clone())).unwrap();
        let err = TxCounter::new(Some(path.clone())).err().expect("must be locked");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        assert!(err.to_string().contains("already sending"), "{err}");
        drop(first);
        // Released with the first counter.
        assert!(TxCounter::new(Some(path.clone())).is_ok());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn damaged_file_is_an_error() {
        for content in ["garbage\n", "18446744073709551615\n", ""] {
            let path = temp_file("damaged");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            let err = TxCounter::new(Some(path.clone())).err().expect(content);
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{content:?}");
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }
}
