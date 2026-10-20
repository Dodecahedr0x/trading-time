//! Pyth price path for the local stack: the replay file looped (each pass continues from the last
//! price) with Markov volatility regimes that scale log returns, so the realized-vol index moves.

use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub struct PriceDriver {
    returns: Vec<f64>,
    pos: usize,
    price: f64,
    storm: bool,
    pub calm_mult: f64,
    pub storm_mult: f64,
    /// Per-step probability of switching regime.
    pub switch_prob: f64,
}

impl PriceDriver {
    /// `prices` are consecutive samples (e.g. the replay CSV column); needs at least two.
    pub fn new(prices: &[f64], start: f64) -> Option<Self> {
        let returns: Vec<f64> = prices.windows(2).filter(|w| w[0] > 0.0 && w[1] > 0.0).map(|w| (w[1] / w[0]).ln()).collect();
        (!returns.is_empty() && start > 0.0).then_some(Self {
            returns,
            pos: 0,
            price: start,
            storm: false,
            calm_mult: 1.0,
            storm_mult: 4.0,
            switch_prob: 0.02,
        })
    }

    pub fn price(&self) -> f64 {
        self.price
    }

    pub fn storm(&self) -> bool {
        self.storm
    }

    /// Advances by `steps` file samples (one Pyth update may span several), returning the new price.
    pub fn step(&mut self, steps: usize, rng: &mut ChaCha8Rng) -> f64 {
        if rng.random_bool(self.switch_prob.clamp(0.0, 1.0)) {
            self.storm = !self.storm;
        }
        let mult = if self.storm { self.storm_mult } else { self.calm_mult };
        let mut r = 0.0;
        for _ in 0..steps.max(1) {
            r += self.returns[self.pos];
            self.pos = (self.pos + 1) % self.returns.len();
        }
        self.price *= (r * mult).exp();
        self.price
    }
}

/// `ts_ms,price` CSV (header optional) -> prices.
pub fn parse_prices(csv: &str) -> Vec<f64> {
    csv.lines().filter_map(|l| l.split(',').nth(1)?.trim().parse::<f64>().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn loops_and_continues_from_last_price() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let mut d = PriceDriver::new(&[100.0, 110.0, 121.0], 180.0).unwrap();
        d.switch_prob = 0.0;
        let p: Vec<f64> = (0..4).map(|_| (d.step(1, &mut rng) * 1e6).round() / 1e6).collect();
        assert_eq!(p, vec![198.0, 217.8, 239.58, 263.538]);
    }

    #[test]
    fn storm_scales_returns() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let mut d = PriceDriver::new(&[100.0, 110.0], 100.0).unwrap();
        d.switch_prob = 1.0;
        d.step(1, &mut rng);
        assert!(d.storm());
        assert!(((d.price() / 100.0).ln() - 4.0 * 1.1f64.ln()).abs() < 1e-12);
    }

    #[test]
    fn parses_csv_and_rejects_degenerate_input() {
        assert_eq!(parse_prices("ts_ms,price\n0,150\n1000,151.5\n"), vec![150.0, 151.5]);
        assert!(PriceDriver::new(&[100.0], 1.0).is_none());
        assert!(PriceDriver::new(&[100.0, 101.0], 0.0).is_none());
    }
}
