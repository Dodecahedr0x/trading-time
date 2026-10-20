//! LP pool that takes the other side of every trade at the model price.
//!
//! Prices are in seconds of expected time-to-move. A position of `units`
//! gains `units × Δprice` USD for longs (time lengthens) and loses it for
//! shorts. Cash only moves between trader wallets, position collateral and
//! pool `liquidity`, so their sum is conserved.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Expects moves to take longer (volatility falls).
    Long,
    /// Expects moves to come sooner (volatility rises).
    Short,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Long => 1.0,
            Side::Short => -1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PoolConfig {
    /// Fee on notional at open and at close.
    pub trade_fee: f64,
    /// Price premium per unit of skew / liquidity.
    pub skew_impact: f64,
    /// Funding per second paid by the heavy side at skew / liquidity = 1.
    pub funding_k: f64,
    /// Borrow fee per second on notional, paid to LPs by everyone.
    pub borrow_rate: f64,
    pub max_leverage: f64,
    /// Maintenance margin as a fraction of current notional.
    pub maintenance: f64,
    /// Liquidation penalty as a fraction of current notional, paid to LPs.
    pub liq_fee: f64,
    /// Cap on each side's open interest as a fraction of liquidity.
    pub max_side_oi: f64,
    /// Cap on |long OI − short OI| as a fraction of liquidity. Opens that would
    /// push the skew past it are rejected; opens that reduce the skew always pass.
    pub max_net_skew: f64,
    /// Auto-deleveraging: when liquidations leave the pool one-sided, close an
    /// equal entry notional of the opposite side pro rata at the mark, so the
    /// pool's net exposure is what it was before the liquidations.
    pub adl: bool,
    /// Settle PnL on ln(price) instead of price: a position gains
    /// `entry_notional × ln(exit / entry)`, so payoffs are symmetric in log-time
    /// instead of convex for longs.
    pub log_quote: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            trade_fee: 0.001,
            skew_impact: 0.05,
            funding_k: 2e-6,
            borrow_rate: 1e-7,
            max_leverage: 5.0,
            maintenance: 0.05,
            liq_fee: 0.01,
            max_side_oi: 0.5,
            max_net_skew: 1.0,
            adl: false,
            log_quote: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub id: u64,
    pub owner: usize,
    pub side: Side,
    pub units: f64,
    /// Execution price in seconds.
    pub entry: f64,
    pub collateral: f64,
    pub opened_at: f64,
    /// PnL settles on ln(price); see [`PoolConfig::log_quote`].
    pub log: bool,
    funding_snap: f64,
    borrow_snap: f64,
}

impl Position {
    pub fn entry_notional(&self) -> f64 {
        self.units * self.entry
    }

    /// Exposure at `price`, used for margin, fees and penalties. Constant for log positions.
    pub fn notional(&self, price: f64) -> f64 {
        if self.log { self.entry_notional() } else { self.units * price }
    }

    pub fn pnl(&self, price: f64) -> f64 {
        let move_ = if self.log {
            self.entry_notional() * (price / self.entry).ln()
        } else {
            self.units * (price - self.entry)
        };
        self.side.sign() * move_
    }
}

/// Prices a trade can fill against: the oracle `mark` and the `target` it is
/// moving toward. Trades fill at whichever is worse for the trader, so the gap
/// between them cannot be farmed. Liquidations and PnL use `mark` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quote {
    pub mark: f64,
    pub target: f64,
}

impl Quote {
    /// Base fill price before skew impact: buyers pay the higher, sellers get the lower.
    pub fn fill(&self, buy: bool) -> f64 {
        if buy { self.mark.max(self.target) } else { self.mark.min(self.target) }
    }
}

impl From<f64> for Quote {
    fn from(mark: f64) -> Self {
        Self { mark, target: mark }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoolError {
    Leverage,
    FeeExceedsCollateral,
    OiCap,
    NoPosition,
    /// Would be liquidatable at the mark immediately after filling.
    Undercollateralized,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PoolError::Leverage => "leverage out of range",
            PoolError::FeeExceedsCollateral => "fee exceeds collateral",
            PoolError::OiCap => "open interest cap reached",
            PoolError::NoPosition => "no such position",
            PoolError::Undercollateralized => "fill vs mark gap exceeds margin",
        })
    }
}

