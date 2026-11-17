//! Exclusive, non-blocking file locks (flock). The lock lasts as long as the
//! returned file is open, and the kernel drops it when the process ends.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::Path;

use sha2::{Digest, Sha256};

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

/// A lock on a key for the whole computer, across users and their state
/// folders: an abstract Unix socket named after a hash of the key (the name
/// tells nothing about the key). It lasts as long as the returned socket is
/// open, the kernel drops it with the process, and a child process does not
/// keep it (close-on-exec). Another holder makes this fail with
/// [`io::ErrorKind::WouldBlock`].
pub fn key_lock(key: &[u8]) -> io::Result<UnixDatagram> {
    let mut h = Sha256::new();
    h.update(b"pshb-sender-lock-v1");
    h.update(key);
    let digest = h.finalize();
    let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    let addr = SocketAddr::from_abstract_name(format!("ps-hid-bridge-sender-{name}"))?;
    UnixDatagram::bind_addr(&addr).map_err(|e| {
        if e.kind() == io::ErrorKind::AddrInUse {
            io::Error::new(io::ErrorKind::WouldBlock, e)
        } else {
            e
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_lock_is_per_key() {
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&std::process::id().to_le_bytes().repeat(2));
        let first = key_lock(&key).unwrap();
        assert_eq!(key_lock(&key).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        key[31] ^= 1;
        let _other = key_lock(&key).unwrap();
        key[31] ^= 1;
        drop(first);
        key_lock(&key).unwrap();
    }

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
