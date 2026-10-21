//! JSON views shared by the REST routes and the WebSocket channels.
use std::collections::HashMap;

use serde_json::{Value, json};
use tradingtime_interface::{Market, ONE, Operator, OperatorStatus, Trader};
use tradingtime_math::curve::{self, Curve};
use tradingtime_math::{Fx, Side, funding, position, to_f64};

use crate::candles;
use crate::store::{FillRow, OrderRow, Store, b58};

#[derive(Debug)]
pub enum ViewError {
    BadRequest(String),
    NotFound,
    NotSynced,
    Internal(String),
}

impl From<anyhow::Error> for ViewError {
    fn from(e: anyhow::Error) -> Self {
        ViewError::Internal(e.to_string())
    }
}

pub type ViewResult = Result<Value, ViewError>;

const QUOTE_LEVELS: u64 = 8;
const DAY: i64 = 86_400;

pub fn secs(ln: Fx) -> f64 {
    to_f64(ln).exp()
}

pub fn usdc(v: i128) -> f64 {
    v as f64 / 1e6
}

fn side_str(s: u8) -> &'static str {
    if s == Side::Short as u8 { "short" } else { "long" }
}

fn market(store: &Store) -> Result<Market, ViewError> {
    store.market()?.ok_or(ViewError::NotSynced)
}

pub fn health(store: &Store) -> Value {
    json!({ "slot": store.slot() })
}

pub fn market_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    let mark = m.effective_mark(now);
    let index = m.index_ln.get();
    let period = m.params.funding_period_secs.max(1) as f64;
    let volume: u64 = store.trade_sizes(now - DAY)?.iter().map(|t| t.1).sum();
    let change = match store.mark_near(now - DAY)? {
        Some(past) => (to_f64(mark) - past).exp() - 1.0,
        None => 0.0,
    };
    let sol = (m.last_pyth_price > 0).then(|| m.last_pyth_price as f64 * 10f64.powi(m.last_pyth_expo));
    let sol_change = match (sol, store.pyth_price_near(now - DAY)?) {
        (Some(p), Some(past)) if past > 0.0 => p / past - 1.0,
        _ => 0.0,
    };
    Ok(json!({
        "sol_price_usd": sol,
        "sol_change_24h": sol_change,
        "sol_price_ts": m.last_publish_time,
        "mark_secs": secs(mark),
        "index_secs": secs(index),
        "mark_ln": mark.to_string(),
        "index_ln": index.to_string(),
        "funding_rate_per_hour": to_f64(mark - index) * 3_600.0 / period,
        "long_oi_usdc": usdc(m.long_q as i128),
        "short_oi_usdc": usdc(m.short_q as i128),
        "volume_24h_usdc": usdc(volume as i128),
        "change_24h": change,
        "paused": m.paused != 0,
        "insurance_usdc": usdc(m.insurance as i128),
        "slot": store.slot(),
        "updated_ts": m.mark_updated_ts.max(m.index_updated_ts).max(m.funding_updated_ts),
    }))
}

pub fn candles_view(store: &Store, interval: &str, limit: usize) -> ViewResult {
    let iv = candles::interval_secs(interval).ok_or_else(|| ViewError::BadRequest(format!("bad interval {interval}")))?;
    if limit == 0 || limit > 500 {
        return Err(ViewError::BadRequest("limit must be 1..=500".into()));
    }
    let now = store.now();
    let since = (now - iv * limit as i64).div_euclid(iv) * iv;
    let mut points = Vec::new();
    // The close of the bucket before the window seeds forward-fill.
    if let Some(prev) = store.mark_before(since)? {
        points.push((since - 1, prev));
    }
    points.extend(store.mark_points(since)?);
    let trades = store.trade_sizes(since)?;
    let out = candles::build(&points, &trades, iv, limit, now);
    serde_json::to_value(out).map_err(|e| ViewError::Internal(e.to_string()))
}

pub fn index_view(store: &Store, limit: usize) -> ViewResult {
    let pts = store.index_points(limit)?;
    let mut out = Vec::with_capacity(pts.len());
    for p in pts {
        let mark = store.mark_near(p.ts)?.map(f64::exp).unwrap_or_else(|| secs(p.index_ln));
        out.push(json!({ "t": p.ts, "index_secs": secs(p.index_ln), "mark_secs": mark }));
    }
    Ok(Value::Array(out))
}

struct OperatorRow {
    owner: String,
    op: Operator,
    q: i64,
    fresh: bool,
}

