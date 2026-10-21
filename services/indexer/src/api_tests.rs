use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytemuck::bytes_of;
use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use tradingtime_interface::events::*;
use tradingtime_interface::PodI128;

use crate::api::{AppState, Notify, router};
use crate::decode::Event;
use crate::store::tests::{market, operator, order_ev, trade_ev, trader};
use crate::store::{Store, b58};

const NOW: i64 = 1_000_000;
const LONG: [u8; 32] = [7; 32];
const SHORT: [u8; 32] = [8; 32];
const BROKE: [u8; 32] = [9; 32];

fn seeded() -> AppState {
    seeded_funded_at(NOW - 20)
}

fn seeded_funded_at(funding_updated_ts: i64) -> AppState {
    let mut s = Store::open_in_memory().unwrap();
    s.set_chain(100, NOW);
    let mut m = market(3600.0, 3000.0, NOW - 10);
    m.funding_updated_ts = funding_updated_ts;
    m.long_q = 1_000_000_000;
    m.paused = 1;
    s.upsert_account(&[1; 32], 100, bytes_of(&m)).unwrap();
    // In profit long, losing short, and a long far below maintenance.
    s.upsert_account(&[2; 32], 100, bytes_of(&trader(LONG, 100_000_000, 1_000_000_000, 1800.0))).unwrap();
    s.upsert_account(&[3; 32], 100, bytes_of(&trader(SHORT, 1_000_000_000, -1_000_000_000, 1800.0))).unwrap();
    s.upsert_account(&[4; 32], 100, bytes_of(&trader(BROKE, 1_000_000, 1_000_000_000, 7200.0))).unwrap();
    s.upsert_account(&[5; 32], 100, bytes_of(&operator([20; 32], [3; 32], 3600.0, 95))).unwrap();
    s.upsert_account(&[6; 32], 100, bytes_of(&operator([21; 32], [4; 32], 3700.0, 10))).unwrap();
    for (i, ts) in [NOW - 7200, NOW - 3600, NOW - 100].iter().enumerate() {
        s.insert_event(&format!("t{i}"), 0, 90, &Event::Trade(trade_ev(LONG, 0, 1_000_000_000, 3000.0 + i as f64 * 100.0, *ts)))
            .unwrap();
    }
    let idx = IndexEvent {
        index_ln: PodI128::new(tradingtime_math::from_f64(3000f64.ln())),
        var_ewma: PodI128::new(1),
        pyth_price: 1,
        pyth_expo: -8,
        _pad: 0,
        publish_time: NOW - 50,
        ts: NOW - 50,
    };
    s.insert_event("i0", 0, 99, &Event::Index(idx)).unwrap();
    s.insert_event("c0", 0, 99, &Event::Cash(CashEvent { owner: LONG, kind: CASH_FUNDING, _pad: [0; 7], amount: -1_500_000, ts: NOW - 5 }))
        .unwrap();
    s.record_order_placement(&LONG, 1, NOW + 1000, 98).unwrap();
    s.insert_event("o1", 0, 98, &Event::Order(order_ev(LONG, 1, ORDER_PLACED, 0, NOW - 20))).unwrap();
    s.record_order_placement(&LONG, 2, NOW - 1, 98).unwrap();
    s.insert_event("o2", 0, 98, &Event::Order(order_ev(LONG, 2, ORDER_PLACED, 0, NOW - 20))).unwrap();
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

#[tokio::test]
async fn health_and_unsynced_market() {
    let empty = AppState::new(Store::open_in_memory().unwrap());
    let (st, v) = get(&empty, "/v1/health").await;
    assert_eq!((st, v["slot"].as_u64()), (StatusCode::OK, Some(0)));
    for uri in ["/v1/market", "/v1/quotes", "/v1/operators", "/v1/liquidatable", "/v1/positions"] {
        let (st, v) = get(&empty, uri).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert!(v["error"].is_string());
    }
    let (st, v) = get(&seeded(), "/v1/health").await;
    assert_eq!((st, v["slot"].as_u64()), (StatusCode::OK, Some(100)));
}

#[tokio::test]
async fn market_route() {
    let (st, v) = get(&seeded(), "/v1/market").await;
    assert_eq!(st, StatusCode::OK);
    assert!((f(&v["mark_secs"]) - 3600.0).abs() < 1e-6);
    assert!((f(&v["index_secs"]) - 3000.0).abs() < 1e-6);
    assert!(v["mark_ln"].as_str().unwrap().parse::<i128>().is_ok());
    assert!(v["index_ln"].as_str().unwrap().parse::<i128>().is_ok());
    let rate = (3600f64 / 3000.0).ln() / 24.0;
    assert!((f(&v["funding_rate_per_hour"]) - rate).abs() < 1e-9);
    assert_eq!(f(&v["long_oi_usdc"]), 1000.0);
    assert_eq!(f(&v["short_oi_usdc"]), 0.0);
    assert_eq!(f(&v["volume_24h_usdc"]), 3000.0);
    // 24h ago the earliest point (3000s) stands in; now 3600s.
    assert!((f(&v["change_24h"]) - 0.2).abs() < 1e-6);
    assert_eq!(v["paused"], true);
    assert_eq!(f(&v["insurance_usdc"]), 0.0);
    assert_eq!(v["slot"], 100);
    assert_eq!(v["updated_ts"], NOW - 10);
}

#[tokio::test]
async fn candles_route() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/candles?interval=1h&limit=5").await;
    assert_eq!(st, StatusCode::OK);
    let c = v.as_array().unwrap();
    assert!(!c.is_empty() && c.len() <= 5);
    for k in ["t", "open_secs", "high_secs", "low_secs", "close_secs", "volume_usdc"] {
        assert!(c[0].get(k).is_some(), "{k}");
    }
    let total: f64 = c.iter().map(|x| f(&x["volume_usdc"])).sum();
    assert_eq!(total, 3000.0);
    assert!(c.windows(2).all(|w| w[1]["t"].as_i64() > w[0]["t"].as_i64()));
    assert!(c.iter().all(|x| x["t"].as_i64().unwrap() % 3600 == 0));
    let (st, d) = get(&s, "/v1/candles?interval=1d").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(d.as_array().unwrap().len(), 1);
    assert_eq!(get(&s, "/v1/candles?interval=7m").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(get(&s, "/v1/candles?interval=1h&limit=501").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(get(&s, "/v1/candles?interval=4h&limit=x").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn index_route() {
    let (st, v) = get(&seeded(), "/v1/index?limit=10").await;
    assert_eq!(st, StatusCode::OK);
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["t"], NOW - 50);
    assert!((f(&a[0]["index_secs"]) - 3000.0).abs() < 1e-6);
    assert!((f(&a[0]["mark_secs"]) - 3200.0).abs() < 1e-6);
}

#[tokio::test]
async fn quotes_and_operators_routes() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/quotes").await;
    assert_eq!(st, StatusCode::OK);
    assert!((f(&v["mark_secs"]) - 3600.0).abs() < 1e-6);
    let asks = v["asks"].as_array().unwrap();
    let bids = v["bids"].as_array().unwrap();
    assert_eq!((asks.len(), bids.len()), (8, 8));
    assert!(f(&asks[0]["time_secs"]) > f(&bids[0]["time_secs"]));
    assert!(f(&asks[0]["size_usdc"]) > 0.0);
    let ops = v["operators"].as_array().unwrap();
    assert_eq!(ops.len(), 2);
    let (_, ops2) = get(&s, "/v1/operators").await;
    assert_eq!(&ops2, &v["operators"]);
    let fresh = ops.iter().find(|o| o["owner"] == b58(&[20; 32])).unwrap();
    assert_eq!(fresh["fresh"], true);
    assert_eq!(fresh["status"], "active");
    assert_eq!(f(&fresh["inventory_usdc"]), -1000.0);
    assert_eq!(f(&fresh["inventory_limit_usdc"]), 20_000.0);
    assert_eq!(fresh["updated_slot"], 95);
    assert!((f(&fresh["fair_secs"]) - 3600.0).abs() < 1e-6);
    for k in ["half_spread", "impact_per_usdc"] {
        assert!(fresh[k].is_number());
    }
    let stale = ops.iter().find(|o| o["owner"] == b58(&[21; 32])).unwrap();
    assert_eq!(stale["fresh"], false);
}

#[tokio::test]
async fn trader_route() {
    let s = seeded();
    let (st, v) = get(&s, &format!("/v1/traders/{}", b58(&LONG))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["owner"], b58(&LONG));
    assert_eq!(v["trader"], b58(&[2; 32]));
    assert_eq!(f(&v["cash_usdc"]), 100.0);
    assert_eq!(f(&v["q_usdc"]), 1000.0);
    assert!((f(&v["entry_secs"]) - 1800.0).abs() < 1e-6);
    let upnl = 1000.0 * 2f64.ln();
    assert!((f(&v["upnl_usdc"]) - upnl).abs() < 1e-3);
    // 20s of accrued funding since the last crank.
    let funding = -1000.0 * 1.2f64.ln() * 20.0 / 86_400.0;
    assert!((f(&v["funding_usdc"]) - funding).abs() < 1e-5);
    assert!((f(&v["equity_usdc"]) - (100.0 + upnl + funding)).abs() < 1e-3);
    assert!(f(&v["liq_secs"]) < 1800.0);
    assert_eq!(f(&v["reserved_usdc"]), 0.0);
    assert_eq!(v["is_operator"], false);

    let flat = AppState::new({
        let mut st = Store::open_in_memory().unwrap();
        st.upsert_account(&[1; 32], 1, bytes_of(&market(10.0, 10.0, 0))).unwrap();
        st.upsert_account(&[2; 32], 1, bytes_of(&trader(LONG, 5, 0, 1.0))).unwrap();
        st
    });
    let (_, v) = get(&flat, &format!("/v1/traders/{}", b58(&LONG))).await;
    assert!(v["entry_secs"].is_null() && v["liq_secs"].is_null());

    assert_eq!(get(&s, &format!("/v1/traders/{}", b58(&[99; 32]))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&s, "/v1/traders/not-a-key").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fills_orders_funding_routes() {
    let s = seeded();
    let o = b58(&LONG);
    let (st, v) = get(&s, &format!("/v1/traders/{o}/fills?limit=2")).await;
    assert_eq!(st, StatusCode::OK);
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["signature"], "t2");
    assert_eq!(a[0]["t"], NOW - 100);
    assert_eq!(a[0]["slot"], 50);
    assert_eq!(a[0]["side"], "long");
    assert_eq!(a[0]["kind"], "swap");
    assert_eq!(f(&a[0]["size_usdc"]), 1000.0);
    assert!((f(&a[0]["price_secs"]) - 3200.0).abs() < 1e-6);
    assert_eq!(f(&a[0]["fee_usdc"]), 0.5);
    assert_eq!(a[0]["operator"], b58(&[9; 32]));

    let (_, v) = get(&s, &format!("/v1/traders/{o}/orders")).await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["order_id"], 1);
    assert_eq!(a[0]["side"], "long");
    assert_eq!(a[0]["status"], "open");
    assert_eq!(a[0]["expiry_t"], NOW + 1000);
    assert_eq!(a[0]["created_slot"], 98);
    assert_eq!(f(&a[0]["size_usdc"]), 1.0);
    assert_eq!(f(&a[0]["filled_usdc"]), 0.0);
    assert!((f(&a[0]["limit_secs"]) - 3600.0).abs() < 1e-6);
    assert_eq!(a[1]["status"], "expired");

    let (_, v) = get(&s, "/v1/orders/open").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["owner"], o);
    assert_eq!(a[0]["order_id"], 1);
    for k in ["side", "size_usdc", "filled_usdc", "limit_secs", "expiry_t"] {
        assert!(a[0].get(k).is_some(), "{k}");
    }

    let (_, v) = get(&s, &format!("/v1/traders/{o}/funding?limit=5")).await;
    assert_eq!(v, serde_json::json!([{ "t": NOW - 5, "amount_usdc": -1.5 }]));
    assert_eq!(get(&s, &format!("/v1/traders/{o}/fills?limit=0")).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn trader_funding_accrues_to_chain_now() {
    // One funding period since the last crank at mark/index = 3600/3000: a 1000 USDC long owes 1000·ln 1.2.
    let s = seeded_funded_at(NOW - 86_400);
    let (_, v) = get(&s, &format!("/v1/traders/{}", b58(&LONG))).await;
    let owed = 1000.0 * 1.2f64.ln();
    assert!((f(&v["funding_usdc"]) + owed).abs() < 1e-3, "{v}");
    let upnl = 1000.0 * 2f64.ln();
    assert!((f(&v["equity_usdc"]) - (100.0 + upnl - owed)).abs() < 1e-3, "{v}");
    let (_, p) = get(&s, "/v1/positions?side=long&in_profit=true").await;
    assert!((f(&p[0]["equity_usdc"]) - f(&v["equity_usdc"])).abs() < 1e-9);
}

#[tokio::test]
async fn positions_and_liquidatable_routes() {
    let s = seeded();
    let (st, v) = get(&s, "/v1/positions").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 3);
    let (_, v) = get(&s, "/v1/positions?side=short").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["owner"], b58(&SHORT));
    assert_eq!(a[0]["trader"], b58(&[3; 32]));
    assert_eq!(f(&a[0]["q_usdc"]), -1000.0);
    assert!(f(&a[0]["upnl_usdc"]) < 0.0 && a[0]["equity_usdc"].is_number());
    let (_, v) = get(&s, "/v1/positions?side=long&in_profit=true&limit=10").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["owner"], b58(&LONG));
    assert_eq!(get(&s, "/v1/positions?side=up").await.0, StatusCode::BAD_REQUEST);

    let (_, v) = get(&s, "/v1/liquidatable").await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 1, "{v}");
    assert_eq!(a[0]["owner"], b58(&BROKE));
    assert_eq!(a[0]["trader"], b58(&[4; 32]));
    assert_eq!(f(&a[0]["maintenance_usdc"]), 50.0);
    assert!(f(&a[0]["equity_usdc"]) < 50.0);
}

