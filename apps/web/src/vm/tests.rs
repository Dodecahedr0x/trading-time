use super::chart::layout;
use super::format::*;
use super::ladder::{ladder, walk};
use super::market::market_bar;
use super::payoff::{buy_label, liquidation_secs, payoff, pnl};
use super::rows::*;
use super::ticket::*;
use super::Side;
use crate::types::*;

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn money_formats() {
    assert_eq!(money(1284.5), "$1,284.50");
    assert_eq!(money(-1284.5), "\u{2212}$1,284.50");
    assert_eq!(money(1_234_567.891), "$1,234,567.89");
    assert_eq!(money(-0.004), "$0.00");
    assert_eq!(money_signed(3465.7359), "+$3,465.74");
    assert_eq!(money_signed(-3.2), "\u{2212}$3.20");
    assert_eq!(money_signed(0.0), "$0.00");
    assert_eq!(money_whole(5000.0), "$5,000");
    assert_eq!(money_whole(999.6), "$1,000");
    assert_eq!(money_compact(1_250_000.0), "$1.25M");
    assert_eq!(money_compact(350_000.0), "$350.0K");
    assert_eq!(money_compact(812.0), "$812");
    assert_eq!(money_compact(999_999.0), "$1.00M");
    assert_eq!(money_compact(999.6), "$1.0K");
    assert_eq!(money_compact(999.4), "$999");
}

#[test]
fn durations() {
    assert_eq!(duration(45.0), "45s");
    assert_eq!(duration(2700.0), "45m");
    assert_eq!(duration(750.0), "12m 30s");
    assert_eq!(duration(5400.0), "1h 30m");
    assert_eq!(duration(7200.0), "2h");
    assert_eq!(duration(2.4 * 86400.0), "2.4d");
    assert_eq!(duration(59.6), "1m");
    assert_eq!(duration(3700.0), "1h 1m");
    assert_eq!(hms(5405.0), "1:30:05");
    assert_eq!(hms(45.0), "0:00:45");
}

#[test]
fn misc_formats_and_parsing() {
    assert_eq!(pct_signed(0.024, 2), "+2.40%");
    assert_eq!(pct_signed(-0.01, 1), "\u{2212}1.0%");
    assert_eq!(pct_signed(0.0, 2), "0.00%");
    assert_eq!(timestamp(1_760_968_980), "10-20 14:03");
    assert_eq!(short_addr("7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h"), "7thn\u{2026}kz4h");
    assert_eq!(parse_amount("5,000"), Some(5000.0));
    assert_eq!(parse_amount("$12.5"), Some(12.5));
    assert_eq!(parse_amount("abc"), None);
    assert_eq!(parse_amount("-5"), None);
    assert_eq!(parse_duration("1h 30m"), Some(5400.0));
    assert_eq!(parse_duration("90m"), Some(5400.0));
    assert_eq!(parse_duration("1:30:00"), Some(5400.0));
    assert_eq!(parse_duration("5400"), Some(5400.0));
    assert!(close(parse_duration("2.4d").unwrap(), 207_360.0, 1e-6));
    assert_eq!(parse_duration(""), None);
    assert_eq!(parse_duration("abc"), None);
}

#[test]
fn payoff_long_and_short() {
    assert!(close(pnl(5000.0, 3600.0, 7200.0), 3465.7359, 1e-3));
    assert!(close(liquidation_secs(5000.0, 3600.0, 5000.0, 500).unwrap(), 1392.2677, 1e-3));
    assert!(close(liquidation_secs(-5000.0, 3600.0, 5000.0, 500).unwrap(), 9308.5548, 1e-3));
    assert_eq!(liquidation_secs(0.0, 3600.0, 5000.0, 500), None);

    let p = payoff(Side::Long, 5000.0, 3600.0, 5000.0, 5000.0, 500);
    assert_eq!(p.headline, "You are buying long time");
    assert!(p.sentence.contains("1h") && p.sentence.contains("calmer"), "{}", p.sentence);
    assert_eq!(p.if_double, "+$3,465.74");
    assert_eq!(p.if_half, "\u{2212}$3,465.74");
    assert_eq!(p.closed_out, "23m 12s");
    assert!(p.if_double_gain);

    let s = payoff(Side::Short, 5000.0, 3600.0, -5000.0, 5000.0, 500);
    assert_eq!(s.headline, "You are buying short time");
    assert!(s.sentence.contains("wilder"), "{}", s.sentence);
    assert_eq!(s.if_double, "\u{2212}$3,465.74");
    assert_eq!(s.if_half, "+$3,465.74");
    assert_eq!(s.closed_out, "2h 35m");
    assert!(!s.if_double_gain);

    assert_eq!(buy_label(Side::Long, 5000.0), "Buy long time \u{b7} $5,000");
    assert_eq!(buy_label(Side::Short, 250.0), "Buy short time \u{b7} $250");
}

