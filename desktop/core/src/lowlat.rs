//! Latency outside the app: what the PC does with a packet after `send()`
//! and how fast its cores wake up for an input event.
//!
//! * USB network adapters with the `cdc_ncm` driver collect outgoing packets
//!   to send several in one USB transfer: a lone packet waits for the driver's
//!   timer, up to three rounds of `tx_timer_usecs` (400 µs by default), so
//!   about 1 ms. [`adapter_info`] finds out whether the way to the board goes
//!   through such an adapter. 0 switches the waiting off; [`AdapterTimer`]
//!   does that while the app runs and puts it back afterwards, if the user
//!   may write the setting (set up once as root, see
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
const NCM_TIMER: &str = "cdc_ncm/tx_timer_usecs";

/// A network adapter that bundles packets (cdc_ncm).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterInfo {
    pub iface: String,
    /// The driver's collection timer right now (µs, 0 = sends at once); a
    /// lone packet waits up to about three times this long.
    pub timer_us: u32,
    /// This user may change it.
    pub writable: bool,
}

/// Does the way to `board` go through an adapter that can bundle packets?
/// Sends nothing (a UDP "connect" only picks the route).
pub fn adapter_info(board: SocketAddr) -> Option<AdapterInfo> {
    let iface = route_interface(board)?;
    adapter_info_in(Path::new(SYS_CLASS_NET), &iface)
}

/// [`adapter_info`] for a known interface, below `sys_class_net`.
pub fn adapter_info_in(sys_class_net: &Path, iface: &str) -> Option<AdapterInfo> {
    if !valid_iface(iface) {
        return None;
    }
    let timer_us = read_timer(sys_class_net, iface)?;
    Some(AdapterInfo {
        iface: iface.to_string(),
        timer_us,
        writable: may_write(&sys_class_net.join(iface).join(NCM_TIMER)),
    })
}

fn valid_iface(iface: &str) -> bool {
    !iface.is_empty() && !iface.contains('/') && !iface.starts_with('.')
}

fn read_timer(sys_class_net: &Path, iface: &str) -> Option<u32> {
    let text = fs::read_to_string(sys_class_net.join(iface).join(NCM_TIMER)).ok()?;
    text.trim().parse().ok()
}

fn write_timer(sys_class_net: &Path, iface: &str, us: u32) -> io::Result<()> {
    // Truncating like `echo 0 >` (sysfs ignores it, a plain file needs it).
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(sys_class_net.join(iface).join(NCM_TIMER))?
        .write_all(format!("{us}\n").as_bytes())
}

fn may_write(path: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// The adapter towards the board, as the status shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterStatus {
    pub iface: String,
    /// The driver's own timer (µs): what it bundles with when the app does
    /// not switch it (0 = it never bundles).
    pub timer_us: u32,
    /// The app switches it (the user may write the setting).
    pub managed: bool,
    /// It sends at once right now.
    pub at_once_now: bool,
}

/// Switches the bundling of the adapter towards the board off while it is
/// active (the app runs) and back on afterwards, if the user may write the
/// setting.
/// A file notes what to put back, so that a crash does not leave the
/// adapter switched for good: the next start puts it back.
pub struct AdapterTimer {
    sys: PathBuf,
    state_file: Option<PathBuf>,
    info: Option<AdapterInfo>,
    /// The interface the app set to 0 and the value to put back.
    saved: Option<(String, u32)>,
    active: bool,
}

impl AdapterTimer {
    pub fn new(state_file: Option<PathBuf>) -> Self {
        Self::with_root(SYS_CLASS_NET, state_file)
    }

    /// With another directory instead of /sys/class/net (tests).
    pub fn with_root(sys_class_net: impl Into<PathBuf>, state_file: Option<PathBuf>) -> Self {
        let t = Self {
            sys: sys_class_net.into(),
            state_file,
            info: None,
            saved: None,
            active: false,
        };
        t.recover();
        t
    }

    /// Put back what an earlier run left switched (it ended without doing so).
    fn recover(&self) {
        let Some(path) = &self.state_file else {
            return;
        };
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let mut parts = text.split_whitespace();
        if let (Some(iface), Some(Ok(us))) = (parts.next(), parts.next().map(str::parse::<u32>))
            && valid_iface(iface)
            && read_timer(&self.sys, iface) == Some(0)
        {
            let _ = write_timer(&self.sys, iface, us);
        }
        let _ = fs::remove_file(path);
    }