#[tokio::test]
async fn ws_subscribe_snapshot_and_broadcast() {
    let s = seeded();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(s.clone())).into_future());
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws")).await.unwrap();
    let trader_ch = format!("trader:{}", b58(&LONG));
    let sub = serde_json::json!({ "subscribe": ["market", "trades", trader_ch] });
    ws.send(Message::Text(sub.to_string().into())).await.unwrap();

    let mut next = async || -> Value {
        let m = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
        serde_json::from_str(m.to_text().unwrap()).unwrap()
    };
    let snap1 = next().await;
    assert_eq!(snap1["channel"], "market");
    assert!(snap1["data"]["mark_secs"].is_number());
    let snap2 = next().await;
    assert_eq!(snap2["channel"], trader_ch);
    assert_eq!(snap2["data"]["owner"], b58(&LONG));

    // Unsubscribed channels are filtered; subscribed ones arrive with REST-shaped data.
    s.publish(Notify::Quotes);
    s.publish(Notify::Trade(serde_json::json!({ "signature": "x" })));
    let m = next().await;
    assert_eq!(m, serde_json::json!({ "channel": "trades", "data": { "signature": "x" } }));
    s.publish(Notify::Trader(b58(&SHORT)));
    s.publish(Notify::Trader(b58(&LONG)));
    let m = next().await;
    assert_eq!(m["channel"], trader_ch);
    assert_eq!(f(&m["data"]["q_usdc"]), 1000.0);
    s.publish(Notify::Market);
    assert_eq!(next().await["channel"], "market");
}

