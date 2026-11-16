//! Touchpads and touchscreens as a mouse.
//!
//! The PlayStation knows a relative mouse only, so finger contacts (absolute
//! positions, mostly multitouch slots) become what a mouse sends:
//!
//! * one finger moving: motion, converted from millimetres on the surface
//!   to counts of a 1000-dpi mouse, with an acceleration of its own (a
//!   touchpad is small: slow is precise, fast covers distance);
//! * two fingers moving: wheel steps (one per [`SCROLL_MM`]), locked to the
//!   main axis;
//! * pressing a clickpad (the whole surface is the button): left; with two
//!   fingers on it right, with three middle; a finger pressing in the
//!   bottom-right corner also right. Pads with real buttons pass them on;
//! * a short tap without moving: a click (one finger left, two right, three
//!   middle), not for touches that begin right after a key press (a palm
//!   while typing);
//! * a finger that touches down in the bottom strip of a clickpad (a thumb
//!   on the button) is "resting": it neither moves nor scrolls, so the thumb
//!   can click while another finger moves.
//!
//! A touchscreen behaves the same way (the PlayStation has no pointer that
//! could jump to a position). Times are microseconds on the monotonic clock,
//! as the kernel stamps the events.

use evdev::{EventType, InputEvent};

use crate::engine::Event;

// linux/input-event-codes.h
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TOOL_TYPE: u16 = 0x37;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const MT_TOOL_PALM: i32 = 2;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_QUINTTAP: u16 = 0x148;
const BTN_TOUCH: u16 = 0x14a;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
const BTN_TOOL_QUADTAP: u16 = 0x14f;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;

/// Counts per millimetre of a 1000-dpi mouse.
const COUNTS_PER_MM: f64 = 1000.0 / 25.4;
/// Acceleration: this gain up to SLOW (mm/s), rising to FAST_GAIN at FAST.
const SLOW: f64 = 20.0;
const SLOW_GAIN: f64 = 0.5;
const FAST: f64 = 150.0;
const FAST_GAIN: f64 = 2.5;
/// Two-finger travel per wheel step.
pub const SCROLL_MM: f64 = 4.0;
/// Scrolling locks to one axis once it leads by this factor (and has gone
/// this far).
const SCROLL_LOCK_RATIO: f64 = 2.0;
const SCROLL_LOCK_MM: f64 = 1.0;
/// A tap: all fingers down and up again within this time ...
const TAP_US: u64 = 180_000;
/// ... each moving less than this (a finger on a screen wobbles more).
const TAP_MOVE_MM: f64 = 1.5;
const TAP_MOVE_MM_SCREEN: f64 = 3.0;
/// How long a tap's click is held: the board sends button states and the
/// PlayStation polls every millisecond, a press and release in the same
/// packet would never reach it.
pub const TAP_HOLD_US: u64 = 30_000;
/// No tap from touches that begin this soon after a key press.
const TYPING_US: u64 = 300_000;
/// The bottom part of a clickpad where a finger counts as resting on the
/// button (fraction of the height).
const BUTTON_AREA: f64 = 0.15;

/// The user's settings (`[touchpad]` in the configuration).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchSettings {
    /// Factor on the motion (1.0 = as designed).
    pub speed: f64,
    /// A short tap clicks.
    pub tap: bool,
    /// The content follows the fingers (wheel steps reversed).
    pub natural_scroll: bool,
}

impl Default for TouchSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            tap: true,
            natural_scroll: false,
        }
    }
}

/// One axis of the surface: range and units per millimetre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub min: i32,
    pub max: i32,
    pub per_mm: f64,
}

/// The device as the touch handling needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: Axis,
    pub y: Axis,
    /// Multitouch slots; 0 for a single-touch device (ABS_X/ABS_Y and the
    /// BTN_TOOL_* keys for the number of fingers).
    pub slots: usize,
    /// Clickpad: the surface itself is the button (INPUT_PROP_BUTTONPAD).
    pub buttonpad: bool,
    /// Touchscreen (INPUT_PROP_DIRECT).
    pub direct: bool,
}

impl Geometry {
    /// From the axes as (minimum, maximum, resolution in units per mm; 0 =
    /// unknown). Without a resolution the width counts as about 100 mm (a
    /// touchpad) or 300 mm (a touchscreen), with square units.
    pub fn new(x: (i32, i32, i32), y: (i32, i32, i32), slots: usize, buttonpad: bool, direct: bool) -> Self {
        let width_mm = if direct { 300.0 } else { 100.0 };
        let per_mm_x = if x.2 > 0 {
            x.2 as f64
        } else {
            (x.1 - x.0).max(1) as f64 / width_mm
        };
        let per_mm_y = if y.2 > 0 { y.2 as f64 } else { per_mm_x };
        Self {
            x: Axis {
                min: x.0,
                max: x.1,
                per_mm: per_mm_x,
            },
            y: Axis {
                min: y.0,
                max: y.1,
                per_mm: per_mm_y,
            },
            slots,
            buttonpad,
            direct,
        }
    }