fn operator_rows(store: &Store, m: &Market) -> Result<Vec<OperatorRow>, ViewError> {
    let q_by_trader: HashMap<[u8; 32], i64> = store.traders()?.into_iter().map(|(k, t)| (k, t.q)).collect();
    let slot = store.slot();
    let mut rows: Vec<OperatorRow> = store
        .operators()?
        .into_iter()
        .map(|(_, op)| OperatorRow {
            owner: b58(&op.owner),
            q: q_by_trader.get(&op.trader).copied().unwrap_or(0),
            fresh: op.is_fresh(slot, m.params.curve_max_age_slots),
            op,
        })
        .collect();
    rows.sort_by(|a, b| b.op.updated_slot.cmp(&a.op.updated_slot).then(a.owner.cmp(&b.owner)));
    Ok(rows)
}

pub fn operator_status(status: u8) -> &'static str {
    match OperatorStatus::from_u8(status) {
        Some(OperatorStatus::Active) => "active",
        Some(OperatorStatus::Unbonding) => "unbonding",
        _ => "closed",
    }
}

fn operator_json(r: &OperatorRow) -> Value {
    let status = operator_status(r.op.status);
    json!({
        "owner": r.owner,
        "fair_secs": secs(r.op.fair_ln.get()),
        "half_spread": to_f64(r.op.half_spread.get()),
        "impact_per_usdc": to_f64(r.op.impact_per_usdc.get()),
        "inventory_limit_usdc": usdc(r.op.inventory_limit as i128),
        "inventory_usdc": usdc(r.q as i128),
        "updated_slot": r.op.updated_slot,
        "fresh": r.fresh,
        "status": status,
    })
}

pub fn operators_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    Ok(Value::Array(operator_rows(store, &m)?.iter().map(operator_json).collect()))
}

/// Aggregated ladder: level k is the average price of the k-th of 8 equal slices of the
/// side's total depth (Σ curve room), routed across all fresh curves.
pub fn ladder(curves: &[Curve], side: Side) -> Vec<Value> {
    let limit = match side {
        Side::Long => ONE * 100,
        Side::Short => -ONE * 100,
    };
    let depth: u128 = curves.iter().map(|c| curve::room(c, side) as u128).sum();
    let mut out = Vec::new();
    let (mut prev_total, mut prev_notional) = (0u64, 0i128);
    for k in 1..=QUOTE_LEVELS {
        let target = (depth * k as u128 / QUOTE_LEVELS as u128).min(u64::MAX as u128) as u64;
        if target == 0 {
            continue;
        }
        let Some(r) = curve::route(curves, side, target, limit) else { break };
        if r.total <= prev_total {
            break;
        }
        let chunk = r.total - prev_total;
        let avg = (r.ln_notional - prev_notional) / chunk as i128;
        out.push(json!({ "time_secs": secs(avg), "size_usdc": usdc(chunk as i128) }));
        prev_total = r.total;
        prev_notional = r.ln_notional;
    }
    out
}

pub fn quotes_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    let rows = operator_rows(store, &m)?;
    let curves: Vec<Curve> = rows.iter().filter(|r| r.fresh).take(8).map(|r| r.op.curve(r.q)).collect();
    Ok(json!({
        "mark_secs": secs(m.effective_mark(store.now())),
        "asks": ladder(&curves, Side::Long),
        "bids": ladder(&curves, Side::Short),
        "operators": rows.iter().map(operator_json).collect::<Vec<_>>(),
    }))
}

pub struct Derived {
    pub equity: i64,
    pub upnl: i64,
    pub funding: i64,
    pub maintenance: i64,
    pub entry_ln: Option<Fx>,
    pub liq_ln: Option<Fx>,
}

/// The funding index a CrankFunding at `now` would write.
pub fn funding_index_at(m: &Market, now: i64) -> Fx {
    let fi = m.funding_index.get();
    let dt = now - m.funding_updated_ts;
    funding::accrue(fi, m.effective_mark(now), m.index_ln.get(), dt, m.params.funding_period_secs).unwrap_or(fi)
}

pub fn derive(m: &Market, t: &Trader, now: i64) -> Derived {
    let mut p = t.position();
    let funding = position::settle_funding(&mut p, funding_index_at(m, now)).unwrap_or(0);
    let mark = m.effective_mark(now);
    let equity = position::equity(&p, mark).unwrap_or(i64::MIN);
    let upnl = (p.q as i128 * mark - p.cost).div_euclid(ONE) as i64;
    let maintenance = position::maintenance(p.q, m.params.maintenance_bps);
    let (entry_ln, liq_ln) = if p.q == 0 {
        (None, None)
    } else {
        let q = p.q as i128;
        let liq = (p.cost + (maintenance as i128 - p.cash as i128) * ONE) / q;
        (Some(p.cost / q), Some(liq))
    };
    Derived { equity, upnl, funding, maintenance, entry_ln, liq_ln }
}

fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

