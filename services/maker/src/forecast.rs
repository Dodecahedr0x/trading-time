//! Fast realized-vol forecast of the on-chain index, mirroring `book_sim::{Estimator, Forecaster}`.

use crate::feed::PriceTick;
use tradingtime_interface::Market;
use tradingtime_math::{Fx, LN2, ONE, exp_fx, from_f64, from_int, fx_div, fx_mul, index, ln_fx};

pub const DEFAULT_HALF_LIFE_SECS: u32 = 900;

/// EWMA of squared log returns per second, sampled on each second's last trade.
#[derive(Clone, Debug)]
pub struct Forecaster {
    half_life_secs: u32,
    var: Option<Fx>,
    /// Last closed second and its closing price.
    anchor: Option<(i64, f64)>,
    /// Second still accumulating trades and its latest price.
    open: Option<(i64, f64)>,
}

impl Forecaster {
    pub fn new(half_life_secs: u32) -> Self {
        Forecaster { half_life_secs, var: None, anchor: None, open: None }
    }

    /// Sets the variance per second (Fx), e.g. the one implied by the on-chain index.
    pub fn seed(&mut self, var: Fx) {
        self.var = Some(var);
    }

    pub fn var(&self) -> Option<Fx> {
        self.var
    }

    pub fn on_tick(&mut self, t: &PriceTick) {
        if !(t.price.is_finite() && t.price > 0.0) {
            return;
        }
        let sec = t.ts_ms.div_euclid(1000);
        match self.open {
            Some((s, _)) if sec < s => {}
            Some((s, ref mut p)) if sec == s => *p = t.price,
            Some((s, p)) => {
                self.close(s, p);
                self.open = Some((sec, t.price));
            }
            None => self.open = Some((sec, t.price)),
        }
    }

    fn close(&mut self, sec: i64, price: f64) {
        if let Some((s0, p0)) = self.anchor {
            let r = from_f64((price / p0).ln());
            let dt = sec - s0;
            self.var = match self.var {
                Some(v) => index::ewma_update(v, r, dt, self.half_life_secs).or(Some(v)),
                None => fx_mul(r, r).map(|r2| r2 / dt as i128),
            };
        }
        self.anchor = Some((sec, price));
    }

    /// Expected index ln-time one funding period ahead: the index variance decays toward the
    /// fast estimate, `v_now + (v_index − v_now)·2^(−period/half_life)`, priced like CrankIndex.
    pub fn fair_ln(&self, m: &Market) -> Option<Fx> {
        let p = &m.params;
        let calib = p.calib.get();
        let var_index = implied_var(m)?;
        let var_now = self.var.unwrap_or(var_index);
        let decay = fx_div(fx_mul(LN2, from_int(p.funding_period_secs as i64))?, from_int(p.index_half_life_secs.max(1) as i64))?;
        let w = exp_fx(-decay)?;
        let blend = var_now.checked_add(fx_mul(var_index - var_now, w)?)?;
        index::index_ln(blend, calib, p.band_bps, p.index_min_secs, p.index_max_secs)
    }
}

/// Variance per second (Fx) at which CrankIndex would produce `m.index_ln` (inverse of `index_ln`).
pub fn implied_var(m: &Market) -> Option<Fx> {
    let p = &m.params;
    let x = ln_fx(ONE + p.band_bps as i128 * ONE / 10_000)?;
    exp_fx(ln_fx(p.calib.get())? + 2 * ln_fx(x)? - m.index_ln.get())
}
