//! Order-book venue simulation. Market makers quote, takers cross, and the
//! realized-vol oracle is only the funding index rather than the trade price.

use crate::agents::{AgentKind, NoiseProfile};
use crate::band::BandTracker;
use crate::book::{Account, Book, BookConfig};
use crate::feed::{Bar, PriceFeed};
use crate::metrics::{Calibration, KindPnl};
use crate::oracle::{model_time_clamped, sigma_per_sec};
use crate::pool::Side;
use crate::vol::EwmaVol;
use rand::SeedableRng;
use rand::seq::SliceRandom;
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

/// Prices the perp as the funding index expected one funding horizon ahead.
///
/// The index is an EWMA of variance, so from the current index variance it
/// decays toward the variance running now: v(h) = v_now + (v_index − v_now)·2^(−h/half_life).
#[derive(Clone, Copy, Debug)]
pub struct Forecaster {
    pub band_x: f64,
    pub min_time: f64,
    pub max_time: f64,
    pub index_sigma: f64,
    pub index_half_life: f64,
    /// Funding horizon in seconds (1 / funding_k).
    pub horizon: f64,
}

impl Forecaster {
    /// Expected index (seconds) one horizon ahead if volatility stays at `sigma_now`.
    pub fn time(&self, sigma_now: f64) -> f64 {
        let (now, index) = (sigma_now * sigma_now, self.index_sigma * self.index_sigma);
        let var = now + (index - now) * 0.5f64.powf(self.horizon / self.index_half_life);
        model_time_clamped(self.band_x, var.sqrt(), self.min_time, self.max_time)
    }
}

pub struct BookView<'a> {
    /// Oracle model price (funding index).
    pub index: f64,
    pub forecast: Forecaster,
    /// Book mid, or the index when a side is empty.
    pub mark: f64,
    /// The feed's true volatility, on synthetic feeds.
    pub true_sigma: Option<f64>,
    pub account: &'a Account,
    pub equity: f64,
}

pub trait BookAgent {
    fn kind(&self) -> AgentKind;
    /// Sees every bar, including during warmup.
    fn observe(&mut self, _bar: &Bar) {}
    fn act(&mut self, view: &BookView, rng: &mut ChaCha8Rng) -> Vec<BookAction>;
    /// One-line state for the run summary (market makers only).
    fn describe(&self) -> Option<String> {
        None
    }
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

    pub fn sigma(&self) -> f64 {
        self.vol.sigma()
    }
}

/// Quotes a ladder around its forecast of the index (from its own vol estimate),
/// shifted against its inventory, with a half-spread that widens after fast moves.
/// Stops quoting the side that would take inventory past `inventory_limit`.
///
/// With `adapt`, the whole half-spread is scaled by a factor the maker tunes
/// once a day by symmetric steps: tighter while its average daily PnL is
/// positive (to win more flow), wider while it is negative (adverse selection).
/// Averaging matters: spread income arrives as many small gains and adverse
/// selection as rare large losses, so steering on the share of winning days
/// tightens too far. The scale settles where average PnL is about zero, so
/// competition rather than a fixed parameter sets the spread.
pub struct MarketMaker {
    pub est: Estimator,
    pub half_spread: f64,
    pub levels: usize,
    pub step: f64,
    pub size: f64,
    /// ln-price shift per USD of inventory.
    pub skew: f64,
    pub inventory_limit: f64,
    /// Half-spread added per unit of recent mean |Δ fair| per bar.
    pub move_mult: f64,
    pub adapt: bool,
    /// Multiplier on the half-spread chosen by adaptation.
    pub scale: f64,
    move_ema: f64,
    last: Option<f64>,
    bars: u64,
    window_equity: Option<f64>,
    /// EMA of daily equity change.
    pnl_ema: f64,
}

/// Bars between spread adjustments (one day of one-minute bars).
const ADAPT_WINDOW: u64 = 1_440;
const ADAPT_STEP: f64 = 0.9;
/// Weight of the latest day in the PnL average (about a 5-day memory).
const ADAPT_EMA: f64 = 0.2;