pub fn trader_view(store: &Store, owner: &str) -> ViewResult {
    let m = market(store)?;
    let (pda, t) = store.trader_by_owner(owner)?.ok_or(ViewError::NotFound)?;
    let d = derive(&m, &t, store.now());
    Ok(json!({
        "owner": owner,
        "trader": b58(&pda),
        "cash_usdc": usdc(t.cash as i128),
        "q_usdc": usdc(t.q as i128),
        "entry_secs": d.entry_ln.map(|l| finite(secs(l))).unwrap_or(Value::Null),
        "equity_usdc": usdc(d.equity as i128),
        "upnl_usdc": usdc(d.upnl as i128),
        "funding_usdc": usdc(d.funding as i128),
        "liq_secs": d.liq_ln.map(|l| finite(secs(l))).unwrap_or(Value::Null),
        "reserved_usdc": usdc((t.margin_need() - t.q.unsigned_abs() as u128) as i128),
        "reserved_long_usdc": usdc(t.reserved_long as i128),
        "reserved_short_usdc": usdc(t.reserved_short as i128),
        "is_operator": t.is_operator != 0,
    }))
}

pub fn fill_json(f: &FillRow) -> Value {
    let kind = match f.kind {
        1 => "limit",
        2 => "liquidation",
        3 => "adl",
        _ => "swap",
    };
    json!({
        "signature": f.signature,
        "slot": f.slot,
        "t": f.ts,
        "side": side_str(f.side),
        "kind": kind,
        "size_usdc": usdc(f.size as i128),
        "price_secs": secs(f.ln_price),
        "fee_usdc": usdc(f.fee as i128),
        "operator": f.operator,
    })
}

pub fn fills_view(store: &Store, owner: &str, limit: usize) -> ViewResult {
    Ok(Value::Array(store.fills_for(owner, limit)?.iter().map(fill_json).collect()))
}

fn order_status(o: &OrderRow, now: i64) -> &str {
    match (o.status.as_str(), o.expiry_ts) {
        ("open", Some(e)) if e > 0 && e < now => "expired",
        (s, _) => s,
    }
}

pub fn trader_orders_view(store: &Store, owner: &str) -> ViewResult {
    let now = store.now();
    Ok(Value::Array(
        store
            .orders_for(owner)?
            .iter()
            .map(|o| {
                json!({
                    "order_id": o.order_id,
                    "side": side_str(o.side),
                    "size_usdc": usdc(o.size as i128),
                    "filled_usdc": usdc(o.filled as i128),
                    "limit_secs": secs(o.limit_ln),
                    "expiry_t": o.expiry_ts,
                    "created_slot": o.created_slot,
                    "status": order_status(o, now),
                })
            })
            .collect(),
    ))
}

pub fn open_orders_view(store: &Store) -> ViewResult {
    let now = store.now();
    Ok(Value::Array(
        store
            .open_orders()?
            .iter()
            .filter(|o| order_status(o, now) == "open")
            .map(|o| {
                json!({
                    "owner": o.owner,
                    "order_id": o.order_id,
                    "side": side_str(o.side),
                    "size_usdc": usdc(o.size as i128),
                    "filled_usdc": usdc(o.filled as i128),
                    "limit_secs": secs(o.limit_ln),
                    "expiry_t": o.expiry_ts,
                })
            })
            .collect(),
    ))
}

pub fn funding_view(store: &Store, owner: &str, limit: usize) -> ViewResult {
    Ok(Value::Array(
        store
            .funding_for(owner, limit)?
            .iter()
            .map(|(t, a)| json!({ "t": t, "amount_usdc": usdc(*a as i128) }))
            .collect(),
    ))
}

