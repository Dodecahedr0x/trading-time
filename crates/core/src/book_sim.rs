//! Order-book venue simulation. Market makers quote, takers cross, and the
//! realized-vol oracle is only the funding index rather than the trade price.

use crate::agents::AgentKind;
use crate::band::BandTracker;
use crate::book::{Account, Book, BookConfig};
use crate::feed::{Bar, PriceFeed};
use crate::metrics::{Calibration, KindPnl};
use crate::oracle::{model_time_clamped, sigma_per_sec};
use crate::pool::Side;
use crate::vol::EwmaVol;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Debug, PartialEq)]
pub enum BookAction {
    Market {
        side: Side,
        notional: f64,
    },
    /// Replaces all of the agent's resting orders: (side, price, size).
    Quotes(Vec<(Side, f64, f64)>),
    CloseAll,
}

pub struct BookView<'a> {
    /// Oracle model price (funding index).
    pub index: f64,
    /// Book mid, or the index when a side is empty.
    pub mark: f64,
    pub true_model: Option<f64>,
    pub account: &'a Account,
    pub equity: f64,
}

pub trait BookAgent {
    fn kind(&self) -> AgentKind;
    /// Sees every bar, including during warmup.
    fn observe(&mut self, _bar: &Bar) {}
    fn act(&mut self, view: &BookView, rng: &mut ChaCha8Rng) -> Vec<BookAction>;
}

/// An agent's own realized-vol model price.
#[derive(Clone, Debug)]
pub struct Estimator {
    vol: EwmaVol,
    band_x: f64,
    min: f64,
    max: f64,
}

impl Estimator {
    pub fn new(half_life: f64, cfg: &BookSimConfig) -> Self {
        Self { vol: EwmaVol::new(half_life, cfg.init_sigma), band_x: cfg.band_x, min: cfg.min_time, max: cfg.max_time }
    }

    pub fn update(&mut self, bar: &Bar) {
        self.vol.update(bar);
    }

    pub fn model(&self) -> f64 {
        model_time_clamped(self.band_x, self.vol.sigma(), self.min, self.max)
    }
}

/// Quotes a ladder around a blend of the index and its own estimate, shifted
/// against its inventory, with a half-spread that widens after fast moves.
/// Stops quoting the side that would take inventory past `inventory_limit`.
pub struct MarketMaker {
    pub est: Estimator,
    pub index_weight: f64,
    pub half_spread: f64,
    pub levels: usize,
    pub step: f64,
    pub size: f64,
    /// ln-price shift per USD of inventory.
    pub skew: f64,
    pub inventory_limit: f64,
    /// Half-spread added per unit of recent mean |Δ fair| per bar.
    pub move_mult: f64,
    move_ema: f64,
    last: Option<f64>,
}

impl MarketMaker {
    pub fn new(est: Estimator, half_spread: f64, size: f64, inventory_limit: f64) -> Self {
        Self {
            est,
            index_weight: 0.5,
            half_spread,
            levels: 3,
            step: 0.005,
            size,
            // Shift the center by one base half-spread at the inventory limit.
            skew: half_spread / inventory_limit,
            inventory_limit,
            move_mult: 1.5,
            move_ema: 0.0,
            last: None,
        }
    }
}

impl BookAgent for MarketMaker {
    fn kind(&self) -> AgentKind {
        AgentKind::MarketMaker
    }

    fn observe(&mut self, bar: &Bar) {
        self.est.update(bar);
    }

    fn act(&mut self, view: &BookView, _rng: &mut ChaCha8Rng) -> Vec<BookAction> {
        let fair = self.index_weight * view.index.ln() + (1.0 - self.index_weight) * self.est.model().ln();
        if let Some(last) = self.last {
            self.move_ema += 0.05 * ((fair - last).abs() - self.move_ema);
        }
        self.last = Some(fair);
        if view.equity <= 0.0 {
            return vec![BookAction::Quotes(vec![])];
        }
        let q = view.account.q;
        let center = fair - self.skew * q;
        let half = self.half_spread + self.move_mult * self.move_ema;
        let mut quotes = Vec::new();
        let (mut bid_room, mut ask_room) = (self.inventory_limit - q, self.inventory_limit + q);
        for i in 0..self.levels {
            let off = half + i as f64 * self.step;
            let (bid, ask) = (self.size.min(bid_room), self.size.min(ask_room));
            if bid > 0.0 {
                quotes.push((Side::Long, (center - off).exp(), bid));
            }
            if ask > 0.0 {
                quotes.push((Side::Short, (center + off).exp(), ask));
            }
            bid_room -= bid;
            ask_room -= ask;
        }
        vec![BookAction::Quotes(quotes)]
    }
}

