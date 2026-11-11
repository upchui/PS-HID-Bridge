//! Input engine: turns raw input events into what the board should see,
//! and decides when the bridge is active.
//!
//! Pure state machine (no I/O, clock passed in) so every rule is unit-tested:
//! * Triggers (combo, double tap, hold) and an optional window rule decide
//!   whether the bridge *should* be active.
//! * Activation waits until no key or button is pressed: grabbing a device
//!   while a key is down leaves that key stuck on the desktop. Hold triggers
//!   are the exception (the trigger key is held by definition).
//! * Deactivation is immediate: release everything, ungrab.
//! * The emergency combination (Ctrl+Alt+Shift+Esc) always deactivates and
//!   cannot be configured away.
//! * The key that fires a trigger is never forwarded to the PlayStation.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::keymap::{self, Keymap};
use crate::mouse::{MouseCurve, MouseFilter};
use crate::transport::KeyState;
use crate::trigger::{Action, Combo, Trigger, TriggerKind, emergency_combo};

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_HWHEEL: u16 = 0x06;
pub const REL_WHEEL: u16 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Key or button; `down` false = released. Auto-repeat must be filtered out.
    Key { code: u16, down: bool },
    /// Relative axis (REL_X, REL_Y, REL_WHEEL, REL_HWHEEL; others ignored).
    Rel { axis: u16, value: i32 },
    /// End of an evdev frame (SYN_REPORT).
    Sync,
    /// Window rule: whether the configured window has the focus.
    Focus(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Activated,
    Deactivated,
    WaitingForRelease,
    Paused,
    Resumed,
    Emergency,
    ReleasedAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Grab (true) or release (false) the input devices.
    Grab(bool),
    /// Bridge on/off; off releases everything on the board.
    SetActive(bool),
    State(KeyState),
    Motion(i32, i32, i32, i32),
    ReleaseAll,
    Notice(Notice),
}

#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    pub triggers: Vec<Trigger>,
    /// A window rule is configured (Focus events drive activation).
    pub window_rule: bool,
    pub keymap: Keymap,
    pub mouse: MouseCurve,
}

#[derive(Default)]
struct DoubleTapState {
    last_tap: Option<Instant>,
    clean: bool, // no other key pressed since this key went down
}

pub struct Engine {
    cfg: EngineConfig,
    emergency: Combo,
    pressed: Vec<u16>, // in press order
    suppressed: BTreeSet<u16>,
    latched: Vec<bool>, // per trigger: fired, waiting for its keys to be released
    taps: Vec<DoubleTapState>,
    manual: bool,
    paused: bool,
    hold: bool,
    /// A hold trigger was overridden (emergency stop, Off) while held: it
    /// stays ineffective until its keys are released.
    hold_blocked: bool,
    focus: bool,
    active: bool,
    pending: bool,
    sent: Option<KeyState>,
    rel: [i32; 4],
    mouse: MouseFilter,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        let n = cfg.triggers.len();
        let mouse = MouseFilter::new(cfg.mouse);
        Self {
            emergency: emergency_combo(),
            pressed: Vec::new(),
            suppressed: BTreeSet::new(),
            latched: vec![false; n],
            taps: (0..n).map(|_| DoubleTapState::default()).collect(),
            manual: false,
            paused: false,
            hold: false,
            hold_blocked: false,
            focus: false,
            active: false,
            pending: false,
            sent: None,
            rel: [0; 4],
            mouse,
            cfg,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_mouse_curve(&mut self, curve: MouseCurve) {
        self.mouse.curve = curve;
    }

    /// Apply an action from outside (GUI button, tray menu).
    pub fn command(&mut self, action: Action) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.apply_action(action, &mut fx);
        self.update(&mut fx);
        fx
    }

    pub fn handle(&mut self, ev: Event, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match ev {
            Event::Key { code, down: true } => self.key_down(code, &mut fx),
            Event::Key { code, down: false } => self.key_up(code, now, &mut fx),
            Event::Rel { axis, value } => {
                let i = match axis {
                    REL_X => 0,
                    REL_Y => 1,
                    REL_WHEEL => 2,
                    REL_HWHEEL => 3,
                    _ => return fx,
                };
                self.rel[i] = self.rel[i].saturating_add(value);
            }
            Event::Sync => {
                let [dx, dy, wheel, pan] = std::mem::take(&mut self.rel);
                if self.active && (dx | dy | wheel | pan) != 0 {
                    let (dx, dy) = self.mouse.apply(dx, dy);
                    fx.push(Effect::Motion(dx, dy, wheel, pan));
                }
            }
            Event::Focus(f) => {
                if f != self.focus {
                    self.focus = f;
                    self.paused = false; // a pause only lasts for one focus period
                    self.update(&mut fx);
                }
            }
        }
        fx
    }