    fn in_button_area(&self, y: i32) -> bool {
        self.buttonpad
            && f64::from(y) >= f64::from(self.y.max) - BUTTON_AREA * f64::from(self.y.max - self.y.min)
    }

    fn right_half(&self, x: i32) -> bool {
        2 * i64::from(x) >= i64::from(self.x.min) + i64::from(self.x.max)
    }
}

/// A contact as the kernel reports it (also for a resync, see
/// [`Touch::reset`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotState {
    /// Tracking id; negative: no contact.
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub palm: bool,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    now: SlotState,
    /// At the end of the previous frame.
    last: SlotState,
    start_x: i32,
    start_y: i32,
    /// Began in the button area of a clickpad (a thumb on the button).
    resting: bool,
}

impl Default for Slot {
    fn default() -> Self {
        let none = SlotState {
            id: -1,
            ..Default::default()
        };
        Self {
            now: none,
            last: none,
            start_x: 0,
            start_y: 0,
            resting: false,
        }
    }
}

impl Slot {
    fn active(&self) -> bool {
        self.now.id >= 0 && !self.now.palm
    }
    /// The same contact as at the end of the previous frame.
    fn continues(&self) -> bool {
        self.active() && self.last.id == self.now.id && !self.last.palm
    }
}

/// From the first finger down to the last one up: a tap if it was short
/// and nothing moved or clicked.
#[derive(Clone, Copy, Debug)]
struct Sequence {
    start_us: u64,
    fingers: usize,
    moved: bool,
    clicked: bool,
    typing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScrollAxis {
    Vertical,
    Horizontal,
}

pub struct Touch {
    geo: Geometry,
    settings: TouchSettings,
    slots: Vec<Slot>,
    /// The slot the next ABS_MT_* events are for (None: beyond our slots).
    slot: Option<usize>,
    /// Single touch: a contact (BTN_TOUCH), and the id given to it.
    touching: bool,
    next_id: i32,
    /// Fingers the BTN_TOOL_* keys report (devices with fewer slots than
    /// fingers count them this way).
    tool_fingers: usize,
    /// Fingers that counted (not resting) at the end of the last frame.
    last_counting: usize,
    last_frame_us: Option<u64>,
    /// Fractions of counts and of a wheel step not sent yet.
    rem: (f64, f64),
    scroll: (f64, f64),
    scroll_axis: Option<ScrollAxis>,
    /// The physical button of a clickpad changed in this frame.
    pad_button: Option<bool>,
    /// The button a clickpad press sent (held until it is let go).
    click: Option<u16>,
    /// Real buttons of a pad that has them: changes of this frame, and the
    /// ones held.
    buttons: Vec<(u16, bool)>,
    held: Vec<u16>,
    seq: Option<Sequence>,
    /// A tap's click and when it is let go.
    tap_held: Option<(u16, u64)>,
    /// The last key press on a keyboard (for taps while typing).
    typed_us: Option<u64>,
}

impl Touch {
    pub fn new(geo: Geometry, settings: TouchSettings) -> Self {
        Self {
            geo,
            settings,
            slots: vec![Slot::default(); geo.slots.max(1)],
            slot: Some(0),
            touching: false,
            next_id: 0,
            tool_fingers: 0,
            last_counting: 0,
            last_frame_us: None,
            rem: (0.0, 0.0),
            scroll: (0.0, 0.0),
            scroll_axis: None,
            pad_button: None,
            click: None,
            buttons: Vec::new(),
            held: Vec::new(),
            seq: None,
            tap_held: None,
            typed_us: None,
        }
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geo
    }

    pub fn set_settings(&mut self, settings: TouchSettings) {
        self.settings = settings;
    }

    /// A key was pressed on a keyboard: touches that begin soon after are no
    /// taps (a palm resting on the touchpad while typing).
    pub fn note_typing(&mut self, now_us: u64) {
        self.typed_us = Some(now_us);
    }

    /// Does this event belong to the touch handling? (The others, e.g. the
    /// keys of a keyboard built into the same device, go their usual way.)
    pub fn handles(ev: &InputEvent) -> bool {
        match ev.event_type() {
            EventType::ABSOLUTE => true,
            EventType::KEY => matches!(
                ev.code(),
                BTN_LEFT | BTN_RIGHT | BTN_MIDDLE | BTN_TOUCH | BTN_TOOL_FINGER..=BTN_TOOL_QUADTAP
            ),
            _ => false,
        }
    }

    /// One event of the current frame (not SYN_REPORT, see [`frame`](Self::frame)).
    pub fn feed(&mut self, ev: &InputEvent) {
        let (code, value) = (ev.code(), ev.value());
        match ev.event_type() {
            EventType::ABSOLUTE => self.feed_abs(code, value),
            EventType::KEY if value == 0 || value == 1 => self.feed_key(code, value == 1),
            _ => {}
        }
    }

