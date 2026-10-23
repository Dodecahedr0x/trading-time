use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytemuck::bytes_of;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use tradingtime_interface::PodI128;
use tradingtime_interface::events::*;

use crate::api::{AppState, router};
use crate::decode::Event;
use crate::store::tests::{market, operator, trade_ev, trader};
use crate::store::{Store, b58};

const NOW: i64 = 1_000_000;
const DAY: i64 = 86_400;
const USDC: i64 = 1_000_000;
/// Long in profit, deposited 1500 and withdrew 500.
const A: [u8; 32] = [7; 32];
const A_PDA: [u8; 32] = [2; 32];
/// Short in loss, liquidated once.
const B: [u8; 32] = [8; 32];
const B_PDA: [u8; 32] = [3; 32];
/// Flat, never deposited.
const C: [u8; 32] = [10; 32];
/// Small long with a high return, never traded as taker.
const D: [u8; 32] = [11; 32];
/// Active maker with a trader account; N is unbonding without one.
const M: [u8; 32] = [20; 32];
const M_PDA: [u8; 32] = [5; 32];
const N: [u8; 32] = [21; 32];

#[allow(clippy::too_many_arguments)]
fn fill(s: &mut Store, sig: &str, taker: [u8; 32], op: [u8; 32], side: u8, kind: u8, size: i64, ts: i64) {
    let mut e = trade_ev(taker, side, size as u64, 3600.0, ts);
    e.operator = op;
    e.kind = kind;
    if kind == TRADE_ADL {
        e.fee = 0;
    }
    s.insert_event(sig, 0, 90, &Event::Trade(e)).unwrap();
}

fn cash(s: &mut Store, sig: &str, owner: [u8; 32], kind: u8, amount: i64) {
    let e = CashEvent { owner, kind, _pad: [0; 7], amount, ts: NOW - 5000 };
    s.insert_event(sig, 0, 90, &Event::Cash(e)).unwrap();
}

fn liquidation(s: &mut Store, sig: &str, trader: [u8; 32], ts: i64) {
    let e = LiquidationEvent {
        trader,
        liquidator: [30; 32],
        size: 100 * USDC as u64,
        bankruptcy_ln: PodI128::new(tradingtime_math::from_f64(3000f64.ln())),
        penalty: USDC as u64,
        insurance_used: 0,
        ts,
    };
    s.insert_event(sig, 0, 90, &Event::Liquidation(e)).unwrap();
}

fn seeded() -> AppState {
    let mut s = Store::open_in_memory().unwrap();
    s.set_chain(100, NOW);
    let mut m = market(3600.0, 3600.0, NOW - 10);
    m.funding_updated_ts = NOW - 10;
    m.long_q = 1_100 * USDC as u64;
    m.short_q = 1_000 * USDC as u64;
    m.insurance = 120 * USDC as u64;
    m.treasury = 40 * USDC as u64;
    s.upsert_account(&[1; 32], 100, bytes_of(&m)).unwrap();
    s.upsert_account(&A_PDA, 100, bytes_of(&trader(A, 999_500_000, 1_000 * USDC, 1800.0))).unwrap();
    s.upsert_account(&B_PDA, 100, bytes_of(&trader(B, 2_000 * USDC, -1_000 * USDC, 1800.0))).unwrap();
    s.upsert_account(&[4; 32], 100, bytes_of(&trader(C, 0, 0, 1.0))).unwrap();
    s.upsert_account(&[14; 32], 100, bytes_of(&trader(D, 100 * USDC, 100 * USDC, 900.0))).unwrap();
    let mut mt = trader(M, 51_000 * USDC, -1_000 * USDC, 3600.0);
    mt.is_operator = 1;
    s.upsert_account(&M_PDA, 100, bytes_of(&mt)).unwrap();
    let mut op = operator(M, M_PDA, 3600.0, 100);
    op.bond = 1_000 * USDC as u64;
    s.upsert_account(&[6; 32], 100, bytes_of(&op)).unwrap();
    let mut on = operator(N, [13; 32], 3700.0, 100);
    on.status = 1;
    s.upsert_account(&[12; 32], 100, bytes_of(&on)).unwrap();

    cash(&mut s, "d0", A, CASH_DEPOSIT, 1_500 * USDC);
    cash(&mut s, "d1", A, CASH_WITHDRAW, 500 * USDC);
    cash(&mut s, "d2", A, CASH_FUNDING, -USDC);
    cash(&mut s, "d3", B, CASH_DEPOSIT, 2_000 * USDC);
    cash(&mut s, "d4", D, CASH_DEPOSIT, 100 * USDC);
    cash(&mut s, "d5", M, CASH_DEPOSIT, 50_000 * USDC);

    fill(&mut s, "f1", A, M, 0, TRADE_SWAP, 1_000 * USDC, NOW - 100);
    // Named by the trader PDA: still A's.
    fill(&mut s, "f2", A_PDA, M, 0, TRADE_SWAP, 500 * USDC, NOW - 3600);
    fill(&mut s, "f3", A, M, 0, TRADE_SWAP, 200 * USDC, NOW - 2 * DAY);
    fill(&mut s, "f4", B, M, 1, TRADE_SWAP, 1_000 * USDC, NOW - 50);
    fill(&mut s, "f5", B, A, 1, TRADE_ADL, 100 * USDC, NOW - 30);
    fill(&mut s, "f6", C, M, 0, TRADE_LIMIT, 300 * USDC, NOW - 20);

    liquidation(&mut s, "l0", B_PDA, NOW - 40);
    liquidation(&mut s, "l1", D, NOW - 2 * DAY);
    AppState::new(s)
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let res = router(state.clone()).oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn has_keys(v: &Value, keys: &[&str]) {
    let o = v.as_object().unwrap_or_else(|| panic!("not an object: {v}"));
    for k in keys {
        assert!(o.contains_key(*k), "missing {k} in {v}");
    }
    assert_eq!(o.len(), keys.len(), "unexpected keys in {v}");
}

fn owners(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|r| r["owner"].as_str().unwrap().to_string()).collect()
}

