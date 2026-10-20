//! The simulator and the on-chain math must agree: log-quoted accounts and the propAMM curve.

use tradingtime::book::{Book, BookConfig};
use tradingtime::book_sim::prop_amm_ladder;
use tradingtime::pool::Side;
use tradingtime_math::{Side as TakerSide, USDC, curve, from_f64, position, to_f64};

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

#[test]
fn book_accounts_match_math_positions() {
    let cfg = BookConfig { taker_fee: 0.0, ..Default::default() };
    let mut book = Book::new(cfg, &[1_000_000.0, 50_000.0]);
    let mut pos = position::Position { cash: 50_000 * USDC as i64, ..Default::default() };
    for (i, (side, size, price)) in
        [(Side::Long, 4_000.0, 5_400.0), (Side::Long, 2_500.0, 6_100.0), (Side::Short, 9_000.0, 4_900.0), (Side::Long, 3_000.0, 5_000.0)]
            .into_iter()
            .enumerate()
    {
        let maker_side = if side == Side::Long { Side::Short } else { Side::Long };
        book.replace_quotes(0, &[(maker_side, price, size)], price);
        let f = book.market_fills(1, side, size, if side == Side::Long { 1e12 } else { 1e-12 }, 0.0);
        assert!((f.size - size).abs() < 1e-9, "fill {i}");
        let dq = (size * USDC as f64).round() as i64 * if side == Side::Long { 1 } else { -1 };
        position::apply_fill(&mut pos, dq, from_f64(price.ln())).unwrap();
    }
    for mark in [3_000.0f64, 5_400.0, 9_000.0] {
        let sim = book.accounts[1].equity(mark);
        let math = position::equity(&pos, from_f64(mark.ln())).unwrap() as f64 / USDC as f64;
        assert!(rel(math, sim) < 1e-9, "mark {mark}: math {math} sim {sim}");
    }
    assert!(rel(pos.q as f64 / USDC as f64, book.accounts[1].q) < 1e-12);
}

#[test]
fn prop_amm_ladder_matches_curve_integral() {
    for &(fair, h, impact, limit, q) in &[
        (5_400f64.ln(), 0.0002, 0.001 / 10_000.0, 60_000.0, 0.0),
        (4_000f64.ln(), 0.0005, 0.0025 / 10_000.0, 20_000.0, 7_500.0),
        (9_000f64.ln(), 0.0, 0.0005 / 10_000.0, 30_000.0, -12_000.0),
    ] {
        let ladder = prop_amm_ladder(fair, h, impact, limit, q, 2_000.0);
        let c = curve::Curve {
            fair_ln: from_f64(fair),
            half_spread: from_f64(h),
            impact_per_usdc: from_f64(impact),
            inventory_limit: (limit * USDC as f64) as u64,
            q: (q * USDC as f64) as i64,
        };
        for (quote_side, taker) in [(Side::Long, TakerSide::Short), (Side::Short, TakerSide::Long)] {
            let rows: Vec<_> = ladder.iter().filter(|r| r.0 == quote_side).collect();
            let total: f64 = rows.iter().map(|r| r.2).sum();
            assert!(rel(total, curve::room(&c, taker) as f64 / USDC as f64) < 1e-9, "room {quote_side:?}");
            let mut x = 0u64;
            for r in rows {
                let size = (r.2 * USDC as f64).round() as u64;
                let mid = curve::marginal(&c, taker, x + size / 2).unwrap();
                assert!((r.1.ln() - to_f64(mid)).abs() < 1e-9, "step at {x}: {} vs {}", r.1.ln(), to_f64(mid));
                x += size;
            }
            let sim_center = fair - impact * q;
            assert!((to_f64(curve::center(&c).unwrap()) - sim_center).abs() < 1e-9);
        }
    }
}
