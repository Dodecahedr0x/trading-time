use super::format::{hms, money_whole, DASH};
use crate::types::{LevelJson, QuotesJson};

#[derive(Clone, Debug, PartialEq)]
pub struct LadderRow {
    pub time_secs: f64,
    /// "1:30:05"
    pub time: String,
    /// "$2,000"
    pub size: String,
    /// "$6,000" running total from the mark outward
    pub total: String,
    /// 0..=1 depth bar width (cumulative / max cumulative on either side)
    pub depth: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    /// Above the mark, farthest first (top of the panel); calm, where Long time fills.
    pub asks: Vec<LadderRow>,
    /// Below the mark, nearest first; wild, where Short time fills.
    pub bids: Vec<LadderRow>,
    pub mark: String,
    /// Ask-bid gap as fraction of mark, e.g. "0.04%".
    pub spread: String,
}

/// Asks nearest (lowest time) first.
pub fn sorted_asks(q: &QuotesJson) -> Vec<LevelJson> {
    let mut v: Vec<LevelJson> = q.asks.iter().filter(|l| l.size_usdc > 0.0 && l.time_secs > 0.0).cloned().collect();
    v.sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
    v
}

/// Bids nearest (highest time) first.
pub fn sorted_bids(q: &QuotesJson) -> Vec<LevelJson> {
    let mut v: Vec<LevelJson> = q.bids.iter().filter(|l| l.size_usdc > 0.0 && l.time_secs > 0.0).cloned().collect();
    v.sort_by(|a, b| b.time_secs.total_cmp(&a.time_secs));
    v
}

fn cumulate(levels: &[LevelJson], max_rows: usize) -> Vec<(f64, f64, f64)> {
    let mut total = 0.0;
    levels
        .iter()
        .take(max_rows)
        .map(|l| {
            total += l.size_usdc;
            (l.time_secs, l.size_usdc, total)
        })
        .collect()
}

pub fn ladder(q: &QuotesJson, max_rows: usize) -> Ladder {
    let asks = cumulate(&sorted_asks(q), max_rows);
    let bids = cumulate(&sorted_bids(q), max_rows);
    let max = asks.iter().chain(bids.iter()).map(|r| r.2).fold(0.0, f64::max);
    let row = |&(t, s, c): &(f64, f64, f64)| LadderRow {
        time_secs: t,
        time: hms(t),
        size: money_whole(s),
        total: money_whole(c),
        depth: if max > 0.0 { c / max } else { 0.0 },
    };
    let spread = match (asks.first(), bids.first()) {
        (Some(a), Some(b)) if q.mark_secs > 0.0 => format!("{:.2}%", (a.0 - b.0) / q.mark_secs * 100.0),
        _ => DASH.to_string(),
    };
    Ladder {
        asks: asks.iter().rev().map(row).collect(),
        bids: bids.iter().map(row).collect(),
        mark: if q.mark_secs > 0.0 { hms(q.mark_secs) } else { DASH.to_string() },
        spread,
    }
}

/// Walks `levels` (nearest first) to fill `size_usdc`; returns (average secs, ln-averaged; worst secs).
/// None when depth is insufficient or size <= 0.
pub fn walk(levels: &[LevelJson], size_usdc: f64) -> Option<(f64, f64)> {
    if size_usdc <= 0.0 {
        return None;
    }
    let mut left = size_usdc;
    let mut ln_sum = 0.0;
    for l in levels {
        let take = left.min(l.size_usdc);
        ln_sum += take * l.time_secs.ln();
        left -= take;
        if left <= 1e-9 {
            return Some(((ln_sum / size_usdc).exp(), l.time_secs));
        }
    }
    None
}
