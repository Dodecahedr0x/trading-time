//! Step loop tying feed, oracle, band, pool and agents together.

use crate::agents::{Action, Agent, AgentKind, AgentView, FastVol, Informed, Momentum, Noise, NoiseProfile};
use crate::band::BandTracker;
use crate::feed::PriceFeed;
use crate::implied::{ImpliedConfig, ImpliedVol, blend_sigma};
use crate::metrics::{Calibration, KindPnl, Summary};
use crate::oracle::{cap_move, model_time_clamped, sigma_per_sec};
use crate::pool::{Pool, PoolConfig, Position, Quote, SettleKind, Settlement, Side};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Debug)]
pub struct AgentMix {
    pub noise: usize,
    pub informed: usize,
    pub momentum: usize,
    /// Trade toward the uncapped target when the mark is capped.
    pub chaser: usize,
    /// Public-data arbitrageurs with faster vol estimates (15 min, 1 h, ...).
    pub fast: usize,
}

#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Band half-width, e.g. 0.01 = ±1%.
    pub band_x: f64,
    /// Hits averaged by the realized index.
    pub index_window: usize,
    pub ewma_half_life: f64,
    /// Starting volatility estimate per √second.
    pub init_sigma: f64,
    pub min_time: f64,
    pub max_time: f64,
    /// Max |ln| change of the model price per minute; `None` = uncapped.
    pub max_move_per_min: Option<f64>,
    /// Fill trades at the worse of the capped mark and the uncapped target.
    pub worst_of_pricing: bool,
    /// Weight of implied vol in the oracle's variance (0 = realized only).
    /// Ignored when the feed has no implied vol (CSV).
    pub iv_weight: f64,
    pub implied: ImpliedConfig,
    pub pool: PoolConfig,
    pub lp_liquidity: f64,
    /// Bars before agents start trading, letting the EWMA settle.
    pub warmup_bars: u64,
    pub user_wallet: f64,
    pub agent_wallet: f64,
    pub agents: AgentMix,
    pub seed: u64,
    /// Snapshots kept for charts.
    pub history_len: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            band_x: 0.01,
            index_window: 20,
            ewma_half_life: 4.0 * 3_600.0,
            init_sigma: sigma_per_sec(0.8),
            min_time: 60.0,
            max_time: 7.0 * 86_400.0,
            max_move_per_min: None,
            worst_of_pricing: true,
            iv_weight: 0.0,
            implied: ImpliedConfig::default(),
            pool: PoolConfig::default(),
            lp_liquidity: 1_000_000.0,
            warmup_bars: 240,
            user_wallet: 10_000.0,
            agent_wallet: 20_000.0,
            agents: AgentMix { noise: 20, informed: 2, momentum: 3, chaser: 2, fast: 2 },
            seed: 1,
            history_len: 1_500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub t: f64,
    pub price: f64,
    pub upper: f64,
    pub lower: f64,
    pub model: f64,
    pub target: f64,
    pub implied_model: Option<f64>,
    pub true_model: Option<f64>,
    pub index: Option<f64>,
    pub lp_equity: f64,
    pub long_oi: f64,
    pub short_oi: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Hit { duration: f64, predicted: f64, up: bool },
    Opened { owner: usize, agent: AgentKind, side: Side, notional: f64, price: f64 },
    Closed { owner: usize, agent: AgentKind, payout: f64, pnl: f64 },
    Liquidated { owner: usize, agent: AgentKind, payout: f64 },
    Deleveraged { owner: usize, agent: AgentKind, payout: f64 },
    Rejected { owner: usize, agent: AgentKind, reason: String },
    Shock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub t: f64,
    pub kind: EventKind,
}

pub struct Slot {
    pub kind: AgentKind,
    pub wallet: f64,
    pub initial: f64,
    /// Notional opened plus closed.
    pub volume: f64,
    agent: Option<Box<dyn Agent>>,
}

/// Owner id of the interactive user.
pub const USER: usize = 0;

pub struct Sim {
    pub cfg: SimConfig,
    feed: Box<dyn PriceFeed>,
    vol: crate::vol::EwmaVol,
    iv: ImpliedVol,
    /// Latest implied vol per √second, when the feed provides one.
    pub implied_sigma: Option<f64>,
    pub band: BandTracker,
    pub pool: Pool,
    pub slots: Vec<Slot>,
    rng: ChaCha8Rng,
    pub t: f64,
    pub price: f64,
    pub model: f64,
    /// Uncapped model price; equals `model` when no cap is set.
    pub target: f64,
    pub steps: u64,
    model_at_ref: f64,
    pub calibration: Calibration,
    pub events: VecDeque<Event>,
    pub history: VecDeque<Snapshot>,
    pub liquidations: u64,
    pub adl: u64,
    pub trades: u64,
    pub rejected: u64,
    pub max_skew_ratio: f64,
    pub skew_pnl: f64,
    gap_sum: f64,
    wide_gaps: u64,
    initial_cash: f64,
    start_t: f64,
}

