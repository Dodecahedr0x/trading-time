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
    let r = TAKER_FEE_BPS as f64 / 10_000.0;
    assert!(close(max_size(None, 1000.0, Side::Long), 1000.0 / (1.0 + r), 1e-9));
    let t = trader(2000.0, 5000.0, 1000.0);
    assert!(close(max_size(Some(&t), 0.0, Side::Long), 2000.0 / (1.0 + r), 1e-9));
    assert!(close(max_size(Some(&t), 0.0, Side::Short), 6000.0 / (1.0 + r), 1e-9));
    assert_eq!(max_size(Some(&trader(6000.0, 5000.0, 0.0)), 0.0, Side::Long), 0.0);
    assert_eq!(preset(1999.0005, 100), 1999.0);
    assert_eq!(preset(1999.0005, 25), 499.0);
    assert_eq!(base_units(12.3456789), 12_345_678);
    assert_eq!(secs_to_ln(3600.0), tradingtime_math::from_f64(3600f64.ln()));

    let e = estimate_market(&quotes(), Side::Long, 4000.0).unwrap();
    assert!(close(e.entry_secs, 3614.9965, 1e-3));
    assert_eq!(e.worst_secs, 3620.0);
    assert!(close(e.fee_usdc, 4000.0 * r, 1e-9));
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

    assert!(close(deposit_needed(None, Side::Long, OrderType::Market, 5000.0), 5000.0 * (1.0 + r + 0.01), 1e-9));
    assert!(close(deposit_needed(Some(&trader(0.0, 5000.0, 0.0)), Side::Long, OrderType::Market, 5000.0), 5000.0 * (r + 0.01), 1e-9));
    assert_eq!(deposit_needed(Some(&trader(2000.0, 5000.0, 0.0)), Side::Short, OrderType::Market, 3000.0), 0.0);
    // Limit orders net against the position: a 3000 short on a 2000 long needs max(2000, 1000) + fee.
    assert_eq!(deposit_needed(Some(&trader(2000.0, 5000.0, 0.0)), Side::Short, OrderType::Limit, 3000.0), 0.0);
    let full = trader(5000.0, 5000.0, 0.0);
    assert!(close(deposit_needed(Some(&full), Side::Short, OrderType::Limit, 5000.0), 5000.0 * r, 1e-9));
    assert!(close(deposit_needed(Some(&full), Side::Short, OrderType::Limit, 15_000.0), 5000.0 + 15_000.0 * r, 1e-9));
    assert!(close(deposit_needed(Some(&full), Side::Long, OrderType::Limit, 1000.0), 1000.0 + 1000.0 * r, 1e-9));
    // Resting orders: the larger side's exposure is what must be covered.
    let resting = TraderJson { reserved_short_usdc: 12_000.0, ..full.clone() };
    assert!(close(deposit_needed(Some(&resting), Side::Short, OrderType::Limit, 1000.0), 3000.0 + 1000.0 * r, 1e-9));
    assert!(close(deposit_needed(Some(&resting), Side::Long, OrderType::Limit, 1000.0), 2000.0 + 1000.0 * r, 1e-9));
    assert!(close(deposit_needed(Some(&resting), Side::Short, OrderType::Market, 1000.0), 3000.0 + 1000.0 * (r + 0.01), 1e-9));
    assert!(close(deposit_needed(None, Side::Long, OrderType::Limit, 1000.0), 1000.0 + 1000.0 * r, 1e-9));
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

mod analytics {
    use super::close;
    use crate::types::*;
    use crate::vm::analytics::*;
    use crate::vm::Side;

    #[test]
    fn hash_routes() {
        assert_eq!(page_from_hash(""), Page::Trade);
        assert_eq!(page_from_hash("#"), Page::Trade);
        assert_eq!(page_from_hash("#/"), Page::Trade);
        assert_eq!(page_from_hash("#/trade"), Page::Trade);
        assert_eq!(page_from_hash("#/analytics"), Page::Analytics);
        assert_eq!(page_from_hash("#analytics"), Page::Analytics);
        assert_eq!(page_from_hash("#/analytics/"), Page::Analytics);
        assert_eq!(page_from_hash("#/nope"), Page::Trade);
    }

