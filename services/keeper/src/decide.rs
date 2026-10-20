//! Pure keeper decisions; everything here is deterministic and chain-free.

use solana_pubkey::Pubkey;
use tradingtime_interface::{LimitOrder, MAX_ADL_PER_IX, MAX_OPERATORS_PER_IX, Market, Operator, Params, Side};
use tradingtime_math::curve::{self, Curve};

use crate::indexer::PositionRow;

pub const FUNDING_EVERY_MS: i64 = 60_000;
pub const MARK_EVERY_MS: i64 = 2_000;

pub fn should_crank_index(market: &Market, pyth_publish_time: i64) -> bool {
    pyth_publish_time > market.last_publish_time
}

pub fn due(last_ms: Option<i64>, now_ms: i64, every_ms: i64) -> bool {
    last_ms.is_none_or(|l| now_ms - l >= every_ms)
}

/// Fresh Active operators, best first (tightest spread, then most recent update), at most 8.
pub fn fresh<'a>(operators: &'a [(Pubkey, Operator)], slot: u64, params: &Params) -> Vec<&'a Operator> {
    let mut v: Vec<&Operator> =
        operators.iter().map(|(_, o)| o).filter(|o| o.is_fresh(slot, params.curve_max_age_slots)).collect();
    v.sort_by(|a, b| a.half_spread.get().cmp(&b.half_spread.get()).then(b.updated_slot.cmp(&a.updated_slot)));
    v.truncate(MAX_OPERATORS_PER_IX);
    v
}

/// Owners of `fresh` operators; the client ix builders take operator owners.
pub fn fresh_operators(operators: &[(Pubkey, Operator)], slot: u64, params: &Params) -> Vec<Pubkey> {
    fresh(operators, slot, params).into_iter().map(|o| Pubkey::new_from_array(o.owner)).collect()
}

/// True when some curve with room quotes at least as well as the order's limit at x = 0.
pub fn limit_reachable(order: &LimitOrder, curves: &[Curve]) -> bool {
    let Some(side) = Side::from_u8(order.side) else { return false };
    let limit = order.limit_ln.get();
    curves.iter().any(|c| {
        curve::room(c, side) > 0
            && curve::marginal(c, side, 0).is_some_and(|m| match side {
                Side::Long => m <= limit,
                Side::Short => m >= limit,
            })
    })
}

pub fn order_live(order: &LimitOrder, now_secs: i64) -> bool {
    order.filled < order.size && (order.expiry_ts == 0 || order.expiry_ts > now_secs)
}

/// Expired orders anyone may close (the program requires `0 < expiry_ts <= now`).
pub fn order_expired(order: &LimitOrder, now_secs: i64) -> bool {
    order.expiry_ts > 0 && order.expiry_ts <= now_secs
}

pub fn opposite(side: Side) -> Side {
    match side {
        Side::Long => Side::Short,
        Side::Short => Side::Long,
    }
}

