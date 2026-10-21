//! Aggregate analytics views behind `/v1/analytics/*` (contract: .agent/analytics-api.md).
use std::collections::HashMap;

use serde_json::{Value, json};
use tradingtime_interface::{Market, Trader};
use tradingtime_math::to_f64;

use crate::store::{Store, b58, owner_of};
use crate::views::{ViewError, ViewResult, derive, fill_json, operator_status, secs, usdc};

const DAY: i64 = 86_400;

pub use crate::candles::interval_secs;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    Pnl,
    Volume,
    PnlPct,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Sort> {
        match s {
            "pnl" => Some(Sort::Pnl),
            "volume" => Some(Sort::Volume),
            "pnl_pct" => Some(Sort::PnlPct),
            _ => None,
        }
    }
}

fn market(store: &Store) -> Result<Market, ViewError> {
    store.market()?.ok_or(ViewError::NotSynced)
}

/// pnl = equity − net deposits; pct = pnl / net deposits (0 when net deposits ≤ 0).
fn pnl(equity: i64, net_deposits: i64) -> (i64, f64) {
    let pnl = equity.saturating_sub(net_deposits);
    let pct = if net_deposits > 0 { pnl as f64 / net_deposits as f64 } else { 0.0 };
    (pnl, pct)
}

fn equity_of(m: &Market, t: &Trader, now: i64) -> i64 {
    derive(m, t, now).equity
}

pub fn summary_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    let since = now - DAY;
    let nets = store.net_deposits()?;
    let (mut traders, mut in_profit) = (0u64, 0u64);
    let (mut tvl, mut trader_pnl, mut maker_pnl) = (0i128, 0i128, 0i128);
    for (_, t) in store.traders()? {
        let equity = equity_of(&m, &t, now);
        tvl += equity as i128;
        let (p, _) = pnl(equity, nets.get(&b58(&t.owner)).copied().unwrap_or(0));
        if t.is_operator != 0 {
            maker_pnl += p as i128;
        } else {
            traders += 1;
            trader_pnl += p as i128;
            in_profit += u64::from(p > 0);
        }
    }
    let ops = store.operators()?;
    tvl += ops.iter().map(|(_, o)| o.bond as i128).sum::<i128>();
    let day = store.fill_totals(since)?;
    let all = store.fill_totals(i64::MIN)?;
    Ok(json!({
        "traders": traders,
        "makers": ops.len(),
        "active_traders_24h": store.active_takers(since)?,
        "trades_24h": day.trades,
        "volume_24h_usdc": usdc(day.volume as i128),
        "volume_total_usdc": usdc(all.volume as i128),
        "fees_24h_usdc": usdc(day.fees as i128),
        "fees_total_usdc": usdc(all.fees as i128),
        "long_oi_usdc": usdc(m.long_q as i128),
        "short_oi_usdc": usdc(m.short_q as i128),
        "deposits_total_usdc": usdc(store.deposits_total()? as i128),
        "tvl_usdc": usdc(tvl),
        "insurance_usdc": usdc(m.insurance as i128),
        "treasury_usdc": usdc(m.treasury as i128),
        "liquidations_24h": store.liquidations_since(since)?,
        "adl_volume_24h_usdc": usdc(day.adl_volume as i128),
        "traders_in_profit": in_profit,
        "total_trader_pnl_usdc": usdc(trader_pnl),
        "total_maker_pnl_usdc": usdc(maker_pnl),
        "updated_ts": now,
    }))
}

/// `limit` contiguous buckets, oldest first, ending at the bucket containing now.
pub fn volume_view(store: &Store, interval: &str, limit: usize) -> ViewResult {
    let iv = interval_secs(interval).ok_or_else(|| ViewError::BadRequest(format!("bad interval {interval}")))?;
    let last = store.now().div_euclid(iv) * iv;
    let first = last - iv * (limit as i64 - 1);
    let got: HashMap<i64, _> = store.volume_buckets(iv, first, last + iv)?.into_iter().map(|b| (b.ts, b)).collect();
    let out = (0..limit as i64)
        .map(|i| {
            let ts = first + i * iv;
            let b = got.get(&ts).cloned().unwrap_or_default();
            json!({
                "ts": ts,
                "volume_usdc": usdc(b.volume as i128),
                "long_volume_usdc": usdc(b.long as i128),
                "short_volume_usdc": usdc(b.short as i128),
                "trades": b.trades,
                "fees_usdc": usdc(b.fees as i128),
                "liquidation_volume_usdc": usdc(b.liquidation as i128),
            })
        })
        .collect();
    Ok(Value::Array(out))
}