fn ln2() -> f64 {
    2f64.ln()
}

#[tokio::test]
async fn summary_route() {
    let (st, v) = get(&seeded(), "/v1/analytics/summary").await;
    assert_eq!(st, StatusCode::OK);
    has_keys(
        &v,
        &[
            "traders",
            "makers",
            "active_traders_24h",
            "trades_24h",
            "volume_24h_usdc",
            "volume_total_usdc",
            "fees_24h_usdc",
            "fees_total_usdc",
            "long_oi_usdc",
            "short_oi_usdc",
            "deposits_total_usdc",
            "tvl_usdc",
            "insurance_usdc",
            "treasury_usdc",
            "liquidations_24h",
            "adl_volume_24h_usdc",
            "traders_in_profit",
            "total_trader_pnl_usdc",
            "total_maker_pnl_usdc",
            "trader_pnl_before_fees_usdc",
            "trader_fees_paid_usdc",
            "maker_pnl_usdc",
            "liquidation_penalties_usdc",
            "updated_ts",
        ],
    );
    assert_eq!((v["traders"].as_u64(), v["makers"].as_u64()), (Some(4), Some(2)));
    // The PDA-named fill folds into A: A, B, C.
    assert_eq!(v["active_traders_24h"], 3);
    assert_eq!(v["trades_24h"], 5);
    assert_eq!(f(&v["volume_24h_usdc"]), 2_900.0);
    assert_eq!(f(&v["volume_total_usdc"]), 3_100.0);
    assert!(close(f(&v["fees_24h_usdc"]), 1.4));
    assert!(close(f(&v["fees_total_usdc"]), 1.5));
    assert_eq!((f(&v["long_oi_usdc"]), f(&v["short_oi_usdc"])), (1_100.0, 1_000.0));
    assert_eq!(f(&v["deposits_total_usdc"]), 53_600.0);
    let equities =
        (999.5 + 1_000.0 * ln2()) + (2_000.0 - 1_000.0 * ln2()) + 0.0 + (100.0 + 100.0 * 4f64.ln()) + 51_000.0;
    assert!(close(f(&v["tvl_usdc"]), equities + 1_000.0), "{v}");
    assert_eq!((f(&v["insurance_usdc"]), f(&v["treasury_usdc"])), (120.0, 40.0));
    assert_eq!(v["liquidations_24h"], 1);
    assert_eq!(f(&v["adl_volume_24h_usdc"]), 100.0);
    assert_eq!(v["traders_in_profit"], 2);
    let trader_pnl = (1_000.0 * ln2() - 0.5) - 1_000.0 * ln2() + 100.0 * 4f64.ln();
    assert!(close(f(&v["total_trader_pnl_usdc"]), trader_pnl), "{v}");
    assert!(close(f(&v["total_maker_pnl_usdc"]), 1_000.0), "{v}");
    assert_eq!(v["updated_ts"], NOW);

    let empty = AppState::new(Store::open_in_memory().unwrap());
    for uri in ["/v1/analytics/summary", "/v1/analytics/leaderboard", "/v1/analytics/makers"] {
        assert_eq!(get(&empty, uri).await.0, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
    }
}

#[tokio::test]
async fn volume_buckets_are_contiguous_and_zero_filled() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/analytics/volume?interval=1h&limit=4").await;
    assert_eq!(st, StatusCode::OK);
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 4);
    for b in a {
        has_keys(
            b,
            &[
                "ts",
                "volume_usdc",
                "long_volume_usdc",
                "short_volume_usdc",
                "trades",
                "fees_usdc",
                "liquidation_volume_usdc",
            ],
        );
    }
    let last = NOW.div_euclid(3600) * 3600;
    let ts: Vec<i64> = a.iter().map(|b| b["ts"].as_i64().unwrap()).collect();
    assert_eq!(ts, vec![last - 3 * 3600, last - 2 * 3600, last - 3600, last]);
    // Gaps are zero-filled.
    for b in &a[..2] {
        assert_eq!((f(&b["volume_usdc"]), b["trades"].as_u64()), (0.0, Some(0)));
    }
    assert_eq!(f(&a[2]["volume_usdc"]), 500.0);
    let cur = &a[3];
    assert_eq!(f(&cur["volume_usdc"]), 2_400.0);
    assert_eq!(f(&cur["long_volume_usdc"]), 1_300.0);
    assert_eq!(f(&cur["short_volume_usdc"]), 1_100.0);
    assert_eq!(cur["trades"], 4);
    assert!(close(f(&cur["fees_usdc"]), 1.15));
    assert_eq!(f(&cur["liquidation_volume_usdc"]), 100.0);

    let (_, v) = get(&s, "/v1/analytics/volume").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 48);
    assert_eq!(a[47]["ts"], last);
    assert!(a.windows(2).all(|w| w[1]["ts"].as_i64().unwrap() - w[0]["ts"].as_i64().unwrap() == 3600));
    let (_, v) = get(&s, "/v1/analytics/volume?interval=5m&limit=2").await;
    assert_eq!(v[1]["ts"], NOW.div_euclid(300) * 300);
    let (_, v) = get(&s, "/v1/analytics/volume?interval=1d&limit=3").await;
    let total: f64 = v.as_array().unwrap().iter().map(|b| f(&b["volume_usdc"])).sum();
    assert_eq!(total, 3_100.0);

    assert_eq!(get(&s, "/v1/analytics/volume?interval=7m").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(get(&s, "/v1/analytics/volume?limit=500").await.1.as_array().unwrap().len(), 500);
    assert_eq!(get(&s, "/v1/analytics/volume?limit=501").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(get(&s, "/v1/analytics/volume?limit=0").await.0, StatusCode::BAD_REQUEST);
    // Unsynced stores still have (empty) buckets.
    let empty = AppState::new(Store::open_in_memory().unwrap());
    assert_eq!(get(&empty, "/v1/analytics/volume?limit=3").await.1.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn leaderboard_pnl_and_filters() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/analytics/leaderboard").await;
    assert_eq!(st, StatusCode::OK);
    // Default: pnl desc, operators excluded.
    assert_eq!(owners(&v), vec![b58(&A), b58(&D), b58(&C), b58(&B)]);
    let a = &v[0];
    has_keys(
        a,
        &[
            "owner",
            "is_operator",
            "equity_usdc",
            "net_deposits_usdc",
            "pnl_usdc",
            "pnl_before_fees_usdc",
            "pnl_pct",
            "upnl_usdc",
            "q_usdc",
            "volume_usdc",
            "trades",
            "fees_paid_usdc",
            "liquidations",
            "last_trade_ts",
        ],
    );
    // A deposited 1500, withdrew 500 (funding is not a deposit), then traded.
    assert_eq!(f(&a["net_deposits_usdc"]), 1_000.0);
    let equity = 999.5 + 1_000.0 * ln2();
    assert!(close(f(&a["equity_usdc"]), equity));
    assert!((f(&a["pnl_usdc"]) - (f(&a["equity_usdc"]) - f(&a["net_deposits_usdc"]))).abs() < 1e-6);
    assert!(close(f(&a["pnl_usdc"]), equity - 1_000.0));
    assert!((f(&a["pnl_pct"]) - f(&a["pnl_usdc"]) / 1_000.0).abs() < 1e-9);
    assert!(close(f(&a["upnl_usdc"]), 1_000.0 * ln2()));
    assert_eq!(f(&a["q_usdc"]), 1_000.0);
    assert_eq!((f(&a["volume_usdc"]), a["trades"].as_u64()), (1_700.0, Some(3)));
    assert!(close(f(&a["fees_paid_usdc"]), 0.85));
    assert!(close(f(&a["pnl_before_fees_usdc"]), equity - 1_000.0 + 0.85));
    assert_eq!(a["last_trade_ts"], NOW - 100);
    assert_eq!(a["is_operator"], false);
    let b = &v[3];
    assert_eq!(b["liquidations"], 1);
    assert_eq!(f(&b["q_usdc"]), -1_000.0);
    assert!(f(&b["pnl_pct"]) < 0.0);
    let c = &v[2];
    assert_eq!((f(&c["pnl_usdc"]), f(&c["pnl_pct"])), (0.0, 0.0));
    assert!(v[1]["last_trade_ts"].is_null());

    let (_, v) = get(&s, "/v1/analytics/leaderboard?makers=false").await;
    assert_eq!(owners(&v).len(), 4);
    let (_, v) = get(&s, "/v1/analytics/leaderboard?makers=true").await;
    assert_eq!(owners(&v), vec![b58(&M), b58(&A), b58(&D), b58(&C), b58(&B)]);
    assert_eq!(v[0]["is_operator"], true);
    assert!(close(f(&v[0]["pnl_usdc"]), 1_000.0));

    assert_eq!(get(&s, "/v1/analytics/leaderboard?makers=yes").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn leaderboard_sorts_and_limits() {
    let s = seeded();
    let (_, v) = get(&s, "/v1/analytics/leaderboard?sort=pnl").await;
    assert_eq!(owners(&v), vec![b58(&A), b58(&D), b58(&C), b58(&B)]);
    let (_, v) = get(&s, "/v1/analytics/leaderboard?sort=volume").await;
    assert_eq!(owners(&v), vec![b58(&A), b58(&B), b58(&C), b58(&D)]);
    let (_, v) = get(&s, "/v1/analytics/leaderboard?sort=pnl_pct").await;
    assert_eq!(owners(&v), vec![b58(&D), b58(&A), b58(&C), b58(&B)]);
    let pct: Vec<f64> = v.as_array().unwrap().iter().map(|r| f(&r["pnl_pct"])).collect();
    assert!(pct.windows(2).all(|w| w[0] >= w[1]), "{pct:?}");

    assert_eq!(get(&s, "/v1/analytics/leaderboard?sort=equity").await.0, StatusCode::BAD_REQUEST);
    let (_, v) = get(&s, "/v1/analytics/leaderboard?sort=volume&limit=2").await;
    assert_eq!(owners(&v), vec![b58(&A), b58(&B)]);
    assert_eq!(get(&s, "/v1/analytics/leaderboard?limit=500").await.0, StatusCode::OK);
    assert_eq!(get(&s, "/v1/analytics/leaderboard?limit=501").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn makers_route() {
    let (st, v) = get(&seeded(), "/v1/analytics/makers").await;
    assert_eq!(st, StatusCode::OK);
    // Sorted by maker volume.
    assert_eq!(owners(&v), vec![b58(&M), b58(&N)]);
    let m = &v[0];
    has_keys(
        m,
        &[
            "owner",
            "status",
            "fresh",
            "bond_usdc",
            "equity_usdc",
            "net_deposits_usdc",
            "pnl_usdc",
            "inventory_usdc",
            "volume_usdc",
            "fills",
            "half_spread_bps",
            "fair_secs",
        ],
    );
    assert_eq!((m["status"].as_str(), m["fresh"].as_bool()), (Some("active"), Some(true)));
    assert_eq!(f(&m["bond_usdc"]), 1_000.0);
    assert!(close(f(&m["equity_usdc"]), 51_000.0));
    // The bond is not a deposit.
    assert_eq!(f(&m["net_deposits_usdc"]), 50_000.0);
    assert!(close(f(&m["pnl_usdc"]), 1_000.0));
    assert_eq!(f(&m["inventory_usdc"]), -1_000.0);
    // Swap and limit fills only; the ADL fill names A, never a maker.
    assert_eq!((f(&m["volume_usdc"]), m["fills"].as_u64()), (3_000.0, Some(5)));
    assert!((f(&m["half_spread_bps"]) - 10.0).abs() < 1e-6);
    assert!((f(&m["fair_secs"]) - 3600.0).abs() < 1e-6);
    let n = &v[1];
    assert_eq!((n["status"].as_str(), n["fresh"].as_bool()), (Some("unbonding"), Some(false)));
    assert_eq!((f(&n["volume_usdc"]), n["fills"].as_u64()), (0.0, Some(0)));
}

#[tokio::test]
async fn trades_route() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/analytics/trades?limit=3").await;
    assert_eq!(st, StatusCode::OK);
    let a = v.as_array().unwrap();
    let sigs: Vec<&str> = a.iter().map(|r| r["signature"].as_str().unwrap()).collect();
    assert_eq!(sigs, vec!["f6", "f5", "f4"]);
    has_keys(
        &a[0],
        &["signature", "slot", "t", "side", "kind", "size_usdc", "price_secs", "fee_usdc", "operator", "trader"],
    );
    assert_eq!(a[0]["trader"], b58(&C));
    assert_eq!(a[0]["kind"], "limit");
    assert_eq!(a[1]["kind"], "adl");
    let (_, v) = get(&s, "/v1/analytics/trades").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 6);
    let f2 = a.iter().find(|r| r["signature"] == "f2").unwrap();
    assert_eq!(f2["trader"], b58(&A));
    assert!(a.windows(2).all(|w| w[0]["t"].as_i64() >= w[1]["t"].as_i64()));
    assert_eq!(get(&s, "/v1/analytics/trades?limit=501").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn liquidations_route() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/analytics/liquidations").await;
    assert_eq!(st, StatusCode::OK);
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 2);
    has_keys(
        &a[0],
        &[
            "t",
            "signature",
            "trader",
            "liquidator",
            "size_usdc",
            "bankruptcy_secs",
            "penalty_usdc",
            "insurance_used_usdc",
        ],
    );
    assert_eq!(a[0]["signature"], "l0");
    assert_eq!(a[0]["t"], NOW - 40);
    // Named by B's PDA, reported as B's wallet.
    assert_eq!(a[0]["trader"], b58(&B));
    assert_eq!(a[0]["liquidator"], b58(&[30; 32]));
    assert_eq!(f(&a[0]["size_usdc"]), 100.0);
    assert!((f(&a[0]["bankruptcy_secs"]) - 3000.0).abs() < 1e-6);
    assert_eq!((f(&a[0]["penalty_usdc"]), f(&a[0]["insurance_used_usdc"])), (1.0, 0.0));
    assert_eq!(a[1]["trader"], b58(&D));
    let (_, v) = get(&s, "/v1/analytics/liquidations?limit=1").await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(get(&s, "/v1/analytics/liquidations?limit=501").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn summary_fees_vs_pnl_breakdown() {
    let (_, v) = get(&seeded(), "/v1/analytics/summary").await;
    // Taker fees of non-operators only: A 0.85 (incl. the PDA-named fill), B 0.5 (ADL is free), C 0.15.
    assert!(close(f(&v["trader_fees_paid_usdc"]), 1.5), "{v}");
    let pnl = f(&v["total_trader_pnl_usdc"]);
    assert!(close(f(&v["trader_pnl_before_fees_usdc"]), pnl + 1.5), "{v}");
    assert_eq!(f(&v["maker_pnl_usdc"]), f(&v["total_maker_pnl_usdc"]));
    // Two liquidations at 1 USDC penalty each, regardless of age.
    assert_eq!(f(&v["liquidation_penalties_usdc"]), 2.0);

    // An operator's own taker fills do not count as trader fees, and its row's
    // before-fees pnl adds back its own fees.
    let s = seeded();
    {
        let mut st = s.store.lock().unwrap();
        fill(&mut st, "f7", M, [9; 32], 0, TRADE_SWAP, 2_000 * USDC, NOW - 10);
    }
    let (_, v) = get(&s, "/v1/analytics/summary").await;
    assert!(close(f(&v["trader_fees_paid_usdc"]), 1.5), "{v}");
    assert!(close(f(&v["fees_total_usdc"]), 2.5), "{v}");
    let (_, v) = get(&s, "/v1/analytics/leaderboard?makers=true").await;
    let m = &v[0];
    assert_eq!(m["is_operator"], true);
    assert!(close(f(&m["pnl_before_fees_usdc"]), f(&m["pnl_usdc"]) + 1.0), "{m}");
}
