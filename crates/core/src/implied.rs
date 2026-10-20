//! Synthetic implied-volatility feed.
//!
//! Real implied vol reacts to regime changes within minutes rather than hours,
//! but it is noisy and carries a variance risk premium. This model derives it
//! from the feed's true σ: a short reaction lag, a multiplicative premium, and
//! mean-reverting (Ornstein–Uhlenbeck) log noise.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::StandardNormal;

#[derive(Clone, Debug)]
pub struct ImpliedConfig {
    /// Half-life in seconds with which IV catches up to the true σ.
    pub lag_half_life: f64,
    /// IV / true σ in steady state, minus 1 (e.g. 0.1 = IV quotes 10% rich).
    pub premium: f64,
    /// Stationary standard deviation of the log noise.
    pub noise: f64,
    /// Mean-reversion half-life of the noise, in seconds.
    pub noise_half_life: f64,
}

impl Default for ImpliedConfig {
    fn default() -> Self {
        Self { lag_half_life: 600.0, premium: 0.1, noise: 0.1, noise_half_life: 3_600.0 }
    }
}

#[derive(Clone, Debug)]
pub struct ImpliedVol {
    cfg: ImpliedConfig,
    rng: ChaCha8Rng,
    /// Lagged log of the true σ.
    level: Option<f64>,
    ou: f64,
}

impl ImpliedVol {
    pub fn new(cfg: ImpliedConfig, seed: u64) -> Self {
        Self { cfg, rng: ChaCha8Rng::seed_from_u64(seed), level: None, ou: 0.0 }
    }

    /// Advances by `dt` seconds toward `true_sigma` and returns the quoted IV per √second.
    pub fn update(&mut self, dt: f64, true_sigma: f64) -> f64 {
        let target = true_sigma.ln();
        let level = match self.level {
            Some(l) => l + (1.0 - 0.5f64.powf(dt / self.cfg.lag_half_life)) * (target - l),
            None => target,
        };
        self.level = Some(level);
        // Exact OU step: keeps the stationary sd at `noise` for any dt.
        let decay = 0.5f64.powf(dt / self.cfg.noise_half_life);
        let z: f64 = self.rng.sample(StandardNormal);
        self.ou = self.ou * decay + self.cfg.noise * (1.0 - decay * decay).sqrt() * z;
        (level + self.ou).exp() * (1.0 + self.cfg.premium)
    }
}

/// Blends two vol estimates in variance space (the model price is ∝ 1/σ²).
pub fn blend_sigma(realized: f64, implied: f64, weight: f64) -> f64 {
    (weight * implied * implied + (1.0 - weight) * realized * realized).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noiseless_tracks_true_sigma_with_lag_and_premium() {
        let mut iv = ImpliedVol::new(ImpliedConfig { premium: 0.1, noise: 0.0, ..Default::default() }, 0);
        assert!((iv.update(60.0, 1e-4) - 1.1e-4).abs() < 1e-12);
        // Regime jumps to 3e-4: one half-life later we are halfway in log space.
        let v = iv.update(600.0, 3e-4);
        assert!((v / 1.1 - (1e-4f64 * 3e-4).sqrt()).abs() < 1e-9, "{v}");
    }

    #[test]
    fn noise_has_configured_stationary_sd() {
        let mut iv = ImpliedVol::new(ImpliedConfig { premium: 0.0, noise: 0.2, ..Default::default() }, 1);
        let logs: Vec<f64> = (0..200_000).map(|_| (iv.update(60.0, 1.0)).ln()).collect();
        let mean = logs.iter().sum::<f64>() / logs.len() as f64;
        let sd = (logs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / logs.len() as f64).sqrt();
        assert!(mean.abs() < 0.03 && (sd - 0.2).abs() < 0.02, "mean {mean} sd {sd}");
    }

    #[test]
    fn blend_is_in_variance_space() {
        assert_eq!(blend_sigma(1.0, 3.0, 0.0), 1.0);
        assert_eq!(blend_sigma(1.0, 3.0, 1.0), 3.0);
        assert!((blend_sigma(1.0, 3.0, 0.5) - 5.0f64.sqrt()).abs() < 1e-12);
    }
}
