//! Scripted traders that take the other side of the LP pool.

use crate::feed::Bar;
use crate::oracle::model_time_clamped;
use crate::pool::{Position, Side};
use crate::vol::EwmaVol;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentKind {
    User,
    Noise,
    Informed,
    Momentum,
    Chaser,
    FastVol,
    MarketMaker,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open { side: Side, collateral: f64, leverage: f64 },
    Close { id: u64 },
}

/// What an agent sees each step.
pub struct AgentView<'a> {
    pub t: f64,
    /// Model price in seconds.
    pub model: f64,
    /// Model price from the feed's true volatility, if the feed knows it.
    pub true_model: Option<f64>,
    /// Uncapped model price the mark is moving toward (public information).
    pub target: f64,
    pub wallet: f64,
    pub positions: &'a [Position],
}

pub trait Agent {
    fn kind(&self) -> AgentKind;
    /// Sees every bar, including during warmup.
    fn observe(&mut self, _bar: &Bar) {}
    fn act(&mut self, view: &AgentView, rng: &mut ChaCha8Rng) -> Vec<Action>;
}

fn close_all(view: &AgentView) -> Vec<Action> {
    view.positions.iter().map(|p| Action::Close { id: p.id }).collect()
}

/// Opens random-side positions and closes them after a random holding time.
/// Retail behaviour, sampled once per trader so the population is heterogeneous.
/// Rates assume one-minute bars. Sides are random: these traders have no edge.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseProfile {
    /// Chance of opening per bar while flat.
    pub open_prob: f64,
    /// Chance of closing per bar while in a position (absent stops).
    pub close_prob: f64,
    pub leverage: f64,
    /// Fraction of equity posted per trade.
    pub stake: f64,
    /// Exit after losing this fraction of the stake.
    pub stop_loss: f64,
    /// Exit after gaining this multiple of the stake.
    pub take_profit: f64,
}

impl NoiseProfile {
    /// 0.2–2 trades a day, held 2 h–3 days, mostly 2–3× leverage, staking 10–50%
    /// of equity, with a stop at −30–80% and a target at +30–150% of the stake.
    pub fn sample(rng: &mut ChaCha8Rng) -> Self {
        let mut log_uniform = |lo: f64, hi: f64| (lo.ln() + rng.random::<f64>() * (hi / lo).ln()).exp();
        let trades_per_day = log_uniform(0.2, 2.0);
        let hold_hours = log_uniform(2.0, 72.0);
        Self {
            open_prob: trades_per_day / 1_440.0,
            close_prob: 1.0 / (hold_hours * 60.0),
            leverage: [1.0, 2.0, 2.0, 3.0, 3.0, 5.0][rng.random_range(0..6)],
            stake: rng.random_range(0.1..0.5),
            stop_loss: rng.random_range(0.3..0.8),
            take_profit: rng.random_range(0.3..1.5),
        }
    }

    /// Whether to open a position this bar; returns the random side if so.
    pub fn wants_open(&self, rng: &mut ChaCha8Rng) -> Option<Side> {
        (rng.random::<f64>() < self.open_prob).then(|| if rng.random::<bool>() { Side::Long } else { Side::Short })
    }

    /// Whether to exit a position whose PnL is `pnl` on a stake of `stake`.
    pub fn wants_close(&self, pnl: f64, stake: f64, rng: &mut ChaCha8Rng) -> bool {
        pnl <= -self.stop_loss * stake || pnl >= self.take_profit * stake || rng.random::<f64>() < self.close_prob
    }
}

pub struct Noise {
    pub profile: NoiseProfile,
}

impl Agent for Noise {
    fn kind(&self) -> AgentKind {
        AgentKind::Noise
    }

    fn act(&mut self, view: &AgentView, rng: &mut ChaCha8Rng) -> Vec<Action> {
        let pr = &self.profile;
        match view.positions.first() {
            None => match pr.wants_open(rng) {
                Some(side) => vec![Action::Open { side, collateral: pr.stake * view.wallet, leverage: pr.leverage }],
                None => vec![],
            },
            Some(p) if pr.wants_close(p.pnl(view.model), p.collateral, rng) => close_all(view),
            Some(_) => vec![],
        }
    }
}

/// Trades the gap between the mark and a reference it expects the mark to reach:
/// the feed's true volatility (`Informed`) or the uncapped target (`Chaser`).
pub struct Informed {
    /// `Informed` or `Chaser`.
    pub kind: AgentKind,
    /// Minimum |ln(true_model / model)| edge to enter.
    pub threshold: f64,
    pub collateral_frac: f64,
    pub leverage: f64,
}

impl Agent for Informed {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    fn act(&mut self, view: &AgentView, _rng: &mut ChaCha8Rng) -> Vec<Action> {
        let reference = if self.kind == AgentKind::Chaser { Some(view.target) } else { view.true_model };
        let Some(reference) = reference else { return vec![] };
        trade_gap((reference / view.model).ln(), self.threshold, self.collateral_frac, self.leverage, view)
    }
}

/// Opens toward `gap` (ln reference/mark) beyond `threshold`; closes once it shrinks below a quarter.
fn trade_gap(gap: f64, threshold: f64, collateral_frac: f64, leverage: f64, view: &AgentView) -> Vec<Action> {
    match view.positions.first() {
        None if gap.abs() > threshold => {
            let side = if gap > 0.0 { Side::Long } else { Side::Short };
            vec![Action::Open { side, collateral: view.wallet * collateral_frac, leverage }]
        }
        Some(p) if gap * p.side.sign() < threshold / 4.0 => close_all(view),
        _ => vec![],
    }
}

