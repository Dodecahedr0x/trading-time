use super::format::{duration, money_compact, pct_signed, DASH};
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
    pub paused: bool,
}

pub fn market_bar(m: &MarketJson) -> MarketBar {
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
        paused: m.paused,
    }
}