    fn feed_abs(&mut self, code: u16, value: i32) {
        let single = self.geo.slots == 0;
        match code {
            ABS_MT_SLOT if !single => {
                self.slot = usize::try_from(value).ok().filter(|&s| s < self.slots.len());
            }
            ABS_MT_TRACKING_ID | ABS_MT_POSITION_X | ABS_MT_POSITION_Y | ABS_MT_TOOL_TYPE if !single => {
                let Some(s) = self.slot.map(|i| &mut self.slots[i].now) else {
                    return;
                };
                match code {
                    ABS_MT_TRACKING_ID => s.id = value,
                    ABS_MT_POSITION_X => s.x = value,
                    ABS_MT_POSITION_Y => s.y = value,
                    _ => s.palm = value == MT_TOOL_PALM,
                }
            }
            ABS_X if single => self.slots[0].now.x = value,
            ABS_Y if single => self.slots[0].now.y = value,
            _ => {}
        }
    }

    fn feed_key(&mut self, code: u16, down: bool) {
        match code {
            BTN_TOUCH if self.geo.slots == 0 => {
                if down && !self.touching {
                    self.next_id = self.next_id.wrapping_add(1) & 0xffff;
                    self.slots[0].now.id = self.next_id;
                } else if !down {
                    self.slots[0].now.id = -1;
                }
                self.touching = down;
            }
            BTN_TOOL_FINGER | BTN_TOOL_DOUBLETAP | BTN_TOOL_TRIPLETAP | BTN_TOOL_QUADTAP
            | BTN_TOOL_QUINTTAP => {
                let n = match code {
                    BTN_TOOL_FINGER => 1,
                    BTN_TOOL_DOUBLETAP => 2,
                    BTN_TOOL_TRIPLETAP => 3,
                    BTN_TOOL_QUADTAP => 4,
                    _ => 5,
                };
                if down {
                    self.tool_fingers = n;
                } else if self.tool_fingers == n {
                    self.tool_fingers = 0;
                }
            }
            BTN_LEFT if self.geo.buttonpad => self.pad_button = Some(down),
            BTN_LEFT | BTN_RIGHT | BTN_MIDDLE => self.buttons.push((code, down)),
            _ => {}
        }
    }

    /// The frame is complete (SYN_REPORT at `now_us`): what it means for the
    /// mouse, ending with [`Event::Sync`] if anything.
    pub fn frame(&mut self, now_us: u64) -> Vec<Event> {
        let mut out = Vec::new();
        let geo = self.geo;

        // New contacts, and resting thumbs that moved up out of the button
        // area (from then on they count).
        for s in &mut self.slots {
            if s.active() && !s.continues() {
                s.start_x = s.now.x;
                s.start_y = s.now.y;
                s.resting = geo.in_button_area(s.now.y);
            } else if s.active() && s.resting && !geo.in_button_area(s.now.y) {
                s.resting = false;
                s.last.palm = true; // no motion from the jump out of the area
            }
        }
        let active = self.slots.iter().filter(|s| s.active()).count();
        let resting = self.slots.iter().filter(|s| s.active() && s.resting).count();
        let mut counting = active - resting;
        // More fingers than slots (BTN_TOOL_*; a single-touch device has
        // one): the extra ones count too.
        if self.tool_fingers > active {
            counting += self.tool_fingers - active;
        }

        // Motion and scrolling, only between frames with the same fingers.
        let moving: Vec<Slot> = self
            .slots
            .iter()
            .filter(|s| s.active() && !s.resting)
            .copied()
            .collect();
        if counting == 1 && self.last_counting == 1 && moving.len() == 1 && moving[0].continues() {
            let s = moving[0];
            let (dx, dy) = (s.now.x - s.last.x, s.now.y - s.last.y);
            let (mx, my) = self.motion(dx, dy, now_us);
            if mx != 0 {
                out.push(Event::Rel {
                    axis: REL_X,
                    value: mx,
                });
            }
            if my != 0 {
                out.push(Event::Rel {
                    axis: REL_Y,
                    value: my,
                });
            }
        }
        if counting == 2
            && self.last_counting == 2
            && moving.len() == 2
            && moving.iter().all(|s| s.continues())
        {
            let dx = moving.iter().map(|s| f64::from(s.now.x - s.last.x)).sum::<f64>() / 2.0;
            let dy = moving.iter().map(|s| f64::from(s.now.y - s.last.y)).sum::<f64>() / 2.0;
            let (wheel, pan) = self.scroll_steps(dx / geo.x.per_mm, dy / geo.y.per_mm);
            if wheel != 0 {
                out.push(Event::Rel {
                    axis: REL_WHEEL,
                    value: wheel,
                });
            }
            if pan != 0 {
                out.push(Event::Rel {
                    axis: REL_HWHEEL,
                    value: pan,
                });
            }
        } else if counting != 2 {
            self.scroll = (0.0, 0.0);
            self.scroll_axis = None;
        }
        if counting != 1 {
            self.rem = (0.0, 0.0);
        }

        // Buttons.
        let mut clicked = !self.buttons.is_empty();
        if let Some(down) = self.pad_button.take() {
            clicked = true;
            if down && self.click.is_none() {
                let button = self.pad_click_button(counting);
                self.click = Some(button);
                out.push(Event::Key {
                    code: button,
                    down: true,
                });
            } else if !down && let Some(button) = self.click.take() {
                out.push(Event::Key {
                    code: button,
                    down: false,
                });
            }
        }
        for (code, down) in std::mem::take(&mut self.buttons) {
            if down == self.held.contains(&code) {
                continue;
            }
            if down {
                self.held.push(code);
            } else {
                self.held.retain(|c| *c != code);
            }
            out.push(Event::Key { code, down });
        }

        // Tapping.
        let tap_fingers = if counting > 0 { counting } else { resting };
        if self.seq.is_none() && tap_fingers > 0 {
            self.seq = Some(Sequence {
                start_us: now_us,
                fingers: 0,
                moved: false,
                clicked: false,
                typing: self
                    .typed_us
                    .is_some_and(|t| now_us.saturating_sub(t) < TYPING_US),
            });
        }
        let limit = if geo.direct {
            TAP_MOVE_MM_SCREEN
        } else {
            TAP_MOVE_MM
        };
        let moved = self.slots.iter().any(|s| {
            s.active()
                && (f64::from(s.now.x - s.start_x) / geo.x.per_mm)
                    .hypot(f64::from(s.now.y - s.start_y) / geo.y.per_mm)
                    > limit
        });
        if let Some(seq) = &mut self.seq {
            seq.fingers = seq.fingers.max(tap_fingers);
            seq.moved |= moved;
            seq.clicked |= clicked || self.click.is_some();
        }
        if active == 0
            && self.tool_fingers == 0
            && let Some(seq) = self.seq.take()
            && self.settings.tap
            && !seq.moved
            && !seq.clicked
            && !seq.typing
            && now_us.saturating_sub(seq.start_us) <= TAP_US
            && (1..=3).contains(&seq.fingers)
        {
            let button = [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE][seq.fingers - 1];
            if let Some((held, _)) = self.tap_held.take() {
                out.push(Event::Key {
                    code: held,
                    down: false,
                });
            }
            out.push(Event::Key {
                code: button,
                down: true,
            });
            self.tap_held = Some((button, now_us + TAP_HOLD_US));
        }
        if active == 0 && self.tool_fingers == 0 {
            self.seq = None;
        }

        for s in &mut self.slots {
            s.last = s.now;
        }
        self.last_counting = counting;
        self.last_frame_us = Some(now_us);
        if !out.is_empty() {
            out.push(Event::Sync);
        }
        out
    }

