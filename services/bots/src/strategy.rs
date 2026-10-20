//! Trader personas. Pure: `decide` maps an observation of the market and the bot's account to an action.

use rand::Rng;
use rand_chacha::ChaCha8Rng;
use tradingtime_math::Side;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persona {
    /// Random direction and size; the bulk of the flow.
    Noise,
    /// Follows the recent trend in mark time.
    Momentum,
    /// Fades mark away from the index (funding pulls it back).
    MeanRevert,
    /// Rare, large trades held for a long time.
    Whale,
    /// Rests limit orders a few percent away from the mark.
    LimitTrader,
}

impl Persona {
    pub const ALL: [Persona; 5] = [Persona::Noise, Persona::Momentum, Persona::MeanRevert, Persona::Whale, Persona::LimitTrader];

    /// Persona mix for `n` bots: mostly noise, at least one of each once n >= 5.
    pub fn assign(i: usize) -> Persona {
        const CYCLE: [Persona; 8] = [
            Persona::Noise,
            Persona::Momentum,
            Persona::MeanRevert,
            Persona::LimitTrader,
            Persona::Whale,
            Persona::Noise,
            Persona::Momentum,
            Persona::Noise,
        ];
        CYCLE[i % CYCLE.len()]
    }

    /// Probability of acting on a given tick.
    pub fn activity(self) -> f64 {
        match self {
            Persona::Noise => 0.25,
            Persona::Momentum | Persona::MeanRevert => 0.15,
            Persona::LimitTrader => 0.2,
            Persona::Whale => 0.03,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Persona::Noise => "noise",
            Persona::Momentum => "momentum",
            Persona::MeanRevert => "mean-revert",
            Persona::Whale => "whale",
            Persona::LimitTrader => "limit",
        }
    }
}

/// What a bot sees each tick. Money in USDC, prices in seconds.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub mark_secs: f64,
    pub index_secs: f64,
    /// Recent mark samples, oldest first.
    pub mark_history: Vec<f64>,
    /// Signed position (+ long time).
    pub q_usdc: f64,
    pub equity_usdc: f64,
    /// Exposure already committed (margin need − |q|) by resting orders.
    pub reserved_usdc: f64,
    pub open_orders: usize,
    pub wallet_usdc: f64,
    pub has_quotes: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Market { side: Side, size_usdc: f64 },
    Limit { side: Side, size_usdc: f64, limit_secs: f64, ttl_secs: i64 },
    Close,
    CancelOrders,
    Deposit { usdc: f64 },
    Withdraw { usdc: f64 },
}

/// Keeps a margin cushion below 1× for spread, fees and funding.
pub const USABLE_EQUITY: f64 = 0.9;
pub const MIN_TRADE_USDC: f64 = 25.0;

/// Largest size that keeps exposure within `USABLE_EQUITY` after trading `side`.
pub fn capacity(obs: &Observation, side: Side) -> f64 {
    let budget = obs.equity_usdc * USABLE_EQUITY - obs.reserved_usdc;
    let q = obs.q_usdc;
    let reducing = match side {
        Side::Long => (-q).max(0.0),
        Side::Short => q.max(0.0),
    };
    // Reducing up to |q| is free; beyond that the flip needs margin, as does any increase.
    reducing + (budget - if reducing > 0.0 { 0.0 } else { q.abs() }).max(0.0)
}

pub fn trend(history: &[f64]) -> f64 {
    match (history.first(), history.last()) {
        (Some(&a), Some(&b)) if a > 0.0 && b > 0.0 && history.len() >= 3 => (b / a).ln(),
        _ => 0.0,
    }
}

fn opposite(s: Side) -> Side {
    match s {
        Side::Long => Side::Short,
        Side::Short => Side::Long,
    }
}

fn side_of(rng: &mut ChaCha8Rng) -> Side {
    if rng.random_bool(0.5) { Side::Long } else { Side::Short }
}

/// Fraction of capacity, log-uniform in [lo, hi].
fn frac(rng: &mut ChaCha8Rng, lo: f64, hi: f64) -> f64 {
    (lo.ln() + rng.random::<f64>() * (hi.ln() - lo.ln())).exp()
}

fn sized(obs: &Observation, side: Side, f: f64) -> Option<Action> {
    let size = (capacity(obs, side) * f).floor();
    (size >= MIN_TRADE_USDC).then_some(Action::Market { side, size_usdc: size })
}

