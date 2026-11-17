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
//! * The key that fires a combination trigger is never forwarded to the
//!   PlayStation. A double tap is recognised only on its second release, so
//!   both taps have reached the PlayStation by then (with the bridge on).
//! * Keyboards that are not selected are only watched ([`Event::TriggerKey`]):
//!   their keys count for triggers and the emergency stop, but never reach
//!   the PlayStation and never delay an activation (they are not grabbed).
//! * Profiles ([`EngineProfile`]) carry the mouse settings and a remapping of
//!   keys and buttons. The remapping applies on the way out only: triggers
//!   and the emergency stop always see the physical keys. A key keeps the
//!   target it got when it went down until it is released, so switching
//!   profiles never leaves anything pressed on the PlayStation.
//! * The effective profile is the one the focused window asks for (see
//!   [`Engine::window`]), else the one chosen by hand or by a trigger.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::keymap::{self, Keymap};
use crate::mouse::{MouseCurve, MouseFilter};
use crate::transport::KeyState;
use crate::trigger::{Action, Combo, Trigger, TriggerKind, emergency_combo};
use crate::window::{self, WindowInfo};

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_HWHEEL: u16 = 0x06;
pub const REL_WHEEL: u16 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Key or button; `down` false = released. Auto-repeat must be filtered out.
    Key { code: u16, down: bool },
    /// Key of a keyboard that is only watched (not selected): it counts for
    /// triggers and the emergency stop, and is never forwarded.
    TriggerKey { code: u16, down: bool },
    /// Relative axis (REL_X, REL_Y, REL_WHEEL, REL_HWHEEL; others ignored).
    Rel { axis: u16, value: i32 },
    /// End of an evdev frame (SYN_REPORT).
    Sync,
    /// Window rule: whether the configured window has the focus.
    Focus(bool),
}

impl Event {
    /// Code and direction of a key event of either kind.
    pub fn key(&self) -> Option<(u16, bool)> {
        match *self {
            Event::Key { code, down } | Event::TriggerKey { code, down } => Some((code, down)),
            _ => None,
        }
    }
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
    /// Another profile applies (by a trigger or the focused window).
    ProfileChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Grab (true) or release (false) the input devices.
    Grab(bool),
    /// Bridge on/off; off releases everything on the board.
    SetActive(bool),
    State(KeyState),
    Motion(i32, i32, i32, i32),
    ReleaseAll,
    /// "Release all" forgot these keys and buttons: the device layer must
    /// not press them again from the kernel's key state until their next
    /// real release (a mouse whose receiver lost the release keeps
    /// reporting the button as held).
    Forget(Vec<u16>),
    Notice(Notice),
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub triggers: Vec<Trigger>,
    /// A window rule is configured (Focus events drive activation).
    pub window_rule: bool,
    /// Its pattern (see [`window::matches`]), for [`Engine::window`].
    pub window_pattern: String,
    pub keymap: Keymap,
    /// At least one; the first is "Standard".
    pub profiles: Vec<EngineProfile>,
    /// The profile chosen by hand at the start.
    pub profile: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            triggers: Vec::new(),
            window_rule: false,
            window_pattern: String::new(),
            keymap: Keymap::default(),
            profiles: vec![EngineProfile::default()],
            profile: 0,
        }
    }
}

/// Mouse settings and remapped keys that apply together.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineProfile {
    pub name: String,
    /// Applies while a window matching this pattern has the focus ("" =
    /// only by hand or by a trigger).
    pub window: String,
    pub mouse: MouseCurve,
    /// Physical code -> code sent instead (None: not sent at all).
    pub remap: BTreeMap<u16, Option<u16>>,
}

impl Default for EngineProfile {
    fn default() -> Self {
        Self {
            name: STANDARD_PROFILE.into(),
            window: String::new(),
            mouse: MouseCurve::default(),
            remap: BTreeMap::new(),
        }
    }
}

/// Name of the built-in first profile.
pub const STANDARD_PROFILE: &str = "Standard";

#[derive(Default)]
struct DoubleTapState {
    last_tap: Option<Instant>,
    clean: bool, // no other key pressed since this key went down
}

