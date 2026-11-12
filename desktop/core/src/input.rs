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
use std::time::{Duration, Instant, SystemTime};

use evdev::raw_stream::RawDevice;
use evdev::{EventType, InputEvent, SynchronizationCode};

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
    /// An event from device `dev` (a stable id, see [`InputSet::info`]);
    /// `ts_us`: kernel timestamp on the monotonic clock (see
    /// [`crate::clock`]), if the device could be switched to it. When a
    /// device is closed, key-up events (without timestamp) release whatever
    /// it still held.
    Event {
        dev: usize,
        event: Event,
        ts_us: Option<u64>,
    },
    /// The device disappeared or failed and was closed.
    Gone { path: PathBuf, error: String },
    /// A device could not be opened (it stays closed until the next update).
    OpenFailed { path: PathBuf, error: String },
    /// Grabbing failed; `busy`: another program holds the device exclusively.
    GrabFailed {
        path: PathBuf,
        name: String,
        busy: bool,
        error: String,
    },
    /// Releasing the grab failed. As a last resort the device was closed
    /// (which ends the grab) and opened again, unless `reopened` is false.
    ReleaseFailed {
        path: PathBuf,
        name: String,
        error: String,
        reopened: bool,
    },
    /// Device nodes in /dev/input appeared, disappeared or changed their
    /// permissions: time to look at the device list again.
    Hotplug,
}

const EVIOCGRAB: libc::c_ulong = 0x4004_4590; // _IOW('E', 0x90, int)
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0; // _IOW('E', 0xa0, int)

