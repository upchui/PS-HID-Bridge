//! Mouse sensitivity, optional acceleration, the vertical axis and the wheel.
//!
//! gain(speed) = sensitivity * (1 + accel * max(0, speed - threshold)), capped
//! at `max_gain`, where speed is the length of one evdev frame's (dx, dy).
//! The vertical axis gets `y_ratio` times that gain, reversed with `invert_y`.
//! The wheel (both directions) is multiplied by `wheel`.
//! Fractions are carried over so slow movements and single notches are not
//! lost. With the defaults the raw counts pass through unchanged.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseCurve {
    pub sensitivity: f64,
    pub accel: f64,
    pub threshold: f64,
    pub max_gain: f64,
    /// Up and down reversed (as some games offer for the camera).
    pub invert_y: bool,
    /// Vertical sensitivity relative to the horizontal one.
    pub y_ratio: f64,
    /// Factor on the wheel's notches (0.5 = one step per two notches).
    pub wheel: f64,
}

impl Default for MouseCurve {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            accel: 0.0,
            threshold: 2.0,
            max_gain: 8.0,
            invert_y: false,
            y_ratio: 1.0,
            wheel: 1.0,
        }
    }
}

impl MouseCurve {
    /// Motion passes through unchanged.
    pub fn is_raw(&self) -> bool {
        self.sensitivity == 1.0 && self.accel == 0.0 && self.y_ratio == 1.0 && !self.invert_y
    }

    pub fn gain(&self, speed: f64) -> f64 {
        let g = self.sensitivity * (1.0 + self.accel * (speed - self.threshold).max(0.0));
        g.clamp(0.0, self.max_gain.max(self.sensitivity))
    }
}

#[derive(Debug, Default)]
pub struct MouseFilter {
    pub curve: MouseCurve,
    rem_x: f64,
    rem_y: f64,
    rem_wheel: f64,
    rem_pan: f64,
}

impl MouseFilter {
    pub fn new(curve: MouseCurve) -> Self {
        Self {
            curve,
            ..Default::default()
        }
    }

    /// Scale one frame of motion; returns whole counts.
    pub fn apply(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        if self.curve.is_raw() {
            return (dx, dy);
        }
        let speed = ((dx as f64).powi(2) + (dy as f64).powi(2)).sqrt();
        let g = self.curve.gain(speed);
        let gy = g * self.curve.y_ratio * if self.curve.invert_y { -1.0 } else { 1.0 };
        let fx = dx as f64 * g + self.rem_x;
        let fy = dy as f64 * gy + self.rem_y;
        let (ox, oy) = (fx.trunc(), fy.trunc());
        self.rem_x = fx - ox;
        self.rem_y = fy - oy;
        (ox as i32, oy as i32)
    }

    /// Scale one frame of wheel notches (vertical, horizontal).
    pub fn apply_wheel(&mut self, wheel: i32, pan: i32) -> (i32, i32) {
        let f = self.curve.wheel;
        if f == 1.0 {
            return (wheel, pan);
        }
        (
            scale(wheel, f, &mut self.rem_wheel),
            scale(pan, f, &mut self.rem_pan),
        )
    }

    /// Forget carried fractions (the bridge went off: the next movement
    /// starts afresh).
    pub fn reset(&mut self) {
        self.rem_x = 0.0;
        self.rem_y = 0.0;
        self.rem_wheel = 0.0;
        self.rem_pan = 0.0;
    }
}

/// `v * f` in whole steps, the rest carried in `rem`; turning the other
/// way drops what was carried (half a notch up does not eat a notch down).
fn scale(v: i32, f: f64, rem: &mut f64) -> i32 {
    if v == 0 {
        return 0;
    }
    if (*rem < 0.0) != (v < 0) {
        *rem = 0.0;
    }
    let total = v as f64 * f + *rem;
    let whole = total.trunc();
    *rem = total - whole;
    whole as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_passthrough() {
        let mut f = MouseFilter::new(MouseCurve::default());
        assert_eq!(f.apply(7, -3), (7, -3));
        assert_eq!(f.apply_wheel(-2, 1), (-2, 1));
    }

    #[test]
    fn half_sensitivity_keeps_fractions() {
        let mut f = MouseFilter::new(MouseCurve {
            sensitivity: 0.5,
            ..Default::default()
        });
        let (mut sx, mut sy) = (0, 0);
        for _ in 0..100 {
            let (x, y) = f.apply(1, -1);
            sx += x;
            sy += y;
        }
        assert_eq!((sx, sy), (50, -50));
    }

    #[test]
    fn acceleration_increases_with_speed() {
        let c = MouseCurve {
            accel: 0.1,
            ..Default::default()
        };
        assert_eq!(c.gain(1.0), 1.0);
        assert!(c.gain(10.0) > c.gain(5.0));
        assert_eq!(c.gain(1000.0), c.max_gain);
    }

    #[test]
    fn inverted_vertical_axis() {
        let mut f = MouseFilter::new(MouseCurve {
            invert_y: true,
            ..Default::default()
        });
        assert!(!f.curve.is_raw());
        assert_eq!(f.apply(3, 5), (3, -5));
        assert_eq!(f.apply(0, -2), (0, 2));
    }

    #[test]
    fn vertical_ratio_keeps_fractions() {
        let mut f = MouseFilter::new(MouseCurve {
            y_ratio: 0.5,
            ..Default::default()
        });
        assert!(!f.curve.is_raw());
        let (mut sx, mut sy) = (0, 0);
        for _ in 0..10 {
            let (x, y) = f.apply(1, 1);
            sx += x;
            sy += y;
        }
        assert_eq!((sx, sy), (10, 5));
    }

    #[test]
    fn slower_wheel_steps_every_second_notch() {
        let mut f = MouseFilter::new(MouseCurve {
            wheel: 0.5,
            ..Default::default()
        });
        let steps: Vec<i32> = (0..4).map(|_| f.apply_wheel(1, 0).0).collect();
        assert_eq!(steps, [0, 1, 0, 1]);
        // Half a notch carried upwards does not eat the next notch down.
        assert_eq!(f.apply_wheel(1, 0).0, 0);
        assert_eq!(f.apply_wheel(-1, 0).0, 0);
        assert_eq!(f.apply_wheel(-1, 0).0, -1);
    }

    #[test]
    fn faster_wheel() {
        let mut f = MouseFilter::new(MouseCurve {
            wheel: 1.5,
            ..Default::default()
        });
        assert_eq!(f.apply_wheel(1, -1), (1, -1));
        assert_eq!(f.apply_wheel(1, -1), (2, -2));
        // Motion is not touched by the wheel factor.
        assert_eq!(f.apply(4, 4), (4, 4));
    }
}
