//! Analytics page view-model (.agent/analytics-api.md "Web").

use super::format::{duration, money, money_compact, money_signed, pct_signed, short_addr, timestamp, DASH};
use super::rows::{side_label, side_of};
use super::Side;
use crate::types::*;

/// Hash route: `#/analytics` = Analytics, `#/docs[/section]` = Docs; empty, `#/` or anything else = Trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Trade,
    Analytics,
    Docs,
}

pub fn page_from_hash(hash: &str) -> Page {
    let h = hash.trim_start_matches('#').trim_start_matches('/');
    let h = h.split(['?', '/']).next().unwrap_or("");
    if h.eq_ignore_ascii_case("analytics") {
        Page::Analytics
    } else if h.eq_ignore_ascii_case("docs") {
        Page::Docs
    } else {
        Page::Trade
    }
}

/// (query value, label)
pub const SORTS: [(&str, &str); 3] = [("pnl", "PnL"), ("volume", "Volume"), ("pnl_pct", "PnL %")];

/// "12s ago", "5m ago", "3h ago", "2d ago"; future/zero skew -> "just now"; t <= 0 -> dash.
pub fn relative_time(t: i64, now: i64) -> String {
    if t <= 0 {
        return DASH.into();
    }
    let d = now - t;
    match d {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{d}s ago"),
        60..=3_599 => format!("{}m ago", d / 60),
        3_600..=86_399 => format!("{}h ago", d / 3_600),
        _ => format!("{}d ago", d / 86_400),
    }
}