impl MarketMaker {
    pub fn new(est: Estimator, half_spread: f64, size: f64, inventory_limit: f64, adapt: bool) -> Self {
        Self {
            est,
            half_spread,
            levels: 3,
            step: 0.005,
            size,
            // Shift the center by one base half-spread at the inventory limit.
            skew: half_spread / inventory_limit,
            inventory_limit,
            move_mult: 1.5,
            adapt,
            scale: 1.0,
            move_ema: 0.0,
            last: None,
            bars: 0,
            window_equity: None,
            pnl_ema: 0.0,
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
        let fair = view.forecast.time(self.est.sigma()).ln();
        if let Some(last) = self.last {
            self.move_ema += 0.05 * ((fair - last).abs() - self.move_ema);
        }
        self.last = Some(fair);
        if view.equity <= 0.0 {
            return vec![BookAction::Quotes(vec![])];
        }
        self.bars += 1;
        if self.adapt && self.bars.is_multiple_of(ADAPT_WINDOW) {
            if let Some(start) = self.window_equity {
                self.pnl_ema += ADAPT_EMA * (view.equity - start - self.pnl_ema);
                let step = if self.pnl_ema >= 0.0 { ADAPT_STEP } else { 1.0 / ADAPT_STEP };
                self.scale = (self.scale * step).clamp(0.05, 20.0);
            }
            self.window_equity = Some(view.equity);
        }
        let q = view.account.q;
        let center = fair - self.skew * q;
        let half = self.scale * (self.half_spread + self.move_mult * self.move_ema);
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

    fn describe(&self) -> Option<String> {
        Some(format!(
            "est {:>3.0}m base {:.2}% scale {:>5.2} → half-spread now {:.2}%",
            self.est.vol.half_life() / 60.0,
            100.0 * self.half_spread,
            self.scale,
            100.0 * self.scale * (self.half_spread + self.move_mult * self.move_ema)
        ))
    }
}

/// Random market orders, closed after a random holding time.
pub struct NoiseTaker {
    pub profile: NoiseProfile,
    /// Equity and stake when the current position was opened.
    entry: Option<(f64, f64)>,
}

impl NoiseTaker {
    pub fn new(profile: NoiseProfile) -> Self {
        Self { profile, entry: None }
    }
}

impl BookAgent for NoiseTaker {
    fn kind(&self) -> AgentKind {
        AgentKind::Noise
    }

    fn act(&mut self, view: &BookView, rng: &mut ChaCha8Rng) -> Vec<BookAction> {
        let pr = &self.profile;
        if view.account.q == 0.0 {
            self.entry = None;
            let Some(side) = pr.wants_open(rng) else { return vec![] };
            let stake = pr.stake * view.equity;
            self.entry = Some((view.equity, stake));
            return vec![BookAction::Market { side, notional: stake * pr.leverage }];
        }
        let (equity0, stake) = self.entry.unwrap_or((view.equity, pr.stake * view.equity));
        if pr.wants_close(view.equity - equity0, stake, rng) { vec![BookAction::CloseAll] } else { vec![] }
    }
}

/// Trades the mark toward the index forecast from its own fast estimate
/// (`FastVol`) or from the feed's true volatility (`Informed`, synthetic only).
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
        let sigma = match (&self.est, view.true_sigma) {
            (Some(e), _) => e.sigma(),
            (None, Some(s)) => s,
            (None, None) => return vec![],
        };
        let reference = view.forecast.time(sigma);
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

fn view(book: &Book, i: usize, index: f64, mark: f64, forecast: Forecaster, true_sigma: Option<f64>) -> BookView<'_> {
    let account = &book.accounts[i];
    BookView { index, forecast, mark, true_sigma, account, equity: account.equity(mark) }
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
    /// Makers tune their spreads daily from their own PnL.
    pub mm_adaptive: bool,
    /// Starting (or, without adaptation, fixed) spread multiplier.
    pub mm_scale: f64,
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
            mm: 6,
            mm_wallet: 200_000.0,
            mm_inventory_limit: 20_000.0,
            mm_half_life: 900.0,
            mm_adaptive: true,
            mm_scale: 1.0,
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

/// Seeds noise-trader profiles separately from trading randomness.
pub(crate) const PROFILE_SEED: u64 = 0x5EED_0F7E_7124_DE55;

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
            // Heterogeneous makers: estimator 1× or 2× the base half-life, base spread 0.25/0.5/1%.
            let est = Estimator::new(cfg.mm_half_life * (1 + i % 2) as f64, &cfg);
            let base = 0.0025 * 2f64.powi((i % 3) as i32);
            let mut mm = MarketMaker::new(est, base, 10_000.0, cfg.mm_inventory_limit, cfg.mm_adaptive);
            mm.scale = cfg.mm_scale;
            agents.push(Box::new(mm));
            initial.push(cfg.mm_wallet);
        }
        let mut profiles = ChaCha8Rng::seed_from_u64(cfg.seed ^ PROFILE_SEED);
        let takers: Vec<Box<dyn BookAgent>> = (0..cfg.noise)
            .map(|_| Box::new(NoiseTaker::new(NoiseProfile::sample(&mut profiles))) as Box<dyn BookAgent>)
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