    #[test]
    fn relative_times() {
        let now = 1_792_512_680;
        assert_eq!(relative_time(now - 12, now), "12s ago");
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now + 5, now), "just now");
        assert_eq!(relative_time(now - 60, now), "1m ago");
        assert_eq!(relative_time(now - 3_599, now), "59m ago");
        assert_eq!(relative_time(now - 7_200, now), "2h ago");
        assert_eq!(relative_time(now - 3 * 86_400 - 5, now), "3d ago");
        assert_eq!(relative_time(0, now), "\u{2014}");
    }

    #[test]
    fn owners_shorten() {
        assert_eq!(owner_short("7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h"), "7thn\u{2026}kz4h");
        assert_eq!(owner_short(""), "\u{2014}");
        assert_eq!(owner_short("short"), "short");
    }

    fn summary() -> AnalyticsSummaryJson {
        AnalyticsSummaryJson {
            traders: 12,
            makers: 4,
            active_traders_24h: 9,
            trades_24h: 340,
            volume_24h_usdc: 512_000.0,
            volume_total_usdc: 1_200_000.0,
            fees_24h_usdc: 256.0,
            fees_total_usdc: 600.0,
            long_oi_usdc: 30_000.0,
            short_oi_usdc: 28_000.0,
            deposits_total_usdc: 2_400_000.0,
            tvl_usdc: 2_350_000.0,
            insurance_usdc: 120.0,
            treasury_usdc: 40.0,
            liquidations_24h: 3,
            adl_volume_24h_usdc: 1_500.0,
            traders_in_profit: 5,
            total_trader_pnl_usdc: -1234.5,
            total_maker_pnl_usdc: 1100.0,
            trader_pnl_before_fees_usdc: -634.5,
            trader_fees_paid_usdc: 600.0,
            maker_pnl_usdc: 1100.0,
            liquidation_penalties_usdc: 45.0,
            updated_ts: 1_792_512_680,
        }
    }

    #[test]
    fn kpi_cards() {
        let k = kpis(Some(&summary()));
        let ids: Vec<&str> = k.iter().map(|k| k.id).collect();
        assert_eq!(
            ids,
            ["kpi-volume-24h", "kpi-oi", "kpi-traders", "kpi-tvl", "kpi-fees", "kpi-insurance", "kpi-liquidations-24h"]
        );
        let v = |id: &str| k.iter().find(|k| k.id == id).unwrap().clone();
        assert_eq!(v("kpi-volume-24h").value, "$512.0K");
        assert_eq!(v("kpi-volume-24h").sub, "340 trades \u{b7} $1.20M all time");
        assert_eq!(v("kpi-oi").value, "$58.0K");
        assert_eq!(v("kpi-traders").value, "12");
        assert!(v("kpi-traders").sub.contains("4 makers"));
        assert_eq!(v("kpi-tvl").value, "$2.35M");
        assert_eq!(v("kpi-fees").value, "$256.00");
        assert_eq!(v("kpi-insurance").value, "$120.00");
        assert!(v("kpi-insurance").sub.contains("\u{2212}$1,234.50"));
        assert_eq!(v("kpi-liquidations-24h").value, "3");
        assert_eq!(v("kpi-liquidations-24h").sub, "ADL $1.5K");
        let empty = kpis(None);
        assert_eq!(empty.len(), 7);
        assert!(empty.iter().all(|k| k.value == "\u{2014}"));
        assert_eq!(empty[0].id, "kpi-volume-24h");
    }

    fn bucket(ts: i64, long: f64, short: f64) -> VolumeBucketJson {
        VolumeBucketJson { ts, volume_usdc: long + short, long_volume_usdc: long, short_volume_usdc: short, ..Default::default() }
    }

    #[test]
    fn volume_bars_stack() {
        let b = [bucket(0, 300.0, 100.0), bucket(3600, 0.0, 0.0), bucket(7200, 100.0, 100.0), bucket(10_800, 50.0, 0.0)];
        let g = volume_layout(&b, 400.0, 100.0);
        assert_eq!(g.bars.len(), 4);
        let a = &g.bars[0];
        assert!(close(a.x, 14.0, 1e-9));
        assert!(close(a.w, 72.0, 1e-9));
        assert!(close(a.long_h, 75.0, 1e-9));
        assert!(close(a.long_y, 25.0, 1e-9));
        assert!(close(a.short_h, 25.0, 1e-9));
        assert!(close(a.short_y, 0.0, 1e-9));
        assert_eq!(g.bars[1].long_h, 0.0);
        assert_eq!(g.bars[1].short_y, 100.0);
        assert!(close(g.bars[2].long_h + g.bars[2].short_h, 50.0, 1e-9));
        assert!(close(g.bars[2].x, 214.0, 1e-9));
        assert_eq!(g.y_ticks.len(), 3);
        assert_eq!(g.y_ticks[0].label, "$400");
        assert!(close(g.y_ticks[0].pos, 0.0, 1e-9));
        assert_eq!(g.y_ticks[1].label, "$200");
        assert!(close(g.y_ticks[2].pos, 100.0, 1e-9));
        let xs: Vec<f64> = g.x_ticks.iter().map(|t| t.pos).collect();
        assert_eq!(xs, [50.0, 250.0, 350.0]);
        assert_eq!(g.x_ticks[0].label, "01-01 00:00");
        assert!(g.bars[0].title.contains("long $300"));
    }

    #[test]
    fn volume_bars_edge_cases() {
        assert!(volume_layout(&[], 400.0, 100.0).bars.is_empty());
        let z = volume_layout(&[bucket(0, 0.0, 0.0), bucket(60, 0.0, 0.0)], 400.0, 100.0);
        assert_eq!(z.bars.len(), 2);
        assert!(z.bars.iter().all(|b| b.long_h == 0.0 && b.short_h == 0.0));
        assert!(z.y_ticks.is_empty());
        assert_eq!(z.x_ticks.len(), 2);
        let neg = volume_layout(&[bucket(0, -5.0, 10.0)], 100.0, 100.0);
        assert_eq!(neg.bars[0].long_h, 0.0);
        assert!(close(neg.bars[0].short_h, 100.0, 1e-9));
    }

    #[test]
    fn pnl_bins() {
        let h = pnl_histogram(&[-100.0, -10.0, 0.0, 5.0, 40.0, 100.0, f64::NAN], 4, 400.0, 80.0);
        assert_eq!(h.len(), 4);
        let edges: Vec<(f64, f64)> = h.iter().map(|b| (b.lo, b.hi)).collect();
        assert_eq!(edges, [(-100.0, -50.0), (-50.0, 0.0), (0.0, 50.0), (50.0, 100.0)]);
        let counts: Vec<usize> = h.iter().map(|b| b.count).collect();
        assert_eq!(counts, [1, 1, 3, 1]);
        assert_eq!(h.iter().map(|b| b.gain).collect::<Vec<_>>(), [false, false, true, true]);
        assert!(close(h[2].h, 80.0, 1e-9));
        assert!(close(h[2].y, 0.0, 1e-9));
        assert!(close(h[0].h, 80.0 / 3.0, 1e-9));
        assert!(close(h[1].x, 101.0, 1e-9));
        assert!(close(h[1].w, 98.0, 1e-9));
        // Odd bin counts round up so zero stays an edge.
        assert_eq!(pnl_histogram(&[1.0], 5, 100.0, 10.0).len(), 6);
        assert!(pnl_histogram(&[], 8, 100.0, 10.0).is_empty());
        let zeros = pnl_histogram(&[0.0, 0.0], 2, 100.0, 10.0);
        assert_eq!(zeros.iter().map(|b| b.count).collect::<Vec<_>>(), [0, 2]);
    }

    #[test]
    fn leaderboard_and_pnls() {
        let now = 1_000_000;
        let rows = vec![
            LeaderboardRowJson {
                owner: "7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h".into(),
                equity_usdc: 5200.0,
                pnl_usdc: 200.0,
                pnl_before_fees_usdc: 210.5,
                fees_paid_usdc: 10.5,
                pnl_pct: 0.04,
                q_usdc: 3000.0,
                volume_usdc: 21_000.0,
                trades: 14,
                last_trade_ts: Some(now - 80),
                ..Default::default()
            },
            LeaderboardRowJson {
                owner: "Op".into(),
                is_operator: true,
                pnl_usdc: -50.0,
                pnl_before_fees_usdc: -45.0,
                fees_paid_usdc: 5.0,
                pnl_pct: -0.01,
                q_usdc: -1800.0,
                ..Default::default()
            },
        ];
        let r = leaderboard_rows(&rows, now);
        assert_eq!(r[0].rank, 1);
        assert_eq!(r[0].owner_short, "7thn\u{2026}kz4h");
        assert_eq!(r[0].pnl, "+$200.00");
        assert_eq!(r[0].pnl_pct, "+4.00%");
        assert!(r[0].gain);
        assert_eq!(r[0].position, "$3.0K long");
        assert_eq!(r[0].position_side, Some(Side::Long));
        assert_eq!(r[0].volume, "$21.0K");
        assert_eq!(r[0].last_trade, "1m ago");
        assert_eq!(r[1].rank, 2);
        assert!(r[1].maker);
        assert!(!r[1].gain);
        assert_eq!(r[1].pnl, "\u{2212}$50.00");
        assert_eq!(r[1].position, "$1.8K short");
        assert_eq!(r[1].last_trade, "\u{2014}");
        assert_eq!(trader_pnls(&rows), [200.0]);
        assert_eq!((r[0].pnl_before_fees.as_str(), r[0].fees.as_str()), ("+$210.50", "$10.50"));
        assert!(r[0].before_fees_gain);
        assert_eq!((r[1].pnl_before_fees.as_str(), r[1].fees.as_str()), ("\u{2212}$45.00", "$5.00"));
        assert!(!r[1].before_fees_gain);
    }

    #[test]
    fn flow_of_funds_sim_numbers() {
        // The sim: traders lost $21.8k, $13.6k of it to fees and $8.2k to makers.
        let s = AnalyticsSummaryJson {
            trader_pnl_before_fees_usdc: -8_200.0,
            trader_fees_paid_usdc: 13_600.0,
            total_trader_pnl_usdc: -21_800.0,
            maker_pnl_usdc: 8_150.0,
            total_maker_pnl_usdc: 8_150.0,
            insurance_usdc: 10_000.0,
            treasury_usdc: 3_600.0,
            liquidation_penalties_usdc: 420.0,
            ..Default::default()
        };
        let f = flow_of_funds(Some(&s)).unwrap();
        let t: Vec<(&str, &str, Tone, bool)> = f.traders.iter().map(|l| (l.label, l.value.as_str(), l.tone, l.total)).collect();
        assert_eq!(
            t,
            [
                ("Trader PnL before fees", "\u{2212}$8,200.00", Tone::Loss, false),
                ("Fees paid", "\u{2212}$13,600.00", Tone::Loss, false),
                ("Trader PnL", "\u{2212}$21,800.00", Tone::Loss, true),
            ]
        );
        let c: Vec<(&str, &str, Tone)> = f.counterparties.iter().map(|l| (l.label, l.value.as_str(), l.tone)).collect();
        assert_eq!(
            c,
            [
                ("Makers' PnL", "+$8,150.00", Tone::Gain),
                ("Insurance fund", "$10,000.00", Tone::Plain),
                ("Treasury", "$3,600.00", Tone::Plain),
                ("Liquidation penalties", "$420.00", Tone::Plain),
            ]
        );
        assert_eq!(f.bar.len(), 2);
        assert_eq!((f.bar[0].label, f.bar[0].value.as_str()), ("Fees", "$13.6K"));
        assert_eq!((f.bar[1].label, f.bar[1].value.as_str()), ("Net to makers", "$8.2K"));
        assert!(close(f.bar[0].pct, 13_600.0 / 21_800.0 * 100.0, 1e-9));
        assert!(close(f.bar[0].pct + f.bar[1].pct, 100.0, 1e-9));
        assert_eq!(f.sentence, "Traders lost $21.8K in total: they paid $13.6K in fees and $8.2K net to makers.");
    }

    #[test]
    fn flow_of_funds_edge_cases() {
        assert!(flow_of_funds(None).is_none());
        let z = flow_of_funds(Some(&AnalyticsSummaryJson::default())).unwrap();
        assert!(z.bar.is_empty());
        assert_eq!(z.sentence, "No trader fees or PnL yet.");
        assert_eq!(z.traders[1].value, "$0.00");
        assert_eq!(z.traders[1].tone, Tone::Plain);
        // Traders beat the makers before fees: only fees are an outflow.
        let s = AnalyticsSummaryJson {
            trader_pnl_before_fees_usdc: 500.0,
            trader_fees_paid_usdc: 200.0,
            total_trader_pnl_usdc: 300.0,
            maker_pnl_usdc: -500.0,
            ..Default::default()
        };
        let f = flow_of_funds(Some(&s)).unwrap();
        assert_eq!(f.traders[0].tone, Tone::Gain);
        assert_eq!(f.counterparties[0].tone, Tone::Loss);
        assert_eq!(f.bar.len(), 1);
        assert_eq!((f.bar[0].label, f.bar[0].pct), ("Fees", 100.0));
        assert_eq!(f.sentence, "Traders made $300 in total: they won $500 net from makers and paid $200 in fees.");
        assert_eq!(flow_sentence(100.0, 300.0, -200.0), "Traders lost $200 in total: they won $100 net from makers and paid $300 in fees.");
    }

    #[test]
    fn makers_trades_liquidations() {
        let m = maker_rows(&[MakerRowJson {
            owner: "Op".into(),
            status: "active".into(),
            fresh: true,
            bond_usdc: 1000.0,
            equity_usdc: 51_000.0,
            pnl_usdc: 1000.0,
            inventory_usdc: -1800.0,
            volume_usdc: 90_000.0,
            fills: 120,
            half_spread_bps: 2.0,
            fair_secs: 5400.0,
            ..Default::default()
        }]);
        assert_eq!(m[0].status, "active");
        assert_eq!(m[0].bond, "$1.0K");
        assert_eq!(m[0].pnl, "+$1,000.00");
        assert_eq!(m[0].inventory, "\u{2212}$1,800.00");
        assert_eq!(m[0].spread, "2.0 bps");
        assert_eq!(m[0].fair, "1h 30m");
        assert_eq!(m[0].fills, "120");
        let fill = FillJson {
            signature: "s".into(),
            slot: 1,
            t: 990,
            side: SideStr::Short,
            kind: "swap".into(),
            size_usdc: 250.0,
            price_secs: 3600.0,
            fee_usdc: 0.125,
            operator: "Op".into(),
        };
        let t = trade_rows(&[AnalyticsTradeJson { fill, trader: "Tr".into() }], 1000);
        assert_eq!(t[0].time, "10s ago");
        assert_eq!(t[0].side, Side::Short);
        assert_eq!(t[0].side_label, "Short time");
        assert_eq!(t[0].size, "$250.00");
        assert_eq!(t[0].price, "1h");
        assert_eq!(t[0].fee, "$0.13");
        assert_eq!(t[0].trader_short, "Tr");
        let l = liquidation_rows(
            &[LiquidationJson {
                t: 400,
                trader: "T".into(),
                liquidator: String::new(),
                size_usdc: 3000.0,
                bankruptcy_secs: 900.0,
                penalty_usdc: 30.0,
                ..Default::default()
            }],
            1000,
        );
        assert_eq!(l[0].time, "10m ago");
        assert_eq!(l[0].liquidator_short, "\u{2014}");
        assert_eq!(l[0].bankruptcy, "15m");
        assert_eq!(l[0].penalty, "$30.00");
        assert_eq!(l[0].insurance, "$0.00");
    }
}

