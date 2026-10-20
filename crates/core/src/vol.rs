//! Exponentially weighted volatility estimator over irregular time steps.
//!
//! Each bar contributes the Parkinson range variance `ln(H/L)² / (4 ln 2)` plus
//! the squared gap from the previous close, which is several times more
//! efficient than close-to-close returns.

use crate::feed::Bar;

#[derive(Clone, Debug)]
pub struct EwmaVol {
    half_life: f64,
    var: f64,
    last: Option<(f64, f64)>,
}

const PARKINSON: f64 = 0.360_673_760_222_241_3; // 1 / (4 ln 2)

impl EwmaVol {
    /// `half_life` in seconds, `initial_sigma` per √second.
    pub fn new(half_life: f64, initial_sigma: f64) -> Self {
        Self { half_life, var: initial_sigma * initial_sigma, last: None }
    }

    /// Feeds a bar closing at `bar.t`; the first bar only sets the reference.
    pub fn update(&mut self, bar: &Bar) {
        if let Some((t0, prev_close)) = self.last {
            let dt = bar.t - t0;
            if dt > 0.0 {
                let gap = (bar.open / prev_close).ln();
                let range = (bar.high / bar.low).ln();
                let inst = (gap * gap + PARKINSON * range * range) / dt;
                let alpha = 1.0 - 0.5f64.powf(dt / self.half_life);
                self.var += alpha * (inst - self.var);
            }
        }
        self.last = Some((bar.t, bar.close));
    }

    pub fn half_life(&self) -> f64 {
        self.half_life
    }

    /// Current estimate per √second.
    pub fn sigma(&self) -> f64 {
        self.var.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{Model, PriceFeed, SynthFeed};

    #[test]
    fn flat_price_halves_variance_each_half_life() {
        let flat = |t| Bar { t, open: 10.0, high: 10.0, low: 10.0, close: 10.0 };
        let mut v = EwmaVol::new(100.0, 1.0);
        v.update(&flat(0.0));
        v.update(&flat(100.0));
        assert!((v.sigma() - 0.5f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn converges_to_true_sigma() {
        let sigma = 1.5e-4;
        let mut feed = SynthFeed::new(Model::Gbm { sigma }, 3, 100.0, 60.0);
        let mut v = EwmaVol::new(6.0 * 3600.0, 1e-5);
        for _ in 0..20_000 {
            let b = feed.next_bar().unwrap();
            v.update(&b);
        }
        assert!((v.sigma() / sigma - 1.0).abs() < 0.15, "{}", v.sigma());
    }
}