pub struct Engine {
    cfg: EngineConfig,
    emergency: Combo,
    pressed: Vec<u16>, // in press order
    /// Held on watched keyboards (only for triggers, never forwarded).
    watched: Vec<u16>,
    suppressed: BTreeSet<u16>,
    /// Forgotten by "release all" while still held: no longer forwarded,
    /// but the triggers and the emergency stop still see them until their
    /// real release (the device layer passes that edge on).
    forgotten: BTreeSet<u16>,
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
    /// Profile chosen by hand or by a trigger.
    manual_profile: usize,
    /// Profile the focused window asks for (overrides `manual_profile`).
    window_profile: Option<usize>,
    /// Target of each forwarded key, fixed when it went down.
    targets: BTreeMap<u16, Option<u16>>,
    /// The focused window as last reported.
    last_window: Option<WindowInfo>,
    /// A profile was chosen by hand since the focus last moved: the window's
    /// profile does not apply until it moves again.
    hand_picked: bool,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        let mut cfg = cfg;
        if cfg.profiles.is_empty() {
            cfg.profiles.push(EngineProfile::default());
        }
        let manual_profile = cfg.profile.min(cfg.profiles.len() - 1);
        let n = cfg.triggers.len();
        let mouse = MouseFilter::new(cfg.profiles[manual_profile].mouse);
        Self {
            emergency: emergency_combo(),
            pressed: Vec::new(),
            watched: Vec::new(),
            suppressed: BTreeSet::new(),
            forgotten: BTreeSet::new(),
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
            manual_profile,
            window_profile: None,
            targets: BTreeMap::new(),
            last_window: None,
            hand_picked: false,
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

    /// Index of the profile in effect.
    pub fn profile(&self) -> usize {
        self.window_profile.unwrap_or(self.manual_profile)
    }

    /// Index of the profile chosen by hand or by a trigger.
    pub fn manual_profile(&self) -> usize {
        self.manual_profile
    }

    pub fn profiles(&self) -> &[EngineProfile] {
        &self.cfg.profiles
    }

    /// New settings for the profiles. Keys held keep their targets until
    /// released. The window's profile is found again by [`Self::window`].
    pub fn set_profiles(&mut self, profiles: Vec<EngineProfile>) {
        if profiles.is_empty() {
            return;
        }
        self.manual_profile = self.manual_profile.min(profiles.len() - 1);
        self.window_profile = self.window_profile.filter(|i| *i < profiles.len());
        self.cfg.profiles = profiles;
        self.mouse.curve = self.cfg.profiles[self.profile()].mouse;
    }

    /// The focused window (None: no window, or not known), on every change
    /// and after new settings: picks the profile that asks for it, then
    /// applies the window rule.
    pub fn window(&mut self, info: Option<&WindowInfo>, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        if info != self.last_window.as_ref() {
            self.last_window = info.cloned();
            self.hand_picked = false;
        }
        let wanted = info.filter(|_| !self.hand_picked).and_then(|w| {
            self.cfg
                .profiles
                .iter()
                .position(|p| window::matches(&p.window, w))
        });
        self.switch_profile(self.manual_profile, wanted, &mut fx);
        let focus =
            self.cfg.window_rule && info.is_some_and(|w| window::matches(&self.cfg.window_pattern, w));
        fx.extend(self.handle(Event::Focus(focus), now));
        fx
    }

    fn switch_profile(&mut self, manual: usize, window: Option<usize>, fx: &mut Vec<Effect>) {
        let before = self.profile();
        self.manual_profile = manual;
        self.window_profile = window;
        let now = self.profile();
        self.mouse.curve = self.cfg.profiles[now].mouse;
        if now != before {
            fx.push(Effect::Notice(Notice::ProfileChanged));
        }
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
            Event::Key { code, down: true } => self.key_down(code, false, &mut fx),
            Event::Key { code, down: false } => self.key_up(code, false, now, &mut fx),
            Event::TriggerKey { code, down: true } => self.key_down(code, true, &mut fx),
            Event::TriggerKey { code, down: false } => self.key_up(code, true, now, &mut fx),
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
                    let (wheel, pan) = self.mouse.apply_wheel(wheel, pan);
                    if (dx | dy | wheel | pan) != 0 {
                        fx.push(Effect::Motion(dx, dy, wheel, pan));
                    }
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

    /// Everything held, forwarded, only watched or forgotten by "release
    /// all": what the triggers see.
    fn all_pressed(&self) -> Vec<u16> {
        let mut all = self.pressed.clone();
        for &c in self.watched.iter().chain(self.forgotten.iter()) {
            if !all.contains(&c) {
                all.push(c);
            }
        }
        all
    }

    fn key_down(&mut self, code: u16, watched: bool, fx: &mut Vec<Effect>) {
        if !watched {
            self.forgotten.remove(&code); // a real edge: no longer forgotten
        }
        let list = if watched {
            &mut self.watched
        } else {
            &mut self.pressed
        };
        if list.contains(&code) {
            return;
        }
        list.push(code);
        if !watched {
            let target = self.cfg.profiles[self.profile()]
                .remap
                .get(&code)
                .copied()
                .unwrap_or(Some(code));
            self.targets.insert(code, target);
        }
        let all = self.all_pressed();

        if self.emergency.contains(code) && self.emergency.satisfied(&all) {
            self.manual = false;
            self.block_hold();
            self.paused = self.cfg.window_rule;
            if !watched {
                self.suppressed.insert(code);
            }
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
                    if !self.latched[i] && c.contains(code) && c.satisfied(&all) {
                        self.latched[i] = true;
                        fired.push(t.action);
                    }
                }
                TriggerKind::DoubleTap { key, .. } => {
                    if key.matches(code) && all.len() == 1 {
                        self.taps[i].clean = true;
                    }
                }
                TriggerKind::Hold(_) => {}
            }
        }
        if !fired.is_empty() && !watched {
            self.suppressed.insert(code);
        }
        for a in fired {
            self.apply_action(a, fx);
        }
        self.update_hold();
        self.update(fx);
        self.forward(fx);
    }