/// Random market orders, closed after a random holding time.
pub struct NoiseTaker {
    pub trade_prob: f64,
    pub close_prob: f64,
    pub max_collateral: f64,
    pub leverage: f64,
}

impl BookAgent for NoiseTaker {
    fn kind(&self) -> AgentKind {
        AgentKind::Noise
    }

    fn act(&mut self, view: &BookView, rng: &mut ChaCha8Rng) -> Vec<BookAction> {
        if view.account.q == 0.0 {
            if rng.random::<f64>() < self.trade_prob {
                let side = if rng.random::<bool>() { Side::Long } else { Side::Short };
                let notional = rng.random_range(0.2..1.0) * self.max_collateral.min(view.equity) * self.leverage;
                return vec![BookAction::Market { side, notional }];
            }
            return vec![];
        }
        if rng.random::<f64>() < self.close_prob { vec![BookAction::CloseAll] } else { vec![] }
    }
}

/// Trades the mark toward a reference: its own fast estimate (`FastVol`) or the
/// feed's true model (`Informed`, synthetic feeds only).
pub struct GapTaker {
    pub kind: AgentKind,
    pub est: Option<Estimator>,
    pub threshold: f64,
    pub collateral_frac: f64,
    pub leverage: f64,
}

impl BookAgent for GapTaker {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    fn observe(&mut self, bar: &Bar) {
        if let Some(e) = &mut self.est {
            e.update(bar);
        }
    }

    fn act(&mut self, view: &BookView, _rng: &mut ChaCha8Rng) -> Vec<BookAction> {
        let reference = match &self.est {
            Some(e) => e.model(),
            None => match view.true_model {
                Some(t) => t,
                None => return vec![],
            },
        };
        let gap = (reference / view.mark).ln();
        let q = view.account.q;
        if q == 0.0 && gap.abs() > self.threshold {
            let side = if gap > 0.0 { Side::Long } else { Side::Short };
            return vec![BookAction::Market { side, notional: view.equity * self.collateral_frac * self.leverage }];
        }
        if q != 0.0 && gap * q.signum() < self.threshold / 4.0 { vec![BookAction::CloseAll] } else { vec![] }
    }
}

/// Follows the trend of the mark with fast/slow EMAs of its log.
pub struct MomentumTaker {
    pub fast_alpha: f64,
    pub slow_alpha: f64,
    pub threshold: f64,
    pub collateral_frac: f64,
    pub leverage: f64,
    emas: Option<(f64, f64)>,
}

impl BookAgent for MomentumTaker {
    fn kind(&self) -> AgentKind {
        AgentKind::Momentum
    }

    fn act(&mut self, view: &BookView, _rng: &mut ChaCha8Rng) -> Vec<BookAction> {
        let lm = view.mark.ln();
        let (fast, slow) = self.emas.unwrap_or((lm, lm));
        let (fast, slow) = (fast + self.fast_alpha * (lm - fast), slow + self.slow_alpha * (lm - slow));
        self.emas = Some((fast, slow));
        let signal = fast - slow;
        let q = view.account.q;
        if q == 0.0 && signal.abs() > self.threshold {
            let side = if signal > 0.0 { Side::Long } else { Side::Short };
            return vec![BookAction::Market { side, notional: view.equity * self.collateral_frac * self.leverage }];
        }
        if q != 0.0 && signal * q.signum() < 0.0 { vec![BookAction::CloseAll] } else { vec![] }
    }
}

fn view(book: &Book, i: usize, index: f64, mark: f64, true_model: Option<f64>) -> BookView<'_> {
    let account = &book.accounts[i];
    BookView { index, mark, true_model, account, equity: account.equity(mark) }
}

#[derive(Clone, Debug)]
pub struct BookSimConfig {
    pub band_x: f64,
    pub index_window: usize,
    pub ewma_half_life: f64,
    pub init_sigma: f64,
    pub min_time: f64,
    pub max_time: f64,
    pub book: BookConfig,
    pub warmup_bars: u64,
    pub mm: usize,
    pub mm_wallet: f64,
    pub mm_inventory_limit: f64,
    /// Half-life of every market maker's own vol estimate.
    pub mm_half_life: f64,
    pub noise: usize,
    pub informed: usize,
    pub fast: usize,
    pub momentum: usize,
    pub taker_wallet: f64,
    /// Market orders never fill more than this far (in ln) from the mark.
    pub slippage: f64,
    pub seed: u64,
}