#[tokio::test]
async fn cross_origin_reads_allowed_for_the_web_app() {
    let req = Request::get("/v1/health").header("origin", "http://127.0.0.1:14173").body(Body::empty()).unwrap();
    let res = router(seeded()).oneshot(req).await.unwrap();
    assert_eq!(res.headers().get("access-control-allow-origin").unwrap(), "*");
}

#[tokio::test]
async fn reservations_are_netted_and_operators_flagged() {
    let mut st = Store::open_in_memory().unwrap();
    st.set_chain(100, NOW);
    st.upsert_account(&[1; 32], 100, bytes_of(&market(3600.0, 3600.0, NOW - 10))).unwrap();
    let mut t = trader(LONG, 2_000_000_000, 1_000_000_000, 1800.0);
    t.reserved_long = 200_000_000;
    t.reserved_short = 1_500_000_000;
    st.upsert_account(&[2; 32], 100, bytes_of(&t)).unwrap();
    let mut mk = trader(SHORT, 20_000_000_000, -1_000_000_000, 7200.0);
    mk.is_operator = 1;
    st.upsert_account(&[3; 32], 100, bytes_of(&mk)).unwrap();
    let s = AppState::new(st);

    let (_, v) = get(&s, &format!("/v1/traders/{}", b58(&LONG))).await;
    // max(|1000 + 200|, |1000 − 1500|) − |1000| = 200 added beyond the position.
    assert_eq!(f(&v["reserved_usdc"]), 200.0);
    assert_eq!(f(&v["reserved_long_usdc"]), 200.0);
    assert_eq!(f(&v["reserved_short_usdc"]), 1500.0);

    let (_, v) = get(&s, "/v1/positions?side=short&in_profit=true").await;
    assert_eq!(v[0]["owner"], b58(&SHORT));
    assert_eq!(v[0]["is_operator"], true);
    let (_, v) = get(&s, "/v1/positions?side=long").await;
    assert_eq!(v[0]["is_operator"], false);
}

