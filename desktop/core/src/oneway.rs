//! One-way time app -> board, from timed pings (PING_TIMED / PONG_TIMED,
//! see protocol/SPEC.md).
//!
//! Every answer gives four times: `t1` the app sent the ping and `t4` it got
//! the answer (app clock), `t2` the board got the ping and `t3` it built the
//! answer (board clock). `(t4 - t1) - (t3 - t2)` is the time on the network,
//! both ways. App and board have no common clock: `t2 - t1` is the way out
//! plus the offset between the clocks. The offset (and its drift, crystals
//! differ by tens of ppm) comes from the fastest answers: of each block of
//! [`BLOCK`] samples the one with the shortest network time counts, assuming
//! it took equally long in both directions; a line through those points
//! gives the offset at any time. The way out of every sample is then
//! `t2 - t1 - offset`: delays on the way out (a busy uplink, a network
//! adapter that bundles packets) show where they happen.

use std::collections::VecDeque;

/// Samples kept (2 minutes at one ping per second).
pub const WINDOW: usize = 120;
/// Of every block of this many samples, the fastest one sets the offset.
const BLOCK: usize = 8;
/// Fit a drift only over at least this long (µs); shorter: offset only.
const DRIFT_SPAN_US: f64 = 5_000_000.0;
/// The offset moved by more than this: the clocks started over.
const JUMP_US: f64 = 50_000.0;

#[derive(Clone, Copy, Debug)]
struct Sample {
    /// App send time, relative to `t_base` (µs).
    t: f64,
    /// `t2 - t1`, relative to `a_base` (µs).
    a: f64,
    /// Time on the network, both ways (µs).
    net: f64,
}

/// One-way times app -> board over the last [`WINDOW`] answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OneWayStats {
    pub last_us: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    /// Oldest first, in milliseconds.
    pub history_ms: Vec<f64>,
}

/// Offset line: `offset(t) = alpha + beta * t` (relative units as in
/// [`Sample`]).
#[derive(Clone, Copy, Debug)]
struct Line {
    alpha: f64,
    beta: f64,
}

impl Line {
    fn at(&self, t: f64) -> f64 {
        self.alpha + self.beta * t
    }
}

#[derive(Debug, Default)]
pub struct OneWay {
    samples: VecDeque<Sample>,
    /// Integer bases, so that the huge raw values (the app's ping clock
    /// starts at a random point up to 2^62 µs) never meet a float.
    t_base: u64,
    a_base: i64,
}

impl OneWay {
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// One answer: `t1`/`t4` app clock, `t2`/`t3` board clock (µs).
    pub fn add(&mut self, t1: u64, t2: u64, t3: u64, t4: u64) {
        if t4 < t1 || t3 < t2 || t3 - t2 > t4 - t1 {
            return; // impossible: not a sample
        }
        let net = ((t4 - t1) - (t3 - t2)) as f64;
        let a_raw = t2 as i64 - t1 as i64;
        if self.samples.is_empty() {
            self.t_base = t1;
            self.a_base = a_raw;
        }
        let s = Sample {
            t: t1.wrapping_sub(self.t_base) as i64 as f64,
            a: (a_raw - self.a_base) as f64,
            net,
        };
        if let Some(line) = self.line()
            && (s.a - net / 2.0 - line.at(s.t)).abs() > JUMP_US
        {
            // The board's clock (or ours) started over.
            self.samples.clear();
            self.add(t1, t2, t3, t4);
            return;
        }
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(s);
    }

    /// The offset line through the fastest sample of every block.
    fn line(&self) -> Option<Line> {
        let points: Vec<(f64, f64)> = self
            .samples
            .iter()
            .collect::<Vec<_>>()
            .chunks(BLOCK)
            .filter_map(|block| {
                block
                    .iter()
                    .min_by(|x, y| x.net.total_cmp(&y.net))
                    .map(|s| (s.t, s.a - s.net / 2.0))
            })
            .collect();
        let (first, last) = (points.first()?, points.last()?);
        if points.len() < 3 || last.0 - first.0 < DRIFT_SPAN_US {
            let mut offsets: Vec<f64> = points.iter().map(|p| p.1).collect();
            offsets.sort_by(f64::total_cmp);
            return Some(Line {
                alpha: offsets[offsets.len() / 2],
                beta: 0.0,
            });
        }
        // Least squares.
        let n = points.len() as f64;
        let mt = points.iter().map(|p| p.0).sum::<f64>() / n;
        let mo = points.iter().map(|p| p.1).sum::<f64>() / n;
        let (mut sto, mut stt) = (0.0, 0.0);
        for (t, o) in &points {
            sto += (t - mt) * (o - mo);
            stt += (t - mt) * (t - mt);
        }
        let beta = if stt > 0.0 { sto / stt } else { 0.0 };
        Some(Line {
            alpha: mo - beta * mt,
            beta,
        })
    }