/// Result of closing or liquidating a position.
#[derive(Clone, Debug, PartialEq)]
pub struct Settlement {
    /// The settled position, or the settled slice of it for a partial ADL.
    pub position: Position,
    pub price: f64,
    /// Cash returned to the owner's wallet.
    pub payout: f64,
    pub kind: SettleKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettleKind {
    Close,
    Liquidation,
    /// Reduced by auto-deleveraging; may be partial.
    Adl,
}

#[derive(Clone, Debug)]
pub struct Pool {
    pub cfg: PoolConfig,
    pub liquidity: f64,
    pub positions: BTreeMap<u64, Position>,
    pub long_oi: f64,
    pub short_oi: f64,
    pub fees_collected: f64,
    pub bad_debt: f64,
    next_id: u64,
    funding_idx: f64,
    borrow_idx: f64,
}

impl Pool {
    pub fn new(cfg: PoolConfig, liquidity: f64) -> Self {
        Self {
            cfg,
            liquidity,
            positions: BTreeMap::new(),
            long_oi: 0.0,
            short_oi: 0.0,
            fees_collected: 0.0,
            bad_debt: 0.0,
            next_id: 1,
            funding_idx: 0.0,
            borrow_idx: 0.0,
        }
    }

    /// Net long open interest (entry notional).
    pub fn skew(&self) -> f64 {
        self.long_oi - self.short_oi
    }

    fn skew_ratio(&self, skew: f64) -> f64 {
        if self.liquidity > 0.0 { (skew / self.liquidity).clamp(-1.0, 1.0) } else { 0.0 }
    }

    /// Fill price for a trade moving skew by `delta`, priced at the mid of the
    /// skew before and after, so a round trip at unchanged skew is free.
    pub fn exec_price(&self, mark: f64, delta: f64) -> f64 {
        let mid = self.skew() + delta / 2.0;
        mark * (1.0 + self.cfg.skew_impact * self.skew_ratio(mid))
    }

    /// Current funding rate per second (positive: longs pay shorts).
    pub fn funding_rate(&self) -> f64 {
        self.cfg.funding_k * self.skew_ratio(self.skew())
    }

    /// Advances funding and borrow indices by `dt` seconds.
    pub fn accrue(&mut self, dt: f64) {
        self.funding_idx += self.funding_rate() * dt;
        self.borrow_idx += self.cfg.borrow_rate * dt;
    }

    /// Funding plus borrow owed by `p` (negative = owed to it).
    pub fn carry_owed(&self, p: &Position) -> f64 {
        let n = p.entry_notional();
        p.side.sign() * n * (self.funding_idx - p.funding_snap) + n * (self.borrow_idx - p.borrow_snap)
    }

    /// Collateral plus PnL at `mark` minus carry.
    pub fn equity(&self, p: &Position, mark: f64) -> f64 {
        p.collateral + p.pnl(mark) - self.carry_owed(p)
    }

    /// Mark price at which `p` hits maintenance margin.
    pub fn liq_price(&self, p: &Position) -> f64 {
        let (u, e, c, f, mm) = (p.units, p.entry, p.collateral, self.carry_owed(p), self.cfg.maintenance);
        if p.log {
            // c + s·N·ln(m/e) − f = mm·N
            let n = p.entry_notional();
            return e * (p.side.sign() * (mm * n - c + f) / n).exp();
        }
        match p.side {
            Side::Long => (u * e - c + f) / (u * (1.0 - mm)),
            Side::Short => (u * e + c - f) / (u * (1.0 + mm)),
        }
    }