    fn key_down(&mut self, code: u16, fx: &mut Vec<Effect>) {
        if self.pressed.contains(&code) {
            return;
        }
        self.pressed.push(code);

        if self.emergency.contains(code) && self.emergency.satisfied(&self.pressed) {
            self.manual = false;
            self.block_hold();
            self.paused = self.cfg.window_rule;
            self.suppressed.insert(code);
            fx.push(Effect::ReleaseAll);
            fx.push(Effect::Notice(Notice::Emergency));
            self.update(fx);
            return;
        }

        for tap in &mut self.taps {
            tap.clean = false;
        }
        let mut fired = Vec::new();
        for (i, t) in self.cfg.triggers.iter().enumerate() {
            match &t.kind {
                TriggerKind::Combo(c) => {
                    if !self.latched[i] && c.contains(code) && c.satisfied(&self.pressed) {
                        self.latched[i] = true;
                        fired.push(t.action);
                    }
                }
                TriggerKind::DoubleTap { key, .. } => {
                    if key.matches(code) && self.pressed.len() == 1 {
                        self.taps[i].clean = true;
                    }
                }
                TriggerKind::Hold(_) => {}
            }
        }
        if !fired.is_empty() {
            self.suppressed.insert(code);
        }
        for a in fired {
            self.apply_action(a, fx);
        }
        self.update_hold();
        self.update(fx);
        self.forward(fx);
    }

    fn key_up(&mut self, code: u16, now: Instant, fx: &mut Vec<Effect>) {
        let Some(pos) = self.pressed.iter().position(|c| *c == code) else {
            return;
        };
        self.pressed.remove(pos);
        self.suppressed.remove(&code);

        let mut fired = Vec::new();
        for (i, t) in self.cfg.triggers.iter().enumerate() {
            match &t.kind {
                TriggerKind::Combo(c) => {
                    if self.latched[i] && !self.pressed.iter().any(|p| c.contains(*p)) {
                        self.latched[i] = false;
                    }
                }
                TriggerKind::DoubleTap { key, window_ms } => {
                    let tap = &mut self.taps[i];
                    if key.matches(code) && tap.clean && self.pressed.is_empty() {
                        let within = tap
                            .last_tap
                            .is_some_and(|t| now.duration_since(t) <= Duration::from_millis(*window_ms));
                        if within {
                            tap.last_tap = None;
                            fired.push(t.action);
                        } else {
                            tap.last_tap = Some(now);
                        }
                    } else if !key.matches(code) {
                        tap.last_tap = None;
                    }
                    tap.clean = false;
                }
                TriggerKind::Hold(_) => {}
            }
        }
        for a in fired {
            self.apply_action(a, fx);
        }
        self.update_hold();
        self.update(fx);
        self.forward(fx);
    }

    fn update_hold(&mut self) {
        let held = self.cfg.triggers.iter().any(|t| match &t.kind {
            TriggerKind::Hold(c) => c.satisfied(&self.pressed),
            _ => false,
        });
        if !held {
            self.hold_blocked = false;
        }
        self.hold = held && !self.hold_blocked;
    }

    /// End an active hold until its keys have been released.
    fn block_hold(&mut self) {
        if self.hold {
            self.hold_blocked = true;
            self.hold = false;
        }
    }

    fn desired(&self) -> bool {
        (self.manual || self.hold || (self.cfg.window_rule && self.focus)) && !self.paused
    }

    fn apply_action(&mut self, action: Action, fx: &mut Vec<Effect>) {
        match action {
            Action::On => {
                self.manual = true;
                self.paused = false;
            }
            Action::Off => {
                self.manual = false;
                self.block_hold();
                // Also overrides an active window rule until the focus changes.
                self.paused = self.cfg.window_rule && self.focus;
            }
            Action::Toggle => {
                let on = self.active || self.pending;
                self.apply_action(if on { Action::Off } else { Action::On }, fx);
            }
            Action::Pause => {
                self.paused = !self.paused;
                fx.push(Effect::Notice(if self.paused {
                    Notice::Paused
                } else {
                    Notice::Resumed
                }));
            }
            Action::ReleaseAll => {
                fx.push(Effect::ReleaseAll);
                fx.push(Effect::Notice(Notice::ReleasedAll));
                self.sent = Some(KeyState::default());
            }
        }
    }