pub fn decide(p: Persona, obs: &Observation, starting_equity: f64, rng: &mut ChaCha8Rng) -> Option<Action> {
    if !rng.random_bool(p.activity()) {
        return None;
    }
    // Bankroll management before trading.
    if obs.equity_usdc < 0.25 * starting_equity && obs.wallet_usdc > MIN_TRADE_USDC && obs.q_usdc.abs() < 1.0 {
        return Some(Action::Deposit { usdc: (starting_equity * 0.5).min(obs.wallet_usdc).floor() });
    }
    if obs.equity_usdc > 2.0 * starting_equity && obs.q_usdc.abs() < 1.0 && obs.reserved_usdc < 1.0 && rng.random_bool(0.3) {
        return Some(Action::Withdraw { usdc: (obs.equity_usdc - starting_equity).floor() });
    }
    if !obs.has_quotes || obs.mark_secs <= 0.0 {
        return None;
    }
    let holding = obs.q_usdc.abs() >= MIN_TRADE_USDC;
    match p {
        Persona::Noise => {
            if holding && rng.random_bool(0.25) {
                return Some(Action::Close);
            }
            let side = side_of(rng);
            sized(obs, side, frac(rng, 0.02, 0.4))
        }
        Persona::Momentum => {
            let t = trend(&obs.mark_history);
            if t.abs() < 0.002 {
                return (holding && rng.random_bool(0.2)).then_some(Action::Close);
            }
            let side = if t > 0.0 { Side::Long } else { Side::Short };
            sized(obs, side, frac(rng, 0.1, 0.5))
        }
        Persona::MeanRevert => {
            if obs.index_secs <= 0.0 {
                return None;
            }
            let gap = (obs.mark_secs / obs.index_secs).ln();
            if gap.abs() < 0.01 {
                return (holding && rng.random_bool(0.3)).then_some(Action::Close);
            }
            // Mark above index: funding pays shorts and pulls the mark down.
            let side = if gap > 0.0 { Side::Short } else { Side::Long };
            sized(obs, side, frac(rng, 0.1, 0.4))
        }
        Persona::Whale => {
            if holding {
                return rng.random_bool(0.3).then_some(Action::Close);
            }
            let side = side_of(rng);
            sized(obs, side, frac(rng, 0.5, 0.85))
        }
        Persona::LimitTrader => {
            if obs.open_orders >= 3 {
                return Some(Action::CancelOrders);
            }
            let side = side_of(rng);
            let size = (capacity(obs, side) * frac(rng, 0.05, 0.3)).floor();
            if size < MIN_TRADE_USDC {
                return holding.then_some(Action::Close);
            }
            // Long time buys below the mark, short time sells above it.
            let away = frac(rng, 0.002, 0.03);
            let limit_secs = match side {
                Side::Long => obs.mark_secs * (-away).exp(),
                Side::Short => obs.mark_secs * away.exp(),
            };
            Some(Action::Limit { side, size_usdc: size, limit_secs, ttl_secs: rng.random_range(120..1_800) })
        }
    }
}

/// Side that closes a position of signed size `q`.
pub fn closing_side(q: f64) -> Side {
    opposite(if q > 0.0 { Side::Long } else { Side::Short })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn obs(q: f64, equity: f64) -> Observation {
        Observation {
            mark_secs: 5_400.0,
            index_secs: 5_400.0,
            mark_history: vec![5_400.0; 5],
            q_usdc: q,
            equity_usdc: equity,
            wallet_usdc: 100_000.0,
            has_quotes: true,
            ..Default::default()
        }
    }

    #[test]
    fn capacity_respects_one_x_and_netting() {
        assert_eq!(capacity(&obs(0.0, 1_000.0), Side::Long), 900.0);
        // Long 600: adding long leaves 300; going short can flip up to 600 + 900.
        assert_eq!(capacity(&obs(600.0, 1_000.0), Side::Long), 300.0);
        assert_eq!(capacity(&obs(600.0, 1_000.0), Side::Short), 1_500.0);
        let mut o = obs(0.0, 1_000.0);
        o.reserved_usdc = 400.0;
        assert_eq!(capacity(&o, Side::Short), 500.0);
    }

    #[test]
    fn trend_and_closing_side() {
        assert!(trend(&[100.0, 101.0, 110.0]) > 0.09);
        assert_eq!(trend(&[100.0, 101.0]), 0.0);
        assert_eq!(closing_side(10.0), Side::Short);
        assert_eq!(closing_side(-10.0), Side::Long);
    }

    #[test]
    fn personas_stay_within_capacity_and_minimums() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for p in Persona::ALL {
            for i in 0..2_000 {
                let mut o = obs(if i % 3 == 0 { 400.0 } else { 0.0 }, 1_000.0);
                o.mark_history = vec![5_000.0, 5_200.0, 5_400.0 + i as f64];
                o.index_secs = 5_000.0;
                match decide(p, &o, 1_000.0, &mut rng) {
                    Some(Action::Market { side, size_usdc }) => {
                        assert!(size_usdc >= MIN_TRADE_USDC && size_usdc <= capacity(&o, side), "{p:?} {size_usdc}");
                    }
                    Some(Action::Limit { side, size_usdc, limit_secs, ttl_secs }) => {
                        assert!(size_usdc <= capacity(&o, side) && ttl_secs >= 120);
                        assert!(match side { Side::Long => limit_secs < o.mark_secs, Side::Short => limit_secs > o.mark_secs });
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn signals_drive_direction() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let mut up = obs(0.0, 10_000.0);
        up.mark_history = vec![5_000.0, 5_100.0, 5_300.0];
        let mut rich = obs(0.0, 10_000.0);
        rich.mark_secs = 6_000.0;
        for _ in 0..500 {
            if let Some(Action::Market { side, .. }) = decide(Persona::Momentum, &up, 10_000.0, &mut rng) {
                assert_eq!(side, Side::Long);
            }
            if let Some(Action::Market { side, .. }) = decide(Persona::MeanRevert, &rich, 10_000.0, &mut rng) {
                assert_eq!(side, Side::Short);
            }
        }
    }

    #[test]
    fn bankroll_tops_up_when_broke_and_waits_without_quotes() {
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let broke = obs(0.0, 100.0);
        let acts: Vec<_> = (0..200).filter_map(|_| decide(Persona::Noise, &broke, 1_000.0, &mut rng)).collect();
        assert!(!acts.is_empty() && acts.iter().all(|a| matches!(a, Action::Deposit { usdc } if *usdc == 500.0)));
        let mut quiet = obs(0.0, 1_000.0);
        quiet.has_quotes = false;
        assert!((0..200).all(|_| decide(Persona::Noise, &quiet, 1_000.0, &mut rng).is_none()));
    }
}