pub fn positions_view(store: &Store, side: Option<Side>, in_profit: bool, limit: usize) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    let mut rows: Vec<(i64, Value)> = store
        .traders()?
        .iter()
        .filter(|(_, t)| t.q != 0)
        .filter(|(_, t)| match side {
            Some(Side::Long) => t.q > 0,
            Some(Side::Short) => t.q < 0,
            None => true,
        })
        .filter_map(|(k, t)| {
            let d = derive(&m, t, now);
            if in_profit && d.upnl <= 0 {
                return None;
            }
            Some((
                d.upnl,
                json!({
                    "owner": b58(&t.owner),
                    "trader": b58(k),
                    "q_usdc": usdc(t.q as i128),
                    "upnl_usdc": usdc(d.upnl as i128),
                    "equity_usdc": usdc(d.equity as i128),
                    "is_operator": t.is_operator != 0,
                }),
            ))
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    Ok(Value::Array(rows.into_iter().take(limit).map(|r| r.1).collect()))
}

pub fn liquidatable_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    Ok(Value::Array(
        store
            .traders()?
            .iter()
            .filter(|(_, t)| t.q != 0)
            .filter_map(|(k, t)| {
                let d = derive(&m, t, now);
                (d.equity < d.maintenance).then(|| {
                    json!({
                        "owner": b58(&t.owner),
                        "trader": b58(k),
                        "equity_usdc": usdc(d.equity as i128),
                        "maintenance_usdc": usdc(d.maintenance as i128),
                    })
                })
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{market as mk_market, operator, trader};

    #[test]
    fn derived_values_match_math() {
        let mut m = mk_market(3600.0, 3600.0, 100);
        m.funding_updated_ts = 100;
        // Long 1000 USDC entered at 1800s, mark 3600s: upnl = 1000 * ln 2.
        let t = trader([1; 32], 100_000_000, 1_000_000_000, 1800.0);
        let d = derive(&m, &t, 100);
        let expect = 1_000_000_000.0 * 2f64.ln();
        assert!((d.upnl as f64 - expect).abs() < 10.0);
        assert_eq!(d.equity, 100_000_000 + d.upnl);
        assert_eq!(d.maintenance, 50_000_000);
        assert!((secs(d.entry_ln.unwrap()) - 1800.0).abs() < 1e-6);
        // At the liq price equity == maintenance.
        let mut at_liq = m;
        at_liq.mark_ln.set(d.liq_ln.unwrap());
        let e = derive(&at_liq, &t, 100);
        assert!((e.equity - e.maintenance).abs() <= 1);
        assert!(derive(&m, &trader([1; 32], 5, 0, 1.0), 100).liq_ln.is_none());
    }

    #[test]
    fn ladder_is_monotone_and_sized() {
        let op = operator([1; 32], [2; 32], 3600.0, 10);
        let curves = vec![op.curve(0), op.curve(0)];
        let asks = ladder(&curves, Side::Long);
        let bids = ladder(&curves, Side::Short);
        assert_eq!(asks.len(), 8);
        assert_eq!(bids.len(), 8);
        let a: Vec<f64> = asks.iter().map(|v| v["time_secs"].as_f64().unwrap()).collect();
        let b: Vec<f64> = bids.iter().map(|v| v["time_secs"].as_f64().unwrap()).collect();
        assert!(a.windows(2).all(|w| w[1] > w[0]) && b.windows(2).all(|w| w[1] < w[0]));
        assert!(a[0] > 3600.0 && b[0] < 3600.0);
        // Depth 2 × 20k per side → 5k slices.
        assert!((asks[0]["size_usdc"].as_f64().unwrap() - 5000.0).abs() < 1.0);
        assert!(ladder(&[], Side::Long).is_empty());
    }

    #[test]
    fn ladder_step_is_depth_over_eight() {
        let mut a = operator([1; 32], [2; 32], 3600.0, 10);
        a.inventory_limit = 10_000_000_000;
        let mut b = operator([3; 32], [4; 32], 3600.0, 10);
        b.inventory_limit = 6_000_000_000;
        let curves = vec![a.curve(0), b.curve(0)];
        for side in [Side::Long, Side::Short] {
            let lv = ladder(&curves, side);
            assert_eq!(lv.len(), 8);
            let sizes: Vec<f64> = lv.iter().map(|v| v["size_usdc"].as_f64().unwrap()).collect();
            assert!(sizes.iter().all(|s| (s - 2000.0).abs() < 1e-6), "{sizes:?}");
            assert!((sizes.iter().sum::<f64>() - 16_000.0).abs() < 1e-6);
        }
        a.inventory_limit = 4_000_000_000;
        b.inventory_limit = 4_000_000_000;
        let lv = ladder(&[a.curve(0), b.curve(0)], Side::Long);
        assert_eq!(lv.len(), 8);
        assert!(lv.iter().all(|v| (v["size_usdc"].as_f64().unwrap() - 1000.0).abs() < 1e-6));
        let mut full = a;
        full.inventory_limit = 0;
        assert!(ladder(&[full.curve(0)], Side::Long).is_empty());
    }

    #[test]
    fn funding_accrues_to_now() {
        // Long 1000 USDC, mark 3700 > index 3600: the long pays, more as time passes.
        let mut m = mk_market(3700.0, 3600.0, 1_000);
        m.funding_updated_ts = 1_000;
        let t = trader([1; 32], 100_000_000, 1_000_000_000, 3700.0);
        let d0 = derive(&m, &t, 1_000);
        let d1 = derive(&m, &t, 1_030);
        let d2 = derive(&m, &t, 1_060);
        assert_eq!(d0.funding, 0);
        assert!(d1.funding < 0 && d2.funding < d1.funding, "{} {}", d1.funding, d2.funding);
        assert_eq!(d1.equity - d2.equity, d1.funding - d2.funding);
        let period = m.params.funding_period_secs;
        let fi = tradingtime_math::funding::accrue(0, m.mark_ln.get(), m.index_ln.get(), 60, period).unwrap();
        let mut p = t.position();
        let expect = position::settle_funding(&mut p, fi).unwrap();
        assert_eq!(d2.funding, expect);
    }
}