/// Public-data arbitrageur: runs its own faster realized-vol estimate on the
/// same bars the oracle sees and trades toward the model price it implies.
pub struct FastVol {
    vol: EwmaVol,
    band_x: f64,
    min_time: f64,
    max_time: f64,
    pub threshold: f64,
    pub collateral_frac: f64,
    pub leverage: f64,
}

impl FastVol {
    pub fn new(half_life: f64, init_sigma: f64, band_x: f64, min_time: f64, max_time: f64) -> Self {
        Self {
            vol: EwmaVol::new(half_life, init_sigma),
            band_x,
            min_time,
            max_time,
            threshold: 0.25,
            collateral_frac: 0.25,
            leverage: 3.0,
        }
    }
}

impl Agent for FastVol {
    fn kind(&self) -> AgentKind {
        AgentKind::FastVol
    }

    fn observe(&mut self, bar: &Bar) {
        self.vol.update(bar);
    }

    fn act(&mut self, view: &AgentView, _rng: &mut ChaCha8Rng) -> Vec<Action> {
        let own = model_time_clamped(self.band_x, self.vol.sigma(), self.min_time, self.max_time);
        trade_gap((own / view.model).ln(), self.threshold, self.collateral_frac, self.leverage, view)
    }
}

/// Follows the trend of the model price using fast/slow EMAs of its log.
pub struct Momentum {
    pub fast_alpha: f64,
    pub slow_alpha: f64,
    pub threshold: f64,
    pub collateral_frac: f64,
    pub leverage: f64,
    emas: Option<(f64, f64)>,
}

impl Momentum {
    pub fn new(fast_alpha: f64, slow_alpha: f64, threshold: f64, collateral_frac: f64, leverage: f64) -> Self {
        Self { fast_alpha, slow_alpha, threshold, collateral_frac, leverage, emas: None }
    }
}

impl Agent for Momentum {
    fn kind(&self) -> AgentKind {
        AgentKind::Momentum
    }

    fn act(&mut self, view: &AgentView, _rng: &mut ChaCha8Rng) -> Vec<Action> {
        let lm = view.model.ln();
        let (fast, slow) = self.emas.unwrap_or((lm, lm));
        let (fast, slow) = (fast + self.fast_alpha * (lm - fast), slow + self.slow_alpha * (lm - slow));
        self.emas = Some((fast, slow));
        let signal = fast - slow;
        match view.positions.first() {
            None if signal.abs() > self.threshold => {
                let side = if signal > 0.0 { Side::Long } else { Side::Short };
                vec![Action::Open { side, collateral: view.wallet * self.collateral_frac, leverage: self.leverage }]
            }
            Some(p) if signal * p.side.sign() < 0.0 => close_all(view),
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn view(model: f64, true_model: Option<f64>, positions: &[Position]) -> AgentView<'_> {
        AgentView { t: 0.0, model, true_model, target: model, wallet: 1_000.0, positions }
    }

    #[test]
    fn noise_profiles_are_heterogeneous_and_in_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        let ps: Vec<NoiseProfile> = (0..200).map(|_| NoiseProfile::sample(&mut rng)).collect();
        for p in &ps {
            let per_day = p.open_prob * 1_440.0;
            let hold_h = 1.0 / (p.close_prob * 60.0);
            assert!((0.2..=2.0).contains(&per_day) && (2.0..=72.0).contains(&hold_h), "{p:?}");
            assert!([1.0, 2.0, 3.0, 5.0].contains(&p.leverage));
        }
        assert!(ps.iter().any(|p| p.leverage == 1.0) && ps.iter().any(|p| p.leverage == 5.0));
        assert_ne!(ps[0], ps[1]);
    }

    #[test]
    fn noise_stops_out_and_takes_profit() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let p = NoiseProfile {
            open_prob: 0.0,
            close_prob: 0.0,
            leverage: 2.0,
            stake: 0.2,
            stop_loss: 0.5,
            take_profit: 1.0,
        };
        assert!(!p.wants_close(-40.0, 100.0, &mut rng));
        assert!(p.wants_close(-50.0, 100.0, &mut rng));
        assert!(p.wants_close(100.0, 100.0, &mut rng));
    }

    #[test]
    fn informed_goes_long_when_true_time_is_longer() {
        let mut a = Informed { kind: AgentKind::Informed, threshold: 0.1, collateral_frac: 0.5, leverage: 2.0 };
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        assert!(a.act(&view(1_000.0, Some(1_050.0), &[]), &mut rng).is_empty());
        let acts = a.act(&view(1_000.0, Some(2_000.0), &[]), &mut rng);
        assert_eq!(acts, vec![Action::Open { side: Side::Long, collateral: 500.0, leverage: 2.0 }]);
    }

    #[test]
    fn momentum_follows_rising_model() {
        let mut a = Momentum::new(0.5, 0.05, 0.05, 0.5, 2.0);
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let mut acts = vec![];
        for m in [1_000.0, 1_100.0, 1_200.0, 1_300.0] {
            acts = a.act(&view(m, None, &[]), &mut rng);
        }
        assert!(matches!(acts[..], [Action::Open { side: Side::Long, .. }]));
    }
}