    pub fn open(
        &mut self,
        owner: usize,
        side: Side,
        collateral: f64,
        leverage: f64,
        quote: impl Into<Quote>,
        t: f64,
    ) -> Result<u64, PoolError> {
        let quote = quote.into();
        if !(leverage > 0.0 && leverage <= self.cfg.max_leverage) {
            return Err(PoolError::Leverage);
        }
        let notional = collateral * leverage;
        let fee = self.cfg.trade_fee * notional;
        if fee >= collateral {
            return Err(PoolError::FeeExceedsCollateral);
        }
        let side_oi = if side == Side::Long { self.long_oi } else { self.short_oi };
        let skew_after = self.skew() + side.sign() * notional;
        if side_oi + notional > self.cfg.max_side_oi * self.liquidity
            || (skew_after.abs() > self.cfg.max_net_skew * self.liquidity && skew_after.abs() > self.skew().abs())
        {
            return Err(PoolError::OiCap);
        }
        let entry = self.exec_price(quote.fill(side == Side::Long), side.sign() * notional);
        let position = Position {
            id: self.next_id,
            owner,
            side,
            units: notional / entry,
            entry,
            collateral: collateral - fee,
            opened_at: t,
            log: self.cfg.log_quote,
            funding_snap: self.funding_idx,
            borrow_snap: self.borrow_idx,
        };
        if self.is_liquidatable(&position, quote.mark) {
            return Err(PoolError::Undercollateralized);
        }
        let id = position.id;
        self.next_id += 1;
        self.liquidity += fee;
        self.fees_collected += fee;
        *self.side_oi_mut(side) += notional;
        self.positions.insert(id, position);
        Ok(id)
    }

    /// Closes at the worse-of, skew-adjusted price, charging the trade fee.
    pub fn close(&mut self, id: u64, quote: impl Into<Quote>) -> Result<Settlement, PoolError> {
        let quote = quote.into();
        let p = self.positions.get(&id).ok_or(PoolError::NoPosition)?;
        let price = self.exec_price(quote.fill(p.side == Side::Short), -p.side.sign() * p.entry_notional());
        let fee = self.cfg.trade_fee * p.notional(price);
        self.fees_collected += fee.min(self.equity(p, price).max(0.0));
        Ok(self.settle(id, price, fee, SettleKind::Close))
    }

    pub fn is_liquidatable(&self, p: &Position, mark: f64) -> bool {
        self.equity(p, mark) < self.cfg.maintenance * p.notional(mark)
    }

    /// Liquidates every under-margined position at `mark`.
    /// Liquidates every under-margined position at `mark`, then auto-deleverages
    /// the opposite side if [`PoolConfig::adl`] is set.
    pub fn liquidate_all(&mut self, mark: f64) -> Vec<Settlement> {
        let ids: Vec<u64> = self.positions.values().filter(|p| self.is_liquidatable(p, mark)).map(|p| p.id).collect();
        let mut out: Vec<Settlement> = ids
            .into_iter()
            .map(|id| {
                let penalty = self.cfg.liq_fee * self.positions[&id].notional(mark);
                self.settle(id, mark, penalty, SettleKind::Liquidation)
            })
            .collect();
        if self.cfg.adl {
            // Positive: more long notional was liquidated, so shorts are now unhedged.
            let net: f64 = out.iter().map(|s| s.position.side.sign() * s.position.entry_notional()).sum();
            let side = if net > 0.0 { Side::Short } else { Side::Long };
            let opposite_oi = if side == Side::Long { self.long_oi } else { self.short_oi };
            if net != 0.0 && opposite_oi > 0.0 {
                let fraction = (net.abs() / opposite_oi).min(1.0);
                let ids: Vec<u64> = self.positions.values().filter(|p| p.side == side).map(|p| p.id).collect();
                out.extend(ids.into_iter().map(|id| self.reduce(id, fraction, mark)));
            }
        }
        out
    }

    fn settle(&mut self, id: u64, price: f64, charge: f64, kind: SettleKind) -> Settlement {
        let p = self.positions.remove(&id).expect("position exists");
        let eq = self.equity(&p, price) - charge;
        let payout = eq.max(0.0);
        self.bad_debt += (-eq).max(0.0);
        self.liquidity += p.collateral - payout;
        *self.side_oi_mut(p.side) -= p.entry_notional();
        Settlement { position: p, price, payout, kind }
    }

