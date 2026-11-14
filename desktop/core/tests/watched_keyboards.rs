//! A keyboard that is not selected is only watched: its keys arrive as
//! trigger keys (never forwarded), and switching the bridge on does not grab
//! it, so it keeps working on the desktop.
//!
//! A virtual keyboard (uinput) types F23, a key no desktop uses. Needs write
//! access to /dev/uinput and read access to the new event node (udev rule
//! from "Set up access"; id feed:6536 as in dropped_events.rs, override with
//! PSHB_GRAB_TEST_IDS=vvvv:pppp). Close PS HID Bridge first.
//!
//! cargo test -p pshb-core --test watched_keyboards -- --ignored --nocapture

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, InputEvent, InputId, KeyCode};

use pshb_core::engine::Event;
use pshb_core::input::{DeviceInfo, GrabGuard, InputMsg, InputSet, Waker};

const EV_KEY: u16 = 1;
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
const F23: u16 = 193;

fn keyboard_id() -> (u16, u16) {
    let spec = std::env::var("PSHB_GRAB_TEST_IDS").unwrap_or_else(|_| "feed:6536".into());
    let first = spec.split(',').next_back().expect("ids");
    let (v, p) = first.split_once(':').expect("vvvv:pppp");
    (
        u16::from_str_radix(v, 16).expect("hex"),
        u16::from_str_radix(p, 16).expect("hex"),
    )
}

fn virtual_keyboard() -> (VirtualDevice, PathBuf) {
    let id = keyboard_id();
    // A, Z and Enter make it a keyboard (DeviceInfo::keyboard).
    let keys: AttributeSet<KeyCode> = [
        KeyCode::KEY_A,
        KeyCode::KEY_Z,
        KeyCode::KEY_ENTER,
        KeyCode::KEY_F23,
    ]
    .into_iter()
    .collect();
    let mut dev = VirtualDevice::builder()
        .expect("open /dev/uinput")
        .name("PSHB watched keyboard test")
        .input_id(InputId::new(BusType::BUS_USB, id.0, id.1, 1))
        .with_keys(&keys)
        .unwrap()
        .build()
        .expect("create virtual device");
    let end = Instant::now() + Duration::from_secs(5);
    let path = loop {
        let found = dev
            .enumerate_dev_nodes_blocking()
            .ok()
            .and_then(|mut it| it.find_map(|p| p.ok()));
        if let Some(p) = found
            && File::open(&p).is_ok()
        {
            break p;
        }
        assert!(
            Instant::now() < end,
            "no readable event node (udev rule for {:04x}:{:04x}?)",
            id.0,
            id.1
        );
        thread::sleep(Duration::from_millis(20));
    };
    (dev, path)
}

/// Does anybody hold an exclusive grab on this node? (Grabs it for a moment
/// if not.)
fn grabbed(path: &Path) -> bool {
    let f = File::open(path).expect("open probe");
    // SAFETY: valid descriptor; EVIOCGRAB takes its argument by value.
    if unsafe { libc::ioctl(f.as_raw_fd(), EVIOCGRAB as _, 1 as libc::c_ulong) } == 0 {
        // SAFETY: as above.
        unsafe { libc::ioctl(f.as_raw_fd(), EVIOCGRAB as _, 0 as libc::c_ulong) };
        return false;
    }
    let e = io::Error::last_os_error();
    assert_eq!(
        e.raw_os_error(),
        Some(libc::EBUSY),
        "probe {}: {e}",
        path.display()
    );
    true
}

#[test]
#[ignore = "needs /dev/uinput and read access to the virtual keyboard; close PS HID Bridge first"]
fn unselected_keyboard_is_watched_not_grabbed() {
    let (mut kbd, path) = virtual_keyboard();
    let event = path.file_name().unwrap().to_str().unwrap().to_string();
    let info = DeviceInfo::from_sysfs(&PathBuf::from("/sys/class/input"), &event).expect("sysfs");
    assert!(info.keyboard, "the virtual device must count as a keyboard");

    let waker = Arc::new(Waker::new().unwrap());
    let mut inputs = InputSet::new(waker, GrabGuard::default());
    let mut msgs = Vec::new();
    inputs.update(&[], std::slice::from_ref(&info), &mut msgs);
    assert_eq!((inputs.len(), inputs.watched()), (0, 1));

    // Bridge on: the watched keyboard stays with the desktop.
    assert!(inputs.set_grab(true).is_empty());
    assert!(!grabbed(&path), "a watched keyboard must not be grabbed");

    for value in [1, 0] {
        kbd.emit(&[InputEvent::new(EV_KEY, F23, value)]).unwrap();
    }
    let end = Instant::now() + Duration::from_secs(2);
    let mut got = Vec::new();
    while Instant::now() < end && got.len() < 2 {
        inputs.wait(Duration::from_millis(100), &mut msgs);
        for m in msgs.drain(..) {
            if let InputMsg::Event { event, .. } = m {
                got.push(event);
            }
        }
    }
    assert_eq!(
        got,
        vec![
            Event::TriggerKey {
                code: F23,
                down: true
            },
            Event::TriggerKey {
                code: F23,
                down: false
            },
        ],
        "only trigger keys (no forwarded keys, no sync)"
    );
    inputs.set_grab(false);

    // Selected after all: from now on it is forwarded (and grabbed with the bridge on).
    inputs.update(std::slice::from_ref(&info), &[], &mut msgs);
    assert_eq!((inputs.len(), inputs.watched()), (1, 0));
    assert!(inputs.set_grab(true).is_empty());
    assert!(grabbed(&path), "a selected keyboard is grabbed");
    inputs.set_grab(false);
}