fn quotes() -> QuotesJson {
    QuotesJson {
        mark_secs: 3600.0,
        asks: vec![LevelJson { time_secs: 3620.0, size_usdc: 3000.0 }, LevelJson { time_secs: 3610.0, size_usdc: 2000.0 }],
        bids: vec![LevelJson { time_secs: 3580.0, size_usdc: 4000.0 }, LevelJson { time_secs: 3590.0, size_usdc: 1000.0 }],
        operators: vec![],
    }
}

#[test]
fn ladder_rows_sorted_with_depth() {
    let l = ladder(&quotes(), 8);
    let at: Vec<f64> = l.asks.iter().map(|r| r.time_secs).collect();
    let bt: Vec<f64> = l.bids.iter().map(|r| r.time_secs).collect();
    assert_eq!(at, vec![3620.0, 3610.0]);
    assert_eq!(bt, vec![3590.0, 3580.0]);
    assert_eq!(l.asks[0].total, "$5,000");
    assert_eq!(l.asks[1].total, "$2,000");
    assert_eq!(l.asks[0].size, "$3,000");
    assert_eq!(l.asks[0].time, "1:00:20");
    assert!(close(l.asks[0].depth, 1.0, 1e-9) && close(l.asks[1].depth, 0.4, 1e-9));
    assert!(close(l.bids[0].depth, 0.2, 1e-9) && close(l.bids[1].depth, 1.0, 1e-9));
    assert_eq!(l.mark, "1:00:00");
    assert_eq!(l.spread, "0.56%");
    let l1 = ladder(&quotes(), 1);
    assert_eq!(l1.asks.len(), 1);
    assert_eq!(l1.asks[0].time_secs, 3610.0);
    assert_eq!(l1.bids[0].time_secs, 3590.0);
}

#[test]
fn walking_levels() {
    let asks = [LevelJson { time_secs: 3610.0, size_usdc: 2000.0 }, LevelJson { time_secs: 3620.0, size_usdc: 3000.0 }];
    let (avg, worst) = walk(&asks, 4000.0).unwrap();
    assert!(close(avg, 3614.9965, 1e-3));
    assert_eq!(worst, 3620.0);
    assert_eq!(walk(&asks, 6000.0), None);
    assert_eq!(walk(&asks, 0.0), None);
}

fn trader(q: f64, equity: f64, reserved: f64) -> TraderJson {
    TraderJson { q_usdc: q, equity_usdc: equity, reserved_usdc: reserved, cash_usdc: equity, ..Default::default() }
}

