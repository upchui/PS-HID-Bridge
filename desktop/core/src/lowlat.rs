//! Latency outside the app: what the PC does with a packet after `send()`
//! and how fast its cores wake up for an input event.
//!
//! * USB network adapters with the `cdc_ncm` driver collect outgoing packets
//!   to send several in one USB transfer: a lone packet waits for the driver's
//!   timer, up to three rounds of `tx_timer_usecs` (400 µs by default), so
//!   about 1 ms. [`adapter_delay`] finds out whether the way to the board
//!   goes through such an adapter; 0 switches the waiting off (root, see
//!   [`crate::access::latency_rule`]).
//! * Idle cores sleep in states that take up to a few hundred microseconds
//!   to wake up from. [`CpuWake`] holds a PM QoS request through
//!   `/dev/cpu_dma_latency` while the bridge is on, so they only use the
//!   light ones; closing the file (also when the process ends) withdraws it.

use std::ffi::CStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

/// Longest wake-up time the cores may need while the bridge is on (µs).
/// Light sleep states (C1/C1E: 1–10 µs) stay allowed, deep ones (C6/C10:
/// hundreds of µs) not.
pub const CPU_WAKE_US: i32 = 20;

const CPU_DMA_LATENCY: &str = "/dev/cpu_dma_latency";
const SYS_CLASS_NET: &str = "/sys/class/net";

/// A network adapter that holds packets back before sending them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterDelay {
    pub iface: String,
    /// The driver's collection timer (µs); a lone packet waits up to about
    /// three times this long.
    pub timer_us: u32,
}

/// Does the way to `board` go through an adapter that holds packets back?
/// Sends nothing (a UDP "connect" only picks the route).
pub fn adapter_delay(board: SocketAddr) -> Option<AdapterDelay> {
    let iface = route_interface(board)?;
    adapter_delay_in(Path::new(SYS_CLASS_NET), &iface)
}

/// [`adapter_delay`] for a known interface, below `sys_class_net`.
pub fn adapter_delay_in(sys_class_net: &Path, iface: &str) -> Option<AdapterDelay> {
    if iface.is_empty() || iface.contains('/') || iface.starts_with('.') {
        return None;
    }
    let text = fs::read_to_string(sys_class_net.join(iface).join("cdc_ncm/tx_timer_usecs")).ok()?;
    let timer_us: u32 = text.trim().parse().ok()?;
    (timer_us > 0).then(|| AdapterDelay {
        iface: iface.to_string(),
        timer_us,
    })
}

/// Name of the interface whose address the kernel would send from to `to`.
fn route_interface(to: SocketAddr) -> Option<String> {
    let bind: SocketAddr = if to.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let sock = UdpSocket::bind(bind).ok()?;
    sock.connect(to).ok()?;
    let local = sock.local_addr().ok()?.ip();
    interface_with(local)
}

fn interface_with(addr: IpAddr) -> Option<String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, freed below with freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut cur = list;
    while !cur.is_null() {
        // SAFETY: `cur` is an element of the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: ifa_addr points to a sockaddr of the family it names.
        let ip = unsafe {
            match i32::from((*ifa.ifa_addr).sa_family) {
                libc::AF_INET => {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    IpAddr::from(u32::from_be(sin.sin_addr.s_addr).to_be_bytes())
                }
                libc::AF_INET6 => {
                    let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    IpAddr::from(sin6.sin6_addr.s6_addr)
                }
                _ => continue,
            }
        };
        if ip == addr {
            // SAFETY: ifa_name is a NUL-terminated string owned by the list.
            found = Some(
                unsafe { CStr::from_ptr(ifa.ifa_name) }
                    .to_string_lossy()
                    .into_owned(),
            );
            break;
        }
    }
    // SAFETY: the list came from getifaddrs and is not used afterwards.
    unsafe { libc::freeifaddrs(list) };
    found
}

/// What [`CpuWake`] does right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuWakeState {
    /// Switched off in the settings.
    #[default]
    Disabled,
    /// The cores are kept out of deep sleep states (the bridge is on).
    Held,
    /// Would work; the bridge is off.
    Ready,
    /// /dev/cpu_dma_latency may not be written by this user.
    NotPermitted,
    /// This system has no /dev/cpu_dma_latency.
    Unavailable,
}

impl CpuWakeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Held => "held",
            Self::Ready => "ready",
            Self::NotPermitted => "not_permitted",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Ok,
    Denied,
    Missing,
}

/// Keeps the cores out of deep sleep states while it is active and enabled.
pub struct CpuWake {
    path: PathBuf,
    file: Option<File>,
    enabled: bool,
    active: bool,
    access: Access,
}

impl CpuWake {
    pub fn new(enabled: bool) -> Self {
        Self::with_path(CPU_DMA_LATENCY, enabled)
    }

