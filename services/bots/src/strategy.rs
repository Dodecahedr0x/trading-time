//! Trader personas. Pure: `decide` maps an observation of the market and the bot's account (plus the
//! bot's own memory, `BotState`) to an action.
//!
//! Bots act rarely (every 30–120 s on average) and hold positions for persona-specific periods, so
//! turnover stays a few times deposits per hour instead of churning fees and spread every tick.

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
    /// Sees the price driver's volatility regime: short time in storms (moves get faster), long time
    /// in the calm after one, held until the regime flips. Makers' fair value lags the regime.
    Informed,
}

/// Tick length assumed when an observation does not say (the sim's `BOT_TICK_MS=1500`).
pub const DEFAULT_TICK_SECS: f64 = 1.5;

impl Persona {
    pub const ALL: [Persona; 6] =
        [Persona::Noise, Persona::Momentum, Persona::MeanRevert, Persona::Whale, Persona::LimitTrader, Persona::Informed];

    /// Persona mix: mostly noise, 2 of every 12 informed, at least one of each once n >= 6.
    pub fn assign(i: usize) -> Persona {
        const CYCLE: [Persona; 12] = [
            Persona::Noise,
            Persona::Momentum,
            Persona::MeanRevert,
            Persona::LimitTrader,
            Persona::Whale,
            Persona::Informed,
            Persona::Noise,
            Persona::Momentum,
            Persona::Noise,
            Persona::MeanRevert,
            Persona::Informed,
            Persona::Noise,
        ];
        CYCLE[i % CYCLE.len()]
    }

    /// Mean seconds between actions (a Poisson clock independent of the tick length).
    pub fn mean_interval_secs(self) -> f64 {
        match self {
            Persona::Informed => 30.0,
            Persona::Momentum | Persona::MeanRevert => 45.0,
            Persona::Noise | Persona::LimitTrader => 60.0,
            Persona::Whale => 120.0,
        }
    }

    /// Probability of acting on a tick lasting `tick_secs`.
    pub fn activity(self, tick_secs: f64) -> f64 {
        (1.0 - (-tick_secs / self.mean_interval_secs()).exp()).clamp(0.0, 1.0)
    }

    /// Holding period range in seconds; `None` means "until the regime changes" (informed).
    pub fn hold_range_secs(self) -> Option<(f64, f64)> {
        match self {
            Persona::Noise => Some((120.0, 1_200.0)),
            Persona::Momentum | Persona::MeanRevert | Persona::LimitTrader => Some((300.0, 1_800.0)),
            Persona::Whale => Some((1_800.0, 7_200.0)),
            Persona::Informed => None,
        }
    }

    fn hold_secs(self, rng: &mut ChaCha8Rng) -> f64 {
        self.hold_range_secs().map_or(f64::INFINITY, |(lo, hi)| rng.random_range(lo..hi))
    }

    pub fn name(self) -> &'static str {
        match self {
            Persona::Noise => "noise",
            Persona::Momentum => "momentum",
            Persona::MeanRevert => "mean-revert",
            Persona::Whale => "whale",
            Persona::LimitTrader => "limit",
            Persona::Informed => "informed",
        }
    }
}

/// What a bot sees each tick. Money in USDC, prices in seconds.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    /// Bot clock in seconds (monotonic; any origin).
    pub now_secs: f64,
    /// Seconds since the previous observation; <= 0 means `DEFAULT_TICK_SECS`.
    pub tick_secs: f64,
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
    /// Price driver volatility regime (true = storm); `None` when no driver runs.
    pub storm: Option<bool>,
}

/// Per-bot memory carried between ticks.
#[derive(Clone, Debug, Default)]
pub struct BotState {
    /// No close or flip before this time while a position is open.
    pub hold_until: Option<f64>,
    last_storm: Option<bool>,
    regime_since: f64,
    /// Informed reaction delay after a regime change.
    react_lag: f64,
    seen_storm: bool,
}

impl BotState {
    fn observe_regime(&mut self, storm: bool, now: f64, rng: &mut ChaCha8Rng) {
        if self.last_storm == Some(storm) {
            return;
        }
        self.last_storm = Some(storm);
        self.regime_since = now;
        self.react_lag = rng.random_range(10.0..40.0);
        self.seen_storm |= storm;
        // A position held "until the regime changes" now has a short deadline.
        if let Some(h) = self.hold_until {
            self.hold_until = Some(h.min(now + self.react_lag));
        }
    }

    /// Side the informed persona wants: short in storms, long in the calm after one.
    fn informed_side(&self) -> Option<Side> {
        match self.last_storm? {
            true => Some(Side::Short),
            false => self.seen_storm.then_some(Side::Long),
        }
    }