    /// Clickpad pressed: which button. Three fingers or more: middle, two:
    /// right; else the lowest finger in the bottom-right corner: right.
    fn pad_click_button(&self, counting: usize) -> u16 {
        if counting >= 3 {
            return BTN_MIDDLE;
        }
        if counting == 2 {
            return BTN_RIGHT;
        }
        let lowest = self.slots.iter().filter(|s| s.active()).max_by_key(|s| s.now.y);
        match lowest {
            Some(s) if self.geo.in_button_area(s.now.y) && self.geo.right_half(s.now.x) => BTN_RIGHT,
            _ => BTN_LEFT,
        }
    }

    /// Device units of one finger's move to mouse counts.
    fn motion(&mut self, dx: i32, dy: i32, now_us: u64) -> (i32, i32) {
        let mm = (
            f64::from(dx) / self.geo.x.per_mm,
            f64::from(dy) / self.geo.y.per_mm,
        );
        let dt_us = self
            .last_frame_us
            .map_or(10_000, |t| now_us.saturating_sub(t))
            .clamp(1_000, 50_000);
        let speed = mm.0.hypot(mm.1) / (dt_us as f64 / 1e6);
        let t = ((speed - SLOW) / (FAST - SLOW)).clamp(0.0, 1.0);
        let gain = (SLOW_GAIN + t * (FAST_GAIN - SLOW_GAIN)) * COUNTS_PER_MM * self.settings.speed;
        let fx = mm.0 * gain + self.rem.0;
        let fy = mm.1 * gain + self.rem.1;
        let (ox, oy) = (fx.trunc(), fy.trunc());
        self.rem = (fx - ox, fy - oy);
        (ox as i32, oy as i32)
    }