const MAX_EVENTS: usize = 500;

impl Sim {
    pub fn new(cfg: SimConfig, mut feed: Box<dyn PriceFeed>) -> Result<Self, String> {
        let first = feed.next_bar().ok_or("feed is empty")?;
        let mut vol = crate::vol::EwmaVol::new(cfg.ewma_half_life, cfg.init_sigma);
        vol.update(&first);
        let model = model_time_clamped(cfg.band_x, vol.sigma(), cfg.min_time, cfg.max_time);

        let mut slots = vec![Slot {
            kind: AgentKind::User,
            wallet: cfg.user_wallet,
            initial: cfg.user_wallet,
            volume: 0.0,
            agent: None,
        }];
        let mix = &cfg.agents;
        let mut profiles = ChaCha8Rng::seed_from_u64(cfg.seed ^ crate::book_sim::PROFILE_SEED);
        let agents = (0..mix.noise)
            .map(|_| Box::new(Noise { profile: NoiseProfile::sample(&mut profiles) }) as Box<dyn Agent>)
            .chain((0..mix.informed).map(|_| {
                let a = Informed { kind: AgentKind::Informed, threshold: 0.25, collateral_frac: 0.25, leverage: 3.0 };
                Box::new(a) as Box<dyn Agent>
            }))
            .chain((0..mix.chaser).map(|_| {
                let a = Informed { kind: AgentKind::Chaser, threshold: 0.05, collateral_frac: 0.25, leverage: 3.0 };
                Box::new(a) as Box<dyn Agent>
            }))
            .chain((0..mix.fast).map(|i| {
                let half_life = 900.0 * 4f64.powi(i as i32);
                let a = FastVol::new(half_life, cfg.init_sigma, cfg.band_x, cfg.min_time, cfg.max_time);
                Box::new(a) as Box<dyn Agent>
            }))
            .chain((0..mix.momentum).enumerate().map(|(i, _)| {
                let fast = 0.05 / (i + 1) as f64;
                Box::new(Momentum::new(fast, fast / 10.0, 0.15, 0.25, 2.0)) as Box<dyn Agent>
            }));
        slots.extend(agents.map(|a| Slot {
            kind: a.kind(),
            wallet: cfg.agent_wallet,
            initial: cfg.agent_wallet,
            volume: 0.0,
            agent: Some(a),
        }));

        let pool = Pool::new(cfg.pool.clone(), cfg.lp_liquidity);
        let initial_cash = slots.iter().map(|s| s.wallet).sum::<f64>() + pool.liquidity;
        let mut sim = Self {
            band: BandTracker::new(cfg.band_x, cfg.index_window, first.t, first.close),
            rng: ChaCha8Rng::seed_from_u64(cfg.seed),
            iv: ImpliedVol::new(cfg.implied.clone(), cfg.seed ^ 0x1D_u64 << 40),
            implied_sigma: None,
            t: first.t,
            price: first.close,
            model,
            target: model,
            model_at_ref: model,
            steps: 0,
            calibration: Calibration::default(),
            events: VecDeque::new(),
            history: VecDeque::new(),
            liquidations: 0,
            adl: 0,
            trades: 0,
            rejected: 0,
            max_skew_ratio: 0.0,
            skew_pnl: 0.0,
            gap_sum: 0.0,
            wide_gaps: 0,
            initial_cash,
            start_t: first.t,
            cfg,
            feed,
            vol,
            pool,
            slots,
        };
        sim.record();
        Ok(sim)
    }

    pub fn true_model(&self) -> Option<f64> {
        self.feed.true_sigma().map(|s| model_time_clamped(self.cfg.band_x, s, self.cfg.min_time, self.cfg.max_time))
    }

    pub fn sigma_hat(&self) -> f64 {
        self.vol.sigma()
    }

    pub fn warmed_up(&self) -> bool {
        self.steps >= self.cfg.warmup_bars
    }

    /// Flips the feed's volatility regime.
    pub fn shock(&mut self) {
        self.feed.shock();
        self.push_event(EventKind::Shock);
    }

