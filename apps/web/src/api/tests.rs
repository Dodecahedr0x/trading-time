use super::*;

#[test]
fn url_joins() {
    assert_eq!(url("http://h:1", "/v1/market"), "http://h:1/v1/market");
    assert_eq!(url("http://h:1/", "/v1/market"), "http://h:1/v1/market");
    assert_eq!(url("http://h:1", "v1/market"), "http://h:1/v1/market");
}

#[test]
fn paths() {
    assert_eq!(candles_path("4h", 200), "/v1/candles?interval=4h&limit=200");
    assert_eq!(index_path(50), "/v1/index?limit=50");
    assert_eq!(trader_path("Abc"), "/v1/traders/Abc");
    assert_eq!(fills_path("Abc", 10), "/v1/traders/Abc/fills?limit=10");
    assert_eq!(orders_path("Abc"), "/v1/traders/Abc/orders");
    assert_eq!(funding_path("Abc", 5), "/v1/traders/Abc/funding?limit=5");
}

#[test]
fn http_status_handling() {
    let m: MarketJson = check_status(200, r#"{"mark_secs":1.5,"slot":7}"#).unwrap();
    assert_eq!(m.mark_secs, 1.5);
    assert_eq!(m.slot, 7);
    let e = check_status::<MarketJson>(503, "not synced").unwrap_err();
    assert_eq!(e, "HTTP 503: not synced");
    assert_eq!(trader_from(404, "").unwrap(), None);
    assert!(trader_from(400, r#"{"error":"x"}"#).unwrap_err().starts_with("HTTP 400"));
    let t = trader_from(200, r#"{"owner":"O","q_usdc":-3.0,"entry_secs":null,"liq_secs":12.5,"is_operator":true}"#)
        .unwrap()
        .unwrap();
    assert_eq!(t.owner, "O");
    assert_eq!(t.q_usdc, -3.0);
    assert_eq!(t.entry_secs, None);
    assert_eq!(t.liq_secs, Some(12.5));
    assert!(t.is_operator);
}

#[test]
fn parse_market() {
    let m: MarketJson = parse_json(
        r#"{"mark_secs":3600.5,"index_secs":3599.0,"mark_ln":"123456789012345678901","index_ln":"-5",
        "funding_rate_per_hour":0.0001,"long_oi_usdc":100.0,"short_oi_usdc":90,"volume_24h_usdc":5e6,
        "change_24h":-0.02,"paused":false,"insurance_usdc":1000,"slot":42,"updated_ts":1700000000,"extra":1}"#,
    )
    .unwrap();
    assert_eq!(m.mark_ln, "123456789012345678901");
    assert_eq!(m.short_oi_usdc, 90.0);
    assert_eq!(m.updated_ts, 1700000000);
    assert_eq!(m.change_24h, -0.02);
}

#[test]
fn parse_candles_index() {
    let c: Vec<CandleJson> = parse_json(
        r#"[{"t":1,"open_secs":1.0,"high_secs":2.0,"low_secs":0.5,"close_secs":1.5,"volume_usdc":10}]"#,
    )
    .unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].high_secs, 2.0);
    let i: Vec<IndexPointJson> = parse_json(r#"[{"t":9,"index_secs":3.0,"mark_secs":3.1},{"t":10}]"#).unwrap();
    assert_eq!(i[1].t, 10);
    assert_eq!(i[0].mark_secs, 3.1);
}

#[test]
fn parse_quotes() {
    let q: QuotesJson = parse_json(
        r#"{"mark_secs":10.0,"asks":[{"time_secs":10.1,"size_usdc":500}],"bids":[{"time_secs":9.9,"size_usdc":400}],
        "operators":[{"owner":"Op","fair_secs":10,"half_spread":0.0002,"impact_per_usdc":1e-7,"inventory_limit_usdc":20000,
        "inventory_usdc":-10,"updated_slot":99,"fresh":true,"status":"active"}]}"#,
    )
    .unwrap();
    assert_eq!(q.asks[0].time_secs, 10.1);
    assert_eq!(q.bids[0].size_usdc, 400.0);
    assert_eq!(q.operators[0].updated_slot, 99);
    assert!(q.operators[0].fresh);
}

#[test]
fn parse_fills_orders_funding() {
    let f: Vec<FillJson> = parse_json(
        r#"[{"signature":"sig","slot":5,"t":6,"side":"long","kind":"swap","size_usdc":100,"price_secs":3.5,"fee_usdc":0.1,"operator":"Op"},
            {"side":"short","kind":"adl"}]"#,
    )
    .unwrap();
    assert_eq!(f[0].side, SideStr::Long);
    assert_eq!(f[1].side, SideStr::Short);
    assert_eq!(f[1].kind, "adl");
    let o: Vec<OrderJson> = parse_json(
        r#"[{"order_id":3,"side":"short","size_usdc":50,"filled_usdc":10,"limit_secs":4.0,"expiry_t":100,"created_slot":8,"status":"open"}]"#,
    )
    .unwrap();
    assert_eq!(o[0].order_id, 3);
    assert_eq!(o[0].status, "open");
    let fu: Vec<FundingJson> = parse_json(r#"[{"t":1,"amount_usdc":-0.25}]"#).unwrap();
    assert_eq!(fu[0].amount_usdc, -0.25);
    assert!(parse_json::<Vec<FillJson>>(r#"[{"kind":"swap"}]"#).is_err());
    assert!(parse_json::<MarketJson>("not json").is_err());
}

#[test]
fn ws_subscribe() {
    let m = subscribe_msg(&["market".to_string(), "trader:Abc".to_string()]);
    let v: serde_json::Value = serde_json::from_str(&m).unwrap();
    assert_eq!(v, serde_json::json!({"subscribe":["market","trader:Abc"]}));
    assert_eq!(subscribe_msg(&[]), r#"{"subscribe":[]}"#);
}

#[test]
fn ws_parse() {
    match parse_ws(r#"{"channel":"market","data":{"mark_secs":2.0}}"#) {
        Some(WsMsg::Market(m)) => assert_eq!(m.mark_secs, 2.0),
        o => panic!("{o:?}"),
    }
    match parse_ws(r#"{"channel":"quotes","data":{"mark_secs":2.0,"asks":[{"time_secs":2.1,"size_usdc":1}]}}"#) {
        Some(WsMsg::Quotes(q)) => assert_eq!(q.asks.len(), 1),
        o => panic!("{o:?}"),
    }
    match parse_ws(r#"{"channel":"trades","data":{"side":"short","size_usdc":7}}"#) {
        Some(WsMsg::Trade(f)) => assert_eq!(f.size_usdc, 7.0),
        o => panic!("{o:?}"),
    }
    match parse_ws(r#"{"channel":"trader:Abc","data":{"owner":"Abc","cash_usdc":5}}"#) {
        Some(WsMsg::Trader(t)) => assert_eq!(t.cash_usdc, 5.0),
        o => panic!("{o:?}"),
    }
    let unk = r#"{"channel":"liquidations","data":{}}"#;
    assert_eq!(parse_ws(unk), Some(WsMsg::Other(unk.to_string())));
    let bad = r#"{"channel":"trades","data":{"kind":"swap"}}"#;
    assert_eq!(parse_ws(bad), Some(WsMsg::Other(bad.to_string())));
    let nochan = r#"{"hello":1}"#;
    assert_eq!(parse_ws(nochan), Some(WsMsg::Other(nochan.to_string())));
    assert_eq!(parse_ws("garbage"), None);
}
