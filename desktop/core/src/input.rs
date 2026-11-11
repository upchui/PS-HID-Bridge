//! Input devices via evdev: listing, selection and reading.
//!
//! [`InputSet`] owns the selected devices. The thread that runs the engine
//! waits on all of them with one poll(), so an event is processed and sent
//! by the thread that received it, and grabs change immediately.

use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use evdev::{Device, EventType};

use crate::engine::Event;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub phys: String,
    pub uniq: String,
    pub vendor: u16,
    pub product: u16,
    pub keyboard: bool,
    pub mouse: bool,
    /// We may open the event node (udev rule / group in place).
    pub accessible: bool,
}

const KEY_A: usize = 30;
const KEY_Z: usize = 44;
const KEY_ENTER: usize = 28;
const BTN_LEFT: usize = 0x110;
const REL_X: usize = 0;
const REL_Y: usize = 1;

/// Is bit `n` set in a sysfs capability bitmap ("hex words, most significant first")?
pub fn caps_bit(caps: &str, n: usize) -> bool {
    let bits = usize::BITS as usize;
    caps.split_whitespace()
        .rev()
        .nth(n / bits)
        .and_then(|w| usize::from_str_radix(w, 16).ok())
        .is_some_and(|w| w & (1 << (n % bits)) != 0)
}

fn sysfs(dir: &Path, attr: &str) -> String {
    fs::read_to_string(dir.join(attr))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn readable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 }
}

impl DeviceInfo {
    /// Describe /dev/input/eventN from sysfs (works without access rights).
    pub fn from_sysfs(sys_root: &Path, event: &str) -> Option<Self> {
        let dev = sys_root.join(event).join("device");
        let name = sysfs(&dev, "name");
        if name.is_empty() {
            return None;
        }
        let keys = sysfs(&dev, "capabilities/key");
        let rel = sysfs(&dev, "capabilities/rel");
        let hex = |s: String| u16::from_str_radix(&s, 16).unwrap_or(0);
        let path = PathBuf::from("/dev/input").join(event);
        Some(Self {
            accessible: readable(&path),
            path,
            name,
            phys: sysfs(&dev, "phys"),
            uniq: sysfs(&dev, "uniq"),
            vendor: hex(sysfs(&dev, "id/vendor")),
            product: hex(sysfs(&dev, "id/product")),
            keyboard: caps_bit(&keys, KEY_A) && caps_bit(&keys, KEY_Z) && caps_bit(&keys, KEY_ENTER),
            mouse: caps_bit(&rel, REL_X) && caps_bit(&rel, REL_Y) && caps_bit(&keys, BTN_LEFT),
        })
    }

    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor, self.product)
    }

    pub fn kind(&self) -> &'static str {
        match (self.keyboard, self.mouse) {
            (true, true) => "keyboard + mouse",
            (true, false) => "keyboard",
            (false, true) => "mouse",
            (false, false) => "other",
        }
    }

    /// Does a user-given selector match this device?
    /// Accepted: exact path, "vvvv:pppp" (hex IDs), or part of the name.
    pub fn matches(&self, spec: &str) -> bool {
        if Path::new(spec) == self.path {
            return true;
        }
        if let Some((v, p)) = spec.split_once(':')
            && let (Ok(v), Ok(p)) = (u16::from_str_radix(v, 16), u16::from_str_radix(p, 16))
        {
            return v == self.vendor && p == self.product;
        }
        !spec.is_empty() && self.name.to_lowercase().contains(&spec.to_lowercase())
    }
}

pub struct DeviceList {
    pub devices: Vec<DeviceInfo>,
    /// Keyboards/mice we may not open (missing udev rule / group).
    pub inaccessible: usize,
}

fn event_number(name: &str) -> u32 {
    name.strip_prefix("event")
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

/// All input event devices, described from sysfs (no access rights needed).
pub fn list_devices() -> io::Result<DeviceList> {
    let root = Path::new("/sys/class/input");
    let mut names: Vec<String> = fs::read_dir(root)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("event"))
        .collect();
    names.sort_by_key(|n| event_number(n));
    let devices: Vec<DeviceInfo> = names
        .iter()
        .filter_map(|n| DeviceInfo::from_sysfs(root, n))
        .collect();
    let inaccessible = devices
        .iter()
        .filter(|d| (d.keyboard || d.mouse) && !d.accessible)
        .count();
    Ok(DeviceList {
        devices,
        inaccessible,
    })
}

