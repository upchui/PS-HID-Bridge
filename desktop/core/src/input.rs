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

use evdev::{Device, EventType, KeyCode, RelativeAxisCode};

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
}

impl DeviceInfo {
    fn from_device(path: &Path, dev: &Device) -> Self {
        let keys = dev.supported_keys();
        let has_key = |k: KeyCode| keys.is_some_and(|s| s.contains(k));
        let rel = dev.supported_relative_axes();
        let has_rel = |r: RelativeAxisCode| rel.is_some_and(|s| s.contains(r));
        let id = dev.input_id();
        Self {
            path: path.to_path_buf(),
            name: dev.name().unwrap_or("").to_string(),
            phys: dev.physical_path().unwrap_or("").to_string(),
            uniq: dev.unique_name().unwrap_or("").to_string(),
            vendor: id.vendor(),
            product: id.product(),
            keyboard: has_key(KeyCode::KEY_A) && has_key(KeyCode::KEY_Z) && has_key(KeyCode::KEY_ENTER),
            mouse: has_rel(RelativeAxisCode::REL_X)
                && has_rel(RelativeAxisCode::REL_Y)
                && has_key(KeyCode::BTN_LEFT),
        }
    }

    pub fn kind(&self) -> &'static str {
        match (self.keyboard, self.mouse) {
            (true, true) => "Tastatur+Maus",
            (true, false) => "Tastatur",
            (false, true) => "Maus",
            (false, false) => "sonstiges",
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
    /// Event nodes we may not open (missing udev rule / group).
    pub inaccessible: usize,
}

fn event_number(p: &Path) -> u32 {
    p.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("event"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

pub fn list_devices() -> io::Result<DeviceList> {
    let mut paths: Vec<PathBuf> = fs::read_dir("/dev/input")?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    paths.sort_by_key(|p| event_number(p));
    let mut list = DeviceList {
        devices: Vec::new(),
        inaccessible: 0,
    };
    for path in paths {
        match Device::open(&path) {
            Ok(dev) => list.devices.push(DeviceInfo::from_device(&path, &dev)),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => list.inaccessible += 1,
            Err(_) => {}
        }
    }
    Ok(list)
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
    Event(Event),
    /// The device disappeared or failed; its reader has stopped.
    Gone {
        path: PathBuf,
        error: String,
    },
    /// Grabbing failed (e.g. another program holds the device).
    GrabFailed {
        path: PathBuf,
        error: String,
    },
}

pub struct Readers {
    grab: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Readers {
    pub fn start(devices: &[DeviceInfo], tx: Sender<InputMsg>) -> io::Result<Self> {
        let grab = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        for info in devices {
            let dev = Device::open(&info.path)?;
            dev.set_nonblocking(true)?;
            let (grab, stop, tx, path) = (grab.clone(), stop.clone(), tx.clone(), info.path.clone());
            threads.push(
                thread::Builder::new()
                    .name("pshb-input".into())
                    .spawn(move || reader(dev, path, grab, stop, tx))?,
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
                    if let Some(e) = mapped
                        && tx.send(InputMsg::Event(e)).is_err()
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

    fn dev(name: &str, kb: bool, mouse: bool) -> DeviceInfo {
        DeviceInfo {
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