        let true_sigma = self.feed.true_sigma();
        let forecast = Forecaster {
            band_x: self.cfg.band_x,
            min_time: self.cfg.min_time,
            max_time: self.cfg.max_time,
            index_sigma: self.vol.sigma(),
            index_half_life: self.cfg.ewma_half_life,
            horizon: 1.0 / self.cfg.book.funding_k,
        };
        for i in 0..self.cfg.mm {
            let view = view(&self.book, i, self.index, self.mark, forecast, true_sigma);
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
            let view = view(&self.book, i, self.index, self.mark, forecast, true_sigma);
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
        let mut makers = Vec::new();
        for (i, a) in self.agents.iter().enumerate() {
            let acct = &self.book.accounts[i];
            KindPnl::add(&mut by_kind, a.kind(), self.initial[i], acct.equity(self.mark), acct.volume);
            if let Some(desc) = a.describe() {
                makers.push((desc, acct.equity(self.mark) - self.initial[i], acct.volume));
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
            makers,
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
    /// Per market maker: state, PnL, volume.
    pub makers: Vec<(String, f64, f64)>,
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
        KindPnl::write_table(f, &self.by_kind)?;
        writeln!(f, "market makers:")?;
        for (i, (desc, pnl, volume)) in self.makers.iter().enumerate() {
            writeln!(f, "  mm{i} {desc}  PnL {pnl:>+10.0}  volume {volume:>11.0}")?;
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
    fn forecast_moves_from_index_toward_current_vol() {
        let fc = |hl: f64| Forecaster {
            band_x: 0.01,
            min_time: 1.0,
            max_time: 1e9,
            index_sigma: 1e-4,
            index_half_life: hl,
            horizon: 86_400.0,
        };
        let index_time = model_time_clamped(0.01, 1e-4, 1.0, 1e9);
        // Vol unchanged: forecast is the index.
        assert!((fc(86_400.0).time(1e-4) / index_time - 1.0).abs() < 1e-12);
        // Vol doubled: a 1-day half-life moves halfway in variance (1 → 2.5), a 7-day one barely.
        assert!((fc(86_400.0).time(2e-4) / index_time - 1.0 / 2.5).abs() < 1e-9);
        let slow = fc(7.0 * 86_400.0).time(2e-4) / index_time;
        assert!(slow > 0.75 && slow < 0.85, "{slow}");
    }

    #[test]
    fn maker_tightens_on_profit_and_widens_on_loss() {
        let cfg = BookSimConfig::default();
        let mut mm = MarketMaker::new(Estimator::new(900.0, &cfg), 0.005, 10_000.0, 20_000.0, true);
        let account = Account::default();
        let forecast = Forecaster {
            band_x: 0.01,
            min_time: 60.0,
            max_time: 1e6,
            index_sigma: cfg.init_sigma,
            index_half_life: cfg.ewma_half_life,
            horizon: 86_400.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let mut run_day = |mm: &mut MarketMaker, equity: f64| {
            let view =
                BookView { index: 5_000.0, forecast, mark: 5_000.0, true_sigma: None, account: &account, equity };
            for _ in 0..ADAPT_WINDOW {
                mm.act(&view, &mut rng);
            }
        };
        run_day(&mut mm, 100_000.0); // sets the baseline
        run_day(&mut mm, 101_000.0);
        assert!((mm.scale - 0.9).abs() < 1e-12, "{}", mm.scale);
        // One −2k day after a +1k day: the average turns negative, so it widens back.
        run_day(&mut mm, 97_000.0);
        assert!((mm.scale - 1.0).abs() < 1e-12, "{}", mm.scale);
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