#[test]
fn ticket_math() {
    assert!(close(max_size(None, 1000.0, Side::Long), 1000.0 / 1.0005, 1e-9));
    let t = trader(2000.0, 5000.0, 1000.0);
    assert!(close(max_size(Some(&t), 0.0, Side::Long), 2000.0 / 1.0005, 1e-9));
    assert!(close(max_size(Some(&t), 0.0, Side::Short), 6000.0 / 1.0005, 1e-9));
    assert_eq!(max_size(Some(&trader(6000.0, 5000.0, 0.0)), 0.0, Side::Long), 0.0);
    assert_eq!(preset(1999.0005, 100), 1999.0);
    assert_eq!(preset(1999.0005, 25), 499.0);
    assert_eq!(base_units(12.3456789), 12_345_678);
    assert_eq!(secs_to_ln(3600.0), tradingtime_math::from_f64(3600f64.ln()));

    let e = estimate_market(&quotes(), Side::Long, 4000.0).unwrap();
    assert!(close(e.entry_secs, 3614.9965, 1e-3));
    assert_eq!(e.worst_secs, 3620.0);
    assert!(close(e.fee_usdc, 2.0, 1e-9));
    assert!(close(e.limit_secs, 3656.3816, 1e-3));
    let s = estimate_market(&quotes(), Side::Short, 2000.0).unwrap();
    assert!(close(s.entry_secs, 3584.9965, 1e-3));
    assert_eq!(s.worst_secs, 3580.0);
    assert!(close(s.limit_secs, 3544.3784, 1e-3));
    let f = estimate_market(&quotes(), Side::Long, 10_000.0).unwrap();
    assert_eq!(f.entry_secs, 3600.0);
    assert!(close(f.limit_secs, 3780.0, 1e-9));
    let fs = estimate_market(&quotes(), Side::Short, 10_000.0).unwrap();
    assert!(close(fs.limit_secs, 3600.0 / 1.05, 1e-9));
    assert_eq!(estimate_market(&QuotesJson::default(), Side::Long, 10.0), None);

    assert!(close(deposit_needed(None, Side::Long, OrderType::Market, 5000.0), 5052.5, 1e-9));
    assert!(close(deposit_needed(Some(&trader(0.0, 5000.0, 0.0)), Side::Long, OrderType::Market, 5000.0), 52.5, 1e-9));
    assert_eq!(deposit_needed(Some(&trader(2000.0, 5000.0, 0.0)), Side::Short, OrderType::Market, 3000.0), 0.0);
    // Limit orders net against the position: a 3000 short on a 2000 long needs max(2000, 1000) + fee.
    assert_eq!(deposit_needed(Some(&trader(2000.0, 5000.0, 0.0)), Side::Short, OrderType::Limit, 3000.0), 0.0);
    let full = trader(5000.0, 5000.0, 0.0);
    assert!(close(deposit_needed(Some(&full), Side::Short, OrderType::Limit, 5000.0), 2.5, 1e-9));
    assert!(close(deposit_needed(Some(&full), Side::Short, OrderType::Limit, 15_000.0), 5007.5, 1e-9));
    assert!(close(deposit_needed(Some(&full), Side::Long, OrderType::Limit, 1000.0), 1000.5, 1e-9));
    // Resting orders: the larger side's exposure is what must be covered.
    let resting = TraderJson { reserved_short_usdc: 12_000.0, ..full.clone() };
    assert!(close(deposit_needed(Some(&resting), Side::Short, OrderType::Limit, 1000.0), 3000.5, 1e-9));
    assert!(close(deposit_needed(Some(&resting), Side::Long, OrderType::Limit, 1000.0), 2000.5, 1e-9));
    assert!(close(deposit_needed(Some(&resting), Side::Short, OrderType::Market, 1000.0), 3010.5, 1e-9));
    assert!(close(deposit_needed(None, Side::Long, OrderType::Limit, 1000.0), 1000.5, 1e-9));
    assert_eq!(margin_need(1000.0, 0.0, 1500.0), 1000.0);
    assert_eq!(margin_need(1000.0, 0.0, 2500.0), 1500.0);
    assert_eq!(q_after(None, Side::Short, 100.0), -100.0);
    assert_eq!(q_after(Some(&trader(2000.0, 5000.0, 0.0)), Side::Short, 3000.0), -1000.0);
}