    /// Two fingers moved by (dx, dy) mm: wheel and pan steps. Fingers moving
    /// down scroll down (wheel negative), unless natural scrolling.
    fn scroll_steps(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        self.scroll.0 += dx;
        self.scroll.1 += dy;
        let (sx, sy) = self.scroll;
        if self.scroll_axis.is_none() {
            if sy.abs() > SCROLL_LOCK_MM && sy.abs() >= SCROLL_LOCK_RATIO * sx.abs() {
                self.scroll_axis = Some(ScrollAxis::Vertical);
            } else if sx.abs() > SCROLL_LOCK_MM && sx.abs() >= SCROLL_LOCK_RATIO * sy.abs() {
                self.scroll_axis = Some(ScrollAxis::Horizontal);
            }
        }
        match self.scroll_axis {
            Some(ScrollAxis::Vertical) => self.scroll.0 = 0.0,
            Some(ScrollAxis::Horizontal) => self.scroll.1 = 0.0,
            None => return (0, 0),
        }
        let steps = |v: &mut f64| {
            let n = (*v / SCROLL_MM).trunc();
            *v -= n * SCROLL_MM;
            n as i32
        };
        let down = steps(&mut self.scroll.1);
        let right = steps(&mut self.scroll.0);
        let sign = if self.settings.natural_scroll { -1 } else { 1 };
        (-down * sign, right * sign)
    }

    /// When [`tick`](Self::tick) has something to do (µs), if ever.
    pub fn deadline(&self) -> Option<u64> {
        self.tap_held.map(|(_, at)| at)
    }

