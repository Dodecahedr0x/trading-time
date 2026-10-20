//! Rolling ±x band: detects hits, resets the reference, and keeps the
//! realized hitting-time index.

use crate::feed::Bar;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub t: f64,
    /// Seconds from the reference reset to the hit (bar granularity).
    pub duration: f64,
    pub up: bool,
}

#[derive(Clone, Debug)]
pub struct BandTracker {
    pub x: f64,
    ref_price: f64,
    ref_time: f64,
    recent: VecDeque<f64>,
    window: usize,
    pub total_hits: u64,
}

impl BandTracker {
    pub fn new(x: f64, window: usize, t0: f64, price0: f64) -> Self {
        Self { x, ref_price: price0, ref_time: t0, recent: VecDeque::new(), window, total_hits: 0 }
    }

    pub fn upper(&self) -> f64 {
        self.ref_price * (1.0 + self.x)
    }

    pub fn lower(&self) -> f64 {
        self.ref_price / (1.0 + self.x)
    }

    pub fn ref_time(&self) -> f64 {
        self.ref_time
    }

    /// Checks `bar` against the band. On a hit, the reference resets to the bar close.
    pub fn on_bar(&mut self, bar: &Bar) -> Option<Hit> {
        let up = bar.high >= self.upper();
        let down = bar.low <= self.lower();
        if !up && !down {
            return None;
        }
        let hit = Hit {
            t: bar.t,
            duration: bar.t - self.ref_time,
            up: if up && down { bar.close >= self.ref_price } else { up },
        };
        self.recent.push_back(hit.duration);
        if self.recent.len() > self.window {
            self.recent.pop_front();
        }
        self.total_hits += 1;
        self.ref_price = bar.close;
        self.ref_time = bar.t;
        Some(hit)
    }

    /// Mean of the last `window` hitting times.
    pub fn realized_index(&self) -> Option<f64> {
        (!self.recent.is_empty()).then(|| self.recent.iter().sum::<f64>() / self.recent.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar { t, open: close, high, low, close }
    }

    #[test]
    fn inside_band_no_hit() {
        let mut b = BandTracker::new(0.01, 3, 0.0, 100.0);
        assert_eq!(b.on_bar(&bar(60.0, 100.9, 99.1, 100.5)), None);
        assert_eq!(b.realized_index(), None);
    }

    #[test]
    fn hits_reset_reference_and_roll_index() {
        let mut b = BandTracker::new(0.01, 2, 0.0, 100.0);
        let h = b.on_bar(&bar(120.0, 101.0, 100.0, 101.0)).unwrap();
        assert_eq!(h, Hit { t: 120.0, duration: 120.0, up: true });
        assert!((b.upper() - 102.01).abs() < 1e-9);

        let h = b.on_bar(&bar(180.0, 101.0, 99.9, 99.9)).unwrap();
        assert!(!h.up);
        assert_eq!(h.duration, 60.0);
        assert_eq!(b.realized_index(), Some(90.0));

        b.on_bar(&bar(480.0, 101.0, 98.0, 98.0)).unwrap();
        assert_eq!(b.realized_index(), Some(180.0)); // window = 2: (60 + 300) / 2
        assert_eq!(b.total_hits, 3);
    }
}