    fn key_up(&mut self, code: u16, watched: bool, now: Instant, fx: &mut Vec<Effect>) {
        let list = if watched {
            &mut self.watched
        } else {
            &mut self.pressed
        };
        let forgotten = !watched && self.forgotten.remove(&code);
        match list.iter().position(|c| *c == code) {
            Some(pos) => {
                list.remove(pos);
                if !watched {
                    self.suppressed.remove(&code);
                    self.targets.remove(&code);
                }
            }
            // A forgotten key let go: it may end a trigger's latch.
            None if forgotten => {}
            None => return,
        }
        let all = self.all_pressed();

        let mut fired = Vec::new();
        for (i, t) in self.cfg.triggers.iter().enumerate() {
            match &t.kind {
                TriggerKind::Combo(c) => {
                    if self.latched[i] && !all.iter().any(|p| c.contains(*p)) {
                        self.latched[i] = false;
                    }
                }
                TriggerKind::DoubleTap { key, window_ms } => {
                    let tap = &mut self.taps[i];
                    if key.matches(code) && tap.clean && all.is_empty() {
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
        let all = self.all_pressed();
        let held = self.cfg.triggers.iter().any(|t| match &t.kind {
            TriggerKind::Hold(c) => c.satisfied(&all),
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
                // Everything that went to the PlayStation is let go, also
                // when the kernel still says it is held (a lost release in
                // the mouse or its receiver): it is forgotten until its next
                // real release. Suppressed keys (trigger keys, keys held
                // since activation) stay with the desktop as before.
                let forgotten: Vec<u16> = self
                    .pressed
                    .iter()
                    .copied()
                    .filter(|c| !self.suppressed.contains(c))
                    .collect();
                self.pressed.retain(|c| self.suppressed.contains(c));
                self.targets.retain(|c, _| self.pressed.contains(c));
                self.forgotten.extend(forgotten.iter().copied());
                fx.push(Effect::ReleaseAll);
                if !forgotten.is_empty() {
                    fx.push(Effect::Forget(forgotten));
                }
                fx.push(Effect::Notice(Notice::ReleasedAll));
                self.sent = Some(KeyState::default());
                self.update_hold();
            }
            // Chosen by hand: also over a window's profile, until the
            // focus moves to another window.
            Action::Profile(i) => {
                let i = (i as usize).min(self.cfg.profiles.len() - 1);
                self.hand_picked = true;
                self.switch_profile(i, None, fx);
            }
            Action::NextProfile => {
                let next = (self.profile() + 1) % self.cfg.profiles.len();
                self.hand_picked = true;
                self.switch_profile(next, None, fx);
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
                self.mouse.reset();
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
            let Some(code) = self.targets.get(&code).copied().unwrap_or(Some(code)) else {
                continue; // switched off in its profile
            };
            if let Some(bit) = keymap::modifier_bit(code) {
                s.mods |= bit;
            } else if let Some(bit) = keymap::button_bit(code) {
                s.buttons |= bit;
            } else if let Some(u) = self.cfg.keymap.usage(code)
                && !keys.contains(&u)
            {
                // (Two keys remapped to the same one count once.)
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
    const R: u16 = 19;
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
        /// On a keyboard that is only watched (not selected).
        fn wdown(&mut self, code: u16) -> Vec<Effect> {
            self.ev(Event::TriggerKey { code, down: true })
        }
        fn wup(&mut self, code: u16) -> Vec<Effect> {
            self.ev(Event::TriggerKey { code, down: false })
        }
        fn wtap(&mut self, code: u16) -> Vec<Effect> {
            let mut fx = self.wdown(code);
            fx.extend(self.wup(code));
            fx
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
    fn release_all_forgets_what_was_forwarded() {
        // A mouse whose receiver lost the release keeps reporting the
        // button as held: "release all" lets go on the PlayStation and
        // forgets the button, so the next event does not press it again.
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        assert!(t.e.is_active());
        assert_eq!(states(&t.down(BTN_LEFT))[0].buttons, 0x01);
        t.down(W);
        let fx = t.e.command(Action::ReleaseAll);
        assert!(fx.contains(&Effect::ReleaseAll));
        assert!(fx.contains(&Effect::Forget(vec![BTN_LEFT, W])));
        // Motion and other keys go on, but the forgotten button stays released.
        assert!(
            states(&t.down(A))
                .iter()
                .all(|s| s.buttons == 0 && !s.keys.contains(&0x1a))
        );
        // Its real release (the kernel passes it when the user clicks again) is a no-op...
        assert!(states(&t.up(BTN_LEFT)).is_empty());
        // ...and the next press works again.
        assert_eq!(states(&t.down(BTN_LEFT))[0].buttons, 0x01);
    }

    #[test]
    fn release_all_keeps_suppressed_keys_with_the_desktop() {
        // The key that fires a trigger is never forwarded; release all
        // does not forget it, so its release still ends the trigger's latch.
        let mut t = T::new(
            vec![
                Trigger::combo("Ctrl+Alt+P", Action::Toggle).unwrap(),
                Trigger::combo("Ctrl+Alt+R", Action::ReleaseAll).unwrap(),
            ],
            false,
        );
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        t.down(BTN_LEFT);
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        let fx = t.down(R);
        let forgot = fx.iter().find_map(|e| match e {
            Effect::Forget(c) => Some(c.clone()),
            _ => None,
        });
        assert_eq!(forgot, Some(vec![BTN_LEFT, KEY_LEFTCTRL, KEY_LEFTALT]));
        t.up(R);
        t.up(KEY_LEFTALT);
        t.up(KEY_LEFTCTRL);
        // The trigger can fire again (its latch was cleared by R's release).
        t.down(BTN_LEFT);
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        assert!(t.down(R).iter().any(|e| matches!(e, Effect::Forget(_))));
    }

    #[test]
    fn emergency_stop_still_sees_keys_forgotten_by_release_all() {
        // "Release all" by Ctrl+Alt+R forgets Ctrl and Alt (they went to the
        // PlayStation); held on and joined by Shift+Esc they are still the
        // emergency combination.
        let mut t = T::new(
            vec![
                Trigger::combo("Ctrl+Alt+P", Action::Toggle).unwrap(),
                Trigger::combo("Ctrl+Alt+R", Action::ReleaseAll).unwrap(),
            ],
            false,
        );
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        assert!(t.e.is_active());
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        let fx = t.down(R);
        assert!(fx.iter().any(|e| matches!(e, Effect::Forget(_))));
        t.down(KEY_LEFTSHIFT);
        let fx = t.down(ESC);
        assert!(fx.contains(&Effect::ReleaseAll), "{fx:?}");
        assert!(!t.e.is_active());
        // Released for real, the forgotten keys end the triggers' latches:
        // the toggle works again.
        for k in [ESC, KEY_LEFTSHIFT, R, KEY_LEFTALT, KEY_LEFTCTRL] {
            t.up(k);
        }
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        assert!(t.e.is_active());
    }

    #[test]
    fn toggle_from_a_watched_keyboard() {
        let mut t = T::toggle();
        t.wdown(KEY_LEFTCTRL);
        t.wdown(KEY_LEFTALT);
        // Its keys are not grabbed and never forwarded: on at once, nothing to wait for.
        let fx = t.wdown(P);
        assert!(fx.contains(&Effect::Grab(true)));
        assert_eq!(states(&fx), vec![KeyState::default()]);
        for k in [P, KEY_LEFTALT, KEY_LEFTCTRL] {
            assert!(states(&t.wup(k)).is_empty());
        }
        // Typing on the selected keyboard is forwarded, on the watched one not.
        assert_eq!(states(&t.down(A))[0].keys, [0x04, 0, 0, 0, 0, 0]);
        assert!(states(&t.wdown(W)).is_empty());
        t.wup(W);
        t.up(A);
        // And off again from the watched keyboard.
        t.wdown(KEY_LEFTCTRL);
        t.wdown(KEY_LEFTALT);
        assert!(t.wdown(P).contains(&Effect::Grab(false)));
        assert!(!t.e.is_active());
    }

    #[test]
    fn combination_across_keyboards() {
        let mut t = T::toggle();
        t.down(KEY_LEFTCTRL); // selected keyboard
        t.wdown(KEY_LEFTALT);
        let fx = t.wdown(P);
        // Ctrl is held on a selected (soon grabbed) keyboard: wait for it.
        assert!(fx.contains(&Effect::Notice(Notice::WaitingForRelease)));
        t.wup(P);
        t.wup(KEY_LEFTALT);
        assert!(t.up(KEY_LEFTCTRL).contains(&Effect::Grab(true)));
    }

    #[test]
    fn watched_keys_do_not_delay_activation() {
        let mut t = T::toggle();
        t.wdown(A); // someone holds a key on the other keyboard
        let fx = t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        assert!(fx.contains(&Effect::Grab(true)));
        assert_eq!(states(&fx), vec![KeyState::default()]);
    }

    #[test]
    fn emergency_stop_from_a_watched_keyboard() {
        let mut t = T::toggle();
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        t.down(A);
        t.wdown(KEY_LEFTCTRL);
        t.wdown(KEY_LEFTALT);
        t.wdown(KEY_LEFTSHIFT);
        let fx = t.wdown(ESC);
        assert!(fx.contains(&Effect::ReleaseAll));
        assert!(fx.contains(&Effect::Notice(Notice::Emergency)));
        assert!(fx.contains(&Effect::Grab(false)));
    }

    #[test]
    fn hold_and_double_tap_from_a_watched_keyboard() {
        let mut t = T::new(
            vec![Trigger {
                kind: TriggerKind::Hold(Combo::parse("Ctrl+Alt").unwrap()),
                action: Action::On,
            }],
            false,
        );
        t.wdown(KEY_LEFTCTRL);
        assert!(t.wdown(KEY_LEFTALT).contains(&Effect::Grab(true)));
        assert!(t.wup(KEY_LEFTALT).contains(&Effect::Grab(false)));

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
        t.wtap(KEY_LEFTCTRL);
        t.wait(100);
        assert!(t.wtap(KEY_LEFTCTRL).contains(&Effect::Grab(true)));
    }

    #[test]
    fn release_all_keeps_watched_keys() {
        let mut t = T::new(
            vec![
                Trigger::combo("Ctrl+Alt+P", Action::Toggle).unwrap(),
                Trigger::combo("Ctrl+Alt+R", Action::ReleaseAll).unwrap(),
            ],
            false,
        );
        t.press_combo(&[KEY_LEFTCTRL, KEY_LEFTALT, P]);
        t.down(A);
        t.wdown(KEY_LEFTCTRL);
        t.wdown(KEY_LEFTALT);
        let fx = t.wdown(R);
        let forgot = fx.iter().find_map(|e| match e {
            Effect::Forget(c) => Some(c.clone()),
            _ => None,
        });
        assert_eq!(forgot, Some(vec![A]));
        // The watched Ctrl+Alt are still held: P on top toggles off.
        t.wup(R);
        assert!(t.wdown(P).contains(&Effect::Grab(false)));
    }

    #[test]
    fn command_on_off() {
        let mut t = T::new(vec![], false);
        assert!(t.e.command(Action::On).contains(&Effect::Grab(true)));
        assert!(t.e.command(Action::Off).contains(&Effect::Grab(false)));
    }

    const CAPSLOCK: u16 = 58;
    const F: u16 = 33;
    const BTN_SIDE: u16 = crate::keymap::BTN_SIDE;

    fn profile(name: &str, window: &str, remap: &[(u16, Option<u16>)]) -> EngineProfile {
        EngineProfile {
            name: name.into(),
            window: window.into(),
            mouse: MouseCurve::default(),
            remap: remap.iter().copied().collect(),
        }
    }

    /// Bridge on (by command), with these profiles and triggers.
    fn with_profiles(triggers: Vec<Trigger>, profiles: Vec<EngineProfile>) -> T {
        let mut t = T {
            e: Engine::new(EngineConfig {
                triggers,
                window_rule: false,
                profiles,
                ..Default::default()
            }),
            now: Instant::now(),
        };
        assert!(t.e.command(Action::On).contains(&Effect::Grab(true)));
        t
    }

    fn notices(fx: &[Effect]) -> Vec<Notice> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Notice(n) => Some(*n),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn remapped_keys_and_buttons() {
        let mut t = with_profiles(
            vec![],
            vec![profile(
                "Standard",
                "",
                &[
                    (CAPSLOCK, Some(KEY_LEFTCTRL)),
                    (BTN_SIDE, Some(F)),
                    (W, None),
                    (A, Some(BTN_LEFT)),
                ],
            )],
        );
        assert_eq!(states(&t.down(CAPSLOCK))[0].mods, 0x01);
        t.up(CAPSLOCK);
        let s = states(&t.down(BTN_SIDE))[0];
        assert_eq!(
            (s.keys[0], s.buttons),
            (0x09, 0),
            "Mouse4 sends F, not the side button"
        );
        t.up(BTN_SIDE);
        // Switched off: nothing changes on the PlayStation.
        assert!(states(&t.down(W)).is_empty());
        assert_eq!(states(&t.down(A))[0].buttons, 0x01, "a key can click");
    }

    #[test]
    fn two_keys_on_one_target_count_once() {
        let mut t = with_profiles(vec![], vec![profile("Standard", "", &[(R, Some(F))])]);
        t.down(F);
        let fx = t.down(R);
        assert!(states(&fx).is_empty(), "F is already held");
        let s = states(&t.up(F));
        assert!(s.is_empty(), "R still holds F");
        assert_eq!(states(&t.up(R))[0].keys, [0; 6]);
    }

    #[test]
    fn triggers_and_emergency_see_the_physical_keys() {
        let mut t = with_profiles(
            vec![Trigger::combo("Mouse4", Action::Toggle).unwrap()],
            vec![profile("Standard", "", &[(BTN_SIDE, Some(F)), (ESC, Some(A))])],
        );
        assert!(t.down(BTN_SIDE).contains(&Effect::Grab(false)));
        t.up(BTN_SIDE);
        t.down(BTN_SIDE);
        // (On waits until everything is released.)
        assert!(t.up(BTN_SIDE).contains(&Effect::Grab(true)));
        t.down(KEY_LEFTCTRL);
        t.down(KEY_LEFTALT);
        t.down(KEY_LEFTSHIFT);
        let fx = t.down(ESC);
        assert!(notices(&fx).contains(&Notice::Emergency));
        assert!(fx.contains(&Effect::Grab(false)));
    }

    #[test]
    fn a_held_key_keeps_its_target_across_a_profile_switch() {
        let mut t = with_profiles(
            vec![Trigger::combo("F9", Action::NextProfile).unwrap()],
            vec![
                profile("Standard", "", &[]),
                profile("Shooter", "", &[(A, Some(F))]),
            ],
        );
        assert_eq!(states(&t.down(A))[0].keys[0], 0x04);
        let fx = t.tap(67); // F9
        assert_eq!(notices(&fx), vec![Notice::ProfileChanged]);
        assert_eq!(t.e.profile(), 1);
        // A is still held as A; released, then pressed again it is F.
        assert!(states(&t.up(A))[0].keys == [0; 6]);
        assert_eq!(states(&t.down(A))[0].keys[0], 0x09);
        // ... and stays F after switching back until released.
        t.tap(67);
        assert_eq!(t.e.profile(), 0);
        assert_eq!(states(&t.up(A))[0].keys, [0; 6]);
        assert_eq!(states(&t.down(A))[0].keys[0], 0x04);
    }

    #[test]
    fn profiles_follow_the_window() {
        let mut t = with_profiles(
            vec![Trigger::combo("F9", Action::Profile(0)).unwrap()],
            vec![
                profile("Standard", "", &[]),
                profile("Stream", "chiaki", &[(A, Some(F))]),
            ],
        );
        let chiaki = WindowInfo::new("Stream", "chiaki-ng", "");
        let other = WindowInfo::new("Terminal", "foot", "");
        let fx = t.e.window(chiaki.as_ref(), t.now);
        assert_eq!(notices(&fx), vec![Notice::ProfileChanged]);
        assert_eq!((t.e.profile(), t.e.manual_profile()), (1, 0));
        assert_eq!(states(&t.down(A))[0].keys[0], 0x09);
        t.up(A);
        // Chosen by hand: wins over the window until the focus moves.
        t.tap(67);
        assert_eq!(t.e.profile(), 0);
        t.e.window(chiaki.as_ref(), t.now);
        assert_eq!(
            t.e.profile(),
            0,
            "the same window again: the choice by hand stays"
        );
        t.e.window(other.as_ref(), t.now);
        assert_eq!(t.e.profile(), 0);
        t.e.window(chiaki.as_ref(), t.now);
        assert_eq!(t.e.profile(), 1);
        t.e.window(None, t.now);
        assert_eq!(t.e.profile(), 0);
    }

    #[test]
    fn window_rule_by_window() {
        let mut t = T {
            e: Engine::new(EngineConfig {
                window_rule: true,
                window_pattern: "chiaki".into(),
                ..Default::default()
            }),
            now: Instant::now(),
        };
        let fx =
            t.e.window(WindowInfo::new("Stream", "chiaki-ng", "").as_ref(), t.now);
        assert!(fx.contains(&Effect::Grab(true)));
        let fx = t.e.window(WindowInfo::new("x", "firefox", "").as_ref(), t.now);
        assert!(fx.contains(&Effect::Grab(false)));
    }

    #[test]
    fn profile_mouse_settings_apply() {
        let mut shooter = profile("Shooter", "", &[]);
        shooter.mouse.invert_y = true;
        shooter.mouse.wheel = 2.0;
        let mut t = with_profiles(vec![], vec![profile("Standard", "", &[]), shooter]);
        let motion = |t: &mut T| {
            t.ev(Event::Rel {
                axis: REL_Y,
                value: 3,
            });
            t.ev(Event::Rel {
                axis: REL_WHEEL,
                value: 1,
            });
            t.ev(Event::Sync)
        };
        assert_eq!(motion(&mut t), vec![Effect::Motion(0, 3, 1, 0)]);
        t.e.command(Action::Profile(1));
        assert_eq!(motion(&mut t), vec![Effect::Motion(0, -3, 2, 0)]);
        // New settings for the same profiles keep the one in use.
        let mut slower = profile("Shooter", "", &[]);
        slower.mouse.wheel = 0.5;
        t.e.set_profiles(vec![profile("Standard", "", &[]), slower]);
        assert_eq!(t.e.profile(), 1);
        assert_eq!(motion(&mut t), vec![Effect::Motion(0, 3, 0, 0)]);
    }
}