/// Short form for display (title carries the full key); empty -> dash.
pub fn owner_short(owner: &str) -> String {
    if owner.is_empty() { DASH.into() } else { short_addr(owner) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kpi {
    pub id: &'static str,
    pub label: &'static str,
    pub value: String,
    pub sub: String,
}

/// The seven KPI cards; values are dashes until the summary arrives.
pub fn kpis(s: Option<&AnalyticsSummaryJson>) -> Vec<Kpi> {
    let k = |id, label, value: String, sub: String| Kpi { id, label, value, sub };
    let Some(s) = s else {
        return [
            ("kpi-volume-24h", "24h volume"),
            ("kpi-oi", "Open interest"),
            ("kpi-traders", "Traders"),
            ("kpi-tvl", "TVL"),
            ("kpi-fees", "24h fees"),
            ("kpi-insurance", "Insurance fund"),
            ("kpi-liquidations-24h", "24h liquidations"),
        ]
        .into_iter()
        .map(|(id, label)| k(id, label, DASH.into(), String::new()))
        .collect();
    };
    vec![
        k(
            "kpi-volume-24h",
            "24h volume",
            money_compact(s.volume_24h_usdc),
            format!("{} trades \u{b7} {} all time", s.trades_24h, money_compact(s.volume_total_usdc)),
        ),
        k(
            "kpi-oi",
            "Open interest",
            money_compact(s.long_oi_usdc + s.short_oi_usdc),
            format!("long {} \u{b7} short {}", money_compact(s.long_oi_usdc), money_compact(s.short_oi_usdc)),
        ),
        k(
            "kpi-traders",
            "Traders",
            s.traders.to_string(),
            format!("{} active 24h \u{b7} {} in profit \u{b7} {} makers", s.active_traders_24h, s.traders_in_profit, s.makers),
        ),
        k("kpi-tvl", "TVL", money_compact(s.tvl_usdc), format!("{} deposited", money_compact(s.deposits_total_usdc))),
        k(
            "kpi-fees",
            "24h fees",
            money(s.fees_24h_usdc),
            format!("{} all time \u{b7} treasury {}", money_compact(s.fees_total_usdc), money_compact(s.treasury_usdc)),
        ),
        k(
            "kpi-insurance",
            "Insurance fund",
            money(s.insurance_usdc),
            format!("trader PnL {} \u{b7} maker PnL {}", money_signed(s.total_trader_pnl_usdc), money_signed(s.total_maker_pnl_usdc)),
        ),
        k(
            "kpi-liquidations-24h",
            "24h liquidations",
            s.liquidations_24h.to_string(),
            format!("ADL {}", money_compact(s.adl_volume_24h_usdc)),
        ),
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeBar {
    pub x: f64,
    pub w: f64,
    /// Long sits on the baseline; short stacks above it.
    pub long_y: f64,
    pub long_h: f64,
    pub short_y: f64,
    pub short_h: f64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub pos: f64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeGeom {
    pub bars: Vec<VolumeBar>,
    /// Horizontal gridlines (y, label), top first.
    pub y_ticks: Vec<Tick>,
    /// Time labels (x, label) for first, middle and last bucket.
    pub x_ticks: Vec<Tick>,
}

const BAR: f64 = 0.72;

/// Stacked long/short bars in a `width`×`height` box (y down), scaled to the tallest bucket.
pub fn volume_layout(buckets: &[VolumeBucketJson], width: f64, height: f64) -> VolumeGeom {
    if buckets.is_empty() || width <= 0.0 || height <= 0.0 {
        return VolumeGeom { bars: vec![], y_ticks: vec![], x_ticks: vec![] };
    }
    let total = |b: &VolumeBucketJson| b.long_volume_usdc.max(0.0) + b.short_volume_usdc.max(0.0);
    let max = buckets.iter().map(total).fold(0.0, f64::max);
    let scale = if max > 0.0 { height / max } else { 0.0 };
    let slot = width / buckets.len() as f64;
    let bars = buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let lh = b.long_volume_usdc.max(0.0) * scale;
            let sh = b.short_volume_usdc.max(0.0) * scale;
            VolumeBar {
                x: i as f64 * slot + slot * (1.0 - BAR) / 2.0,
                w: slot * BAR,
                long_y: height - lh,
                long_h: lh,
                short_y: height - lh - sh,
                short_h: sh,
                title: format!(
                    "{} \u{b7} long {} \u{b7} short {} \u{b7} {} trades",
                    timestamp(b.ts),
                    money_compact(b.long_volume_usdc),
                    money_compact(b.short_volume_usdc),
                    b.trades
                ),
            }
        })
        .collect();
    let y_ticks = if max > 0.0 {
        (0..3).map(|k| {
            let v = max * (1.0 - k as f64 / 2.0);
            Tick { pos: height - v * scale, label: money_compact(v) }
        })
        .collect()
    } else {
        vec![]
    };
    let n = buckets.len();
    let mut idx = vec![0];
    if n > 2 {
        idx.push(n / 2);
    }
    if n > 1 {
        idx.push(n - 1);
    }
    let x_ticks = idx.into_iter().map(|i| Tick { pos: i as f64 * slot + slot / 2.0, label: timestamp(buckets[i].ts) }).collect();
    VolumeGeom { bars, y_ticks, x_ticks }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistBin {
    pub lo: f64,
    pub hi: f64,
    pub count: usize,
    pub x: f64,
    pub w: f64,
    pub y: f64,
    pub h: f64,
    /// Bin lies at or above zero.
    pub gain: bool,
}

/// `bins` (rounded up to even, min 2) equal-width bins symmetric about zero, so zero is always an edge.
/// Values exactly on an inner edge go to the upper bin; the extremes land in the outer bins.
pub fn pnl_histogram(pnls: &[f64], bins: usize, width: f64, height: f64) -> Vec<HistBin> {
    let vals: Vec<f64> = pnls.iter().copied().filter(|v| v.is_finite()).collect();
    if vals.is_empty() {
        return vec![];
    }
    let n = bins.max(2).div_ceil(2) * 2;
    let m = vals.iter().fold(0.0_f64, |a, v| a.max(v.abs()));
    let m = if m > 0.0 { m } else { 1.0 };
    let bw = 2.0 * m / n as f64;
    let mut counts = vec![0usize; n];
    for v in &vals {
        let i = (((v + m) / bw).floor() as isize).clamp(0, n as isize - 1) as usize;
        counts[i] += 1;
    }
    let max = *counts.iter().max().unwrap_or(&1) as f64;
    let slot = width / n as f64;
    counts
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let lo = -m + i as f64 * bw;
            let h = c as f64 / max * height;
            HistBin { lo, hi: lo + bw, count: c, x: i as f64 * slot + 1.0, w: (slot - 2.0).max(1.0), y: height - h, h, gain: lo >= 0.0 }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeaderRow {
    pub rank: usize,
    pub owner: String,
    pub owner_short: String,
    pub maker: bool,
    pub equity: String,
    pub pnl: String,
    pub pnl_pct: String,
    pub gain: bool,
    /// PnL with this account's taker fees added back.
    pub pnl_before_fees: String,
    pub before_fees_gain: bool,
    pub fees: String,
    pub position: String,
    /// None when flat.
    pub position_side: Option<Side>,
    pub volume: String,
    pub trades: String,
    pub last_trade: String,
}

pub fn leaderboard_rows(rows: &[LeaderboardRowJson], now: i64) -> Vec<LeaderRow> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let side = if r.q_usdc > 0.000_001 {
                Some(Side::Long)
            } else if r.q_usdc < -0.000_001 {
                Some(Side::Short)
            } else {
                None
            };
            LeaderRow {
                rank: i + 1,
                owner: r.owner.clone(),
                owner_short: owner_short(&r.owner),
                maker: r.is_operator,
                equity: money(r.equity_usdc),
                pnl: money_signed(r.pnl_usdc),
                pnl_pct: pct_signed(r.pnl_pct, 2),
                gain: r.pnl_usdc >= 0.0,
                pnl_before_fees: money_signed(r.pnl_before_fees_usdc),
                before_fees_gain: r.pnl_before_fees_usdc >= 0.0,
                fees: money(r.fees_paid_usdc),
                position: match side {
                    Some(s) => format!("{} {}", money_compact(r.q_usdc.abs()), if s == Side::Long { "long" } else { "short" }),
                    None => DASH.into(),
                },
                position_side: side,
                volume: money_compact(r.volume_usdc),
                trades: r.trades.to_string(),
                last_trade: r.last_trade_ts.map(|t| relative_time(t, now)).unwrap_or_else(|| DASH.into()),
            }
        })
        .collect()
}

/// Trader (non-operator) PnLs for the distribution.
pub fn trader_pnls(rows: &[LeaderboardRowJson]) -> Vec<f64> {
    rows.iter().filter(|r| !r.is_operator).map(|r| r.pnl_usdc).collect()
}

/// Colour of a flow-of-funds amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Gain,
    Loss,
    Plain,
}

impl Tone {
    pub fn signed(v: f64) -> Tone {
        if v >= 0.0 { Tone::Gain } else { Tone::Loss }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowLine {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
    /// Short muted explanation.
    pub note: &'static str,
    /// Result line (drawn with a rule above it).
    pub total: bool,
}

/// One segment of the "where trader losses went" bar; `pct` of the bar width.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowSeg {
    pub label: &'static str,
    pub value: String,
    pub pct: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowOfFunds {
    /// Before fees, minus fees, equals trader PnL.
    pub traders: Vec<FlowLine>,
    /// Makers, insurance fund, treasury, liquidation penalties.
    pub counterparties: Vec<FlowLine>,
    /// Trader outflows (fees, net to makers); empty when there are none.
    pub bar: Vec<FlowSeg>,
    pub sentence: String,
}

/// "Where trader PnL went", from the summary; None until it arrives.
pub fn flow_of_funds(s: Option<&AnalyticsSummaryJson>) -> Option<FlowOfFunds> {
    let s = s?;
    let line = |label, v: f64, tone, note, total| FlowLine { label, value: money_signed(v), tone, note, total };
    let before = s.trader_pnl_before_fees_usdc;
    let fees = s.trader_fees_paid_usdc.max(0.0);
    let pnl = s.total_trader_pnl_usdc;
    let traders = vec![
        line("Trader PnL before fees", before, Tone::signed(before), "what trading itself made or lost", false),
        line("Fees paid", -fees, if fees > 0.0 { Tone::Loss } else { Tone::Plain }, "taker fees on every fill", false),
        line("Trader PnL", pnl, Tone::signed(pnl), "what traders actually kept", true),
    ];
    let plain = |label, v: f64, note| FlowLine { label, value: money(v), tone: Tone::Plain, note, total: false };
    let counterparties = vec![
        line("Makers' PnL", s.maker_pnl_usdc, Tone::signed(s.maker_pnl_usdc), "the other side of trader PnL before fees", false),
        plain("Insurance fund", s.insurance_usdc, "fees fill this first, up to a target"),
        plain("Treasury", s.treasury_usdc, "fees beyond the insurance target"),
        plain("Liquidation penalties", s.liquidation_penalties_usdc, "paid by liquidated traders"),
    ];
    let to_makers = (-before).max(0.0);
    let out = fees + to_makers;
    let bar = if out > 0.0 {
        [("Fees", fees), ("Net to makers", to_makers)]
            .into_iter()
            .filter(|(_, v)| *v > 0.0)
            .map(|(label, v)| FlowSeg { label, value: money_compact(v), pct: v / out * 100.0 })
            .collect()
    } else {
        vec![]
    };
    Some(FlowOfFunds { traders, counterparties, bar, sentence: flow_sentence(before, fees, pnl) })
}

/// One plain-English sentence about where trader PnL went.
pub fn flow_sentence(before: f64, fees: f64, pnl: f64) -> String {
    let c = money_compact;
    if fees.abs() < 0.005 && before.abs() < 0.005 {
        return "No trader fees or PnL yet.".into();
    }
    let total = if pnl < 0.0 { format!("Traders lost {} in total", c(-pnl)) } else { format!("Traders made {} in total", c(pnl)) };
    if before < 0.0 {
        format!("{total}: they paid {} in fees and {} net to makers.", c(fees), c(-before))
    } else {
        format!("{total}: they won {} net from makers and paid {} in fees.", c(before), c(fees))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MakerRow {
    pub owner: String,
    pub owner_short: String,
    pub status: String,
    pub fresh: bool,
    pub bond: String,
    pub equity: String,
    pub pnl: String,
    pub gain: bool,
    pub inventory: String,
    pub volume: String,
    pub fills: String,
    pub spread: String,
    pub fair: String,
}

pub fn maker_rows(rows: &[MakerRowJson]) -> Vec<MakerRow> {
    rows.iter()
        .map(|r| MakerRow {
            owner: r.owner.clone(),
            owner_short: owner_short(&r.owner),
            status: if r.status.is_empty() { DASH.into() } else { r.status.clone() },
            fresh: r.fresh,
            bond: money_compact(r.bond_usdc),
            equity: money_compact(r.equity_usdc),
            pnl: money_signed(r.pnl_usdc),
            gain: r.pnl_usdc >= 0.0,
            inventory: money_signed(r.inventory_usdc),
            volume: money_compact(r.volume_usdc),
            fills: r.fills.to_string(),
            spread: format!("{:.1} bps", r.half_spread_bps),
            fair: duration(r.fair_secs),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeRow {
    pub time: String,
    pub trader: String,
    pub trader_short: String,
    pub side: Side,
    pub side_label: String,
    pub kind: String,
    pub size: String,
    pub price: String,
    pub fee: String,
}

pub fn trade_rows(rows: &[AnalyticsTradeJson], now: i64) -> Vec<TradeRow> {
    rows.iter()
        .map(|r| {
            let side = side_of(r.fill.side);
            TradeRow {
                time: relative_time(r.fill.t, now),
                trader: r.trader.clone(),
                trader_short: owner_short(&r.trader),
                side,
                side_label: side_label(side),
                kind: r.fill.kind.clone(),
                size: money(r.fill.size_usdc),
                price: duration(r.fill.price_secs),
                fee: money(r.fill.fee_usdc),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiqRow {
    pub time: String,
    pub trader: String,
    pub trader_short: String,
    pub liquidator: String,
    pub liquidator_short: String,
    pub size: String,
    pub bankruptcy: String,
    pub penalty: String,
    pub insurance: String,
}

pub fn liquidation_rows(rows: &[LiquidationJson], now: i64) -> Vec<LiqRow> {
    rows.iter()
        .map(|r| LiqRow {
            time: relative_time(r.t, now),
            trader: r.trader.clone(),
            trader_short: owner_short(&r.trader),
            liquidator: r.liquidator.clone(),
            liquidator_short: owner_short(&r.liquidator),
            size: money(r.size_usdc),
            bankruptcy: duration(r.bankruptcy_secs),
            penalty: money(r.penalty_usdc),
            insurance: money(r.insurance_used_usdc),
        })
        .collect()
}