    /// None until there are two samples.
    pub fn stats(&self) -> Option<OneWayStats> {
        if self.samples.len() < 2 {
            return None;
        }
        let line = self.line()?;
        let ways: Vec<f64> = self
            .samples
            .iter()
            .map(|s| (s.a - line.at(s.t)).max(0.0))
            .collect();
        let mut sorted = ways.clone();
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1];
        Some(OneWayStats {
            last_us: *ways.last().unwrap(),
            p50_us: at(0.50),
            p95_us: at(0.95),
            history_ms: ways.iter().map(|w| w / 1000.0).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A network where the way out takes `up(k)` and the way back `down(k)`
    /// µs; the board's clock starts elsewhere and runs `ppm` faster; the
    /// board answers after 60 µs. Returns the estimated and true way out of
    /// the last sample, and the statistics.
    fn simulate(
        n: usize,
        ppm: f64,
        up: impl Fn(usize) -> f64,
        down: impl Fn(usize) -> f64,
    ) -> (OneWay, Vec<f64>) {
        // The app's ping clock starts at a random point up to 2^62.
        const T0: u64 = 3_000_000_000_000_000_000;
        const B0: u64 = 1_234_567_890;
        let board = |app: f64| B0 + (app * (1.0 + ppm * 1e-6)) as u64;
        let mut w = OneWay::default();
        let mut truth = Vec::new();
        for k in 0..n {
            let send = k as f64 * 200_000.0; // every 200 ms
            let (u, d) = (up(k), down(k));
            let t2 = board(send + u);
            let t3 = t2 + 60;
            let back = send + u + 60.0 / (1.0 + ppm * 1e-6) + d;
            w.add(T0 + send as u64, t2, t3, T0 + back as u64);
            truth.push(u);
        }
        (w, truth)
    }

    /// Deterministic jitter 0..max.
    fn jitter(k: usize, max: f64) -> f64 {
        ((k * 7919 + 13) % 101) as f64 / 100.0 * max
    }

    #[test]
    fn way_out_with_drift_and_jitter_on_the_way_out() {
        // Symmetric base of 100 µs; up to 300 µs extra on the way out only.
        let (w, truth) = simulate(
            120,
            50.0,
            |k| 100.0 + jitter(k, 300.0),
            |k| 100.0 + jitter(k * 3, 10.0),
        );
        let st = w.stats().unwrap();
        // The fastest answers still carry a little jitter: a few µs off.
        let err = (st.last_us - truth.last().unwrap()).abs();
        assert!(err < 25.0, "last {} vs {}", st.last_us, truth.last().unwrap());
        for (est, t) in st.history_ms.iter().zip(&truth) {
            assert!((est * 1000.0 - t).abs() < 30.0, "{} vs {t}", est * 1000.0);
        }
        // The median of 100 + U(0, 300) is about 250.
        assert!((st.p50_us - 250.0).abs() < 30.0, "{}", st.p50_us);
    }

    #[test]
    fn unequal_fastest_ways_shift_the_estimate_by_half_the_difference() {
        // Way out at least 400 µs (e.g. bundling), way back 100 µs: the
        // fastest answers are taken as equal, so the way out reads 150 µs
        // short. A known limit of measuring without a common clock.
        let (w, truth) = simulate(64, 20.0, |k| 400.0 + jitter(k, 50.0), |_| 100.0);
        let st = w.stats().unwrap();
        let err = truth.last().unwrap() - st.last_us;
        assert!((err - 150.0).abs() < 20.0, "error {err}");
    }

    #[test]
    fn huge_clock_values_keep_microseconds() {
        let (w, truth) = simulate(16, 0.0, |_| 80.0, |_| 80.0);
        let st = w.stats().unwrap();
        assert!((st.last_us - truth.last().unwrap()).abs() < 2.0, "{}", st.last_us);
    }

    #[test]
    fn restarted_board_clock_starts_over() {
        let (mut w, _) = simulate(40, 0.0, |_| 100.0, |_| 100.0);
        // The board restarted: its clock is near zero now.
        let t1 = 3_000_000_000_000_000_000 + 9_000_000;
        w.add(t1, 1_000, 1_060, t1 + 260);
        w.add(t1 + 200_000, 201_000, 201_060, t1 + 200_260);
        let st = w.stats().unwrap();
        assert_eq!(st.history_ms.len(), 2);
        assert!((st.last_us - 100.0).abs() < 2.0, "{}", st.last_us);
    }

    #[test]
    fn nonsense_is_ignored() {
        let mut w = OneWay::default();
        w.add(1_000, 50, 40, 2_000); // answered before it arrived
        w.add(2_000, 50, 60, 1_000); // came back before it was sent
        w.add(3_000, 50, 5_000, 3_100); // board took longer than the round trip
        assert!(w.stats().is_none());
    }
}