    /// Bring `active` in line with `desired`.
    fn update(&mut self, fx: &mut Vec<Effect>) {
        let want = self.desired();
        if want && !self.active {
            if self.hold || self.pressed.is_empty() {
                self.activate(fx);
            } else if !self.pending {
                self.pending = true;
                fx.push(Effect::Notice(Notice::WaitingForRelease));
            }
        } else if !want {
            self.pending = false;
            if self.active {
                self.active = false;
                self.sent = None;
                self.rel = [0; 4];
                fx.push(Effect::SetActive(false));
                fx.push(Effect::Grab(false));
                fx.push(Effect::Notice(Notice::Deactivated));
            }
        }
    }

    fn activate(&mut self, fx: &mut Vec<Effect>) {
        self.active = true;
        self.pending = false;
        // Whatever is held right now (only possible with hold triggers)
        // stays with the desktop.
        self.suppressed.extend(self.pressed.iter().copied());
        self.sent = None;
        fx.push(Effect::Grab(true));
        fx.push(Effect::SetActive(true));
        fx.push(Effect::Notice(Notice::Activated));
        self.forward(fx);
    }

    fn key_state(&self) -> KeyState {
        let mut s = KeyState::default();
        let mut keys = Vec::new();
        for &code in &self.pressed {
            if self.suppressed.contains(&code) {
                continue;
            }
            if let Some(bit) = keymap::modifier_bit(code) {
                s.mods |= bit;
            } else if let Some(bit) = keymap::button_bit(code) {
                s.buttons |= bit;
            } else if let Some(u) = self.cfg.keymap.usage(code) {
                keys.push(u);
            }
        }
        if keys.len() > 6 {
            s.keys = [0x01; 6]; // ErrorRollOver
        } else {
            s.keys[..keys.len()].copy_from_slice(&keys);
        }
        s
    }

