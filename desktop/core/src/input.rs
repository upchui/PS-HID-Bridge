//! Input devices via evdev: listing, selection and reader threads.
//!
//! Every selected device gets its own reader thread, which owns the device
//! and therefore also its grab: the controller only flips a shared flag and
//! the readers apply it within one poll period (50 ms). This avoids ioctls on
//! file descriptors that a vanished device may already have closed.

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

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
    /// An event from device number `dev` (index into the list given to Readers::start).
    Event { dev: usize, event: Event },
    /// The device disappeared or failed; its reader has stopped.
    Gone { path: PathBuf, error: String },
    /// Grabbing failed (e.g. another program holds the device).
    GrabFailed { path: PathBuf, error: String },
}

pub struct Readers {
    grab: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Readers {
    /// `grab`: shared flag; while true, every reader grabs its device.
    pub fn start(devices: &[DeviceInfo], tx: Sender<InputMsg>, grab: Arc<AtomicBool>) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        for (index, info) in devices.iter().enumerate() {
            let dev = Device::open(&info.path)?;
            dev.set_nonblocking(true)?;
            let (grab, stop, tx, path) = (grab.clone(), stop.clone(), tx.clone(), info.path.clone());
            threads.push(
                thread::Builder::new()
                    .name("pshb-input".into())
                    .spawn(move || reader(index, dev, path, grab, stop, tx))?,
            );
        }
        Ok(Self { grab, stop, threads })
    }

    pub fn set_grab(&self, on: bool) {
        self.grab.store(on, Ordering::Release);
    }

    pub fn is_grabbed(&self) -> bool {
        self.grab.load(Ordering::Acquire)
    }

    /// Handle to release the grab from another thread (watchdog).
    pub fn grab_flag(&self) -> Arc<AtomicBool> {
        self.grab.clone()
    }
}

impl Drop for Readers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn reader(
    index: usize,
    mut dev: Device,
    path: PathBuf,
    grab: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    tx: Sender<InputMsg>,
) {
    let fd = dev.as_raw_fd();
    let mut grabbed = false;
    while !stop.load(Ordering::Acquire) {
        let want = grab.load(Ordering::Acquire);
        if want != grabbed {
            let res = if want { dev.grab() } else { dev.ungrab() };
            match res {
                Ok(()) => grabbed = want,
                Err(e) if want => {
                    let _ = tx.send(InputMsg::GrabFailed {
                        path: path.clone(),
                        error: e.to_string(),
                    });
                    // Do not retry in a tight loop; wait for the next change.
                    while grab.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
                        thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                Err(_) => grabbed = false,
            }
        }

        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the duration of the call.
        let r = unsafe { libc::poll(&mut pfd, 1, 50) };
        if r <= 0 {
            continue;
        }
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    let mapped = match ev.event_type() {
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
                    };
                    if let Some(event) = mapped
                        && tx.send(InputMsg::Event { dev: index, event }).is_err()
                    {
                        return;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => {
                let _ = tx.send(InputMsg::Gone {
                    path: path.clone(),
                    error: e.to_string(),
                });
                return;
            }
        }
    }
    if grabbed {
        let _ = dev.ungrab();
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