    /// Advances one bar. Returns false when the feed is exhausted.
    pub fn step(&mut self) -> bool {
        let Some(bar) = self.feed.next_bar() else { return false };
        let dt = bar.t - self.t;
        self.t = bar.t;
        self.price = bar.close;
        self.steps += 1;
        self.vol.update(&bar);
        self.implied_sigma = self.feed.true_sigma().map(|s| self.iv.update(dt.max(0.0), s));
        let sigma = match self.implied_sigma {
            Some(iv) => blend_sigma(self.vol.sigma(), iv, self.cfg.iv_weight),
            None => self.vol.sigma(),
        };
        self.target = model_time_clamped(self.cfg.band_x, sigma, self.cfg.min_time, self.cfg.max_time);
        let prev_model = self.model;
        self.model = match self.cfg.max_move_per_min {
            Some(cap) => cap_move(self.model, self.target, cap * dt.max(0.0) / 60.0),
            None => self.target,
        };
        self.skew_pnl -= self.pool.skew() * (self.model / prev_model).ln();
        let gap = if self.cfg.worst_of_pricing { (self.target / self.model).ln().abs() } else { 0.0 };
        self.gap_sum += gap;
        self.wide_gaps += u64::from(gap > 0.05);
        self.pool.accrue(dt.max(0.0));

        if let Some(hit) = self.band.on_bar(&bar) {
            let predicted = self.model_at_ref;
            if self.warmed_up() {
                self.calibration.record(predicted, hit.duration);
            }
            self.push_event(EventKind::Hit { duration: hit.duration, predicted, up: hit.up });
            self.model_at_ref = self.model;
        }

        for s in self.pool.liquidate_all(self.model) {
            let (owner, payout) = (s.position.owner, s.payout);
            let agent = self.slots[owner].kind;
            self.slots[owner].wallet += payout;
            if s.kind == SettleKind::Adl {
                self.adl += 1;
                self.push_event(EventKind::Deleveraged { owner, agent, payout });
            } else {
                self.liquidations += 1;
                self.push_event(EventKind::Liquidated { owner, agent, payout });
            }
        }

        for slot in &mut self.slots {
            if let Some(agent) = slot.agent.as_mut() {
                agent.observe(&bar);
            }
        }

        if self.warmed_up() {
            let true_model = self.true_model();
            for i in 1..self.slots.len() {
                let positions: Vec<Position> = self.pool.positions.values().filter(|p| p.owner == i).cloned().collect();
                let view = AgentView {
                    t: self.t,
                    model: self.model,
                    true_model,
                    target: self.target,
                    wallet: self.slots[i].wallet,
                    positions: &positions,
                };
                let agent = self.slots[i].agent.as_mut().expect("scripted slot");
                let actions = agent.act(&view, &mut self.rng);
                for mut a in actions {
                    // Scripted agents use the most leverage the pool allows, up to what they want.
                    if let Action::Open { leverage, .. } = &mut a {
                        *leverage = leverage.min(self.pool.cfg.max_leverage);
                    }
                    let _ = self.execute(i, a);
                }
            }
        }

        self.max_skew_ratio = self.max_skew_ratio.max(self.pool.skew().abs() / self.cfg.lp_liquidity);
        self.record();
        debug_assert!((self.total_cash() - self.initial_cash).abs() < 1e-6 * self.initial_cash, "cash not conserved");
        true
    }

    /// Executes an action for `owner` at the current model price.
    pub fn execute(&mut self, owner: usize, action: Action) -> Result<u64, String> {
        let agent = self.slots[owner].kind;
        match action {
            Action::Open { side, collateral, leverage } => {
                let collateral = collateral.min(self.slots[owner].wallet);
                if collateral <= 0.0 {
                    return Err("insufficient wallet".into());
                }
                match self.pool.open(owner, side, collateral, leverage, self.quote(), self.t) {
                    Ok(id) => {
                        self.slots[owner].wallet -= collateral;
                        self.slots[owner].volume += self.pool.positions[&id].entry_notional();
                        self.trades += 1;
                        let p = &self.pool.positions[&id];
                        let (notional, price) = (p.entry_notional(), p.entry);
                        self.push_event(EventKind::Opened { owner, agent, side, notional, price });
                        Ok(id)
                    }
                    Err(e) => {
                        self.rejected += 1;
                        self.push_event(EventKind::Rejected { owner, agent, reason: e.to_string() });
                        Err(e.to_string())
                    }
                }
            }
            Action::Close { id } => {
                if self.pool.positions.get(&id).is_none_or(|p| p.owner != owner) {
                    return Err("not your position".into());
                }
                let Settlement { position, payout, .. } =
                    self.pool.close(id, self.quote()).map_err(|e| e.to_string())?;
                self.slots[owner].wallet += payout;
                self.slots[owner].volume += position.entry_notional();
                self.trades += 1;
                let pnl = payout - position.collateral;
                self.push_event(EventKind::Closed { owner, agent, payout, pnl });
                Ok(id)
            }
        }
    }

    /// Prices trades fill against at this step.
    pub fn quote(&self) -> Quote {
        let target = if self.cfg.worst_of_pricing { self.target } else { self.model };
        Quote { mark: self.model, target }
    }