    fn forward(&mut self, fx: &mut Vec<Effect>) {
        if !self.active {
            return;
        }
        let s = self.key_state();
        if self.sent != Some(s) {
            self.sent = Some(s);
            fx.push(Effect::State(s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{BTN_EXTRA, BTN_LEFT};
    use crate::trigger::{KEY_LEFTALT, KEY_LEFTCTRL, KEY_LEFTSHIFT, KeyAlt};

    const P: u16 = 25;
    const A: u16 = 30;
    const W: u16 = 17;
    const ESC: u16 = 1;

    struct T {
        e: Engine,
        now: Instant,
    }

    impl T {
        fn new(triggers: Vec<Trigger>, window_rule: bool) -> Self {
            Self {
                e: Engine::new(EngineConfig {
                    triggers,
                    window_rule,
                    ..Default::default()
                }),
                now: Instant::now(),
            }
        }
        fn toggle() -> Self {
            Self::new(vec![Trigger::combo("Ctrl+Alt+P", Action::Toggle).unwrap()], false)
        }
        fn ev(&mut self, ev: Event) -> Vec<Effect> {
            self.e.handle(ev, self.now)
        }
        fn down(&mut self, code: u16) -> Vec<Effect> {
            self.ev(Event::Key { code, down: true })
        }
        fn up(&mut self, code: u16) -> Vec<Effect> {
            self.ev(Event::Key { code, down: false })
        }
        fn tap(&mut self, code: u16) -> Vec<Effect> {
            let mut fx = self.down(code);
            fx.extend(self.up(code));
            fx
        }
        fn press_combo(&mut self, codes: &[u16]) -> Vec<Effect> {
            let mut fx = Vec::new();
            for c in codes {
                fx.extend(self.down(*c));
            }
            for c in codes.iter().rev() {
                fx.extend(self.up(*c));
            }
            fx
        }
        fn wait(&mut self, ms: u64) {
            self.now += Duration::from_millis(ms);
        }
    }

    fn states(fx: &[Effect]) -> Vec<KeyState> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::State(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn toggle_waits_for_release_then_grabs() {
        let mut t = T::toggle();
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        let fx = t.down(P);
        assert!(fx.contains(&Effect::Notice(Notice::WaitingForRelease)));
        assert!(!fx.contains(&Effect::Grab(true)));
        assert!(t.e.is_pending());
        t.up(P);
        t.up(KEY_LEFTALT);
        let fx = t.up(KEY_LEFTCTRL);
        assert!(fx.contains(&Effect::Grab(true)));
        assert!(fx.contains(&Effect::SetActive(true)));
        assert_eq!(states(&fx), vec![KeyState::default()]);
        assert!(t.e.is_active());
    }

    #[test]
    fn forwards_keys_mods_buttons_and_motion_when_active() {
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        let fx = t.down(KEY_LEFTSHIFT);
        assert_eq!(states(&fx)[0].mods, 0x02);
        let fx = t.down(A);
        assert_eq!(states(&fx)[0].keys, [0x04, 0, 0, 0, 0, 0]);
        let fx = t.down(BTN_LEFT);
        assert_eq!(states(&fx)[0].buttons, 0x01);
        t.ev(Event::Rel {
            axis: REL_X,
            value: 3,
        });
        t.ev(Event::Rel {
            axis: REL_X,
            value: 2,
        });
        t.ev(Event::Rel {
            axis: REL_WHEEL,
            value: -1,
        });
        assert_eq!(t.ev(Event::Sync), vec![Effect::Motion(5, 0, -1, 0)]);
        // A repeat of the same state is not sent again.
        assert!(states(&t.down(A)).is_empty());
    }

    #[test]
    fn nothing_forwarded_while_inactive() {
        let mut t = T::toggle();
        assert!(t.down(A).is_empty());
        t.ev(Event::Rel {
            axis: REL_X,
            value: 3,
        });
        assert!(t.ev(Event::Sync).is_empty());
    }

    #[test]
    fn toggle_off_is_immediate_and_trigger_key_not_forwarded() {
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        let fx = t.down(P);
        assert!(fx.contains(&Effect::SetActive(false)));
        assert!(fx.contains(&Effect::Grab(false)));
        assert!(states(&fx).iter().all(|s| !s.keys.contains(&0x13)));
        assert!(!t.e.is_active());
    }

    #[test]
    fn combo_needs_release_before_firing_again() {
        let mut t = T::toggle();
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        t.down(P);
        t.up(P);
        // P again while Ctrl+Alt still held: combo is latched until all its keys are up.
        t.down(P);
        assert!(t.e.is_pending());
    }

    #[test]
    fn emergency_always_releases() {
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        assert!(t.e.is_active());
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        t.down(KEY_LEFTSHIFT);
        let fx = t.down(ESC);
        assert!(fx.contains(&Effect::ReleaseAll));
        assert!(fx.contains(&Effect::Notice(Notice::Emergency)));
        assert!(fx.contains(&Effect::Grab(false)));
        assert!(!t.e.is_active());
    }

    #[test]
    fn mouse_button_trigger_is_not_forwarded() {
        let mut t = T::new(vec![Trigger::combo("Mouse5", Action::Toggle).unwrap()], false);
        let fx = t.tap(BTN_EXTRA);
        assert!(fx.contains(&Effect::Grab(true)));
        let fx = t.down(BTN_EXTRA);
        assert!(fx.contains(&Effect::SetActive(false)));
        assert!(states(&fx).iter().all(|s| s.buttons == 0));
    }

    #[test]
    fn double_tap() {
        let mut t = T::new(
            vec![Trigger {
                kind: TriggerKind::DoubleTap {
                    key: KeyAlt(vec![KEY_LEFTCTRL]),
                    window_ms: 300,
                },
                action: Action::Toggle,
            }],
            false,
        );
        t.tap(KEY_LEFTCTRL);
        t.wait(200);
        let fx = t.tap(KEY_LEFTCTRL);
        assert!(fx.contains(&Effect::Grab(true)));
        // Too slow: no trigger.
        t.wait(1000);
        t.tap(KEY_LEFTCTRL);
        t.wait(400);
        assert!(!t.tap(KEY_LEFTCTRL).contains(&Effect::SetActive(false)));
        // Ctrl+A in between breaks the sequence.
        t.wait(1000);
        t.tap(KEY_LEFTCTRL);
        t.down(KEY_LEFTCTRL);
        t.down(A);
        t.up(A);
        assert!(!t.up(KEY_LEFTCTRL).contains(&Effect::SetActive(false)));
    }

    #[test]
    fn hold_activates_immediately_and_keeps_held_keys_on_desktop() {
        let mut t = T::new(
            vec![Trigger {
                kind: TriggerKind::Hold(Combo::parse("Mouse4").unwrap()),
                action: Action::On,
            }],
            false,
        );
        t.down(W); // already held before
        let fx = t.down(crate::keymap::BTN_SIDE);
        assert!(fx.contains(&Effect::Grab(true)));
        assert_eq!(states(&fx), vec![KeyState::default()]);
        let fx = t.down(A);
        assert_eq!(states(&fx)[0].keys, [0x04, 0, 0, 0, 0, 0]);
        let fx = t.up(crate::keymap::BTN_SIDE);
        assert!(fx.contains(&Effect::SetActive(false)));
    }

    fn hold_mouse4() -> T {
        T::new(
            vec![Trigger {
                kind: TriggerKind::Hold(Combo::parse("Mouse4").unwrap()),
                action: Action::On,
            }],
            false,
        )
    }

    #[test]
    fn emergency_stop_beats_a_held_hold_trigger() {
        let mut t = hold_mouse4();
        assert!(t.down(crate::keymap::BTN_SIDE).contains(&Effect::Grab(true)));
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        t.down(KEY_LEFTSHIFT);
        let fx = t.down(ESC);
        assert!(fx.contains(&Effect::Grab(false)));
        // Releasing the combo while Mouse4 is still held must not grab again.
        for k in [ESC, KEY_LEFTSHIFT, KEY_LEFTALT, KEY_LEFTCTRL] {
            assert!(!t.up(k).contains(&Effect::Grab(true)));
        }
        assert!(!t.down(A).contains(&Effect::Grab(true)));
        t.up(A);
        assert!(!t.e.is_active());
        // Released and pressed again: the hold works as before.
        t.up(crate::keymap::BTN_SIDE);
        assert!(t.down(crate::keymap::BTN_SIDE).contains(&Effect::Grab(true)));
    }

    #[test]
    fn off_beats_a_held_hold_trigger() {
        let mut t = hold_mouse4();
        t.down(crate::keymap::BTN_SIDE);
        assert!(t.e.is_active());
        let fx = t.e.command(Action::Off);
        assert!(fx.contains(&Effect::Grab(false)));
        assert!(!t.down(A).contains(&Effect::Grab(true)));
        assert!(!t.e.is_active());
        t.up(A);
        t.up(crate::keymap::BTN_SIDE);
        assert!(t.down(crate::keymap::BTN_SIDE).contains(&Effect::Grab(true)));
    }

    #[test]
    fn off_without_hold_does_not_block_the_next_hold() {
        let mut t = hold_mouse4();
        t.e.command(Action::Off);
        assert!(t.down(crate::keymap::BTN_SIDE).contains(&Effect::Grab(true)));
    }

    #[test]
    fn window_rule_and_pause() {
        let mut t = T::new(vec![Trigger::combo("Pause", Action::Pause).unwrap()], true);
        let fx = t.ev(Event::Focus(true));
        assert!(fx.contains(&Effect::Grab(true)));
        let fx = t.down(119); // PAUSE key
        assert!(fx.contains(&Effect::SetActive(false)));
        t.up(119);
        assert!(!t.e.is_active());
        // Focus away and back: pause is over.
        t.ev(Event::Focus(false));
        let fx = t.ev(Event::Focus(true));
        assert!(fx.contains(&Effect::Grab(true)));
        // Focus lost: off.
        let fx = t.ev(Event::Focus(false));
        assert!(fx.contains(&Effect::Grab(false)));
    }

    #[test]
    fn window_focus_waits_for_release() {
        let mut t = T::new(vec![], true);
        t.down(A);
        let fx = t.ev(Event::Focus(true));
        assert!(!fx.contains(&Effect::Grab(true)));
        let fx = t.up(A);
        assert!(fx.contains(&Effect::Grab(true)));
    }

    #[test]
    fn rollover_with_more_than_six_keys() {
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        let mut last = Vec::new();
        for code in [30, 31, 32, 33, 34, 35, 36] {
            last = t.down(code);
        }
        assert_eq!(states(&last)[0].keys, [0x01; 6]);
        let fx = t.up(36);
        assert_eq!(states(&fx)[0].keys, [0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B]);
    }

    #[test]
    fn command_on_off() {
        let mut t = T::new(vec![], false);
        assert!(t.e.command(Action::On).contains(&Effect::Grab(true)));
        assert!(t.e.command(Action::Off).contains(&Effect::Grab(false)));
    }
}