    /// The adapter towards the board (result of [`adapter_info`]).
    pub fn set_adapter(&mut self, info: Option<AdapterInfo>) {
        let same = matches!((&self.saved, &info), (Some((s, _)), Some(i)) if *s == i.iface);
        if self.saved.is_some() && !same {
            self.restore(); // the route changed while switched
        }
        self.info = info;
        self.apply();
    }

    /// Switch (true) or put back.
    pub fn set_active(&mut self, active: bool) {
        if active != self.active {
            self.active = active;
            self.apply();
        }
    }

    fn apply(&mut self) {
        if !self.active {
            self.restore();
            return;
        }
        if let Some((iface, _)) = &self.saved {
            // Switched already. The adapter may have come back since (dock
            // plugged in again, resume) with the driver's default: switch
            // it again, and keep the value noted first for putting back.
            if let Some(info) = self.info.as_mut().filter(|i| i.iface == *iface && i.writable)
                && read_timer(&self.sys, &info.iface).is_some_and(|us| us != 0)
                && write_timer(&self.sys, &info.iface, 0).is_ok()
            {
                info.timer_us = 0;
            }
            return;
        }
        let Some(info) = &mut self.info else {
            return;
        };
        if !info.writable {
            return;
        }
        // Fresh value: the last check may be half a minute old.
        let Some(us) = read_timer(&self.sys, &info.iface) else {
            return;
        };
        if us == 0 {
            info.timer_us = 0; // sends at once anyway (set outside the app)
            return;
        }
        // Noted first: a crash right after must not leave it switched.
        if let Some(p) = &self.state_file {
            if let Some(dir) = p.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(p, format!("{} {us}\n", info.iface));
        }
        if write_timer(&self.sys, &info.iface, 0).is_ok() {
            info.timer_us = 0;
            self.saved = Some((info.iface.clone(), us));
        } else if let Some(p) = &self.state_file {
            let _ = fs::remove_file(p);
        }
    }

    fn restore(&mut self) {
        let Some((iface, us)) = self.saved.take() else {
            return;
        };
        if read_timer(&self.sys, &iface) == Some(0)
            && write_timer(&self.sys, &iface, us).is_ok()
            && let Some(i) = self.info.as_mut().filter(|i| i.iface == iface)
        {
            i.timer_us = us;
        }
        if let Some(p) = &self.state_file {
            let _ = fs::remove_file(p);
        }
    }

    pub fn status(&self) -> Option<AdapterStatus> {
        let info = self.info.as_ref()?;
        let saved = self
            .saved
            .as_ref()
            .filter(|(i, _)| *i == info.iface)
            .map(|(_, us)| *us);
        Some(AdapterStatus {
            iface: info.iface.clone(),
            timer_us: saved.unwrap_or(info.timer_us),
            managed: info.writable,
            at_once_now: info.timer_us == 0,
        })
    }
}

impl Drop for AdapterTimer {
    fn drop(&mut self) {
        self.restore();
    }
}