#[test]
fn account_rows() {
    let t = TraderJson {
        q_usdc: 5000.0,
        entry_secs: Some(3600.0),
        equity_usdc: 5100.0,
        upnl_usdc: 100.0,
        funding_usdc: -1.5,
        liq_secs: Some(1392.27),
        ..Default::default()
    };
    let rows = position_rows(Some(&t), 3700.0);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.side, Side::Long);
    assert_eq!(r.side_label, "Long time");
    assert_eq!(r.size, "$5,000.00");
    assert_eq!(r.entry, "1h");
    assert_eq!(r.mark, "1h 1m");
    assert_eq!(r.upnl, "+$100.00");
    assert!(r.upnl_positive);
    assert_eq!(r.liq, "23m 12s");
    assert_eq!(r.funding, "\u{2212}$1.50");
    assert_eq!(r.equity, "$5,100.00");
    assert!(position_rows(Some(&trader(0.0, 10.0, 0.0)), 3600.0).is_empty());
    assert!(position_rows(None, 3600.0).is_empty());
    let short = position_rows(Some(&TraderJson { q_usdc: -10.0, ..Default::default() }), 3600.0);
    assert_eq!(short[0].side_label, "Short time");
    assert_eq!(short[0].entry, "\u{2014}");

    let orders = vec![
        OrderJson { order_id: 7, side: SideStr::Short, size_usdc: 1000.0, filled_usdc: 250.0, limit_secs: 5400.0, expiry_t: 1_760_968_980, created_slot: 1, status: "open".into() },
        OrderJson { order_id: 8, side: SideStr::Long, size_usdc: 1.0, filled_usdc: 1.0, limit_secs: 60.0, expiry_t: 0, created_slot: 1, status: "filled".into() },
    ];
    let o = order_rows(&orders);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].order_id, 7);
    assert_eq!(o[0].side_label, "Short time");
    assert_eq!(o[0].size, "$1,000.00");
    assert_eq!(o[0].filled, "$250.00");
    assert_eq!(o[0].limit, "1h 30m");
    assert_eq!(o[0].expiry, "10-20 14:03");

    let fills = vec![FillJson {
        signature: "s".into(), slot: 1, t: 1_760_968_980, side: SideStr::Long, kind: "swap".into(),
        size_usdc: 5000.0, price_secs: 5400.0, fee_usdc: 2.5, operator: "o".into(),
    }];
    let f = fill_rows(&fills);
    assert_eq!(f[0].time, "10-20 14:03");
    assert_eq!(f[0].kind, "Swap");
    assert_eq!(f[0].price, "1h 30m");
    assert_eq!(f[0].fee, "$2.50");
    let fr = funding_rows(&[FundingJson { t: 1_760_968_980, amount_usdc: -0.75 }]);
    assert_eq!(fr[0].amount, "\u{2212}$0.75");
    assert!(!fr[0].positive);
}

#[test]
fn market_bar_values() {
    let m = MarketJson {
        mark_secs: 5400.0,
        index_secs: 3600.0,
        change_24h: 0.024,
        volume_24h_usdc: 1_250_000.0,
        long_oi_usdc: 300_000.0,
        short_oi_usdc: 200_000.0,
        funding_rate_per_hour: 0.000042,
        ..Default::default()
    };
    let b = market_bar(&m);
    assert_eq!(b.mark, "1h 30m");
    assert_eq!(b.index, "1h");
    assert_eq!(b.change, "+2.40%");
    assert!(b.change_calm);
    assert_eq!(b.volume, "$1.25M");
    assert_eq!(b.open_interest, "$500.0K");
    assert_eq!(b.funding, "+0.0042%/h");
    assert_eq!(b.funding_payer, "Long pays");
    let n = market_bar(&MarketJson { funding_rate_per_hour: -0.0001, ..Default::default() });
    assert_eq!(n.funding_payer, "Short pays");
    assert_eq!(n.mark, "\u{2014}");
}

#[test]
fn chart_layout() {
    let c = vec![
        CandleJson { t: 0, open_secs: 3600.0, high_secs: 7200.0, low_secs: 3600.0, close_secs: 7200.0, volume_usdc: 1.0 },
        CandleJson { t: 3600, open_secs: 7200.0, high_secs: 7200.0, low_secs: 3600.0, close_secs: 3600.0, volume_usdc: 1.0 },
    ];
    let g = layout(&c, 200.0, 100.0, 3);
    assert_eq!(g.bars.len(), 2);
    assert!(g.bars[0].calm && !g.bars[1].calm);
    assert!(close(g.bars[0].x, 15.0, 1e-9) && close(g.bars[0].w, 70.0, 1e-9));
    assert!(close(g.bars[1].x, 115.0, 1e-9));
    assert!(close(g.bars[0].body_top, 6.8966, 1e-3));
    assert!(close(g.bars[0].body_h, 86.2069, 1e-3));
    assert!(close(g.bars[0].wick_top, 6.8966, 1e-3) && close(g.bars[0].wick_bottom, 93.1034, 1e-3));
    assert!(close(g.last_y.unwrap(), 93.1034, 1e-3));
    let labels: Vec<&str> = g.axis.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["2h", "1h 24m", "1h"]);
    assert!(close(g.axis[0].y, 6.8966, 1e-3));

    let flat = layout(&c[..1].iter().map(|c| CandleJson { open_secs: 60.0, high_secs: 60.0, low_secs: 60.0, close_secs: 60.0, ..c.clone() }).collect::<Vec<_>>(), 100.0, 100.0, 2);
    assert!(flat.bars[0].body_h >= 1.0 && flat.bars[0].body_top.is_finite());
    assert!(layout(&[], 100.0, 100.0, 3).bars.is_empty());
}