/// EVIOCSCLOCKID: the kernel reads the int the argument points to.
fn set_clock_id(fd: RawFd, clock: libc::c_int) -> io::Result<()> {
    // SAFETY: valid descriptor; the argument points to an int.
    let r = unsafe { libc::ioctl(fd, EVIOCSCLOCKID as _, &clock as *const libc::c_int) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: plain fcntl calls on a valid descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Open an event node for reading without blocking; true if its timestamps
/// are on the monotonic clock.
fn open_device(path: &Path) -> io::Result<(RawDevice, bool)> {
    let dev = RawDevice::open(path)?;
    set_nonblocking(dev.as_raw_fd())?;
    let mono = set_clock_id(dev.as_raw_fd(), libc::CLOCK_MONOTONIC).is_ok();
    Ok((dev, mono))
}

/// Keys and buttons the kernel says are held right now (EVIOCGKEY).
fn held_keys(dev: &RawDevice) -> Option<Vec<u16>> {
    dev.get_key_state().ok().map(|s| s.iter().map(|k| k.0).collect())
}

/// EVIOCGRAB: unlike EVIOCSCLOCKID the kernel uses the argument's *value*
/// (0 = release, anything else = grab). A pointer is never 0, so passing
/// one turns every release into a second grab that fails with EBUSY.
fn ioctl_grab(fd: RawFd, on: bool) -> io::Result<()> {
    loop {
        // SAFETY: valid descriptor; the argument is passed by value.
        let r = unsafe { libc::ioctl(fd, EVIOCGRAB as _, libc::c_ulong::from(on)) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Release a grab. Also fine if this descriptor held none (EINVAL).
fn release_grab(fd: RawFd) -> io::Result<()> {
    match ioctl_grab(fd, false) {
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => Ok(()),
        r => r,
    }
}

fn grab_failed(info: &DeviceInfo, e: io::Error) -> InputMsg {
    InputMsg::GrabFailed {
        path: info.path.clone(),
        name: info.name.clone(),
        busy: e.raw_os_error() == Some(libc::EBUSY),
        error: e.to_string(),
    }
}

/// One sentence for the user about devices that could not be grabbed or
/// released (None if `msgs` holds no such problem).
pub fn grab_problems(msgs: &[InputMsg]) -> Option<String> {
    let mut grab = Vec::new();
    let mut release = Vec::new();
    for m in msgs {
        match m {
            InputMsg::GrabFailed { name, busy: true, .. } => grab.push(format!(
                "\u{201c}{name}\u{201d} is held exclusively by another program"
            )),
            InputMsg::GrabFailed { name, error, .. } => {
                grab.push(format!("\u{201c}{name}\u{201d} cannot be taken over ({error})"))
            }
            InputMsg::ReleaseFailed {
                name,
                error,
                reopened: true,
                ..
            } => release.push(format!(
                "\u{201c}{name}\u{201d} could not be released ({error}), so it was opened again"
            )),
            InputMsg::ReleaseFailed { name, error, .. } => release.push(format!(
                "\u{201c}{name}\u{201d} could not be released ({error}): unplug it or quit the app"
            )),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if !grab.is_empty() {
        parts.push(format!("{}. The bridge stays off.", grab.join("; ")));
    }
    if !release.is_empty() {
        parts.push(format!("{}.", release.join("; ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
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

    /// Release the grab of every open device (from any thread). Returns
    /// how many could not be released; the owner of the devices retries
    /// with [`InputSet::set_grab`]`(false)`, which reopens them if need be.
    pub fn release(&self) -> usize {
        let fds = self.0.fds.lock().unwrap();
        let failed = fds
            .iter()
            .filter(|fd| release_grab(fd.as_raw_fd()).is_err())
            .count();
        self.0.grabbed.store(false, Ordering::Release);
        failed
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
    /// Stable id (events carry it; indices change when devices come and go).
    id: usize,
    info: DeviceInfo,
    dev: RawDevice,
    /// Timestamps are on the monotonic clock.
    mono: bool,
    /// Keys and buttons currently held on this device.
    pressed: Vec<u16>,
    /// The kernel dropped events: skip up to the end of that frame, then
    /// compare with the kernel's key state (see [`process_events`]).
    dropping: bool,
    /// Forgotten by "release all" while the kernel still reports them held
    /// (a mouse whose receiver lost the release): not pressed again from
    /// the kernel's state until their next real release.
    stale: Vec<u16>,
}

impl Open {
    /// Replace the descriptor by a new one for the same node. The old file
    /// (and with it any grab it holds) is closed once its duplicates in the
    /// [`GrabGuard`] are gone, see [`InputSet::devices_changed`].
    fn reopen(&mut self) -> io::Result<()> {
        let (dev, mono) = open_device(&self.info.path)?;
        self.dev = dev;
        self.mono = mono;
        self.dropping = false;
        self.stale.clear();
        Ok(())
    }
}

/// Notices device nodes appearing, disappearing or changing permissions in
/// /dev/input (inotify), so new devices are picked up at once without
/// scanning sysfs all the time.
struct Hotplug {
    fd: OwnedFd,
}

impl Hotplug {
    fn new() -> io::Result<Self> {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB;
        // SAFETY: valid descriptor and NUL-terminated path.
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c"/dev/input".as_ptr(), mask) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd })
    }

    fn drain(&self) {
        let mut buf = [0u8; 4096];
        // SAFETY: reads into a valid buffer from our non-blocking descriptor.
        while unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    }
}

fn pollfd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

/// The open input devices. One thread waits on all of them at once and
/// handles the events itself (no hand-over to another thread), and applies
/// grab changes immediately.
pub struct InputSet {
    devices: Vec<Open>,
    next_id: usize,
    waker: Arc<Waker>,
    guard: GrabGuard,
    hotplug: Option<Hotplug>,
    /// Devices, then the waker, then the hot-plug watch (kept between waits).
    pfds: Vec<libc::pollfd>,
    /// How often key events had to be corrected from the kernel's key state.
    resyncs: u64,
    /// Next comparison of every device with the kernel's key state.
    next_check: Instant,
}

/// How often every device is compared with the kernel's key state, as a
/// safety net for releases that got lost on the way.
const KEY_CHECK_INTERVAL: Duration = Duration::from_millis(250);

impl InputSet {
    /// No devices yet (see [`update`](Self::update)); watches /dev/input for
    /// hot-plugging if the system allows it.
    pub fn new(waker: Arc<Waker>, guard: GrabGuard) -> Self {
        guard.set_fds(&[]);
        guard.0.grabbed.store(false, Ordering::Release);
        let mut set = Self {
            devices: Vec::new(),
            next_id: 0,
            waker,
            guard,
            hotplug: Hotplug::new().ok(),
            pfds: Vec::new(),
            resyncs: 0,
            next_check: Instant::now() + KEY_CHECK_INTERVAL,
        };
        set.devices_changed();
        set
    }

    /// Exactly these devices; fails if one of them cannot be opened.
    pub fn open(devices: &[DeviceInfo], waker: Arc<Waker>, guard: GrabGuard) -> io::Result<Self> {
        let mut set = Self::new(waker, guard);
        for info in devices {
            set.add(info)?;
        }
        set.devices_changed();
        Ok(set)
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// How often events were lost (the kernel dropped them, or a release
    /// went missing) and were corrected from the kernel's key state.
    pub fn resyncs(&self) -> u64 {
        self.resyncs
    }

    /// "Release all" let these keys and buttons go although the kernel may
    /// still report them held (the mouse or its receiver lost the release):
    /// they count as released and are not pressed again from the kernel's
    /// state until their next real release event clears the mark.
    pub fn forget(&mut self, codes: &[u16]) {
        for d in &mut self.devices {
            for &code in codes {
                if d.pressed.contains(&code) {
                    d.pressed.retain(|c| *c != code);
                    if !d.stale.contains(&code) {
                        d.stale.push(code);
                    }
                }
            }
        }
    }

    /// Hot-plugging is noticed by itself ([`InputMsg::Hotplug`]); without
    /// it the caller has to look at the device list regularly.
    pub fn watches_hotplug(&self) -> bool {
        self.hotplug.is_some()
    }

    /// The device behind an event's `dev`.
    pub fn info(&self, dev: usize) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.id == dev).map(|d| &d.info)
    }

    fn add(&mut self, info: &DeviceInfo) -> io::Result<()> {
        let (dev, mono) = open_device(&info.path)?;
        // Keys held already stay with the desktop (no events for them), but
        // count as held, so the safety net does not press them.
        let pressed = held_keys(&dev).unwrap_or_default();
        self.devices.push(Open {
            id: self.next_id,
            info: info.clone(),
            dev,
            mono,
            pressed,
            dropping: false,
            stale: Vec::new(),
        });
        self.next_id += 1;
        Ok(())
    }

    /// A device was closed: key-up events for what it still held (unless
    /// another open device holds the same key).
    fn release_keys(&self, closed: Open, out: &mut Vec<InputMsg>) {
        for code in closed.pressed {
            if !self.devices.iter().any(|d| d.pressed.contains(&code)) {
                out.push(InputMsg::Event {
                    dev: closed.id,
                    event: Event::Key { code, down: false },
                    ts_us: None,
                });
            }
        }
    }

    /// Bring the open devices in line with `wanted` (compared by path).
    /// Devices that stay keep their descriptor and grab; closed ones release
    /// their keys; new ones are grabbed if the others are.
    pub fn update(&mut self, wanted: &[DeviceInfo], out: &mut Vec<InputMsg>) {
        let mut changed = false;
        let mut i = 0;
        while i < self.devices.len() {
            if wanted.iter().any(|w| w.path == self.devices[i].info.path) {
                i += 1;
                continue;
            }
            let closed = self.devices.remove(i);
            // Closing it below ends the grab in any case.
            let _ = release_grab(closed.dev.as_raw_fd());
            self.release_keys(closed, out);
            changed = true;
        }
        let grabbed = self.guard.is_grabbed();
        for info in wanted {
            if self.devices.iter().any(|d| d.info.path == info.path) {
                continue;
            }
            match self.add(info) {
                Ok(()) => {
                    changed = true;
                    let d = self.devices.last().expect("just added");
                    if grabbed && let Err(e) = ioctl_grab(d.dev.as_raw_fd(), true) {
                        out.push(grab_failed(info, e));
                    }
                }
                // Gone again between listing and opening: the next hot-plug
                // event brings the list up to date.
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ENODEV) => {}
                Err(e) => out.push(InputMsg::OpenFailed {
                    path: info.path.clone(),
                    error: e.to_string(),
                }),
            }
        }
        if changed {
            self.devices_changed();
        }
    }

    fn devices_changed(&mut self) {
        self.guard.set_fds(&self.devices);
        self.pfds.clear();
        self.pfds
            .extend(self.devices.iter().map(|d| pollfd(d.dev.as_raw_fd())));
        self.pfds.push(pollfd(self.waker.fd.as_raw_fd()));
        if let Some(h) = &self.hotplug {
            self.pfds.push(pollfd(h.fd.as_raw_fd()));
        }
    }

    /// Grab (true) or release every device. Returns the devices that could
    /// not be grabbed (the caller decides what to do, usually: bridge off)
    /// or released.
    ///
    /// Releasing always covers every device, whatever the grab flag says,
    /// and a device that cannot be released is closed (which ends the grab)
    /// and opened again: a grab left behind takes keyboard and mouse away
    /// from the desktop.
    pub fn set_grab(&mut self, on: bool) -> Vec<InputMsg> {
        let mut failed = Vec::new();
        if on {
            if !self.guard.is_grabbed() {
                for d in &self.devices {
                    if let Err(e) = ioctl_grab(d.dev.as_raw_fd(), true) {
                        failed.push(grab_failed(&d.info, e));
                    }
                }
                self.guard.0.grabbed.store(true, Ordering::Release);
            }
            return failed;
        }
        let mut reopened = false;
        for d in &mut self.devices {
            let Err(e) = release_grab(d.dev.as_raw_fd()) else {
                continue;
            };
            let ok = d.reopen().is_ok();
            reopened |= ok;
            failed.push(InputMsg::ReleaseFailed {
                path: d.info.path.clone(),
                name: d.info.name.clone(),
                error: e.to_string(),
                reopened: ok,
            });
        }
        self.guard.0.grabbed.store(false, Ordering::Release);
        if reopened {
            // Drops the watchdog's duplicates of the old descriptors: only
            // then is the old file closed and its grab gone.
            self.devices_changed();
        }
        failed
    }

    /// Safety net: compare every device that holds something with the
    /// kernel's key state, so a release that got lost somewhere cannot keep
    /// a key held. Runs before the next batch is read, when everything read
    /// so far is processed. EVIOCGKEY deletes the key events still queued
    /// (its state includes them), so the whole state is applied: presses
    /// too, or a click queued right now would be lost.
    fn check_keys(&mut self, out: &mut Vec<InputMsg>) {
        self.next_check = Instant::now() + KEY_CHECK_INTERVAL;
        for d in &mut self.devices {
            if d.pressed.is_empty() || d.dropping {
                continue;
            }
            let Some(held) = held_keys(&d.dev) else {
                continue;
            };
            let missed = resync(&mut d.pressed, &mut d.stale, &held);
            if missed.is_empty() {
                continue;
            }
            self.resyncs += 1;
            for event in missed.into_iter().chain([Event::Sync]) {
                out.push(InputMsg::Event {
                    dev: d.id,
                    event,
                    ts_us: None,
                });
            }
        }
    }

    /// Wait up to `timeout` for input, hot-plugging or [`Waker::wake`] and
    /// append what arrived to `out`.
    pub fn wait(&mut self, timeout: Duration, out: &mut Vec<InputMsg>) {
        let timeout = timeout.min(self.next_check.saturating_duration_since(Instant::now()));
        // SAFETY: valid pollfd array for the duration of the call.
        let r = unsafe {
            libc::poll(
                self.pfds.as_mut_ptr(),
                self.pfds.len() as libc::nfds_t,
                poll_ms(timeout),
            )
        };
        if r > 0 {
            self.read_devices(out);
        }
        // After the batches: the safety net's state query deletes the key
        // events still queued, so nothing read but unprocessed may be left.
        if Instant::now() >= self.next_check {
            self.check_keys(out);
        }
    }

    /// Read every device that poll() reported and turn its events into
    /// messages; devices that fail are closed (their keys released).
    fn read_devices(&mut self, out: &mut Vec<InputMsg>) {
        let n = self.devices.len();
        if self.pfds[n].revents != 0 {
            self.waker.clear();
        }
        if let Some(h) = &self.hotplug
            && self.pfds[n + 1].revents != 0
        {
            h.drain();
            out.push(InputMsg::Hotplug);
        }
        let mut gone = Vec::new();
        for i in 0..n {
            if self.pfds[i].revents == 0 {
                continue;
            }
            let Open {
                id,
                dev,
                mono,
                pressed,
                dropping,
                stale,
                ..
            } = &mut self.devices[i];
            let events: Vec<InputEvent> = match dev.fetch_events() {
                Ok(events) => events.collect(),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    gone.push((i, e.to_string()));
                    continue;
                }
            };
            let mut got = Vec::with_capacity(events.len());
            if process_events(events, pressed, stale, dropping, || held_keys(dev), &mut got) {
                self.resyncs += 1;
            }
            for (event, ts) in got {
                let ts_us = ts.filter(|_| *mono).map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|t| t.as_micros() as u64)
                        .unwrap_or(0)
                });
                out.push(InputMsg::Event {
                    dev: *id,
                    event,
                    ts_us,
                });
            }
        }
        if gone.is_empty() {
            return;
        }
        for (i, error) in gone.into_iter().rev() {
            let d = self.devices.remove(i);
            let path = d.info.path.clone();
            self.release_keys(d, out);
            out.push(InputMsg::Gone { path, error });
        }
        self.devices_changed();
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

/// Bring `pressed` in line with `held` (the kernel's key state): the key
/// events that were missed, releases first. A `stale` code (forgotten by
/// "release all") is not pressed again while the kernel still holds it;
/// once the kernel lets it go, the mark is cleared.
fn resync(pressed: &mut Vec<u16>, stale: &mut Vec<u16>, held: &[u16]) -> Vec<Event> {
    let mut missed = Vec::new();
    pressed.retain(|&code| {
        let keep = held.contains(&code);
        if !keep {
            missed.push(Event::Key { code, down: false });
        }
        keep
    });
    stale.retain(|code| held.contains(code));
    for &code in held {
        if !pressed.contains(&code) && !stale.contains(&code) {
            pressed.push(code);
            missed.push(Event::Key { code, down: true });
        }
    }
    missed
}

/// Turn one batch of raw events from a device into engine events (with the
/// kernel's timestamp; None for corrected ones) and keep `pressed` up to
/// date. Returns true if events had been lost and were corrected.
///
/// When a reader falls behind, the kernel drops its queued events and
/// sends SYN_DROPPED. Everything up to the next SYN_REPORT is then an
/// incomplete frame and is skipped; the kernel's key state (`held_keys`,
/// EVIOCGKEY) then tells which presses and releases were lost. This is done
/// in the same call: a release lost this way must not wait for the next
/// event of the device.
///
/// But only after the whole batch: EVIOCGKEY also deletes the key events
/// still queued for this reader (they are part of the state it returns).
/// Asked in the middle of the batch, it would delete a release while an
/// older press from the rest of the batch is still to come, and that press
/// would then stay held.
fn process_events(
    events: impl IntoIterator<Item = InputEvent>,
    pressed: &mut Vec<u16>,
    stale: &mut Vec<u16>,
    dropping: &mut bool,
    held_keys: impl FnOnce() -> Option<Vec<u16>>,
    out: &mut Vec<(Event, Option<SystemTime>)>,
) -> bool {
    let mut lost = false;
    for ev in events {
        if ev.event_type() == EventType::SYNCHRONIZATION {
            if ev.code() == SynchronizationCode::SYN_DROPPED.0 {
                *dropping = true;
                continue;
            }
            if *dropping && ev.code() == SynchronizationCode::SYN_REPORT.0 {
                *dropping = false;
                lost = true;
                continue;
            }
        }
        if *dropping {
            continue;
        }
        let Some(event) = map_event(&ev) else {
            continue;
        };
        if let Event::Key { code, down } = event {
            // A real edge of a forgotten key ends its stale mark: the kernel
            // sends one only when the key really changed.
            stale.retain(|c| *c != code);
            if !down {
                pressed.retain(|c| *c != code);
            } else if !pressed.contains(&code) {
                pressed.push(code);
            }
        }
        out.push((event, Some(ev.timestamp())));
    }
    // An incomplete frame at the end of the batch is finished in the next
    // one; asking now would delete key events of that frame.
    if lost && !*dropping {
        if let Some(held) = held_keys() {
            out.extend(resync(pressed, stale, &held).into_iter().map(|e| (e, None)));
        }
        out.push((Event::Sync, None));
    }
    lost
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: u16 = BTN_LEFT as u16;
    const KEY_W: u16 = 17;

    fn key(code: u16, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY.0, code, value)
    }
    fn rel(code: u16, value: i32) -> InputEvent {
        InputEvent::new(EventType::RELATIVE.0, code, value)
    }
    fn syn(code: SynchronizationCode) -> InputEvent {
        InputEvent::new(EventType::SYNCHRONIZATION.0, code.0, 0)
    }
    fn report() -> InputEvent {
        syn(SynchronizationCode::SYN_REPORT)
    }

    /// Engine events of one batch; `held`: what the kernel reports when asked.
    fn run(
        events: Vec<InputEvent>,
        pressed: &mut Vec<u16>,
        dropping: &mut bool,
        held: &[u16],
    ) -> (Vec<Event>, bool, usize) {
        let mut stale = Vec::new();
        run_stale(events, pressed, &mut stale, dropping, held)
    }

    fn run_stale(
        events: Vec<InputEvent>,
        pressed: &mut Vec<u16>,
        stale: &mut Vec<u16>,
        dropping: &mut bool,
        held: &[u16],
    ) -> (Vec<Event>, bool, usize) {
        let mut asked = 0;
        let mut out = Vec::new();
        let resynced = process_events(
            events,
            pressed,
            stale,
            dropping,
            || {
                asked += 1;
                Some(held.to_vec())
            },
            &mut out,
        );
        (out.into_iter().map(|(e, _)| e).collect(), resynced, asked)
    }

    #[test]
    fn events_pass_through_without_drops() {
        let (mut pressed, mut dropping) = (Vec::new(), false);
        let (ev, resynced, asked) = run(
            vec![
                key(LEFT, 1),
                rel(0, 5),
                report(),
                key(LEFT, 2),
                key(LEFT, 0),
                report(),
            ],
            &mut pressed,
            &mut dropping,
            &[],
        );
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: true
                },
                Event::Rel { axis: 0, value: 5 },
                Event::Sync,
                // value 2 (auto-repeat) is dropped
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync,
            ]
        );
        assert!(!resynced);
        assert_eq!(asked, 0);
        assert!(pressed.is_empty());
    }

    #[test]
    fn release_lost_by_the_kernel_is_delivered_at_once() {
        // Left button held, then the reader falls behind: the kernel drops
        // the queue (with the release) and sends SYN_DROPPED plus the newest
        // event of an incomplete frame.
        let (mut pressed, mut dropping) = (Vec::new(), false);
        run(vec![key(LEFT, 1), report()], &mut pressed, &mut dropping, &[]);
        assert_eq!(pressed, vec![LEFT]);
        let (ev, resynced, asked) = run(
            vec![
                syn(SynchronizationCode::SYN_DROPPED),
                rel(0, 3),
                report(),
                rel(1, 4),
                report(),
            ],
            &mut pressed,
            &mut dropping,
            &[],
        );
        assert_eq!(
            ev,
            vec![
                // the incomplete frame (rel 0) is skipped, the next one is not
                Event::Rel { axis: 1, value: 4 },
                Event::Sync,
                // the state, after the whole batch
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync,
            ]
        );
        assert!(resynced);
        assert_eq!(asked, 1);
        assert!(pressed.is_empty());
        assert!(!dropping);
    }

    #[test]
    fn state_is_asked_for_after_the_whole_batch() {
        // The read holds the dropped block and a press; the release is still
        // in the kernel's queue. Asking for the state deletes that release,
        // so the state must come after the press, not before it.
        let (mut pressed, mut dropping) = (Vec::new(), false);
        let (ev, resynced, asked) = run(
            vec![
                syn(SynchronizationCode::SYN_DROPPED),
                report(),
                key(LEFT, 1),
                report(),
                rel(0, 2),
                report(),
            ],
            &mut pressed,
            &mut dropping,
            &[],
        );
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: true
                },
                Event::Sync,
                Event::Rel { axis: 0, value: 2 },
                Event::Sync,
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync,
            ]
        );
        assert!(resynced && asked == 1);
        assert!(pressed.is_empty(), "left button stays held");
    }

    #[test]
    fn several_drops_in_one_batch_ask_once() {
        let (mut pressed, mut dropping) = (vec![LEFT], false);
        let (ev, resynced, asked) = run(
            vec![
                syn(SynchronizationCode::SYN_DROPPED),
                report(),
                key(LEFT, 0),
                report(),
                syn(SynchronizationCode::SYN_DROPPED),
                report(),
            ],
            &mut pressed,
            &mut dropping,
            &[LEFT],
        );
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync,
                Event::Key {
                    code: LEFT,
                    down: true
                },
                Event::Sync,
            ]
        );
        assert!(resynced && asked == 1);
        assert_eq!(pressed, vec![LEFT]);
    }

    #[test]
    fn press_lost_by_the_kernel_is_added() {
        let (mut pressed, mut dropping) = (vec![KEY_W], false);
        let (ev, _, _) = run(
            vec![syn(SynchronizationCode::SYN_DROPPED), report()],
            &mut pressed,
            &mut dropping,
            &[LEFT, KEY_W],
        );
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: true
                },
                Event::Sync
            ]
        );
        assert_eq!(pressed, vec![KEY_W, LEFT]);
    }

    #[test]
    fn drop_across_two_batches() {
        // SYN_DROPPED at the end of one read, the rest of the frame in the next.
        let (mut pressed, mut dropping) = (vec![LEFT], false);
        let (ev, resynced, asked) = run(
            vec![syn(SynchronizationCode::SYN_DROPPED), rel(0, 1)],
            &mut pressed,
            &mut dropping,
            &[],
        );
        assert!(ev.is_empty() && !resynced && asked == 0 && dropping);
        let (ev, resynced, asked) = run(vec![key(LEFT, 0), report()], &mut pressed, &mut dropping, &[]);
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync
            ]
        );
        assert!(resynced && asked == 1 && !dropping);
    }

    #[test]
    fn resync_brings_pressed_in_line() {
        let mut pressed = vec![LEFT, KEY_W];
        let mut stale = Vec::new();
        assert_eq!(resync(&mut pressed, &mut stale, &[LEFT, KEY_W]), vec![]);
        assert_eq!(
            resync(&mut pressed, &mut stale, &[KEY_W, 30]),
            vec![
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Key { code: 30, down: true }
            ]
        );
        assert_eq!(pressed, vec![KEY_W, 30]);
        // A press the state query deleted from the queue comes back.
        assert_eq!(
            resync(&mut pressed, &mut stale, &[KEY_W, 30, LEFT]),
            vec![Event::Key {
                code: LEFT,
                down: true
            }]
        );
        assert_eq!(pressed, vec![KEY_W, 30, LEFT]);
    }

    #[test]
    fn forgotten_button_is_not_pressed_again_while_the_kernel_holds_it() {
        // The receiver lost the release: the kernel keeps LEFT held. After
        // "release all" forgot it, neither the safety net nor a resync after
        // dropped events may press it again; the user's next click (the
        // kernel passes only its release) ends the mark.
        let mut pressed = vec![LEFT];
        let mut stale = Vec::new();
        // forget(): what InputSet::forget does per device
        pressed.retain(|c| *c != LEFT);
        stale.push(LEFT);
        assert_eq!(resync(&mut pressed, &mut stale, &[LEFT]), vec![]);
        assert!(pressed.is_empty() && stale == vec![LEFT]);
        let (ev, _, _) = run_stale(
            vec![syn(SynchronizationCode::SYN_DROPPED), report()],
            &mut pressed,
            &mut stale,
            &mut false,
            &[LEFT],
        );
        assert_eq!(ev, vec![Event::Sync]);
        assert!(pressed.is_empty() && stale == vec![LEFT]);
        // The release edge: passed through (the engine ignores it) and the mark is gone.
        let (ev, _, _) = run_stale(
            vec![key(LEFT, 0), report()],
            &mut pressed,
            &mut stale,
            &mut false,
            &[],
        );
        assert_eq!(
            ev,
            vec![
                Event::Key {
                    code: LEFT,
                    down: false
                },
                Event::Sync
            ]
        );
        assert!(stale.is_empty());
        // From now on presses work again.
        let (ev, _, _) = run_stale(
            vec![key(LEFT, 1), report()],
            &mut pressed,
            &mut stale,
            &mut false,
            &[LEFT],
        );
        assert_eq!(
            ev[0],
            Event::Key {
                code: LEFT,
                down: true
            }
        );
        assert_eq!(pressed, vec![LEFT]);
        // The kernel letting go without an event we saw (query flushed it) also ends the mark.
        let mut stale = vec![LEFT];
        assert_eq!(resync(&mut Vec::new(), &mut stale, &[]), vec![]);
        assert!(stale.is_empty());
    }

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
    fn grab_problem_text() {
        let failed = |name: &str, busy| InputMsg::GrabFailed {
            path: PathBuf::from("/dev/input/event26"),
            name: name.into(),
            busy,
            error: "Device or resource busy (os error 16)".into(),
        };
        assert_eq!(grab_problems(&[]), None);
        assert_eq!(
            grab_problems(&[failed("Keyboard", true), failed("Mouse", true)]).unwrap(),
            "\u{201c}Keyboard\u{201d} is held exclusively by another program; \
             \u{201c}Mouse\u{201d} is held exclusively by another program. The bridge stays off."
        );
        let released = InputMsg::ReleaseFailed {
            path: PathBuf::from("/dev/input/event4"),
            name: "Mouse".into(),
            error: "x".into(),
            reopened: true,
        };
        assert_eq!(
            grab_problems(&[released]).unwrap(),
            "\u{201c}Mouse\u{201d} could not be released (x), so it was opened again."
        );
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
