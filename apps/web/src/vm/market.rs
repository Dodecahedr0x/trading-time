use super::format::{duration, money, money_compact, pct_signed, DASH};
use crate::types::MarketJson;

pub const EXPLAIN: &str = "You trade how long SOL takes to move 1% up or down, not its price. Long time gains when moves get slower (calmer market). Short time gains when moves get faster (wilder market).";

#[derive(Clone, Debug, PartialEq)]
pub struct MarketBar {
    pub mark: String,
    pub index: String,
    pub change: String,
    /// true when 24h change > 0 (moves got slower → calm colour)
    pub change_calm: bool,
    pub volume: String,
    pub open_interest: String,
    /// "+0.0042%/h"
    pub funding: String,
    /// "Long pays" / "Short pays" / "—"
    pub funding_payer: String,
    /// "Long time pays 0.0042%" / "Long time receives …" / "—"
    pub funding_text: String,
    /// Annualised SOL volatility implied by the mark, "82% a year".
    pub volatility: String,
    pub paused: bool,
    /// "$189.04"
    pub sol_price: String,
    /// "+1.27%"
    pub sol_change: String,
    pub sol_up: bool,
    /// The ±1% band around the current price that the market is timing: "$187.15 – $190.93".
    pub band: String,
}

pub fn market_bar(m: &MarketJson) -> MarketBar {
    let sol = m.sol_price_usd.filter(|p| *p > 0.0 && p.is_finite());
    let payer = if m.funding_rate_per_hour > 0.0 {
        "Long pays"
    } else if m.funding_rate_per_hour < 0.0 {
        "Short pays"
    } else {
        DASH
    };
    MarketBar {
        mark: duration(m.mark_secs),
        index: duration(m.index_secs),
        change: pct_signed(m.change_24h, 2),
        change_calm: m.change_24h >= 0.0,
        volume: money_compact(m.volume_24h_usdc),
        open_interest: money_compact(m.long_oi_usdc + m.short_oi_usdc),
        funding: format!("{}/h", pct_signed(m.funding_rate_per_hour, 4)),
        funding_payer: payer.into(),
        funding_text: funding_text(m.funding_rate_per_hour),
        volatility: implied_vol(m.mark_secs).map_or_else(|| DASH.into(), |v| format!("{:.0}% a year", v * 100.0)),
        paused: m.paused,
        sol_price: sol.map_or_else(|| DASH.into(), money),
        sol_change: if sol.is_some() { pct_signed(m.sol_change_24h, 2) } else { DASH.into() },
        sol_up: m.sol_change_24h >= 0.0,
        band: sol.map_or_else(|| DASH.into(), |p| format!("{} \u{2013} {}", money(p * 0.99), money(p * 1.01))),
    }
}

/// Annualised volatility whose calibrated time-to-move equals `secs` (inverse of the index formula).
pub fn implied_vol(secs: f64) -> Option<f64> {
    let p = tradingtime_interface::Params::DEFAULT;
    let x = (1.0 + p.band_bps as f64 / 10_000.0).ln();
    let c = tradingtime_math::to_f64(p.calib.get());
    (secs > 0.0).then(|| (c * x * x / secs * 365.0 * 86_400.0).sqrt())
}

pub fn funding_text(rate_per_hour: f64) -> String {
    if rate_per_hour == 0.0 || !rate_per_hour.is_finite() {
        return DASH.into();
    }
    let verb = if rate_per_hour > 0.0 { "pays" } else { "receives" };
    format!("Long time {verb} {:.4}%", rate_per_hour.abs() * 100.0)
}
