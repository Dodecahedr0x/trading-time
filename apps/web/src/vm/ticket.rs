use super::ladder::{sorted_asks, sorted_bids, walk};
use super::Side;
use crate::types::{QuotesJson, TraderJson};
use tradingtime_interface::Fx;

pub const TAKER_FEE_BPS: u16 = 5;
pub const MAINTENANCE_BPS: u16 = 500;
pub const MAX_LEVERAGE_BPS: u16 = 10_000;
/// Slippage tolerance for market orders, applied to the worst level walked (ln space).
pub const MARKET_SLIPPAGE: f64 = 0.01;
/// Market limit when the ladder cannot be walked: mark ×/÷ (1 + this).
pub const NO_DEPTH_BAND: f64 = 0.05;
pub const USDC: f64 = 1_000_000.0;

fn fee_rate() -> f64 {
    TAKER_FEE_BPS as f64 / 10_000.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
}

fn signed(side: Side, size: f64) -> f64 {
    match side {
        Side::Long => size,
        Side::Short => -size,
    }
}

/// Largest new size (USDC) at 1× for `side`: free equity (equity − reserved) plus `wallet_usdc` that would be
/// auto-deposited, minus same-side exposure, plus opposite exposure that the order first closes.
pub fn max_size(trader: Option<&TraderJson>, wallet_usdc: f64, side: Side) -> f64 {
    let (q, free) = trader.map(|t| (t.q_usdc, t.equity_usdc - t.reserved_usdc)).unwrap_or((0.0, 0.0));
    let lev = MAX_LEVERAGE_BPS as f64 / 10_000.0;
    let cap = (free + wallet_usdc.max(0.0)) * lev;
    let same = signed(side, q);
    let room = if same >= 0.0 { cap - same } else { cap + same.abs() };
    (room / (1.0 + fee_rate())).max(0.0)
}

/// 25/50/75/100% presets, rounded down to whole dollars.
pub fn preset(max: f64, pct: u32) -> f64 {
    (max * pct as f64 / 100.0).floor().max(0.0)
}

/// USDC -> base units (6dp), floor.
pub fn base_units(usdc: f64) -> u64 {
    (usdc.max(0.0) * USDC + 1e-6).floor() as u64
}

/// ln(secs) as Fx.
pub fn secs_to_ln(secs: f64) -> Fx {
    tradingtime_math::from_f64(secs.ln())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub entry_secs: f64,
    pub worst_secs: f64,
    pub fee_usdc: f64,
    /// Marginal limit for a market Swap (worst level ± slippage, ln space).
    pub limit_secs: f64,
}

/// Market estimate from the ladder: Long walks asks, Short walks bids. Falls back to mark ± 5% when depth is
/// unknown (None only when mark is 0 / size <= 0).
pub fn estimate_market(q: &QuotesJson, side: Side, size_usdc: f64) -> Option<Estimate> {
    if q.mark_secs <= 0.0 || size_usdc <= 0.0 {
        return None;
    }
    let levels = match side {
        Side::Long => sorted_asks(q),
        Side::Short => sorted_bids(q),
    };
    let fee_usdc = size_usdc * fee_rate();
    let dir = signed(side, 1.0);
    Some(match walk(&levels, size_usdc) {
        Some((entry, worst)) => {
            Estimate { entry_secs: entry, worst_secs: worst, fee_usdc, limit_secs: worst * (dir * MARKET_SLIPPAGE).exp() }
        }
        None => {
            let band = 1.0 + NO_DEPTH_BAND;
            let limit = if dir > 0.0 { q.mark_secs * band } else { q.mark_secs / band };
            Estimate { entry_secs: q.mark_secs, worst_secs: q.mark_secs, fee_usdc, limit_secs: limit }
        }
    })
}

/// Signed exposure after the order.
pub fn q_after(trader: Option<&TraderJson>, side: Side, size_usdc: f64) -> f64 {
    trader.map(|t| t.q_usdc).unwrap_or(0.0) + signed(side, size_usdc)
}

/// Share of the order size deposited on top of 1× margin to absorb spread and price impact,
/// which the program charges against equity before its margin check at the mark.
pub const SLIPPAGE_BUFFER: f64 = 0.01;

/// The program's margin exposure: max(|q + R_long|, |q − R_short|) over open limit orders.
pub fn margin_need(q: f64, reserved_long: f64, reserved_short: f64) -> f64 {
    (q + reserved_long).abs().max((q - reserved_short).abs())
}

/// Amount to deposit before trading `size_usdc` so equity covers the program's netted exposure at 1× plus the fee
/// (and, for Market, a slippage buffer). 0 when the trader already has enough.
/// Market moves the position; Limit adds its size to its side's open orders.
pub fn deposit_needed(trader: Option<&TraderJson>, side: Side, kind: OrderType, size_usdc: f64) -> f64 {
    let lev = MAX_LEVERAGE_BPS as f64 / 10_000.0;
    let (q, rl, rs) = trader.map(|t| (t.q_usdc, t.reserved_long_usdc, t.reserved_short_usdc)).unwrap_or_default();
    let (exposure, buffer) = match (kind, side) {
        (OrderType::Market, _) => (margin_need(q + signed(side, size_usdc), rl, rs), SLIPPAGE_BUFFER),
        (OrderType::Limit, Side::Long) => (margin_need(q, rl + size_usdc, rs), 0.0),
        (OrderType::Limit, Side::Short) => (margin_need(q, rl, rs + size_usdc), 0.0),
    };
    let required = exposure / lev + size_usdc * (fee_rate() + buffer);
    let available = trader.map(|t| t.equity_usdc).unwrap_or(0.0);
    (required - available).max(0.0)
}