    /// Settles `fraction` of a position at `price` with no fee (ADL).
    fn reduce(&mut self, id: u64, fraction: f64, price: f64) -> Settlement {
        if fraction >= 1.0 {
            return self.settle(id, price, 0.0, SettleKind::Adl);
        }
        let p = self.positions[&id].clone();
        let eq = fraction * self.equity(&p, price);
        let payout = eq.max(0.0);
        self.bad_debt += (-eq).max(0.0);
        let mut slice = p.clone();
        slice.units *= fraction;
        slice.collateral *= fraction;
        self.liquidity += slice.collateral - payout;
        *self.side_oi_mut(p.side) -= slice.entry_notional();
        let rest = self.positions.get_mut(&id).expect("position exists");
        // Carry is proportional to entry notional, so scaling units scales it too.
        rest.units -= slice.units;
        rest.collateral -= slice.collateral;
        Settlement { position: slice, price, payout, kind: SettleKind::Adl }
    }

    fn side_oi_mut(&mut self, side: Side) -> &mut f64 {
        match side {
            Side::Long => &mut self.long_oi,
            Side::Short => &mut self.short_oi,
        }
    }

    pub fn total_collateral(&self) -> f64 {
        self.positions.values().map(|p| p.collateral).sum()
    }

    /// LP value if every position were settled at `mark` (no fees or impact).
    pub fn lp_equity(&self, mark: f64) -> f64 {
        self.liquidity + self.positions.values().map(|p| p.collateral - self.equity(p, mark).max(0.0)).sum::<f64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> Pool {
        Pool::new(PoolConfig { skew_impact: 0.0, ..Default::default() }, 1_000_000.0)
    }

    fn cash(p: &Pool, wallets: f64) -> f64 {
        wallets + p.total_collateral() + p.liquidity
    }

    #[test]
    fn long_profits_when_time_lengthens() {
        let mut p = pool();
        let id = p.open(0, Side::Long, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        assert_eq!(p.positions[&id].collateral, 995.0); // 0.1% fee on 5k
        let s = p.close(id, 1_100.0).unwrap();
        // 5 units × +100s = +500, minus 5.5 close fee
        assert!((s.payout - (995.0 + 500.0 - 5.5)).abs() < 1e-9, "{}", s.payout);
        assert_eq!(p.long_oi, 0.0);
    }

    #[test]
    fn short_profits_when_time_shortens() {
        let mut p = pool();
        let id = p.open(0, Side::Short, 1_000.0, 2.0, 1_000.0, 0.0).unwrap();
        let s = p.close(id, 900.0).unwrap();
        assert!((s.payout - (998.0 + 200.0 - 1.8)).abs() < 1e-9, "{}", s.payout);
    }

    #[test]
    fn rejects_bad_leverage_and_oi_cap() {
        let mut p = pool();
        assert_eq!(p.open(0, Side::Long, 100.0, 6.0, 1.0, 0.0), Err(PoolError::Leverage));
        assert_eq!(p.open(0, Side::Long, 200_000.0, 3.0, 1.0, 0.0), Err(PoolError::OiCap));
        assert_eq!(p.close(99, 1.0), Err(PoolError::NoPosition));
    }

    #[test]
    fn net_skew_cap_blocks_only_skew_increasing_opens() {
        let mut p = Pool::new(PoolConfig { skew_impact: 0.0, max_net_skew: 0.01, ..Default::default() }, 1_000_000.0);
        p.open(0, Side::Long, 2_000.0, 5.0, 1_000.0, 0.0).unwrap(); // skew 10k = cap
        assert_eq!(p.open(0, Side::Long, 200.0, 1.0, 1_000.0, 0.0), Err(PoolError::OiCap));
        p.open(1, Side::Short, 4_000.0, 5.0, 1_000.0, 0.0).unwrap(); // skew 10k → −10k
        assert_eq!(p.open(1, Side::Short, 200.0, 1.0, 1_000.0, 0.0), Err(PoolError::OiCap));
        assert!(p.open(0, Side::Long, 4_000.0, 5.0, 1_000.0, 0.0).is_ok());
    }

    #[test]
    fn skew_impact_charges_the_heavy_side() {
        let mut p = Pool::new(PoolConfig::default(), 1_000_000.0);
        p.open(0, Side::Long, 20_000.0, 5.0, 1_000.0, 0.0).unwrap(); // skew +100k
        let long_px = p.exec_price(1_000.0, 10_000.0);
        let short_px = p.exec_price(1_000.0, -10_000.0);
        assert!(long_px > 1_000.0 && short_px > 1_000.0 && long_px > short_px);
    }

    #[test]
    fn funding_flows_from_heavy_side() {
        let mut p = Pool::new(PoolConfig { skew_impact: 0.0, borrow_rate: 0.0, ..Default::default() }, 1_000_000.0);
        let l = p.open(0, Side::Long, 10_000.0, 5.0, 1_000.0, 0.0).unwrap();
        let s = p.open(1, Side::Short, 5_000.0, 5.0, 1_000.0, 0.0).unwrap();
        p.accrue(3_600.0);
        let (lp, sp) = (&p.positions[&l], &p.positions[&s]);
        assert!(p.carry_owed(lp) > 0.0);
        assert!(p.carry_owed(sp) < 0.0, "light side receives funding");
        assert!(p.carry_owed(lp) + p.carry_owed(sp) > 0.0, "pool nets the imbalance");
    }

    #[test]
    fn liquidation_at_liq_price_and_cash_conserved() {
        let mut p = pool();
        let start = cash(&p, 1_000.0);
        let id = p.open(0, Side::Short, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        let wallets = 0.0;
        let liq = p.liq_price(&p.positions[&id]);
        assert!(liq > 1_000.0 && liq < 1_200.0, "{liq}");
        assert!(p.liquidate_all(liq * 0.999).is_empty());
        let s = p.liquidate_all(liq * 1.001);
        assert_eq!(s.len(), 1);
        assert!((cash(&p, wallets + s[0].payout) - start).abs() < 1e-6);
    }

    #[test]
    fn bad_debt_is_absorbed_by_pool() {
        let mut p = pool();
        let id = p.open(0, Side::Short, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        let before = p.liquidity;
        let s = p.close(id, 2_000.0).unwrap(); // loses 5k on 995 collateral
        assert_eq!(s.payout, 0.0);
        assert!(p.bad_debt > 4_000.0);
        assert!((p.liquidity - before - 995.0).abs() < 1e-9);
    }

    #[test]
    fn worst_of_fills_against_the_trader() {
        let mut p = pool();
        let rising = Quote { mark: 1_000.0, target: 1_100.0 };
        let l = p.open(0, Side::Long, 1_000.0, 2.0, rising, 0.0).unwrap();
        let s = p.open(1, Side::Short, 1_000.0, 2.0, rising, 0.0).unwrap();
        assert_eq!(p.positions[&l].entry, 1_100.0);
        assert_eq!(p.positions[&s].entry, 1_000.0);
        // Long round trip while the gap persists loses the gap; no free edge.
        let closed = p.close(l, rising).unwrap();
        assert_eq!(closed.price, 1_000.0);
        assert!(closed.payout < 998.0 - 150.0);
        assert_eq!(p.close(s, rising).unwrap().price, 1_100.0);
    }

    #[test]
    fn rejects_fill_that_is_instantly_liquidatable() {
        let mut p = pool();
        let gap = Quote { mark: 1_000.0, target: 1_300.0 };
        assert_eq!(p.open(0, Side::Long, 1_000.0, 5.0, gap, 0.0), Err(PoolError::Undercollateralized));
        assert!(p.open(0, Side::Long, 1_000.0, 1.0, gap, 0.0).is_ok());
    }

    fn log_pool() -> Pool {
        Pool::new(PoolConfig { skew_impact: 0.0, log_quote: true, ..Default::default() }, 1_000_000.0)
    }

    #[test]
    fn log_quote_pnl_is_symmetric_in_log_time() {
        let mut p = log_pool();
        let l = p.open(0, Side::Long, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        let s = p.open(1, Side::Short, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        // Time ×9 (vol drops 3×): long gains N·ln 9, not N·8.
        let gain = p.positions[&l].pnl(9_000.0);
        assert!((gain - 5_000.0 * 9f64.ln()).abs() < 1e-6);
        // Halving and doubling are mirror images.
        assert!((p.positions[&l].pnl(2_000.0) + p.positions[&l].pnl(500.0)).abs() < 1e-6);
        assert!((p.positions[&s].pnl(500.0) - p.positions[&l].pnl(2_000.0)).abs() < 1e-6);
    }

    #[test]
    fn log_quote_liq_price_is_maintenance_boundary() {
        let mut p = log_pool();
        for side in [Side::Long, Side::Short] {
            let id = p.open(0, side, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
            let pos = p.positions[&id].clone();
            let liq = p.liq_price(&pos);
            let (inside, outside) =
                if side == Side::Long { (liq * 1.001, liq * 0.999) } else { (liq * 0.999, liq * 1.001) };
            assert!(!p.is_liquidatable(&pos, inside) && p.is_liquidatable(&pos, outside), "{side:?} {liq}");
            assert!((p.close(id, 1_000.0).unwrap().payout - (995.0 - 5.0)).abs() < 1e-9);
        }
    }

    /// Walks the mark from 1000 to 1000/9 in small steps, liquidating each step,
    /// with a balanced long/short book. Returns the LP PnL and the final pool.
    fn gradual_crash(adl: bool) -> (f64, Pool) {
        let mut p = Pool::new(PoolConfig { skew_impact: 0.0, log_quote: true, adl, ..Default::default() }, 1_000_000.0);
        p.open(0, Side::Long, 1_000.0, 3.0, 1_000.0, 0.0).unwrap();
        p.open(1, Side::Short, 1_000.0, 3.0, 1_000.0, 0.0).unwrap();
        let lp0 = p.lp_equity(1_000.0);
        let mut mark = 1_000.0;
        for _ in 0..200 {
            mark *= (9f64.ln() / -200.0).exp();
            p.liquidate_all(mark);
        }
        (p.lp_equity(mark) - lp0, p)
    }

    #[test]
    fn adl_keeps_pool_flat_through_gradual_gap() {
        let (without, _) = gradual_crash(false);
        let (with, pool) = gradual_crash(true);
        // Without ADL the pool is left short the whole remaining move after the long is liquidated.
        assert!(without < -4_000.0, "{without}");
        // With ADL the short is closed alongside; the pool keeps fees and penalties.
        assert!(with > 0.0, "{with}");
        assert!(pool.positions.is_empty());
        assert_eq!((pool.long_oi, pool.short_oi), (0.0, 0.0));
    }

    #[test]
    fn adl_reduces_pro_rata_and_conserves_cash() {
        let mut p = Pool::new(PoolConfig { skew_impact: 0.0, adl: true, ..Default::default() }, 1_000_000.0);
        let cash0 = p.liquidity + 3_000.0;
        p.open(0, Side::Long, 1_000.0, 5.0, 1_000.0, 0.0).unwrap(); // 5k long, gets liquidated
        let a = p.open(1, Side::Short, 1_000.0, 5.0, 1_000.0, 0.0).unwrap(); // 5k
        let b = p.open(2, Side::Short, 1_000.0, 5.0, 1_000.0, 0.0).unwrap(); // 5k
        let out = p.liquidate_all(800.0);
        let adl: Vec<_> = out.iter().filter(|s| s.kind == SettleKind::Adl).collect();
        assert_eq!(adl.len(), 2);
        // 5k liquidated across 10k of shorts: each halved.
        assert!((p.positions[&a].entry_notional() - 2_500.0).abs() < 1e-6);
        assert!((p.positions[&b].entry_notional() - 2_500.0).abs() < 1e-6);
        assert!((p.skew() - -5_000.0).abs() < 1e-6, "skew restored to pre-liquidation -5k");
        let paid: f64 = out.iter().map(|s| s.payout).sum();
        assert!((p.liquidity + p.total_collateral() + paid - cash0).abs() < 1e-6);
    }

    #[test]
    fn lp_equity_mirrors_trader_pnl() {
        let mut p = pool();
        p.open(0, Side::Long, 1_000.0, 5.0, 1_000.0, 0.0).unwrap();
        let lp0 = p.lp_equity(1_000.0);
        assert!((p.lp_equity(1_050.0) - (lp0 - 250.0)).abs() < 1e-6);
    }
}