    fn reacted(&self, now: f64) -> bool {
        now - self.regime_since >= self.react_lag
    }
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

fn momentum_signal(obs: &Observation) -> Option<Side> {
    let t = trend(&obs.mark_history);
    (t.abs() >= 0.002).then_some(if t > 0.0 { Side::Long } else { Side::Short })
}

fn mean_revert_signal(obs: &Observation) -> Option<Side> {
    if obs.index_secs <= 0.0 {
        return None;
    }
    let gap = (obs.mark_secs / obs.index_secs).ln();
    // Mark above index: funding pays shorts and pulls the mark down.
    (gap.abs() >= 0.01).then_some(if gap > 0.0 { Side::Short } else { Side::Long })
}

pub fn decide(p: Persona, obs: &Observation, st: &mut BotState, starting_equity: f64, rng: &mut ChaCha8Rng) -> Option<Action> {
    let now = obs.now_secs;
    // Bookkeeping runs every tick so regime changes and fills are never missed.
    if let Some(storm) = obs.storm {
        st.observe_regime(storm, now, rng);
    }
    let holding = obs.q_usdc.abs() >= MIN_TRADE_USDC;
    let pos_side = if obs.q_usdc > 0.0 { Side::Long } else { Side::Short };
    if !holding {
        st.hold_until = None;
    } else if st.hold_until.is_none() {
        // Position we did not open this session (restart, limit fill): start a hold now.
        st.hold_until = Some(match p {
            Persona::Informed if st.informed_side() != Some(pos_side) => st.regime_since + st.react_lag,
            _ => now + p.hold_secs(rng),
        });
    }

    let tick = if obs.tick_secs > 0.0 { obs.tick_secs } else { DEFAULT_TICK_SECS };
    if !rng.random_bool(p.activity(tick)) {
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

    let action = if holding {
        if st.hold_until.is_some_and(|h| now < h) {
            // Inside the holding period: no close, no flip, no add.
            return (p == Persona::LimitTrader && obs.open_orders >= 3).then_some(Action::CancelOrders);
        }
        // Hold expired: signal personas re-check and may extend; the rest close.
        let keep = match p {
            Persona::Momentum => momentum_signal(obs) == Some(pos_side),
            Persona::MeanRevert => mean_revert_signal(obs) == Some(pos_side),
            Persona::Informed => st.informed_side() == Some(pos_side),
            Persona::Noise | Persona::Whale | Persona::LimitTrader => false,
        };
        if keep {
            st.hold_until = Some(now + p.hold_secs(rng));
            return None;
        }
        Some(Action::Close)
    } else {
        match p {
            Persona::Noise => {
                let side = side_of(rng);
                sized(obs, side, frac(rng, 0.02, 0.4))
            }
            Persona::Momentum => momentum_signal(obs).and_then(|s| sized(obs, s, frac(rng, 0.1, 0.5))),
            Persona::MeanRevert => mean_revert_signal(obs).and_then(|s| sized(obs, s, frac(rng, 0.1, 0.4))),
            Persona::Whale => {
                let side = side_of(rng);
                sized(obs, side, frac(rng, 0.5, 0.85))
            }
            Persona::Informed => {
                st.informed_side().filter(|_| st.reacted(now)).and_then(|s| sized(obs, s, frac(rng, 0.15, 0.4)))
            }
            Persona::LimitTrader => {
                if obs.open_orders >= 3 {
                    return Some(Action::CancelOrders);
                }
                let side = side_of(rng);
                let size = (capacity(obs, side) * frac(rng, 0.05, 0.3)).floor();
                if size < MIN_TRADE_USDC {
                    return None;
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
    };
    match action {
        Some(Action::Market { .. }) => st.hold_until = Some(now + p.hold_secs(rng)),
        Some(Action::Close) => st.hold_until = None,
        _ => {}
    }
    action
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
            tick_secs: 60.0,
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

    fn apply(o: &mut Observation, a: &Action) {
        match *a {
            Action::Market { side: Side::Long, size_usdc } => o.q_usdc += size_usdc,
            Action::Market { side: Side::Short, size_usdc } => o.q_usdc -= size_usdc,
            Action::Close => o.q_usdc = 0.0,
            _ => {}
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
            let mut st = BotState::default();
            let mut markets = 0;
            for i in 0..2_000 {
                let mut o = obs(if i % 3 == 0 { 400.0 } else { 0.0 }, 1_000.0);
                o.now_secs = i as f64 * 100.0;
                o.storm = Some((i / 50) % 2 == 0);
                o.mark_history = vec![5_000.0, 5_200.0, 5_400.0 + i as f64];
                o.index_secs = 5_000.0;
                match decide(p, &o, &mut st, 1_000.0, &mut rng) {
                    Some(Action::Market { side, size_usdc }) => {
                        markets += 1;
                        assert!(size_usdc >= MIN_TRADE_USDC && size_usdc <= capacity(&o, side), "{p:?} {size_usdc}");
                    }
                    Some(Action::Limit { side, size_usdc, limit_secs, ttl_secs }) => {
                        assert!(size_usdc >= MIN_TRADE_USDC && size_usdc <= capacity(&o, side) && ttl_secs >= 120);
                        assert!(match side { Side::Long => limit_secs < o.mark_secs, Side::Short => limit_secs > o.mark_secs });
                    }
                    _ => {}
                }
            }
            assert!(p == Persona::LimitTrader || markets > 0, "{p:?} never traded");
        }
    }

    #[test]
    fn signals_drive_direction() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let (mut s1, mut s2) = (BotState::default(), BotState::default());
        let mut up = obs(0.0, 10_000.0);
        up.mark_history = vec![5_000.0, 5_100.0, 5_300.0];
        let mut rich = obs(0.0, 10_000.0);
        rich.mark_secs = 6_000.0;
        for _ in 0..500 {
            if let Some(Action::Market { side, .. }) = decide(Persona::Momentum, &up, &mut s1, 10_000.0, &mut rng) {
                assert_eq!(side, Side::Long);
            }
            if let Some(Action::Market { side, .. }) = decide(Persona::MeanRevert, &rich, &mut s2, 10_000.0, &mut rng) {
                assert_eq!(side, Side::Short);
            }
        }
    }

    #[test]
    fn bankroll_tops_up_when_broke_and_waits_without_quotes() {
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let mut st = BotState::default();
        let broke = obs(0.0, 100.0);
        let acts: Vec<_> = (0..200).filter_map(|_| decide(Persona::Noise, &broke, &mut st, 1_000.0, &mut rng)).collect();
        assert!(!acts.is_empty() && acts.iter().all(|a| matches!(a, Action::Deposit { usdc } if *usdc == 500.0)));
        let mut quiet = obs(0.0, 1_000.0);
        quiet.has_quotes = false;
        assert!((0..200).all(|_| decide(Persona::Noise, &quiet, &mut st, 1_000.0, &mut rng).is_none()));
    }

    #[test]
    fn hold_periods_are_respected() {
        for p in [Persona::Noise, Persona::Momentum, Persona::MeanRevert, Persona::Whale] {
            let (lo, hi) = p.hold_range_secs().unwrap();
            let mut rng = ChaCha8Rng::seed_from_u64(11);
            let mut st = BotState::default();
            let mut o = obs(0.0, 10_000.0);
            o.tick_secs = DEFAULT_TICK_SECS;
            // Signals that later flip, so signal personas would like to exit.
            o.mark_history = vec![5_000.0, 5_100.0, 5_300.0];
            o.index_secs = 5_000.0;
            let mut opened_at = None;
            let mut hold_end = 0.0;
            let mut closed = false;
            for k in 0..20_000 {
                o.now_secs = k as f64 * DEFAULT_TICK_SECS;
                if opened_at.is_some() {
                    o.mark_history.reverse();
                    o.index_secs = if o.index_secs == 5_000.0 { 6_000.0 } else { 5_000.0 };
                }
                let Some(a) = decide(p, &o, &mut st, 10_000.0, &mut rng) else { continue };
                match (&a, opened_at) {
                    (Action::Market { .. }, None) => {
                        opened_at = Some(o.now_secs);
                        hold_end = st.hold_until.unwrap();
                        let h = hold_end - o.now_secs;
                        assert!((lo..hi).contains(&h), "{p:?} hold {h}");
                    }
                    (Action::Close, Some(t0)) => {
                        assert!(o.now_secs >= hold_end && o.now_secs - t0 >= lo, "{p:?} closed after {}", o.now_secs - t0);
                        closed = true;
                        break;
                    }
                    (Action::Market { .. }, Some(_)) => panic!("{p:?} traded while holding: {a:?}"),
                    _ => {}
                }
                apply(&mut o, &a);
            }
            assert!(closed, "{p:?} never completed a round trip");
        }
    }

    #[test]
    fn informed_side_follows_regime() {
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        let mut st = BotState::default();
        let mut o = obs(0.0, 10_000.0);
        o.tick_secs = DEFAULT_TICK_SECS;
        let mut t = 0.0;
        let mut run = |o: &mut Observation, st: &mut BotState, storm: bool, secs: f64| {
            o.storm = Some(storm);
            let mut acts = Vec::new();
            let end = t + secs;
            while t < end {
                o.now_secs = t;
                if let Some(a) = decide(Persona::Informed, o, st, 10_000.0, &mut rng) {
                    apply(o, &a);
                    acts.push(a);
                }
                t += DEFAULT_TICK_SECS;
            }
            acts
        };
        // Calm with no storm seen yet: no view, no trades.
        assert!(run(&mut o, &mut st, false, 600.0).is_empty());
        // Storm: short time, then hold for the whole regime.
        let storm = run(&mut o, &mut st, true, 900.0);
        assert!(matches!(storm.as_slice(), [Action::Market { side: Side::Short, .. }]), "{storm:?}");
        assert!(o.q_usdc < 0.0);
        // Calm after the storm: close the short, go long, hold.
        let calm = run(&mut o, &mut st, false, 900.0);
        assert!(matches!(calm.as_slice(), [Action::Close, Action::Market { side: Side::Long, .. }]), "{calm:?}");
        assert!(o.q_usdc > 0.0);
        // Next storm flips it back.
        let storm = run(&mut o, &mut st, true, 900.0);
        assert!(matches!(storm.as_slice(), [Action::Close, Action::Market { side: Side::Short, .. }]), "{storm:?}");
    }

    /// One simulated hour of 12 bots at a 1.5 s tick: turnover a few times deposits, not 27×.
    #[test]
    fn one_hour_sim_turnover_within_target() {
        const N: usize = 12;
        let mut rng = ChaCha8Rng::seed_from_u64(2026);
        let ticks = (3_600.0 / DEFAULT_TICK_SECS) as usize;
        let personas: Vec<Persona> = (0..N).map(Persona::assign).collect();
        let equity: Vec<f64> = personas.iter().map(|&p| if p == Persona::Whale { 150_000.0 } else { 50_000.0 }).collect();
        let deposits: f64 = equity.iter().sum();
        let mut states = vec![BotState::default(); N];
        let mut q = [0.0f64; N];
        let mut orders = [0usize; N];
        let mut actions = [0u32; N];
        let mut trades = [0u32; N];
        let mut turnover = 0.0;
        let (mut mark, index, mut storm) = (5_400.0f64, 5_400.0, false);
        let mut history: Vec<f64> = Vec::new();
        for k in 0..ticks {
            // Regime switches ~0.02 per 10 s Pyth step; time falls faster in storms.
            if rng.random_bool(0.02 * DEFAULT_TICK_SECS / 10.0) {
                storm = !storm;
            }
            let vol: f64 = if storm { 0.004 } else { 0.001 };
            mark *= (rng.random_range(-vol..vol) - if storm { 0.0005 } else { -0.0002 }).exp();
            history.push(mark);
            if history.len() > 30 {
                history.remove(0);
            }
            for i in 0..N {
                let o = Observation {
                    now_secs: k as f64 * DEFAULT_TICK_SECS,
                    tick_secs: DEFAULT_TICK_SECS,
                    mark_secs: mark,
                    index_secs: index,
                    mark_history: history.clone(),
                    q_usdc: q[i],
                    equity_usdc: equity[i],
                    open_orders: orders[i],
                    wallet_usdc: equity[i],
                    has_quotes: true,
                    storm: Some(storm),
                    ..Default::default()
                };
                let Some(a) = decide(personas[i], &o, &mut states[i], equity[i], &mut rng) else { continue };
                actions[i] += 1;
                match a {
                    Action::Market { side, size_usdc } => {
                        assert!(size_usdc <= capacity(&o, side));
                        q[i] += if side == Side::Long { size_usdc } else { -size_usdc };
                        turnover += size_usdc;
                        trades[i] += 1;
                    }
                    Action::Close => {
                        turnover += q[i].abs();
                        q[i] = 0.0;
                        trades[i] += 1;
                    }
                    Action::Limit { side, size_usdc, .. } => {
                        // A third of resting orders fill.
                        if rng.random_bool(0.33) {
                            q[i] += if side == Side::Long { size_usdc } else { -size_usdc };
                            turnover += size_usdc;
                            trades[i] += 1;
                        } else {
                            orders[i] += 1;
                        }
                    }
                    Action::CancelOrders => orders[i] = 0,
                    Action::Deposit { .. } | Action::Withdraw { .. } => {}
                }
            }
        }
        let ratio = turnover / deposits;
        let max_trades = *trades.iter().max().unwrap();
        let max_actions = *actions.iter().max().unwrap();
        let total: u32 = trades.iter().sum();
        assert!((0.5..=6.0).contains(&ratio), "turnover {ratio:.2}x deposits");
        // At most one action per 30 s on average, and no bot near the old 2,000 trades/hour.
        assert!(max_actions <= 120 && max_trades <= 60, "max actions {max_actions}, trades {max_trades}");
        assert!(total >= 2 * N as u32, "too quiet: {total} trades");
    }
}
