//! A button release must not get lost when the kernel drops events.
//!
//! A virtual gaming mouse (uinput) holds the left button and then sends far
//! more motion than the kernel queues for a reader that falls behind, with
//! the release somewhere in between. The kernel drops the queue and sends
//! SYN_DROPPED; afterwards the release must reach the engine at once, not
//! only with the next event of the mouse (which made the button stick on
//! the PlayStation until the next click).
//!
//! Needs write access to /dev/uinput and read access to the new event node
//! (udev rule from "Set up access"; ids as in grab_release.rs, override with
//! PSHB_GRAB_TEST_IDS=vvvv:pppp). Close PS HID Bridge first.
//!
//! cargo test -p pshb-core --test dropped_events -- --ignored --nocapture

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, InputEvent, InputId, KeyCode, RelativeAxisCode};

use pshb_core::engine::Event;
use pshb_core::input::{DeviceInfo, GrabGuard, InputMsg, InputSet, Waker};

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;

fn mouse_id() -> (u16, u16) {
    let spec = std::env::var("PSHB_GRAB_TEST_IDS").unwrap_or_else(|_| "0db0:1fc6".into());
    let first = spec.split(',').next_back().expect("ids");
    let (v, p) = first.split_once(':').expect("vvvv:pppp");
    (
        u16::from_str_radix(v, 16).expect("hex"),
        u16::from_str_radix(p, 16).expect("hex"),
    )
}

fn virtual_mouse() -> (VirtualDevice, PathBuf) {
    let id = mouse_id();
    let keys: AttributeSet<KeyCode> = [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT].into_iter().collect();
    let axes: AttributeSet<RelativeAxisCode> = [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]
        .into_iter()
        .collect();
    let mut dev = VirtualDevice::builder()
        .expect("open /dev/uinput")
        .name("PSHB dropped events test mouse")
        .input_id(InputId::new(BusType::BUS_USB, id.0, id.1, 1))
        .with_keys(&keys)
        .unwrap()
        .with_relative_axes(&axes)
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

/// Read for `time`; the left-button events that arrived.
fn read(inputs: &mut InputSet, time: Duration) -> Vec<bool> {
    let end = Instant::now() + time;
    let mut msgs = Vec::new();
    let mut left = Vec::new();
    while Instant::now() < end {
        inputs.wait(end.saturating_duration_since(Instant::now()), &mut msgs);
        for m in msgs.drain(..) {
            if let InputMsg::Event {
                event: Event::Key { code: BTN_LEFT, down },
                ..
            } = m
            {
                left.push(down);
            }
        }
    }
    left
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule, see the file comment"]
fn release_lost_in_a_queue_overflow_arrives_at_once() {
    let (mut mouse, path) = virtual_mouse();
    let event = path.file_name().unwrap().to_str().unwrap().to_string();
    let info = DeviceInfo::from_sysfs(&PathBuf::from("/sys/class/input"), &event).expect("sysfs");
    let waker = Arc::new(Waker::new().unwrap());
    let mut inputs = InputSet::open(&[info], waker, GrabGuard::default()).expect("open");

    // Press and see it arrive.
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 1)]).unwrap();
    assert_eq!(read(&mut inputs, Duration::from_millis(50)), vec![true]);

    // Without reading: lots of motion, the release, lots of motion (the
    // kernel's queue for us holds only a few hundred events).
    let motion = [InputEvent::new(EV_REL, 0, 1), InputEvent::new(EV_REL, 1, -1)];
    for _ in 0..3000 {
        mouse.emit(&motion).unwrap();
    }
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 0)]).unwrap();
    for _ in 0..3000 {
        mouse.emit(&motion).unwrap();
    }

    // Read now (the mouse stays quiet): the release must come right away,
    // well before the 250 ms safety net, and the drop must be noticed.
    let left = read(&mut inputs, Duration::from_millis(100));
    println!("left button events: {left:?}, resyncs: {}", inputs.resyncs());
    assert_eq!(left.last(), Some(&false), "release missing");
    assert!(
        inputs.resyncs() >= 1,
        "no SYN_DROPPED seen: the flood was too small"
    );
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule, see the file comment"]
fn release_dropped_as_the_very_last_event_is_not_lost() {
    // The kernel's queue per reader holds a power of two of events (64 or
    // more, depending on the device). Filled so that it overflows exactly at
    // the release, only SYN_DROPPED and SYN_REPORT are left: the release
    // itself is gone and nothing follows that would wake a reader again.
    // This is a button let go while the app lagged for a moment.
    let (mut mouse, path) = virtual_mouse();
    let event = path.file_name().unwrap().to_str().unwrap().to_string();
    let info = DeviceInfo::from_sysfs(&PathBuf::from("/sys/class/input"), &event).expect("sysfs");
    let waker = Arc::new(Waker::new().unwrap());
    let mut inputs = InputSet::open(&[info], waker, GrabGuard::default()).expect("open");

    let mut exact = 0;
    for size in [64usize, 128, 256, 512] {
        mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 1)]).unwrap();
        assert_eq!(read(&mut inputs, Duration::from_millis(30)), vec![true]);
        let before = inputs.resyncs();
        // Frames of two events each; event number `size` is the release's SYN_REPORT.
        for _ in 0..(size - 2) / 2 {
            mouse.emit(&[InputEvent::new(EV_REL, 0, 1)]).unwrap();
        }
        mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 0)]).unwrap();
        // Well before the 250 ms safety net.
        let left = read(&mut inputs, Duration::from_millis(40));
        println!(
            "queue {size}: left button events {left:?}, resyncs {}",
            inputs.resyncs() - before
        );
        // A release may come twice (corrected, then the original one that was
        // still queued); the engine ignores the second.
        assert!(
            !left.is_empty() && left.iter().all(|down| !down),
            "queue {size}: release missing"
        );
        exact += usize::from(inputs.resyncs() > before);
    }
    assert!(exact >= 1, "no size overflowed the queue");
}

