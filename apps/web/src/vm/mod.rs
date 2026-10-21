//! Host-testable view-model: formatting and derived display rows.

pub mod analytics;
pub mod docs;
pub mod chart;
pub mod format;
pub mod ladder;
pub mod market;
pub mod payoff;
pub mod rows;
pub mod ticket;

#[cfg(test)]
mod tests;

pub use tradingtime_interface::Side;

pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Long => "long",
        Side::Short => "short",
    }
}

/// TradingView-style timeframes: (indexer `interval` value, button label). Shared by the price chart and analytics.
pub const TIMEFRAMES: [(&str, &str); 11] = [
    ("1m", "1m"),
    ("3m", "3m"),
    ("5m", "5m"),
    ("15m", "15m"),
    ("30m", "30m"),
    ("1h", "1H"),
    ("2h", "2H"),
    ("4h", "4H"),
    ("6h", "6H"),
    ("12h", "12H"),
    ("1d", "1D"),
];

/// Seconds per candle for a `TIMEFRAMES` value ("15m" -> 900); 3600 when unknown.
pub fn timeframe_secs(v: &str) -> i64 {
    let (n, unit) = v.split_at(v.len().saturating_sub(1));
    let n: i64 = n.parse().unwrap_or(1);
    match unit {
        "m" => n * 60,
        "h" => n * 3_600,
        "d" => n * 86_400,
        _ => 3_600,
    }
}