    fn push_event(&mut self, kind: EventKind) {
        self.events.push_back(Event { t: self.t, kind });
        if self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    fn record(&mut self) {
        self.history.push_back(Snapshot {
            t: self.t,
            price: self.price,
            upper: self.band.upper(),
            lower: self.band.lower(),
            model: self.model,
            target: self.target,
            implied_model: self
                .implied_sigma
                .map(|s| model_time_clamped(self.cfg.band_x, s, self.cfg.min_time, self.cfg.max_time)),
            true_model: self.true_model(),
            index: self.band.realized_index(),
            lp_equity: self.pool.lp_equity(self.model),
            long_oi: self.pool.long_oi,
            short_oi: self.pool.short_oi,
        });
        if self.history.len() > self.cfg.history_len {
            self.history.pop_front();
        }
    }

    fn total_cash(&self) -> f64 {
        self.slots.iter().map(|s| s.wallet).sum::<f64>() + self.pool.total_collateral() + self.pool.liquidity
    }

    /// Wallet plus equity of open positions at the model price.
    pub fn net_worth(&self, owner: usize) -> f64 {
        self.slots[owner].wallet
            + self
                .pool
                .positions
                .values()
                .filter(|p| p.owner == owner)
                .map(|p| self.pool.equity(p, self.model).max(0.0))
                .sum::<f64>()
    }

    pub fn summary(&self) -> Summary {
        let mut by_kind: Vec<KindPnl> = Vec::new();
        for (i, s) in self.slots.iter().enumerate() {
            KindPnl::add(&mut by_kind, s.kind, s.initial, self.net_worth(i), s.volume);
        }
        Summary {
            steps: self.steps,
            sim_days: (self.t - self.start_t) / 86_400.0,
            hits: self.band.total_hits,
            calibration: self.calibration.clone(),
            lp_start: self.cfg.lp_liquidity,
            lp_equity: self.pool.lp_equity(self.model),
            fees: self.pool.fees_collected,
            bad_debt: self.pool.bad_debt,
            liquidations: self.liquidations,
            adl: self.adl,
            trades: self.trades,
            rejected: self.rejected,
            max_skew_ratio: self.max_skew_ratio,
            skew_pnl: self.skew_pnl,
            mean_gap: self.gap_sum / self.steps.max(1) as f64,
            wide_gap_frac: self.wide_gaps as f64 / self.steps.max(1) as f64,
            by_kind,
        }
    }

    /// Hash of the key state, for determinism checks.
    pub fn state_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        for v in [self.t, self.price, self.model, self.pool.liquidity, self.pool.long_oi, self.pool.short_oi] {
            v.to_bits().hash(&mut h);
        }
        for s in &self.slots {
            s.wallet.to_bits().hash(&mut h);
        }
        self.trades.hash(&mut h);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{Model, SynthFeed};

    fn regime_sim(seed: u64) -> Sim {
        let feed = SynthFeed::new(
            Model::Regime { sigmas: vec![sigma_per_sec(0.5), sigma_per_sec(1.5)], switch_prob: 1.0 / 1_440.0 },
            seed,
            150.0,
            60.0,
        );
        Sim::new(SimConfig { seed, ..Default::default() }, Box::new(feed)).unwrap()
    }

    #[test]
    fn deterministic_for_same_seed() {
        let (mut a, mut b) = (regime_sim(5), regime_sim(5));
        for _ in 0..5_000 {
            a.step();
            b.step();
        }
        assert_eq!(a.state_hash(), b.state_hash());
        assert!(a.trades > 0);
        let mut c = regime_sim(6);
        for _ in 0..5_000 {
            c.step();
        }
        assert_ne!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn cash_conserved_over_long_run() {
        let mut s = regime_sim(7);
        for _ in 0..30_000 {
            s.step();
        }
        let total = s.total_cash();
        assert!((total - s.initial_cash).abs() < 1e-6 * s.initial_cash);
        assert!(s.band.total_hits > 50);
    }

    #[test]
    fn user_can_open_and_close() {
        let mut s = regime_sim(1);
        let id = s.execute(USER, Action::Open { side: Side::Long, collateral: 1_000.0, leverage: 2.0 }).unwrap();
        assert_eq!(s.slots[USER].wallet, 9_000.0);
        assert!(s.execute(1, Action::Close { id }).is_err(), "others cannot close it");
        s.execute(USER, Action::Close { id }).unwrap();
        assert!(s.slots[USER].wallet > 9_990.0);
    }

    #[test]
    fn shock_after_warmup_moves_model_toward_true() {
        let mut s = regime_sim(2);
        for _ in 0..2_000 {
            s.step();
        }
        s.shock();
        let target = s.true_model().unwrap();
        let before = (s.model / target).ln().abs();
        for _ in 0..200 {
            s.step();
        }
        // Regime may switch again randomly; only require progress when it did not.
        if s.true_model() == Some(target) {
            assert!((s.model / target).ln().abs() < before);
        }
    }
}