impl Default for BookSimConfig {
    fn default() -> Self {
        Self {
            band_x: 0.01,
            index_window: 20,
            ewma_half_life: 4.0 * 3_600.0,
            init_sigma: sigma_per_sec(0.8),
            min_time: 60.0,
            max_time: 7.0 * 86_400.0,
            book: BookConfig::default(),
            warmup_bars: 240,
            mm: 3,
            mm_wallet: 200_000.0,
            mm_inventory_limit: 100_000.0,
            mm_half_life: 3_600.0,
            noise: 20,
            informed: 2,
            fast: 2,
            momentum: 3,
            taker_wallet: 20_000.0,
            slippage: 0.1,
            seed: 1,
        }
    }
}

pub struct BookSim {
    pub cfg: BookSimConfig,
    feed: Box<dyn PriceFeed>,
    vol: EwmaVol,
    pub band: BandTracker,
    pub book: Book,
    agents: Vec<Box<dyn BookAgent>>,
    initial: Vec<f64>,
    rng: ChaCha8Rng,
    pub t: f64,
    start_t: f64,
    pub index: f64,
    pub mark: f64,
    pub steps: u64,
    index_at_ref: f64,
    mark_at_ref: f64,
    pub calib_index: Calibration,
    pub calib_mark: Calibration,
    pub liquidations: u64,
    pub adl_liquidations: u64,
    spread_sum: f64,
    spread_bars: u64,
    empty_bars: u64,
    gap_sum: f64,
    initial_value: f64,
}

impl BookSim {
    pub fn new(cfg: BookSimConfig, mut feed: Box<dyn PriceFeed>) -> Result<Self, String> {
        let first = feed.next_bar().ok_or("feed is empty")?;
        let mut vol = EwmaVol::new(cfg.ewma_half_life, cfg.init_sigma);
        vol.update(&first);
        let index = model_time_clamped(cfg.band_x, vol.sigma(), cfg.min_time, cfg.max_time);

        let mut agents: Vec<Box<dyn BookAgent>> = Vec::new();
        let mut initial = Vec::new();
        for i in 0..cfg.mm {
            // Same fair-value model; competing on base spread (0.25%, 0.5%, 1%, ...).
            let est = Estimator::new(cfg.mm_half_life, &cfg);
            let mm = MarketMaker::new(est, 0.0025 * 2f64.powi(i as i32), 10_000.0, cfg.mm_inventory_limit);
            agents.push(Box::new(mm));
            initial.push(cfg.mm_wallet);
        }
        let takers: Vec<Box<dyn BookAgent>> = (0..cfg.noise)
            .map(|_| {
                Box::new(NoiseTaker { trade_prob: 0.01, close_prob: 0.005, max_collateral: 5_000.0, leverage: 3.0 })
                    as Box<dyn BookAgent>
            })
            .chain((0..cfg.informed).map(|_| {
                Box::new(GapTaker {
                    kind: AgentKind::Informed,
                    est: None,
                    threshold: 0.25,
                    collateral_frac: 0.25,
                    leverage: 3.0,
                }) as Box<dyn BookAgent>
            }))
            .chain((0..cfg.fast).map(|i| {
                Box::new(GapTaker {
                    kind: AgentKind::FastVol,
                    est: Some(Estimator::new(900.0 * 4f64.powi(i as i32), &cfg)),
                    threshold: 0.25,
                    collateral_frac: 0.25,
                    leverage: 3.0,
                }) as Box<dyn BookAgent>
            }))
            .chain((0..cfg.momentum).map(|i| {
                let fast = 0.05 / (i + 1) as f64;
                Box::new(MomentumTaker {
                    fast_alpha: fast,
                    slow_alpha: fast / 10.0,
                    threshold: 0.15,
                    collateral_frac: 0.25,
                    leverage: 2.0,
                    emas: None,
                }) as Box<dyn BookAgent>
            }))
            .collect();
        initial.extend(std::iter::repeat_n(cfg.taker_wallet, takers.len()));
        agents.extend(takers);

        let book = Book::new(cfg.book.clone(), &initial);
        let initial_value = book.total_value();
        Ok(Self {
            band: BandTracker::new(cfg.band_x, cfg.index_window, first.t, first.close),
            rng: ChaCha8Rng::seed_from_u64(cfg.seed),
            t: first.t,
            start_t: first.t,
            index,
            mark: index,
            steps: 0,
            index_at_ref: index,
            mark_at_ref: index,
            calib_index: Calibration::default(),
            calib_mark: Calibration::default(),
            liquidations: 0,
            adl_liquidations: 0,
            spread_sum: 0.0,
            spread_bars: 0,
            empty_bars: 0,
            gap_sum: 0.0,
            initial_value,
            cfg,
            feed,
            vol,
            book,
            agents,
            initial,
        })
    }