#[test]
fn timeframes_run_one_minute_to_one_day() {
    use super::TIMEFRAMES;
    let values: Vec<&str> = TIMEFRAMES.iter().map(|t| t.0).collect();
    assert_eq!(values, ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1d"]);
    assert_eq!(TIMEFRAMES.last().unwrap().1, "1D");
}

#[test]
fn market_header_volatility_and_funding_text() {
    use super::market::{funding_text, implied_vol};
    let v = implied_vol(4_440.0).unwrap();
    assert!((v - 0.8).abs() < 0.01, "{v}");
    assert!(implied_vol(0.0).is_none());
    assert_eq!(funding_text(0.000042), "Long time pays 0.0042%");
    assert_eq!(funding_text(-0.0001), "Long time receives 0.0100%");
    assert_eq!(funding_text(0.0), "—");
}

#[test]
fn timeframe_seconds() {
    use super::{timeframe_secs, TIMEFRAMES};
    assert_eq!(TIMEFRAMES.map(|t| timeframe_secs(t.0)), [60, 180, 300, 900, 1_800, 3_600, 7_200, 14_400, 21_600, 43_200, 86_400]);
    assert_eq!(timeframe_secs("x"), 3_600);
}

#[test]
fn market_header_shows_sol_price_and_band() {
    let m = MarketJson { sol_price_usd: Some(189.0), sol_change_24h: 0.0127, ..Default::default() };
    let b = market_bar(&m);
    assert_eq!((b.sol_price.as_str(), b.sol_change.as_str(), b.sol_up), ("$189.00", "+1.27%", true));
    assert_eq!(b.band, "$187.11 \u{2013} $190.89");
    let none = market_bar(&MarketJson::default());
    assert_eq!((none.sol_price.as_str(), none.band.as_str()), ("\u{2014}", "\u{2014}"));
}
