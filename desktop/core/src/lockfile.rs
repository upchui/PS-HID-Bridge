//! Exclusive, non-blocking file locks (flock). The lock lasts as long as the
//! returned file is open, and the kernel drops it when the process ends.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

/// Lock `path` (created if missing). Another holder makes this fail with
/// [`io::ErrorKind::WouldBlock`].
pub fn exclusive(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: plain syscall on a valid descriptor.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails_until_the_first_is_closed() {
        let path = std::env::temp_dir().join(format!("pshb-lock-test-{}", std::process::id()));
        let first = exclusive(&path).unwrap();
        assert_eq!(exclusive(&path).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        drop(first);
        assert!(exclusive(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
