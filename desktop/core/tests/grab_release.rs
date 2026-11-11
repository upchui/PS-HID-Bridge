//! Devices must be free again as soon as the bridge is off.
//!
//! Runs the real bridge service against virtual input devices (uinput) that
//! look like real ones with several event nodes: a keyboard converter with a
//! boot keyboard, an NKRO keyboard and a mouse node, and a gaming mouse with
//! a keyboard node. After every step a second descriptor tries to grab each
//! node: EBUSY means somebody still holds it.
//!
//! Needs write access to /dev/uinput and read access to the new event nodes;
//! the vendor/product IDs are those of a udev rule written by "Set up access"
//! (override with PSHB_GRAB_TEST_IDS=vvvv:pppp,vvvv:pppp). Close PS HID
//! Bridge first: it would pick up the virtual devices too. Key presses only
//! happen while the devices are grabbed; the releases after switching off
//! reach the desktop, which ignores releases of keys it never saw pressed.
//!
//! cargo test -p pshb-core --test grab_release -- --ignored --nocapture --test-threads=1

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, InputEvent, InputId, KeyCode, RelativeAxisCode};

use pshb_core::config::Config;
use pshb_core::engine::Notice;
use pshb_core::service::{Service, ServiceCmd, ServiceConfig, ServiceEvent};
use pshb_core::trigger::Action;

const NAME: &str = "PSHB grab test";
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTALT: u16 = 56;
const KEY_P: u16 = 25;
const BTN_LEFT: u16 = 0x110;

fn ids() -> Vec<(u16, u16)> {
    let spec = std::env::var("PSHB_GRAB_TEST_IDS").unwrap_or_else(|_| "feed:6536,0db0:1fc6".into());
    spec.split(',')
        .map(|p| {
            let (v, d) = p.split_once(':').expect("vvvv:pppp");
            (
                u16::from_str_radix(v, 16).expect("hex"),
                u16::from_str_radix(d, 16).expect("hex"),
            )
        })
        .collect()
}

struct Node {
    dev: VirtualDevice,
    path: PathBuf,
    label: &'static str,
}

impl Node {
    fn new(label: &'static str, id: (u16, u16), keys: &[u16], rel: bool) -> Node {
        let keyset: AttributeSet<KeyCode> = keys.iter().map(|k| KeyCode::new(*k)).collect();
        let name = format!("{NAME} {label}");
        let mut b = VirtualDevice::builder()
            .expect("open /dev/uinput")
            .name(&name)
            .input_id(InputId::new(BusType::BUS_USB, id.0, id.1, 1))
            .with_keys(&keyset)
            .unwrap();
        if rel {
            let axes: AttributeSet<RelativeAxisCode> = [
                RelativeAxisCode::REL_X,
                RelativeAxisCode::REL_Y,
                RelativeAxisCode::REL_WHEEL,
                RelativeAxisCode::REL_HWHEEL,
            ]
            .into_iter()
            .collect();
            b = b.with_relative_axes(&axes).unwrap();
        }
        let mut dev = b.build().expect("create virtual device");
        let end = Instant::now() + Duration::from_secs(5);
        let path = loop {
            let found = dev
                .enumerate_dev_nodes_blocking()
                .ok()
                .and_then(|mut it| it.find_map(|p| p.ok()));
            if let Some(p) = found
                && readable(&p)
            {
                break p;
            }
            assert!(
                Instant::now() < end,
                "{label}: no readable event node (udev rule for {:04x}:{:04x}?)",
                id.0,
                id.1
            );
            thread::sleep(Duration::from_millis(20));
        };
        Node { dev, path, label }
    }

    fn key(&mut self, code: u16, down: bool) {
        self.dev
            .emit(&[InputEvent::new(EV_KEY, code, down as i32)])
            .unwrap();
        thread::sleep(Duration::from_millis(15));
    }

    fn motion(&mut self) {
        self.dev
            .emit(&[InputEvent::new(EV_REL, 0, 3), InputEvent::new(EV_REL, 1, -2)])
            .unwrap();
        thread::sleep(Duration::from_millis(5));
    }
}

fn readable(p: &Path) -> bool {
    File::open(p).is_ok()
}

/// Does anybody hold an exclusive grab on this node? (Grabs it for a moment
/// if not; closing the probe would release it anyway.)
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