    fn warmed_up(&self) -> bool {
        self.steps >= self.cfg.warmup_bars
    }

    fn refresh_mark(&mut self) {
        self.mark = self.book.mid().unwrap_or(self.index);
    }

    pub fn step(&mut self) -> bool {
        let Some(bar) = self.feed.next_bar() else { return false };
        let dt = (bar.t - self.t).max(0.0);
        self.t = bar.t;
        self.steps += 1;
        self.vol.update(&bar);
        self.index = model_time_clamped(self.cfg.band_x, self.vol.sigma(), self.cfg.min_time, self.cfg.max_time);
        self.book.accrue_funding(self.mark, self.index, dt);

        if let Some(hit) = self.band.on_bar(&bar) {
            if self.warmed_up() {
                self.calib_index.record(self.index_at_ref, hit.duration);
                self.calib_mark.record(self.mark_at_ref, hit.duration);
            }
            self.index_at_ref = self.index;
            self.mark_at_ref = self.mark;
        }

        for a in &mut self.agents {
            a.observe(&bar);
        }
        if !self.warmed_up() {
            self.refresh_mark();
            return true;
        }

        let true_model = self
            .feed
            .true_sigma()
            .map(|s| model_time_clamped(self.cfg.band_x, s, self.cfg.min_time, self.cfg.max_time));
        for i in 0..self.cfg.mm {
            let view = view(&self.book, i, self.index, self.mark, true_model);
            let actions = self.agents[i].act(&view, &mut self.rng);
            let mark = self.mark;
            for a in actions {
                if let BookAction::Quotes(q) = a {
                    self.book.replace_quotes(i, &q, mark);
                }
            }
        }
        self.refresh_mark();

        for l in self.book.liquidate_all(self.mark) {
            self.liquidations += 1;
            self.adl_liquidations += u64::from(l.adl > 0.0);
        }
        self.refresh_mark();

        let mut order: Vec<usize> = (self.cfg.mm..self.agents.len()).collect();
        order.shuffle(&mut self.rng);
        for i in order {
            let view = view(&self.book, i, self.index, self.mark, true_model);
            let actions = self.agents[i].act(&view, &mut self.rng);
            for a in actions {
                self.execute(i, a);
            }
        }
        self.refresh_mark();

        match (self.book.best_bid(), self.book.best_ask()) {
            (Some(b), Some(a)) => {
                self.spread_sum += (a / b).ln();
                self.spread_bars += 1;
            }
            _ => self.empty_bars += 1,
        }
        self.gap_sum += (self.mark / self.index).ln().abs();
        debug_assert!((self.book.total_value() - self.initial_value).abs() < 1e-6 * self.initial_value);
        true
    }

    fn execute(&mut self, owner: usize, action: BookAction) {
        let (side, size) = match action {
            BookAction::Market { side, notional } => {
                let room = self.book.accounts[owner].capacity(side, self.mark, self.cfg.book.max_leverage);
                (side, notional.min(room))
            }
            BookAction::CloseAll => {
                let q = self.book.accounts[owner].q;
                (if q > 0.0 { Side::Short } else { Side::Long }, q.abs())
            }
            BookAction::Quotes(_) => return,
        };
        if size <= 0.0 {
            return;
        }
        let limit = self.mark * (side.sign() * self.cfg.slippage).exp();
        self.book.market(owner, side, size, limit, self.cfg.book.taker_fee);
    }