/// All devices matching any of the selectors (keyboards and mice only).
pub fn select(devices: &[DeviceInfo], specs: &[String]) -> Vec<DeviceInfo> {
    devices
        .iter()
        .filter(|d| (d.keyboard || d.mouse) && specs.iter().any(|s| d.matches(s)))
        .cloned()
        .collect()
}

pub enum InputMsg {
    /// An event from device number `dev` (index into the list given to
    /// [`InputSet::open`]); `ts_us`: kernel timestamp on the monotonic clock
    /// (see [`crate::clock`]), if the device could be switched to it.
    Event {
        dev: usize,
        event: Event,
        ts_us: Option<u64>,
    },
    /// The device disappeared or failed and was closed.
    Gone { path: PathBuf, error: String },
    /// Grabbing failed (e.g. another program holds the device).
    GrabFailed { path: PathBuf, error: String },
}

const EVIOCGRAB: libc::c_ulong = 0x4004_4590; // _IOW('E', 0x90, int)
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0; // _IOW('E', 0xa0, int)

fn ioctl_int(fd: RawFd, req: libc::c_ulong, value: libc::c_int) -> io::Result<()> {
    // SAFETY: both requests take an int argument; fd is a valid descriptor.
    let r = unsafe { libc::ioctl(fd, req as _, &value as *const libc::c_int) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Wakes a thread waiting in [`InputSet::wait`] or [`Waker::wait`] (eventfd).
pub struct Waker {
    fd: OwnedFd,
}

impl Waker {
    pub fn new() -> io::Result<Self> {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh descriptor that nothing else owns.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: writes 8 bytes from a valid u64 to our eventfd.
        unsafe { libc::write(self.fd.as_raw_fd(), (&one as *const u64).cast(), 8) };
    }

    fn clear(&self) {
        let mut v: u64 = 0;
        // SAFETY: reads 8 bytes into a valid u64 from our (non-blocking) eventfd.
        unsafe { libc::read(self.fd.as_raw_fd(), (&mut v as *mut u64).cast(), 8) };
    }

    /// Sleep until woken or the timeout passed (when no devices are open).
    pub fn wait(&self, timeout: Duration) {
        let mut pfd = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the duration of the call.
        unsafe { libc::poll(&mut pfd, 1, poll_ms(timeout)) };
        self.clear();
    }
}

fn poll_ms(d: Duration) -> libc::c_int {
    // Round up: waking too early only costs a loop iteration.
    d.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int
}

/// Lets another thread (the watchdog) release the grab even while the thread
/// that owns the devices hangs. It keeps duplicates of the device
/// descriptors: a grab belongs to the open file, not to one descriptor.
#[derive(Clone, Default)]
pub struct GrabGuard(Arc<GuardInner>);

#[derive(Default)]
struct GuardInner {
    grabbed: AtomicBool,
    fds: Mutex<Vec<OwnedFd>>,
}

impl GrabGuard {
    pub fn is_grabbed(&self) -> bool {
        self.0.grabbed.load(Ordering::Acquire)
    }

    /// Release the grab of every open device (from any thread).
    pub fn release(&self) {
        let fds = self.0.fds.lock().unwrap();
        for fd in fds.iter() {
            let _ = ioctl_int(fd.as_raw_fd(), EVIOCGRAB, 0);
        }
        self.0.grabbed.store(false, Ordering::Release);
    }

    fn set_fds(&self, devices: &[Open]) {
        let fds = devices
            .iter()
            .filter_map(|d| d.dev.as_fd().try_clone_to_owned().ok())
            .collect();
        *self.0.fds.lock().unwrap() = fds;
    }
}

struct Open {
    index: usize,
    info: DeviceInfo,
    dev: Device,
    /// Timestamps are on the monotonic clock.
    mono: bool,
}

/// The open input devices. One thread waits on all of them at once and
/// handles the events itself (no hand-over to another thread), and applies
/// grab changes immediately.
pub struct InputSet {
    devices: Vec<Open>,
    waker: Arc<Waker>,
    guard: GrabGuard,
}

impl InputSet {
    pub fn open(devices: &[DeviceInfo], waker: Arc<Waker>, guard: GrabGuard) -> io::Result<Self> {
        let mut open = Vec::new();
        for (index, info) in devices.iter().enumerate() {
            let dev = Device::open(&info.path)?;
            dev.set_nonblocking(true)?;
            let mono = ioctl_int(dev.as_raw_fd(), EVIOCSCLOCKID, libc::CLOCK_MONOTONIC).is_ok();
            open.push(Open {
                index,
                info: info.clone(),
                dev,
                mono,
            });
        }
        guard.set_fds(&open);
        guard.0.grabbed.store(false, Ordering::Release);
        Ok(Self {
            devices: open,
            waker,
            guard,
        })
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Grab (true) or release every device. Returns the devices that could
    /// not be grabbed; the caller decides what to do (usually: bridge off).
    pub fn set_grab(&mut self, on: bool) -> Vec<InputMsg> {
        let mut failed = Vec::new();
        if self.guard.is_grabbed() == on {
            return failed;
        }
        for d in &self.devices {
            let r = ioctl_int(d.dev.as_raw_fd(), EVIOCGRAB, on as libc::c_int);
            if let (true, Err(e)) = (on, r) {
                failed.push(InputMsg::GrabFailed {
                    path: d.info.path.clone(),
                    error: e.to_string(),
                });
            }
        }
        self.guard.0.grabbed.store(on, Ordering::Release);
        failed
    }

    /// Wait up to `timeout` for input (or [`Waker::wake`]) and append what
    /// arrived to `out`.
    pub fn wait(&mut self, timeout: Duration, out: &mut Vec<InputMsg>) {
        let mut pfds: Vec<libc::pollfd> = self
            .devices
            .iter()
            .map(|d| libc::pollfd {
                fd: d.dev.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        pfds.push(libc::pollfd {
            fd: self.waker.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        // SAFETY: valid pollfd array for the duration of the call.
        let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, poll_ms(timeout)) };
        if r <= 0 {
            return;
        }
        if pfds.last().is_some_and(|p| p.revents != 0) {
            self.waker.clear();
        }
        let mut gone = Vec::new();
        for (i, p) in pfds[..self.devices.len()].iter().enumerate() {
            if p.revents == 0 {
                continue;
            }
            let d = &mut self.devices[i];
            match d.dev.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        if let Some(event) = map_event(&ev) {
                            let ts_us = d.mono.then(|| {
                                ev.timestamp()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|t| t.as_micros() as u64)
                                    .unwrap_or(0)
                            });
                            out.push(InputMsg::Event {
                                dev: d.index,
                                event,
                                ts_us,
                            });
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => gone.push((i, e.to_string())),
            }
        }
        for (i, error) in gone.into_iter().rev() {
            let d = self.devices.remove(i);
            out.push(InputMsg::Gone {
                path: d.info.path,
                error,
            });
        }
        if self.devices.len() + 1 != pfds.len() {
            self.guard.set_fds(&self.devices);
        }
    }
}

impl Drop for InputSet {
    fn drop(&mut self) {
        // The guard's duplicates keep the files open, so the grab would
        // outlive our descriptors: release it explicitly.
        if self.guard.is_grabbed() {
            self.guard.release();
        }
        self.guard.set_fds(&[]);
    }
}

fn map_event(ev: &evdev::InputEvent) -> Option<Event> {
    match ev.event_type() {
        EventType::KEY => match ev.value() {
            0 => Some(Event::Key {
                code: ev.code(),
                down: false,
            }),
            1 => Some(Event::Key {
                code: ev.code(),
                down: true,
            }),
            _ => None, // auto-repeat
        },
        EventType::RELATIVE => Some(Event::Rel {
            axis: ev.code(),
            value: ev.value(),
        }),
        EventType::SYNCHRONIZATION if ev.code() == 0 => Some(Event::Sync),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_bitmaps() {
        // Typical keyboard: words for bits 128..191, 64..127, 0..63
        let caps = "1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe";
        assert!(caps_bit(caps, KEY_A));
        assert!(caps_bit(caps, KEY_ENTER));
        assert!(!caps_bit(caps, 0));
        assert!(!caps_bit("", 5));
        assert!(caps_bit("3", REL_Y) && !caps_bit("3", 2));
    }

    fn dev(name: &str, kb: bool, mouse: bool) -> DeviceInfo {
        DeviceInfo {
            accessible: true,
            path: PathBuf::from("/dev/input/event7"),
            name: name.into(),
            phys: String::new(),
            uniq: String::new(),
            vendor: 0x046d,
            product: 0xc52b,
            keyboard: kb,
            mouse,
        }
    }

    #[test]
    fn selectors() {
        let d = dev("Logitech K400 Plus", true, true);
        assert!(d.matches("/dev/input/event7"));
        assert!(d.matches("046d:c52b"));
        assert!(d.matches("k400"));
        assert!(!d.matches("razer"));
        assert!(!d.matches(""));
        let list = vec![d.clone(), dev("Power Button", false, false)];
        assert_eq!(select(&list, &["logitech".into(), "power".into()]), vec![d]);
    }
}