    /// With another file instead of /dev/cpu_dma_latency (tests).
    pub fn with_path(path: impl Into<PathBuf>, enabled: bool) -> Self {
        let mut w = Self {
            path: path.into(),
            file: None,
            enabled,
            active: false,
            access: Access::Ok,
        };
        w.check();
        w
    }

    /// The bridge holds the devices (true) or not. Cheap when nothing
    /// changes: called after every input event.
    pub fn set_active(&mut self, active: bool) {
        if active != self.active {
            self.active = active;
            self.apply();
        }
    }

    /// The setting.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.enabled = enabled;
            self.apply();
        }
    }

    /// Look at the permissions again (they may have been set up meanwhile)
    /// and take the request if it is wanted but missing.
    pub fn check(&mut self) {
        self.access = probe(&self.path);
        self.apply();
    }

    pub fn state(&self) -> CpuWakeState {
        if !self.enabled {
            CpuWakeState::Disabled
        } else if self.file.is_some() {
            CpuWakeState::Held
        } else {
            match self.access {
                Access::Ok => CpuWakeState::Ready,
                Access::Denied => CpuWakeState::NotPermitted,
                Access::Missing => CpuWakeState::Unavailable,
            }
        }
    }

    fn apply(&mut self) {
        let want = self.active && self.enabled;
        if !want {
            self.file = None; // closing withdraws the request
            return;
        }
        if self.file.is_some() || self.access != Access::Ok {
            return;
        }
        match open_request(&self.path) {
            Ok(f) => self.file = Some(f),
            Err(e) => self.access = access_of(&e),
        }
    }
}

fn open_request(path: &Path) -> io::Result<File> {
    let mut f = OpenOptions::new().write(true).open(path)?;
    // The kernel takes the value as a binary 32-bit integer.
    f.write_all(&CPU_WAKE_US.to_ne_bytes())?;
    Ok(f)
}

fn probe(path: &Path) -> Access {
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return Access::Missing;
    };
    // SAFETY: valid NUL-terminated path.
    if unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0 {
        Access::Ok
    } else {
        access_of(&io::Error::last_os_error())
    }
}

fn access_of(e: &io::Error) -> Access {
    match e.kind() {
        io::ErrorKind::NotFound => Access::Missing,
        io::ErrorKind::PermissionDenied => Access::Denied,
        _ => Access::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pshb-lowlat-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ncm_adapter_with_timer_holds_packets_back() {
        let root = temp_dir("net");
        for (iface, timer) in [("eth0", Some("400\n")), ("eth1", Some("0\n")), ("wlan0", None)] {
            let dir = root.join(iface);
            fs::create_dir_all(dir.join("cdc_ncm")).unwrap();
            if let Some(t) = timer {
                fs::write(dir.join("cdc_ncm/tx_timer_usecs"), t).unwrap();
            }
        }
        assert_eq!(
            adapter_delay_in(&root, "eth0"),
            Some(AdapterDelay {
                iface: "eth0".into(),
                timer_us: 400
            })
        );
        assert_eq!(adapter_delay_in(&root, "eth1"), None);
        assert_eq!(adapter_delay_in(&root, "wlan0"), None);
        assert_eq!(adapter_delay_in(&root, "missing"), None);
        assert_eq!(adapter_delay_in(&root, "../eth0"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn loopback_is_no_delaying_adapter() {
        assert_eq!(adapter_delay("127.0.0.1:45045".parse().unwrap()), None);
        assert_eq!(
            interface_with("127.0.0.1".parse().unwrap()).as_deref(),
            Some("lo")
        );
    }

    #[test]
    fn request_is_held_only_while_active_and_enabled() {
        let dir = temp_dir("cpu");
        let path = dir.join("cpu_dma_latency");
        fs::write(&path, b"").unwrap();
        let mut w = CpuWake::with_path(&path, true);
        assert_eq!(w.state(), CpuWakeState::Ready);
        w.set_active(true);
        assert_eq!(w.state(), CpuWakeState::Held);
        assert_eq!(fs::read(&path).unwrap(), CPU_WAKE_US.to_ne_bytes());
        w.set_enabled(false);
        assert_eq!(w.state(), CpuWakeState::Disabled);
        w.set_enabled(true);
        assert_eq!(w.state(), CpuWakeState::Held);
        w.set_active(false);
        assert_eq!(w.state(), CpuWakeState::Ready);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_locked_device_is_reported() {
        let dir = temp_dir("cpu-missing");
        let mut w = CpuWake::with_path(dir.join("nothing"), true);
        w.set_active(true);
        assert_eq!(w.state(), CpuWakeState::Unavailable);

        let locked = dir.join("locked");
        fs::write(&locked, b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o444)).unwrap();
        let mut w = CpuWake::with_path(&locked, true);
        // root may write anyway (tests in a container)
        if w.state() != CpuWakeState::Ready {
            w.set_active(true);
            assert_eq!(w.state(), CpuWakeState::NotPermitted);
            // Permissions granted later: the next check takes the request.
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
            w.check();
            assert_eq!(w.state(), CpuWakeState::Held);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
