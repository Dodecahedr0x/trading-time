use super::format::{duration, money, money_signed, timestamp, DASH};
use super::Side;
use crate::types::{FillJson, FundingJson, OrderJson, SideStr, TraderJson};

pub fn side_of(s: SideStr) -> Side {
    match s {
        SideStr::Long => Side::Long,
        SideStr::Short => Side::Short,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionRow {
    pub side: Side,
    /// "Long time" / "Short time"
    pub side_label: String,
    pub size: String,
    pub entry: String,
    pub mark: String,
    pub upnl: String,
    pub upnl_positive: bool,
    pub equity: String,
    pub liq: String,
    pub funding: String,
    pub funding_positive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderRow {
    pub order_id: u64,
    pub side: Side,
    pub side_label: String,
    pub size: String,
    pub filled: String,
    pub limit: String,
    pub expiry: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FillRow {
    pub time: String,
    pub side: Side,
    pub side_label: String,
    pub kind: String,
    pub size: String,
    pub price: String,
    pub fee: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FundingRow {
    pub time: String,
    pub amount: String,
    pub positive: bool,
}

pub fn side_label(side: Side) -> String {
    match side {
        Side::Long => "Long time".into(),
        Side::Short => "Short time".into(),
    }
}

fn opt_duration(v: Option<f64>) -> String {
    v.map(duration).unwrap_or_else(|| DASH.into())
}

/// One row when |q| >= 0.000001 USDC, else empty.
pub fn position_rows(t: Option<&TraderJson>, mark_secs: f64) -> Vec<PositionRow> {
    let Some(t) = t else { return vec![] };
    if t.q_usdc.abs() < 0.000_001 {
        return vec![];
    }
    let side = if t.q_usdc > 0.0 { Side::Long } else { Side::Short };
    vec![PositionRow {
        side,
        side_label: side_label(side),
        size: money(t.q_usdc.abs()),
        entry: opt_duration(t.entry_secs),
        mark: duration(mark_secs),
        upnl: money_signed(t.upnl_usdc),
        upnl_positive: t.upnl_usdc >= 0.0,
        equity: money(t.equity_usdc),
        liq: opt_duration(t.liq_secs),
        funding: money_signed(t.funding_usdc),
        funding_positive: t.funding_usdc >= 0.0,
    }]
}

/// Only status "open" (or empty status).
pub fn order_rows(orders: &[OrderJson]) -> Vec<OrderRow> {
    orders
        .iter()
        .filter(|o| o.status.is_empty() || o.status == "open")
        .map(|o| {
            let side = side_of(o.side);
            OrderRow {
                order_id: o.order_id,
                side,
                side_label: side_label(side),
                size: money(o.size_usdc),
                filled: money(o.filled_usdc),
                limit: duration(o.limit_secs),
                expiry: if o.expiry_t > 0 { timestamp(o.expiry_t) } else { DASH.into() },
            }
        })
        .collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

pub fn fill_rows(fills: &[FillJson]) -> Vec<FillRow> {
    fills
        .iter()
        .map(|f| {
            let side = side_of(f.side);
            FillRow {
                time: timestamp(f.t),
                side,
                side_label: side_label(side),
                kind: capitalize(&f.kind),
                size: money(f.size_usdc),
                price: duration(f.price_secs),
                fee: money(f.fee_usdc),
            }
        })
        .collect()
}

pub fn funding_rows(f: &[FundingJson]) -> Vec<FundingRow> {
    f.iter()
        .map(|r| FundingRow { time: timestamp(r.t), amount: money_signed(r.amount_usdc), positive: r.amount_usdc >= 0.0 })
        .collect()
}