/// /dev/cpu_dma_latency exists, but this user may not use it: setting up
/// low latency should grant it (see [`crate::access::latency_rule`]).
pub fn cpu_wake_needs_access() -> bool {
    probe(Path::new(CPU_DMA_LATENCY)) == Access::Denied
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

    /// Fake /sys/class/net with cdc_ncm adapters (timer value per name).
    fn fake_net(name: &str, adapters: &[(&str, &str)]) -> PathBuf {
        let root = temp_dir(name);
        for (iface, timer) in adapters {
            let dir = root.join(iface).join("cdc_ncm");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("tx_timer_usecs"), timer).unwrap();
        }
        fs::create_dir_all(root.join("wlan0")).unwrap();
        root
    }

    fn timer(root: &Path, iface: &str) -> u32 {
        read_timer(root, iface).unwrap()
    }

    #[test]
    fn ncm_adapters_are_found() {
        let root = fake_net("net", &[("eth0", "400\n"), ("eth1", "0\n")]);
        assert_eq!(
            adapter_info_in(&root, "eth0"),
            Some(AdapterInfo {
                iface: "eth0".into(),
                timer_us: 400,
                writable: true
            })
        );
        assert_eq!(adapter_info_in(&root, "eth1").map(|i| i.timer_us), Some(0));
        assert_eq!(adapter_info_in(&root, "wlan0"), None);
        assert_eq!(adapter_info_in(&root, "missing"), None);
        assert_eq!(adapter_info_in(&root, "../eth0"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn loopback_is_no_bundling_adapter() {
        assert_eq!(adapter_info("127.0.0.1:45045".parse().unwrap()), None);
        assert_eq!(
            interface_with("127.0.0.1".parse().unwrap()).as_deref(),
            Some("lo")
        );
    }

    #[test]
    fn adapter_sends_at_once_only_while_active() {
        let root = fake_net("timer", &[("eth0", "400\n")]);
        let state = root.join("state/adapter-timer");
        let mut t = AdapterTimer::with_root(&root, Some(state.clone()));
        t.set_adapter(adapter_info_in(&root, "eth0"));
        assert_eq!(timer(&root, "eth0"), 400);
        let st = t.status().unwrap();
        assert!(st.managed && !st.at_once_now && st.timer_us == 400);

        t.set_active(true);
        assert_eq!(timer(&root, "eth0"), 0);
        assert_eq!(fs::read_to_string(&state).unwrap(), "eth0 400\n");
        // The periodic check now sees the 0 the app set.
        t.set_adapter(adapter_info_in(&root, "eth0"));
        let st = t.status().unwrap();
        assert!(st.at_once_now && st.timer_us == 400);

        t.set_active(false);
        assert_eq!(timer(&root, "eth0"), 400);
        assert!(!state.exists());
        assert!(!t.status().unwrap().at_once_now);

        // Dropped while active: put back as well.
        t.set_active(true);
        drop(t);
        assert_eq!(timer(&root, "eth0"), 400);
        assert!(!state.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapter_back_with_its_default_is_switched_again() {
        let root = fake_net("replug", &[("eth0", "400\n")]);
        let mut t = AdapterTimer::with_root(&root, None);
        t.set_adapter(adapter_info_in(&root, "eth0"));
        t.set_active(true);
        assert_eq!(timer(&root, "eth0"), 0);
        // Plugged in again: the driver's default is back.
        write_timer(&root, "eth0", 400).unwrap();
        t.set_adapter(adapter_info_in(&root, "eth0")); // the periodic check
        assert_eq!(timer(&root, "eth0"), 0);
        assert!(t.status().unwrap().at_once_now);
        t.set_active(false);
        assert_eq!(timer(&root, "eth0"), 400);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapter_left_switched_by_a_crash_is_put_back() {
        let root = fake_net("crash", &[("eth0", "0\n")]);
        let state = root.join("adapter-timer");
        fs::write(&state, "eth0 400\n").unwrap();
        let _t = AdapterTimer::with_root(&root, Some(state.clone()));
        assert_eq!(timer(&root, "eth0"), 400);
        assert!(!state.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapter_set_outside_the_app_is_left_alone() {
        let root = fake_net("outside", &[("eth0", "0\n")]);
        let mut t = AdapterTimer::with_root(&root, None);
        t.set_adapter(adapter_info_in(&root, "eth0"));
        t.set_active(true);
        t.set_active(false);
        assert_eq!(timer(&root, "eth0"), 0);
        assert!(t.status().unwrap().at_once_now);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn route_change_puts_the_old_adapter_back() {
        let root = fake_net("route", &[("eth0", "400\n"), ("eth1", "300\n")]);
        let mut t = AdapterTimer::with_root(&root, None);
        t.set_adapter(adapter_info_in(&root, "eth0"));
        t.set_active(true);
        assert_eq!(timer(&root, "eth0"), 0);
        t.set_adapter(adapter_info_in(&root, "eth1"));
        assert_eq!(timer(&root, "eth0"), 400);
        assert_eq!(timer(&root, "eth1"), 0);
        t.set_adapter(None);
        assert_eq!(timer(&root, "eth1"), 300);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapter_the_user_may_not_write_is_not_touched() {
        let root = fake_net("locked", &[("eth0", "400\n")]);
        let file = root.join("eth0").join(NCM_TIMER);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
        let info = adapter_info_in(&root, "eth0").unwrap();
        if !info.writable {
            // (as root everything is writable: tests in a container)
            let mut t = AdapterTimer::with_root(&root, None);
            t.set_adapter(Some(info));
            t.set_active(true);
            assert_eq!(timer(&root, "eth0"), 400);
            let st = t.status().unwrap();
            assert!(!st.managed && !st.at_once_now);
        }
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_dir_all(&root).unwrap();
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
