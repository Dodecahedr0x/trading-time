//! Model oracle: maps a volatility estimate to the expected time for price to
//! leave a symmetric ±x band.
//!
//! For driftless Brownian motion in log-price with volatility σ (per √second),
//! the expected exit time from (-X, X) is X²/σ², with X = ln(1+x).

/// Expected seconds to move ±`x` (e.g. 0.01 = 1%) given `sigma` per √second.
pub fn model_time(x: f64, sigma: f64) -> f64 {
    let l = (1.0 + x).ln();
    l * l / (sigma * sigma)
}

/// [`model_time`] clamped to `[min, max]`; non-finite results map to `max`.
pub fn model_time_clamped(x: f64, sigma: f64, min: f64, max: f64) -> f64 {
    let t = model_time(x, sigma);
    if t.is_finite() { t.clamp(min, max) } else { max }
}

/// Moves from `prev` toward `target` by at most `max_log` in log space.
pub fn cap_move(prev: f64, target: f64, max_log: f64) -> f64 {
    prev * (target / prev).clamp((-max_log).exp(), max_log.exp())
}

/// Converts annualized volatility to per-√second.
pub fn sigma_per_sec(annual: f64) -> f64 {
    annual / (365.0 * 86_400.0f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    use rand_distr::StandardNormal;

    #[test]
    fn sol_like_vol_gives_about_80_minutes() {
        let t = model_time(0.01, sigma_per_sec(0.8));
        assert!((t - 4_896.0).abs() < 50.0, "{t}");
    }

    #[test]
    fn cap_move_limits_log_step() {
        assert_eq!(cap_move(100.0, 105.0, 0.1), 105.0);
        assert!((cap_move(100.0, 1_000.0, 0.1) - 100.0 * 0.1f64.exp()).abs() < 1e-9);
        assert!((cap_move(100.0, 10.0, 0.1) - 100.0 * (-0.1f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn clamps_degenerate_sigma() {
        assert_eq!(model_time_clamped(0.01, 0.0, 60.0, 1e6), 1e6);
        assert_eq!(model_time_clamped(0.01, 1.0, 60.0, 1e6), 60.0);
    }

    /// Monte-Carlo check of E[τ] = X²/σ². Discrete monitoring is corrected with
    /// the Broadie–Glasserman barrier shift (0.5826·σ·√dt).
    #[test]
    fn monte_carlo_matches_formula() {
        let x: f64 = 0.01;
        let sigma = (1.0 + x).ln() / 20.0; // E[τ] = 400 s
        let expected = model_time(x, sigma);
        let dt = expected / 4_000.0;
        let barrier = (1.0 + x).ln() - 0.5826 * sigma * dt.sqrt();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let paths = 3_000;
        let mut total = 0.0;
        for _ in 0..paths {
            let (mut lx, mut t) = (0.0f64, 0.0);
            while lx.abs() < barrier {
                let z: f64 = rng.sample(StandardNormal);
                lx += -0.5 * sigma * sigma * dt + sigma * dt.sqrt() * z;
                t += dt;
            }
            total += t;
        }
        let mean = total / paths as f64;
        let err = (mean / expected - 1.0).abs();
        assert!(err < 0.05, "mean {mean} vs {expected} ({:.1}%)", err * 100.0);
    }
}