    pub fn summary(&self) -> BookSummary {
        let mut by_kind: Vec<KindPnl> = Vec::new();
        for (i, a) in self.agents.iter().enumerate() {
            let pnl = self.book.accounts[i].equity(self.mark) - self.initial[i];
            match by_kind.iter_mut().find(|k| k.kind == a.kind()) {
                Some(k) => {
                    k.agents += 1;
                    k.pnl += pnl;
                }
                None => by_kind.push(KindPnl { kind: a.kind(), agents: 1, pnl }),
            }
        }
        let bars = self.steps.saturating_sub(self.cfg.warmup_bars).max(1) as f64;
        BookSummary {
            steps: self.steps,
            sim_days: (self.t - self.start_t) / 86_400.0,
            hits: self.band.total_hits,
            calib_index: self.calib_index.clone(),
            calib_mark: self.calib_mark.clone(),
            volume: self.book.volume,
            fees: self.book.fees,
            insurance: self.book.insurance,
            liquidations: self.liquidations,
            adl_liquidations: self.adl_liquidations,
            adl_volume: self.book.adl_volume,
            adl_shortfall: self.book.adl_shortfall,
            mean_spread: self.spread_sum / self.spread_bars.max(1) as f64,
            empty_frac: self.empty_bars as f64 / bars,
            mean_gap: self.gap_sum / bars,
            by_kind,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BookSummary {
    pub steps: u64,
    pub sim_days: f64,
    pub hits: u64,
    pub calib_index: Calibration,
    pub calib_mark: Calibration,
    pub volume: f64,
    pub fees: f64,
    pub insurance: f64,
    pub liquidations: u64,
    pub adl_liquidations: u64,
    pub adl_volume: f64,
    pub adl_shortfall: f64,
    /// Mean ln(best ask / best bid) while both sides are quoted.
    pub mean_spread: f64,
    /// Fraction of post-warmup bars with an empty side.
    pub empty_frac: f64,
    /// Mean |ln(mark / index)|.
    pub mean_gap: f64,
    pub by_kind: Vec<KindPnl>,
}

impl std::fmt::Display for BookSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "steps          {} ({:.1} days)", self.steps, self.sim_days)?;
        writeln!(f, "band hits      {}", self.hits)?;
        writeln!(
            f,
            "calibration    index {:.3}  book mid {:.3} (realized/predicted, n={})",
            self.calib_index.ratio(),
            self.calib_mark.ratio(),
            self.calib_mark.n
        )?;
        writeln!(f, "volume         {:.0}", self.volume)?;
        writeln!(f, "fees           {:.2}", self.fees)?;
        writeln!(f, "insurance      {:.2}", self.insurance)?;
        writeln!(f, "liquidations   {} ({} needed ADL)", self.liquidations, self.adl_liquidations)?;
        writeln!(f, "ADL volume     {:.0} (shortfall vs mark {:.2})", self.adl_volume, self.adl_shortfall)?;
        writeln!(
            f,
            "spread         mean {:.2}%  one side empty {:.1}% of bars",
            100.0 * self.mean_spread,
            100.0 * self.empty_frac
        )?;
        writeln!(f, "mid vs index   mean |gap| {:.2}%", 100.0 * self.mean_gap)?;
        writeln!(f, "trader PnL by kind:")?;
        for k in &self.by_kind {
            writeln!(f, "  {:<12} n={:<3} {:+.2}", format!("{:?}", k.kind), k.agents, k.pnl)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{Model, SynthFeed};

    fn sim(seed: u64) -> BookSim {
        let feed = SynthFeed::new(Model::Gbm { sigma: sigma_per_sec(0.8) }, seed, 150.0, 60.0);
        BookSim::new(BookSimConfig { seed, ..Default::default() }, Box::new(feed)).unwrap()
    }

    #[test]
    fn runs_deterministically_and_conserves_value() {
        let (mut a, mut b) = (sim(3), sim(3));
        for _ in 0..20_000 {
            a.step();
            b.step();
        }
        assert_eq!(a.book.accounts, b.book.accounts);
        assert!(a.book.volume > 0.0);
        assert!((a.book.total_value() - a.initial_value).abs() < 1e-6 * a.initial_value);
        // Every fill has two sides.
        assert!(a.book.accounts.iter().map(|x| x.q).sum::<f64>().abs() < 1e-3);
    }

    #[test]
    fn market_makers_keep_a_two_sided_book() {
        let mut s = sim(4);
        for _ in 0..10_000 {
            s.step();
        }
        let sum = s.summary();
        assert!(sum.empty_frac < 0.05, "{}", sum.empty_frac);
        assert!(sum.mean_spread > 0.0 && sum.mean_spread < 0.05, "{}", sum.mean_spread);
    }
}