#[tokio::test]
async fn market_shows_sol_price_and_24h_change() {
    let mut s = Store::open_in_memory().unwrap();
    s.set_chain(100, NOW);
    let mut m = market(3600.0, 3000.0, NOW - 10);
    m.last_pyth_price = 18_900_000_000;
    m.last_pyth_expo = -8;
    m.last_publish_time = NOW - 5;
    s.upsert_account(&[1; 32], 100, bytes_of(&m)).unwrap();
    let idx = IndexEvent {
        index_ln: PodI128::new(0),
        var_ewma: PodI128::new(1),
        pyth_price: 18_000_000_000,
        pyth_expo: -8,
        _pad: 0,
        publish_time: NOW - 86_400 - 60,
        ts: NOW - 86_400 - 60,
    };
    s.insert_event("p0", 0, 90, &Event::Index(idx)).unwrap();
    s.insert_event("p1", 0, 95, &Event::Index(IndexEvent { pyth_price: 20_000_000_000, publish_time: NOW - 3_600, ts: NOW - 3_600, ..idx }))
        .unwrap();
    let (_, v) = get(&AppState::new(s), "/v1/market").await;
    assert!((f(&v["sol_price_usd"]) - 189.0).abs() < 1e-9, "{v}");
    assert!((f(&v["sol_change_24h"]) - 0.05).abs() < 1e-9, "{v}");
    assert_eq!(v["sol_price_ts"], NOW - 5);
}