/// Open the virtual mouse in an InputSet.
fn open(path: &std::path::Path) -> InputSet {
    let event = path.file_name().unwrap().to_str().unwrap().to_string();
    let info = DeviceInfo::from_sysfs(&PathBuf::from("/sys/class/input"), &event).expect("sysfs");
    let waker = Arc::new(Waker::new().unwrap());
    InputSet::open(&[info], waker, GrabGuard::default()).expect("open")
}

/// Read for `time`; the button events (code, down) that arrived.
fn read_buttons(inputs: &mut InputSet, time: Duration) -> Vec<(u16, bool)> {
    let end = Instant::now() + time;
    let mut msgs = Vec::new();
    let mut got = Vec::new();
    while Instant::now() < end {
        inputs.wait(end.saturating_duration_since(Instant::now()), &mut msgs);
        for m in msgs.drain(..) {
            if let InputMsg::Event {
                event: Event::Key { code, down },
                ..
            } = m
            {
                got.push((code, down));
            }
        }
    }
    got
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule, see the file comment"]
fn press_in_the_batch_with_its_release_still_queued() {
    // Reading the key state (EVIOCGKEY) also removes the key events still
    // queued for this reader. After an overflow, the first read holds the
    // dropped block and a press, the release is still in the kernel's queue:
    // asking for the state in the middle of that batch deletes the release,
    // and the older press processed afterwards stays held.
    let (mut mouse, path) = virtual_mouse();
    let mut inputs = open(&path);
    read_buttons(&mut inputs, Duration::from_millis(20));

    // Overflow the 128-event queue exactly at a frame end: SYN_DROPPED + SYN_REPORT.
    for _ in 0..64 {
        mouse.emit(&[InputEvent::new(EV_REL, 0, 1)]).unwrap();
    }
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 1)]).unwrap();
    for _ in 0..20 {
        mouse.emit(&[InputEvent::new(EV_REL, 0, 1)]).unwrap();
    }
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 0)]).unwrap();

    // Well before the 250 ms safety net.
    let got = read_buttons(&mut inputs, Duration::from_millis(60));
    println!("button events: {got:?}, resyncs {}", inputs.resyncs());
    assert_eq!(got.last(), Some(&(BTN_LEFT, false)), "left button stays held");
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule, see the file comment"]
fn click_while_another_button_is_held_is_not_lost() {
    // Aiming (right held) and firing (left): the periodic comparison with
    // the kernel's key state must not swallow a press that is still queued.
    let (mut mouse, path) = virtual_mouse();
    let mut inputs = open(&path);
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_RIGHT, 1)]).unwrap();
    assert_eq!(
        read_buttons(&mut inputs, Duration::from_millis(30)),
        vec![(BTN_RIGHT, true)]
    );

    // Let the periodic check become due, then press left without reading.
    thread::sleep(Duration::from_millis(300));
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 1)]).unwrap();
    let pressed = read_buttons(&mut inputs, Duration::from_millis(30));
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_LEFT, 0)]).unwrap();
    mouse.emit(&[InputEvent::new(EV_KEY, BTN_RIGHT, 0)]).unwrap();
    let released = read_buttons(&mut inputs, Duration::from_millis(30));
    println!("pressed: {pressed:?}, released: {released:?}");
    assert_eq!(pressed, vec![(BTN_LEFT, true)], "left click lost");
    assert_eq!(released, vec![(BTN_LEFT, false), (BTN_RIGHT, false)]);
}