pub fn leaderboard_view(store: &Store, sort: Sort, makers: bool, limit: usize) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    let nets = store.net_deposits()?;
    let takers = store.taker_stats()?;
    let liqs = store.liquidation_counts()?;
    struct Row {
        owner: String,
        pnl: i64,
        pct: f64,
        volume: u64,
        json: Value,
    }
    let mut rows: Vec<Row> = store
        .traders()?
        .iter()
        .filter(|(_, t)| makers || t.is_operator == 0)
        .map(|(_, t)| {
            let owner = b58(&t.owner);
            let d = derive(&m, t, now);
            let net = nets.get(&owner).copied().unwrap_or(0);
            let (p, pct) = pnl(d.equity, net);
            let st = takers.get(&owner).cloned().unwrap_or_default();
            let json = json!({
                "owner": owner,
                "is_operator": t.is_operator != 0,
                "equity_usdc": usdc(d.equity as i128),
                "net_deposits_usdc": usdc(net as i128),
                "pnl_usdc": usdc(p as i128),
                "pnl_pct": pct,
                "upnl_usdc": usdc(d.upnl as i128),
                "q_usdc": usdc(t.q as i128),
                "volume_usdc": usdc(st.volume as i128),
                "trades": st.trades,
                "fees_paid_usdc": usdc(st.fees as i128),
                "liquidations": liqs.get(&owner).copied().unwrap_or(0),
                "last_trade_ts": st.last_ts,
            });
            Row { owner, pnl: p, pct, volume: st.volume, json }
        })
        .collect();
    rows.sort_by(|a, b| {
        match sort {
            Sort::Pnl => b.pnl.cmp(&a.pnl),
            Sort::Volume => b.volume.cmp(&a.volume),
            Sort::PnlPct => b.pct.total_cmp(&a.pct),
        }
        .then_with(|| a.owner.cmp(&b.owner))
    });
    Ok(Value::Array(rows.into_iter().take(limit).map(|r| r.json).collect()))
}

pub fn makers_view(store: &Store) -> ViewResult {
    let m = market(store)?;
    let now = store.now();
    let slot = store.slot();
    let traders: HashMap<[u8; 32], Trader> = store.traders()?.into_iter().collect();
    let nets = store.net_deposits()?;
    let stats = store.maker_stats()?;
    let mut rows: Vec<(u64, String, Value)> = store
        .operators()?
        .iter()
        .map(|(_, op)| {
            let owner = b58(&op.owner);
            let (equity, q) = traders.get(&op.trader).map(|t| (equity_of(&m, t, now), t.q)).unwrap_or((0, 0));
            let net = nets.get(&owner).copied().unwrap_or(0);
            let (p, _) = pnl(equity, net);
            let st = stats.get(&owner).cloned().unwrap_or_default();
            let json = json!({
                "owner": owner,
                "status": operator_status(op.status),
                "fresh": op.is_fresh(slot, m.params.curve_max_age_slots),
                "bond_usdc": usdc(op.bond as i128),
                "equity_usdc": usdc(equity as i128),
                "net_deposits_usdc": usdc(net as i128),
                "pnl_usdc": usdc(p as i128),
                "inventory_usdc": usdc(q as i128),
                "volume_usdc": usdc(st.volume as i128),
                "fills": st.trades,
                "half_spread_bps": to_f64(op.half_spread.get()) * 10_000.0,
                "fair_secs": secs(op.fair_ln.get()),
            });
            (st.volume, owner, json)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(Value::Array(rows.into_iter().map(|r| r.2).collect()))
}

pub fn trades_view(store: &Store, limit: usize) -> ViewResult {
    let pdas = store.pda_owners()?;
    Ok(Value::Array(
        store
            .recent_fills(limit)?
            .iter()
            .map(|f| {
                let mut v = fill_json(f);
                v["trader"] = Value::String(owner_of(&pdas, f.trader.clone()));
                v
            })
            .collect(),
    ))
}

pub fn liquidations_view(store: &Store, limit: usize) -> ViewResult {
    Ok(Value::Array(
        store
            .recent_liquidations(limit)?
            .iter()
            .map(|l| {
                json!({
                    "t": l.ts,
                    "signature": l.signature,
                    "trader": l.trader,
                    "liquidator": l.liquidator,
                    "size_usdc": usdc(l.size as i128),
                    "bankruptcy_secs": secs(l.bankruptcy_ln),
                    "penalty_usdc": usdc(l.penalty as i128),
                    "insurance_used_usdc": usdc(l.insurance_used as i128),
                })
            })
            .collect(),
    ))
}