/// ADL counterparties for a liquidated `target_side` position: opposite-side, in-profit, non-operator,
/// largest upnl first. `exclude` holds the target and every operator owner; the program rejects both.
pub fn adl_candidates(target_side: Side, positions: &[PositionRow], exclude: &[Pubkey]) -> Vec<Pubkey> {
    let want_long = target_side == Side::Short;
    let mut v: Vec<&PositionRow> = positions
        .iter()
        .filter(|p| !p.is_operator && !exclude.contains(&p.owner))
        .filter(|p| p.upnl_usdc > 0.0 && if want_long { p.q_usdc > 0.0 } else { p.q_usdc < 0.0 })
        .collect();
    v.sort_by(|a, b| b.upnl_usdc.total_cmp(&a.upnl_usdc));
    v.into_iter().take(MAX_ADL_PER_IX).map(|p| p.owner).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradingtime_interface::{OperatorStatus, PodI128};
    use tradingtime_math::{ONE, from_f64};

    fn pk(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn op(owner: u8, spread: f64, updated_slot: u64, status: OperatorStatus) -> (Pubkey, Operator) {
        let mut o: Operator = bytemuck::Zeroable::zeroed();
        o.owner = [owner; 32];
        o.fair_ln = PodI128::new(from_f64(8.0));
        o.half_spread = PodI128::new(from_f64(spread));
        o.impact_per_usdc = PodI128::new(from_f64(1e-6));
        o.inventory_limit = 1_000_000_000;
        o.updated_slot = updated_slot;
        o.status = status as u8;
        (pk(owner + 100), o)
    }

    fn order(side: Side, limit: f64, size: u64, filled: u64, expiry: i64) -> LimitOrder {
        let mut o: LimitOrder = bytemuck::Zeroable::zeroed();
        o.side = side as u8;
        o.limit_ln = PodI128::new(from_f64(limit));
        o.size = size;
        o.filled = filled;
        o.expiry_ts = expiry;
        o
    }

    fn curve(fair: f64, spread: f64, q: i64) -> Curve {
        Curve { fair_ln: from_f64(fair), half_spread: from_f64(spread), impact_per_usdc: 0, inventory_limit: 1_000_000, q }
    }

    #[test]
    fn crank_index_only_when_publish_time_advances() {
        let mut m: Market = bytemuck::Zeroable::zeroed();
        m.last_publish_time = 100;
        assert!(should_crank_index(&m, 101));
        assert!(!should_crank_index(&m, 100));
        assert!(!should_crank_index(&m, 99));
    }

    #[test]
    fn periodic_due() {
        assert!(due(None, 0, FUNDING_EVERY_MS));
        assert!(!due(Some(1_000), 60_999, FUNDING_EVERY_MS));
        assert!(due(Some(1_000), 61_000, FUNDING_EVERY_MS));
        assert!(due(Some(0), 2_000, MARK_EVERY_MS));
    }

    #[test]
    fn fresh_operators_filters_stale_and_inactive_and_sorts() {
        let p = Params::DEFAULT; // curve_max_age_slots = 10
        let ops = vec![
            op(1, 0.02, 100, OperatorStatus::Active),
            op(2, 0.01, 95, OperatorStatus::Active),
            op(3, 0.01, 99, OperatorStatus::Active),
            op(4, 0.001, 80, OperatorStatus::Active), // stale
            op(5, 0.001, 100, OperatorStatus::Unbonding),
        ];
        assert_eq!(fresh_operators(&ops, 100, &p), vec![pk(3), pk(2), pk(1)]);
    }

    #[test]
    fn fresh_operators_caps_at_eight() {
        let p = Params::DEFAULT;
        let ops: Vec<_> = (0..12).map(|i| op(i, 0.01 + i as f64 * 0.001, 50, OperatorStatus::Active)).collect();
        let f = fresh_operators(&ops, 50, &p);
        assert_eq!(f.len(), MAX_OPERATORS_PER_IX);
        assert_eq!(f[0], pk(0));
        assert_eq!(f[7], pk(7));
    }

    #[test]
    fn limit_reachable_long_needs_ask_at_or_below_limit() {
        let c = [curve(8.0, 0.01, 0)]; // ask 8.01, bid 7.99
        assert!(limit_reachable(&order(Side::Long, 8.02, 10, 0, 0), &c));
        assert!(limit_reachable(&order(Side::Long, 8.01, 10, 0, 0), &c));
        assert!(!limit_reachable(&order(Side::Long, 8.0, 10, 0, 0), &c));
    }

    #[test]
    fn limit_reachable_short_needs_bid_at_or_above_limit() {
        let c = [curve(8.0, 0.01, 0)];
        assert!(limit_reachable(&order(Side::Short, 7.98, 10, 0, 0), &c));
        assert!(!limit_reachable(&order(Side::Short, 8.0, 10, 0, 0), &c));
    }

    #[test]
    fn limit_reachable_any_curve_and_respects_room_and_inventory_skew() {
        let far = curve(9.0, 0.01, 0);
        let near = curve(8.0, 0.01, 0);
        assert!(limit_reachable(&order(Side::Long, 8.5, 10, 0, 0), &[far, near]));
        assert!(!limit_reachable(&order(Side::Long, 8.5, 10, 0, 0), &[]));
        // Operator at -limit has no room to sell to a long taker.
        let full = curve(8.0, 0.01, -1_000_000);
        assert!(!limit_reachable(&order(Side::Long, 8.5, 10, 0, 0), &[full]));
        // Inventory shifts the center: q > 0 lowers the center by impact * q.
        let skewed = Curve { impact_per_usdc: ONE / 10, ..curve(8.0, 0.0, 1_000_000) }; // center 7.9
        assert!(limit_reachable(&order(Side::Long, 7.95, 10, 0, 0), &[skewed]));
        assert!(!limit_reachable(&order(Side::Short, 7.95, 10, 0, 0), &[skewed]));
    }

    #[test]
    fn limit_reachable_rejects_bad_side() {
        let mut o = order(Side::Long, 100.0, 10, 0, 0);
        o.side = 7;
        assert!(!limit_reachable(&o, &[curve(8.0, 0.01, 0)]));
    }

    #[test]
    fn order_liveness() {
        assert!(order_live(&order(Side::Long, 8.0, 10, 0, 0), 1_000));
        assert!(order_live(&order(Side::Long, 8.0, 10, 5, 1_001), 1_000));
        assert!(!order_live(&order(Side::Long, 8.0, 10, 10, 0), 1_000));
        assert!(!order_live(&order(Side::Long, 8.0, 10, 0, 1_000), 1_000));
    }

    fn row(owner: u8, q: f64, upnl: f64) -> PositionRow {
        PositionRow { owner: pk(owner), q_usdc: q, upnl_usdc: upnl, is_operator: false }
    }

    #[test]
    fn adl_never_picks_operators() {
        let mut maker = row(3, -500.0, 90.0);
        maker.is_operator = true;
        let rows = vec![row(1, -100.0, 5.0), maker];
        assert_eq!(adl_candidates(Side::Long, &rows, &[]), vec![pk(1)]);
    }

    #[test]
    fn expired_orders() {
        assert!(order_expired(&order(Side::Long, 8.0, 10, 0, 1_000), 1_000));
        assert!(order_expired(&order(Side::Long, 8.0, 10, 5, 999), 1_000));
        assert!(!order_expired(&order(Side::Long, 8.0, 10, 0, 1_001), 1_000));
        assert!(!order_expired(&order(Side::Long, 8.0, 10, 0, 0), 1_000));
    }

    #[test]
    fn adl_picks_opposite_side_in_profit_largest_first() {
        let rows = vec![row(1, -100.0, 5.0), row(2, -50.0, 20.0), row(3, 70.0, 50.0), row(4, -10.0, -1.0), row(5, -1.0, 9.0)];
        assert_eq!(adl_candidates(Side::Long, &rows, &[]), vec![pk(2), pk(5), pk(1)]);
        assert_eq!(adl_candidates(Side::Short, &rows, &[]), vec![pk(3)]);
    }

    #[test]
    fn adl_caps_at_eight() {
        let rows: Vec<_> = (0..20).map(|i| row(i, 10.0, i as f64 + 1.0)).collect();
        let c = adl_candidates(Side::Short, &rows, &[]);
        assert_eq!(c.len(), MAX_ADL_PER_IX);
        assert_eq!(c[0], pk(19));
    }

    #[test]
    fn adl_skips_accounts_already_in_the_instruction() {
        let rows = vec![row(1, -100.0, 5.0), row(2, -50.0, 20.0), row(5, -1.0, 9.0)];
        assert_eq!(adl_candidates(Side::Long, &rows, &[pk(2), pk(9)]), vec![pk(5), pk(1)]);
    }
}
