//! Mouse sensitivity and optional acceleration.
//!
//! gain(speed) = sensitivity * (1 + accel * max(0, speed - threshold)), capped
//! at `max_gain`, where speed is the length of one evdev frame's (dx, dy).
//! Fractions are carried over so slow movements are not lost.
//! With sensitivity 1 and accel 0 the raw counts pass through unchanged.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseCurve {
    pub sensitivity: f64,
    pub accel: f64,
    pub threshold: f64,
    pub max_gain: f64,
}

impl Default for MouseCurve {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            accel: 0.0,
            threshold: 2.0,
            max_gain: 8.0,
        }
    }
}

impl MouseCurve {
    pub fn is_raw(&self) -> bool {
        self.sensitivity == 1.0 && self.accel == 0.0
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
        let fx = dx as f64 * g + self.rem_x;
        let fy = dy as f64 * g + self.rem_y;
        let (ox, oy) = (fx.trunc(), fy.trunc());
        self.rem_x = fx - ox;
        self.rem_y = fy - oy;
        (ox as i32, oy as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_passthrough() {
        let mut f = MouseFilter::new(MouseCurve::default());
        assert_eq!(f.apply(7, -3), (7, -3));
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
}