    /// Let a tap's click go once its time is up.
    pub fn tick(&mut self, now_us: u64) -> Vec<Event> {
        match self.tap_held {
            Some((button, at)) if now_us >= at => {
                self.tap_held = None;
                vec![
                    Event::Key {
                        code: button,
                        down: false,
                    },
                    Event::Sync,
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Events were lost (SYN_DROPPED): start over from the kernel's state
    /// (`slots` and the current slot, None if unknown: no contact). The next
    /// frame moves nothing (no jump), a running tap is dropped. Buttons the
    /// kernel no longer holds (`held`: its key state) are let go.
    pub fn reset(&mut self, slots: Option<(Vec<SlotState>, usize)>, held: &[u16]) -> Vec<Event> {
        let fresh = |st: SlotState, geo: &Geometry| Slot {
            now: st,
            last: st,
            start_x: st.x,
            start_y: st.y,
            resting: geo.in_button_area(st.y),
        };
        match slots {
            Some((states, current)) => {
                for (i, s) in self.slots.iter_mut().enumerate() {
                    let st = states.get(i).copied().unwrap_or(SlotState {
                        id: -1,
                        ..Default::default()
                    });
                    *s = fresh(st, &self.geo);
                }
                self.slot = Some(current).filter(|&c| c < self.slots.len());
                if self.geo.slots == 0 {
                    self.touching = self.slots[0].now.id >= 0;
                }
            }
            None => {
                self.slots.iter_mut().for_each(|s| *s = Slot::default());
                self.touching = false;
            }
        }
        self.tool_fingers = 0;
        self.seq = None;
        self.scroll = (0.0, 0.0);
        self.scroll_axis = None;
        self.rem = (0.0, 0.0);
        self.last_counting = self.slots.iter().filter(|s| s.active() && !s.resting).count();
        self.pad_button = None;
        self.buttons.clear();
        let mut out = Vec::new();
        if let Some(button) = self.click
            && !held.contains(&BTN_LEFT)
        {
            self.click = None;
            out.push(Event::Key {
                code: button,
                down: false,
            });
        }
        let (gone, kept): (Vec<u16>, Vec<u16>) = self.held.iter().partition(|c| !held.contains(c));
        self.held = kept;
        out.extend(gone.into_iter().map(|code| Event::Key { code, down: false }));
        if !out.is_empty() {
            out.push(Event::Sync);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1000;

    fn abs(code: u16, value: i32) -> InputEvent {
        InputEvent::new(EventType::ABSOLUTE.0, code, value)
    }
    fn key(code: u16, down: bool) -> InputEvent {
        InputEvent::new(EventType::KEY.0, code, i32::from(down))
    }

    /// A clickpad 100 × 60 mm, 10 units per mm, 5 slots.
    fn pad() -> Touch {
        Touch::new(
            Geometry::new((0, 1000, 10), (0, 600, 10), 5, true, false),
            TouchSettings::default(),
        )
    }

    /// Test driver: frames at given times, collecting what comes out.
    struct T {
        t: Touch,
        now: u64,
    }

    impl T {
        fn new(t: Touch) -> Self {
            Self { t, now: 1_000_000 }
        }
        /// One frame `dt_ms` after the previous one.
        fn frame(&mut self, dt_ms: u64, events: &[InputEvent]) -> Vec<Event> {
            self.now += dt_ms * MS;
            for e in events {
                self.t.feed(e);
            }
            self.t.frame(self.now)
        }
        fn down(slot: i32, id: i32, x: i32, y: i32) -> Vec<InputEvent> {
            vec![
                abs(ABS_MT_SLOT, slot),
                abs(ABS_MT_TRACKING_ID, id),
                abs(ABS_MT_POSITION_X, x),
                abs(ABS_MT_POSITION_Y, y),
            ]
        }
        fn mv(slot: i32, x: i32, y: i32) -> Vec<InputEvent> {
            vec![
                abs(ABS_MT_SLOT, slot),
                abs(ABS_MT_POSITION_X, x),
                abs(ABS_MT_POSITION_Y, y),
            ]
        }
        fn up(slot: i32) -> Vec<InputEvent> {
            vec![abs(ABS_MT_SLOT, slot), abs(ABS_MT_TRACKING_ID, -1)]
        }
    }

    fn cat(parts: &[Vec<InputEvent>]) -> Vec<InputEvent> {
        parts.concat()
    }

    fn rel(events: &[Event], axis: u16) -> i32 {
        events
            .iter()
            .map(|e| match e {
                Event::Rel { axis: a, value } if *a == axis => *value,
                _ => 0,
            })
            .sum()
    }

    fn keys(events: &[Event]) -> Vec<(u16, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Key { code, down } => Some((*code, *down)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn one_finger_moves_without_a_jump_on_touch_down() {
        let mut t = T::new(pad());
        let first = t.frame(
            10,
            &cat(&[
                T::down(0, 1, 500, 300),
                vec![key(BTN_TOUCH, true), key(BTN_TOOL_FINGER, true)],
            ]),
        );
        assert!(first.is_empty(), "touching down moves nothing: {first:?}");
        // 1 mm per 10 ms = 100 mm/s: between slow and fast.
        let out = t.frame(10, &T::mv(0, 510, 300));
        let dx = rel(&out, REL_X);
        let gain = SLOW_GAIN + (100.0 - SLOW) / (FAST - SLOW) * (FAST_GAIN - SLOW_GAIN);
        assert_eq!(dx, (COUNTS_PER_MM * gain) as i32);
        assert_eq!(rel(&out, REL_Y), 0);
        assert_eq!(out.last(), Some(&Event::Sync));
    }

    #[test]
    fn slow_is_precise_and_fractions_carry_over() {
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        // 0.1 mm per 10 ms (10 mm/s): 0.1 * 39.37 * 0.5 ≈ 1.97 counts each.
        let total: i32 = (0..10)
            .map(|i| rel(&t.frame(10, &T::mv(0, 501 + i, 300)), REL_X))
            .sum();
        assert_eq!(total, (1.0 * COUNTS_PER_MM * SLOW_GAIN) as i32);
    }

    #[test]
    fn speed_setting_scales_motion() {
        let mut t = T::new(pad());
        t.t.set_settings(TouchSettings {
            speed: 2.0,
            ..Default::default()
        });
        t.frame(10, &T::down(0, 1, 500, 300));
        let out = t.frame(10, &T::mv(0, 500, 301));
        assert_eq!(rel(&out, REL_Y), (0.1 * COUNTS_PER_MM * SLOW_GAIN * 2.0) as i32);
    }

    #[test]
    fn a_second_finger_or_a_new_contact_does_not_jump() {
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 100, 100));
        // The first finger lifts and another one lands far away.
        let out = t.frame(10, &cat(&[T::up(0), T::down(1, 2, 900, 500)]));
        assert_eq!(rel(&out, REL_X), 0);
        let out = t.frame(10, &T::mv(1, 890, 500));
        assert!(rel(&out, REL_X) < 0, "moves from where it landed");
        // A second finger joins: no motion while two fingers are on it.
        let out = t.frame(10, &T::down(0, 3, 200, 200));
        assert!(out.is_empty(), "{out:?}");
        // Back to one finger: no jump from the change either.
        let out = t.frame(10, &T::up(0));
        assert_eq!(rel(&out, REL_X), 0);
    }

    #[test]
    fn two_fingers_scroll_by_axis() {
        let mut t = T::new(pad());
        t.frame(10, &cat(&[T::down(0, 1, 400, 200), T::down(1, 2, 600, 200)]));
        // Both fingers down by 9 mm (and a little sideways): two steps down.
        let mut out = Vec::new();
        for i in 1..=9 {
            out.extend(t.frame(
                10,
                &cat(&[T::mv(0, 400 + i, 200 + 10 * i), T::mv(1, 600 + i, 200 + 10 * i)]),
            ));
        }
        assert_eq!(rel(&out, REL_WHEEL), -2);
        assert_eq!(rel(&out, REL_HWHEEL), 0, "locked to the vertical axis");
        assert_eq!(rel(&out, REL_X), 0);
    }

    #[test]
    fn natural_scrolling_reverses_the_steps() {
        let mut t = T::new(pad());
        t.t.set_settings(TouchSettings {
            natural_scroll: true,
            ..Default::default()
        });
        t.frame(10, &cat(&[T::down(0, 1, 400, 300), T::down(1, 2, 600, 300)]));
        let mut out = Vec::new();
        for i in 1..=5 {
            out.extend(t.frame(
                10,
                &cat(&[T::mv(0, 400 - 10 * i, 300), T::mv(1, 600 - 10 * i, 300)]),
            ));
        }
        // Fingers 5 mm to the left: one step, to the right with natural scrolling.
        assert_eq!(rel(&out, REL_HWHEEL), 1);
        assert_eq!(rel(&out, REL_WHEEL), 0);
    }

    #[test]
    fn clickpad_buttons_by_fingers_and_corner() {
        // One finger in the middle: left, held until let go.
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        let out = t.frame(10, &[key(BTN_LEFT, true)]);
        assert_eq!(keys(&out), [(BTN_LEFT, true)]);
        assert!(
            t.frame(10, &T::mv(0, 520, 300))
                .iter()
                .all(|e| !matches!(e, Event::Key { .. }))
        );
        assert_eq!(keys(&t.frame(10, &[key(BTN_LEFT, false)])), [(BTN_LEFT, false)]);

        // Two fingers: right.
        let mut t = T::new(pad());
        t.frame(10, &cat(&[T::down(0, 1, 400, 300), T::down(1, 2, 600, 300)]));
        assert_eq!(keys(&t.frame(10, &[key(BTN_LEFT, true)])), [(BTN_RIGHT, true)]);
        assert_eq!(keys(&t.frame(10, &[key(BTN_LEFT, false)])), [(BTN_RIGHT, false)]);

        // Three fingers: middle.
        let mut t = T::new(pad());
        t.frame(
            10,
            &cat(&[
                T::down(0, 1, 300, 300),
                T::down(1, 2, 500, 300),
                T::down(2, 3, 700, 300),
            ]),
        );
        assert_eq!(keys(&t.frame(10, &[key(BTN_LEFT, true)])), [(BTN_MIDDLE, true)]);

        // One finger pressing in the bottom-right corner: right.
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 800, 580));
        assert_eq!(keys(&t.frame(10, &[key(BTN_LEFT, true)])), [(BTN_RIGHT, true)]);
    }

    #[test]
    fn a_thumb_resting_on_the_button_neither_moves_nor_counts() {
        let mut t = T::new(pad());
        // Thumb on the bottom-left of the clickpad, index finger above it.
        t.frame(10, &T::down(0, 1, 200, 580));
        t.frame(10, &T::down(1, 2, 500, 300));
        // The thumb clicks while the index finger moves: left (not right for
        // two fingers), and the motion is the index finger's.
        let out = t.frame(
            10,
            &cat(&[vec![key(BTN_LEFT, true)], T::mv(1, 520, 300), T::mv(0, 205, 585)]),
        );
        assert_eq!(keys(&out), [(BTN_LEFT, true)]);
        assert!(rel(&out, REL_X) > 0);
        assert_eq!(rel(&out, REL_WHEEL), 0, "no scrolling with a resting thumb");
    }

    #[test]
    fn pads_with_real_buttons_pass_them_on() {
        let mut t = T::new(Touch::new(
            Geometry::new((0, 1000, 10), (0, 600, 10), 2, false, false),
            TouchSettings::default(),
        ));
        t.frame(10, &T::down(0, 1, 500, 300));
        assert_eq!(keys(&t.frame(10, &[key(BTN_RIGHT, true)])), [(BTN_RIGHT, true)]);
        assert_eq!(keys(&t.frame(10, &[key(BTN_RIGHT, false)])), [(BTN_RIGHT, false)]);
    }

    #[test]
    fn taps_click_and_are_let_go_later() {
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        let out = t.frame(50, &T::up(0));
        assert_eq!(keys(&out), [(BTN_LEFT, true)]);
        let deadline = t.t.deadline().expect("release planned");
        assert_eq!(deadline, t.now + TAP_HOLD_US);
        assert!(
            t.t.tick(deadline - 1).is_empty(),
            "held long enough for the PlayStation's poll"
        );
        assert_eq!(
            t.t.tick(deadline),
            vec![
                Event::Key {
                    code: BTN_LEFT,
                    down: false
                },
                Event::Sync
            ]
        );
        assert_eq!(t.t.deadline(), None);

        // Two fingers: right.
        t.frame(500, &cat(&[T::down(0, 2, 400, 300), T::down(1, 3, 600, 300)]));
        assert_eq!(
            keys(&t.frame(40, &cat(&[T::up(0), T::up(1)]))),
            [(BTN_RIGHT, true)]
        );
    }

    #[test]
    fn no_tap_after_moving_waiting_clicking_or_typing() {
        // Moved: motion, no click.
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        t.frame(10, &T::mv(0, 530, 300));
        assert!(keys(&t.frame(10, &T::up(0))).is_empty());
        // Too long.
        t.frame(500, &T::down(0, 2, 500, 300));
        assert!(keys(&t.frame(200, &T::up(0))).is_empty());
        // Right after a key press on the keyboard (a palm while typing).
        t.t.note_typing(t.now + 400 * MS);
        t.frame(500, &T::down(0, 3, 500, 300));
        assert!(keys(&t.frame(50, &T::up(0))).is_empty());
        // Typing long ago: a tap again.
        t.frame(1000, &T::down(0, 4, 500, 300));
        assert_eq!(keys(&t.frame(50, &T::up(0))), [(BTN_LEFT, true)]);
        // Switched off.
        let mut t = T::new(pad());
        t.t.set_settings(TouchSettings {
            tap: false,
            ..Default::default()
        });
        t.frame(10, &T::down(0, 1, 500, 300));
        assert!(keys(&t.frame(50, &T::up(0))).is_empty());
    }

    #[test]
    fn a_click_is_no_tap_too() {
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        t.frame(10, &[key(BTN_LEFT, true)]);
        t.frame(10, &[key(BTN_LEFT, false)]);
        assert!(keys(&t.frame(10, &T::up(0))).is_empty());
    }

    #[test]
    fn palms_are_ignored() {
        let mut t = T::new(pad());
        t.frame(
            10,
            &cat(&[T::down(0, 1, 500, 300), vec![abs(ABS_MT_TOOL_TYPE, MT_TOOL_PALM)]]),
        );
        assert!(t.frame(10, &T::mv(0, 600, 300)).is_empty());
        assert!(t.frame(10, &T::up(0)).is_empty(), "no tap either");
    }

    #[test]
    fn lost_events_start_over_without_a_jump() {
        let mut t = T::new(pad());
        t.frame(10, &T::down(0, 1, 500, 300));
        t.frame(10, &[key(BTN_LEFT, true)]);
        // Events lost: the finger is now elsewhere, the button let go.
        let state = vec![SlotState {
            id: 1,
            x: 800,
            y: 300,
            palm: false,
        }];
        let out = t.t.reset(Some((state, 0)), &[]);
        assert_eq!(
            out,
            vec![
                Event::Key {
                    code: BTN_LEFT,
                    down: false
                },
                Event::Sync
            ]
        );
        let out = t.frame(10, &T::mv(0, 810, 300));
        let dx = rel(&out, REL_X);
        assert!(dx > 0 && dx < 100, "from the position after the loss: {dx}");
    }

    #[test]
    fn touchscreens_drag_and_tap() {
        // 300 × 200 mm, resolution unknown (assumed from the width).
        let geo = Geometry::new((0, 3000, 0), (0, 2000, 0), 10, false, true);
        assert_eq!(geo.x.per_mm, 10.0);
        let mut t = T::new(Touch::new(geo, TouchSettings::default()));
        t.frame(10, &T::down(0, 1, 1500, 1000));
        assert!(rel(&t.frame(10, &T::mv(0, 1500, 1040)), REL_Y) > 0);
        assert!(keys(&t.frame(10, &T::up(0))).is_empty(), "moved: no tap");
        // A finger on a screen wobbles a little: still a tap.
        t.frame(500, &T::down(0, 2, 1500, 1000));
        t.frame(20, &T::mv(0, 1520, 1010));
        assert_eq!(keys(&t.frame(20, &T::up(0))), [(BTN_LEFT, true)]);
    }

    #[test]
    fn single_touch_pads_work_too() {
        let mut t = T::new(Touch::new(
            Geometry::new((0, 1000, 10), (0, 600, 10), 0, true, false),
            TouchSettings::default(),
        ));
        let s = |x, y| vec![abs(ABS_X, x), abs(ABS_Y, y)];
        t.frame(
            10,
            &cat(&[
                s(500, 300),
                vec![key(BTN_TOUCH, true), key(BTN_TOOL_FINGER, true)],
            ]),
        );
        assert!(rel(&t.frame(10, &s(510, 300)), REL_X) > 0);
        // Two fingers (BTN_TOOL_DOUBLETAP) on a single-touch pad: no motion.
        let out = t.frame(
            10,
            &cat(&[
                vec![key(BTN_TOOL_FINGER, false), key(BTN_TOOL_DOUBLETAP, true)],
                s(520, 300),
            ]),
        );
        assert_eq!(rel(&out, REL_X), 0);
        // Lift, then a short tap: a click.
        t.frame(10, &[key(BTN_TOOL_DOUBLETAP, false), key(BTN_TOUCH, false)]);
        t.frame(
            500,
            &cat(&[
                s(500, 300),
                vec![key(BTN_TOUCH, true), key(BTN_TOOL_FINGER, true)],
            ]),
        );
        let out = t.frame(50, &[key(BTN_TOUCH, false), key(BTN_TOOL_FINGER, false)]);
        assert_eq!(keys(&out), [(BTN_LEFT, true)]);
    }

    #[test]
    fn more_fingers_than_slots_count_from_the_tool_keys() {
        // Two slots, three fingers (BTN_TOOL_TRIPLETAP): a three-finger tap.
        let mut t = T::new(Touch::new(
            Geometry::new((0, 1000, 10), (0, 600, 10), 2, true, false),
            TouchSettings::default(),
        ));
        t.frame(
            10,
            &cat(&[
                T::down(0, 1, 400, 300),
                T::down(1, 2, 600, 300),
                vec![key(BTN_TOOL_TRIPLETAP, true)],
            ]),
        );
        let out = t.frame(
            40,
            &cat(&[T::up(0), T::up(1), vec![key(BTN_TOOL_TRIPLETAP, false)]]),
        );
        assert_eq!(keys(&out), [(BTN_MIDDLE, true)]);
    }
}