struct Rig {
    boot: Node,
    nkro: Node,
    mouse: Node,
    gmouse: Node,
    gkbd: Node,
}

impl Rig {
    fn new() -> Rig {
        let ids = ids();
        let (conv, gaming) = (ids[0], *ids.get(1).unwrap_or(&ids[0]));
        let keyboard: Vec<u16> = (1..=127).collect();
        let buttons = [BTN_LEFT, BTN_LEFT + 1, BTN_LEFT + 2, BTN_LEFT + 3, BTN_LEFT + 4];
        Rig {
            boot: Node::new("Boot", conv, &keyboard, false),
            nkro: Node::new("Keyboard", conv, &keyboard, false),
            mouse: Node::new("Mouse", conv, &buttons, true),
            gmouse: Node::new("Gaming Mouse", gaming, &buttons, true),
            gkbd: Node::new("Gaming Mouse Keys", gaming, &keyboard, false),
        }
    }

    fn nodes(&self) -> [&Node; 5] {
        [&self.boot, &self.nkro, &self.mouse, &self.gmouse, &self.gkbd]
    }

    /// The nodes that are (true) or are not grabbed, as labels.
    fn grabbed_labels(&self, want: bool) -> Vec<&'static str> {
        self.nodes()
            .iter()
            .filter(|n| grabbed(&n.path) == want)
            .map(|n| n.label)
            .collect()
    }

    fn assert_all_grabbed(&self, step: &str) {
        let free = self.grabbed_labels(false);
        assert!(free.is_empty(), "{step}: not grabbed: {free:?}");
    }

    fn assert_all_free(&self, step: &str) {
        // The service ungrabs right after the event that switched it off.
        let end = Instant::now() + Duration::from_millis(300);
        loop {
            let held = self.grabbed_labels(true);
            if held.is_empty() {
                return;
            }
            assert!(Instant::now() < end, "{step}: still grabbed: {held:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ctrl and Alt on the boot keyboard, P on the NKRO node (as a converter
    /// may report them), then released in reverse order.
    fn press_toggle_split(&mut self) {
        self.boot.key(KEY_LEFTCTRL, true);
        self.boot.key(KEY_LEFTALT, true);
        self.nkro.key(KEY_P, true);
    }

    fn release_toggle_split(&mut self) {
        self.nkro.key(KEY_P, false);
        self.boot.key(KEY_LEFTALT, false);
        self.boot.key(KEY_LEFTCTRL, false);
    }

    fn press_toggle(&mut self, node: fn(&mut Rig) -> &mut Node) {
        let n = node(self);
        n.key(KEY_LEFTCTRL, true);
        n.key(KEY_LEFTALT, true);
        n.key(KEY_P, true);
    }

    fn release_toggle(&mut self, node: fn(&mut Rig) -> &mut Node) {
        let n = node(self);
        n.key(KEY_P, false);
        n.key(KEY_LEFTALT, false);
        n.key(KEY_LEFTCTRL, false);
    }

    /// Some input while the bridge is on (it all goes to the service).
    fn use_while_on(&mut self) {
        for _ in 0..20 {
            self.mouse.motion();
            self.gmouse.motion();
        }
        self.gmouse.key(BTN_LEFT, true);
        self.gmouse.key(BTN_LEFT, false);
        self.nkro.key(30, true); // A
        self.nkro.key(30, false);
    }
}

struct Svc {
    svc: Option<Service>,
    rx: Receiver<ServiceEvent>,
}

impl Svc {
    fn start() -> Svc {
        let config = Config {
            devices: vec![NAME.into()],
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let svc = Service::start(
            ServiceConfig {
                config,
                key: [7; 32],
                // Nobody listens: the bridge works the same without a board.
                board: "127.0.0.1:9".parse().unwrap(),
                counter_store: None,
            },
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
        )
        .expect("service");
        let s = Svc { svc: Some(svc), rx };
        s.wait_for(
            "devices open",
            |e| matches!(e, ServiceEvent::Status(st) if st.devices_open == 5),
        );
        s
    }

    fn send(&self, cmd: ServiceCmd) {
        self.svc.as_ref().unwrap().send(cmd);
    }

    fn wait_for(&self, what: &str, want: impl Fn(&ServiceEvent) -> bool) {
        let end = Instant::now() + Duration::from_secs(3);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(e) if want(&e) => return,
                Ok(ServiceEvent::Error(e)) => eprintln!("service error: {e}"),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        panic!("timed out waiting for: {what}");
    }

    fn wait_notice(&self, n: Notice) {
        self.wait_for(&format!("{n:?}"), |e| *e == ServiceEvent::Notice(n));
    }

    fn on(&self) {
        self.send(ServiceCmd::Action(Action::On));
        self.wait_notice(Notice::Activated);
    }
}

fn boot(r: &mut Rig) -> &mut Node {
    &mut r.boot
}
fn nkro(r: &mut Rig) -> &mut Node {
    &mut r.nkro
}
fn gkbd(r: &mut Rig) -> &mut Node {
    &mut r.gkbd
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule for the IDs; see the file header"]
fn devices_are_free_after_switching_off() {
    let mut rig = Rig::new();
    let svc = Svc::start();
    rig.assert_all_free("service started");

    for round in 0..5 {
        // Off with Ctrl+Alt+P split over two nodes.
        svc.on();
        rig.assert_all_grabbed("on");
        rig.use_while_on();
        rig.press_toggle_split();
        svc.wait_notice(Notice::Deactivated);
        rig.assert_all_free(&format!("round {round}: off (keys still down)"));
        rig.release_toggle_split();
        rig.assert_all_free(&format!("round {round}: off (keys released)"));

        // Off with Ctrl+Alt+P on one node each.
        for (name, node) in [
            ("boot", boot as fn(&mut Rig) -> &mut Node),
            ("nkro", nkro),
            ("gaming", gkbd),
        ] {
            svc.on();
            rig.assert_all_grabbed("on");
            rig.press_toggle(node);
            svc.wait_notice(Notice::Deactivated);
            rig.release_toggle(node);
            rig.assert_all_free(&format!("round {round}: off on the {name} node"));
        }

        // Off from the GUI.
        svc.on();
        svc.send(ServiceCmd::Action(Action::Off));
        svc.wait_notice(Notice::Deactivated);
        rig.assert_all_free(&format!("round {round}: off from the GUI"));
    }
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule for the IDs; see the file header"]
fn devices_are_free_after_hotplug_and_restart() {
    let mut rig = Rig::new();
    let mut svc = Svc::start();

    // A node goes away and comes back while the bridge is on.
    svc.on();
    // The new node appears, then the old one goes (the service looks at the
    // device list 200 ms after hot-plug events).
    rig.mouse = Node::new("Mouse 2", ids()[0], &[BTN_LEFT], true);
    thread::sleep(Duration::from_millis(800));
    rig.assert_all_grabbed("on after hot-plug");
    rig.press_toggle_split();
    svc.wait_notice(Notice::Deactivated);
    rig.release_toggle_split();
    rig.assert_all_free("off after hot-plug");

    // The GUI restarts the service while the bridge is on.
    svc.on();
    svc.svc.take();
    rig.assert_all_free("service stopped while on");
    svc = Svc::start();
    rig.assert_all_free("service restarted");
    svc.on();
    rig.assert_all_grabbed("on after restart");
    rig.press_toggle_split();
    svc.wait_notice(Notice::Deactivated);
    rig.release_toggle_split();
    rig.assert_all_free("off after restart");
}

#[test]
#[ignore = "needs /dev/uinput and a udev rule for the IDs; see the file header"]
fn device_held_by_another_program_keeps_the_bridge_off() {
    let rig = Rig::new();
    let svc = Svc::start();
    // Another program holds the NKRO keyboard.
    let other = File::open(&rig.nkro.path).unwrap();
    // SAFETY: valid descriptor; EVIOCGRAB takes its argument by value.
    assert_eq!(
        unsafe { libc::ioctl(other.as_raw_fd(), EVIOCGRAB as _, 1 as libc::c_ulong) },
        0
    );
    svc.send(ServiceCmd::Action(Action::On));
    svc.wait_for("error about the busy device", |e| {
        matches!(e, ServiceEvent::Error(t)
            if t.contains("PSHB grab test Keyboard") && t.contains("held exclusively by another program"))
    });
    svc.wait_notice(Notice::Deactivated);
    drop(other);
    // Everything else was given back.
    rig.assert_all_free("bridge off after the failed grab");
}
