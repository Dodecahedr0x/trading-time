use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use solana_pubkey::Pubkey;
use tradingtime_interface::{LimitOrder, Market, Operator, OperatorStatus, Params, PodI128, Side, Trader};
use tradingtime_keeper::chain::{Action, Chain};
use tradingtime_keeper::indexer::{Indexer, LiquidatableRow, OpenOrderRow, PositionRow};
use tradingtime_keeper::keeper::Keeper;
use tradingtime_client::slot::PerfSample;
use tradingtime_math::from_f64;

fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

const KEEPER: u8 = 200;
const FEED: u8 = 201;

struct FakeChain {
    market: RefCell<Market>,
    slot: u64,
    publish_time: RefCell<i64>,
    operators: Vec<(Pubkey, Operator)>,
    orders: Vec<(Pubkey, LimitOrder)>,
    traders: RefCell<HashMap<Pubkey, Trader>>,
    fail: HashSet<&'static str>,
    reject: Option<fn(&Action) -> bool>,
    sent: RefCell<Vec<Action>>,
    ticks: Cell<usize>,
    /// None: the RPC rejects getRecentPerformanceSamples.
    samples: Option<Vec<PerfSample>>,
    samples_delay_ms: u64,
}

fn kind(a: &Action) -> &'static str {
    match a {
        Action::CreateTrader => "create_trader",
        Action::CrankIndex { .. } => "crank_index",
        Action::CrankFunding => "crank_funding",
        Action::CrankMark { .. } => "crank_mark",
        Action::Liquidate { .. } => "liquidate",
        Action::ExecuteLimitOrder { .. } => "execute",
        Action::CancelExpiredOrder { .. } => "cancel_expired",
    }
}

impl Chain for FakeChain {
    fn keeper(&self) -> Pubkey {
        pk(KEEPER)
    }
    async fn market(&self) -> Result<Market> {
        self.ticks.set(self.ticks.get() + 1);
        Ok(*self.market.borrow())
    }
    async fn slot(&self) -> Result<u64> {
        Ok(self.slot)
    }
    async fn pyth_publish_time(&self, feed: &Pubkey) -> Result<i64> {
        assert_eq!(*feed, pk(FEED));
        Ok(*self.publish_time.borrow())
    }
    async fn operators(&self) -> Result<Vec<(Pubkey, Operator)>> {
        Ok(self.operators.clone())
    }
    async fn orders(&self) -> Result<Vec<(Pubkey, LimitOrder)>> {
        Ok(self.orders.clone())
    }
    async fn trader(&self, owner: &Pubkey) -> Result<Option<Trader>> {
        Ok(self.traders.borrow().get(owner).copied())
    }
    async fn perf_samples(&self) -> Result<Vec<PerfSample>> {
        tokio::time::sleep(std::time::Duration::from_millis(self.samples_delay_ms)).await;
        self.samples.clone().ok_or_else(|| anyhow!("method not found"))
    }
    async fn send(&self, action: &Action) -> Result<()> {
        if self.fail.contains(kind(action)) || self.reject.is_some_and(|f| f(action)) {
            return Err(anyhow!("simulated {}", kind(action)));
        }
        match action {
            Action::CreateTrader => {
                self.traders.borrow_mut().insert(pk(KEEPER), trader(KEEPER, 0));
            }
            Action::CrankIndex { .. } => self.market.borrow_mut().last_publish_time = *self.publish_time.borrow(),
            _ => {}
        }
        self.sent.borrow_mut().push(action.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FakeIndexer {
    liquidatable: Vec<LiquidatableRow>,
    positions: Vec<PositionRow>,
    down: bool,
    position_queries: RefCell<Vec<(Side, bool)>>,
}

impl Indexer for FakeIndexer {
    async fn liquidatable(&self) -> Result<Vec<LiquidatableRow>> {
        if self.down {
            return Err(anyhow!("indexer down"));
        }
        Ok(self.liquidatable.clone())
    }
    async fn positions(&self, side: Side, in_profit: bool) -> Result<Vec<PositionRow>> {
        self.position_queries.borrow_mut().push((side, in_profit));
        Ok(self
            .positions
            .iter()
            .filter(|p| (side == Side::Long) == (p.q_usdc > 0.0) && (!in_profit || p.upnl_usdc > 0.0))
            .cloned()
            .collect())
    }
    async fn open_orders(&self) -> Result<Vec<OpenOrderRow>> {
        Ok(vec![])
    }
}

fn trader(owner: u8, q: i64) -> Trader {
    let mut t: Trader = bytemuck::Zeroable::zeroed();
    t.owner = [owner; 32];
    t.q = q;
    t
}

fn operator(owner: u8, fair: f64, spread: f64, updated_slot: u64) -> (Pubkey, Operator) {
    let mut o: Operator = bytemuck::Zeroable::zeroed();
    o.owner = [owner; 32];
    o.fair_ln = PodI128::new(from_f64(fair));
    o.half_spread = PodI128::new(from_f64(spread));
    o.inventory_limit = 1_000_000_000;
    o.updated_slot = updated_slot;
    o.status = OperatorStatus::Active as u8;
    (pk(owner.wrapping_add(50)), o)
}

fn order(owner: u8, id: u64, side: Side, limit: f64, expiry: i64) -> (Pubkey, LimitOrder) {
    let mut o: LimitOrder = bytemuck::Zeroable::zeroed();
    o.owner = [owner; 32];
    o.order_id = id;
    o.side = side as u8;
    o.size = 1_000_000;
    o.limit_ln = PodI128::new(from_f64(limit));
    o.expiry_ts = expiry;
    (pk(owner.wrapping_add(150)), o)
}

fn chain() -> FakeChain {
    let mut m: Market = bytemuck::Zeroable::zeroed();
    m.params = Params::DEFAULT;
    m.pyth_feed = [FEED; 32];
    m.last_publish_time = 1_000;
    let c = FakeChain {
        market: RefCell::new(m),
        slot: 500,
        publish_time: RefCell::new(1_000),
        operators: vec![],
        orders: vec![],
        traders: RefCell::default(),
        fail: HashSet::new(),
        reject: None,
        sent: RefCell::default(),
        ticks: Cell::new(0),
        samples: None,
        samples_delay_ms: 0,
    };
    c.traders.borrow_mut().insert(pk(KEEPER), trader(KEEPER, 0));
    c
}

fn kinds(c: &FakeChain) -> Vec<&'static str> {
    c.sent.borrow().iter().map(kind).collect()
}

#[tokio::test]
async fn creates_keeper_trader_only_when_missing() {
    let c = chain();
    c.traders.borrow_mut().clear();
    let k = Keeper::new(c, FakeIndexer::default());
    assert!(k.ensure_trader().await.unwrap());
    assert!(!k.ensure_trader().await.unwrap());
    assert_eq!(kinds(&k.chain), vec!["create_trader"]);
}

#[tokio::test]
async fn cranks_index_once_per_new_publish_time() {
    let mut k = Keeper::new(chain(), FakeIndexer::default());
    k.tick(0).await.unwrap();
    assert!(!kinds(&k.chain).contains(&"crank_index"));
    *k.chain.publish_time.borrow_mut() = 1_001;
    let r = k.tick(400).await.unwrap();
    assert_eq!(r.sent, vec![Action::CrankIndex { pyth_feed: pk(FEED) }]);
    let r = k.tick(800).await.unwrap();
    assert!(r.sent.is_empty());
}

#[tokio::test]
async fn funding_every_60s_and_mark_every_2s_with_fresh_operators() {
    let mut c = chain();
    c.operators = vec![operator(1, 8.0, 0.02, 500), operator(2, 8.0, 0.01, 499), operator(3, 8.0, 0.001, 400)];
    let mut k = Keeper::new(c, FakeIndexer::default());
    let r = k.tick(0).await.unwrap();
    assert_eq!(r.sent, vec![Action::CrankFunding, Action::CrankMark { operators: vec![pk(2), pk(1)] }]);
    assert!(k.tick(1_000).await.unwrap().sent.is_empty());
    assert_eq!(k.tick(2_000).await.unwrap().sent, vec![Action::CrankMark { operators: vec![pk(2), pk(1)] }]);
    k.tick(59_999).await.unwrap();
    assert!(k.tick(60_000).await.unwrap().sent.contains(&Action::CrankFunding));
}

#[tokio::test]
async fn no_mark_crank_without_fresh_operators() {
    let mut c = chain();
    c.operators = vec![operator(1, 8.0, 0.02, 100)];
    let mut k = Keeper::new(c, FakeIndexer::default());
    assert_eq!(k.tick(0).await.unwrap().sent, vec![Action::CrankFunding]);
}

#[tokio::test]
async fn liquidates_with_opposite_side_adl_candidates() {
    let mut c = chain();
    c.operators = vec![operator(1, 8.0, 0.01, 500)];
    c.traders.borrow_mut().insert(pk(10), trader(10, 5_000_000)); // long, underwater
    c.traders.borrow_mut().insert(pk(11), trader(11, -2_000_000)); // short, underwater
    let ix = FakeIndexer {
        liquidatable: vec![
            LiquidatableRow { owner: pk(10), equity_usdc: 0.1, maintenance_usdc: 0.25 },
            LiquidatableRow { owner: pk(11), equity_usdc: 0.0, maintenance_usdc: 0.1 },
            LiquidatableRow { owner: pk(12), equity_usdc: 0.0, maintenance_usdc: 0.1 }, // no account: skipped
        ],
        positions: vec![
            pos(20, -3.0, 1.0),
            pos(21, -1.0, 4.0),
            pos(22, 2.0, 7.0),
            pos(23, -9.0, -2.0),
        ],
        ..Default::default()
    };
    let mut k = Keeper::new(c, ix);
    let r = k.tick(0).await.unwrap();
    let liqs: Vec<_> = r.sent.into_iter().filter(|a| matches!(a, Action::Liquidate { .. })).collect();
    assert_eq!(
        liqs,
        vec![
            Action::Liquidate { target: pk(10), operators: vec![pk(1)], adl: vec![pk(21), pk(20)] },
            Action::Liquidate { target: pk(11), operators: vec![pk(1)], adl: vec![pk(22)] },
        ]
    );
    assert_eq!(*k.indexer.position_queries.borrow(), vec![(Side::Short, true), (Side::Long, true)]);
}

fn pos(owner: u8, q: f64, upnl: f64) -> PositionRow {
    PositionRow { owner: pk(owner), q_usdc: q, upnl_usdc: upnl, is_operator: false }
}

#[tokio::test]
async fn never_deleverages_against_operators() {
    let mut c = chain();
    // Op 1 is fresh, op 2 is stale; neither may be an ADL counterparty, flagged by the indexer or not.
    c.operators = vec![operator(1, 8.0, 0.01, 500), operator(2, 8.0, 0.01, 100)];
    c.reject = Some(|a| matches!(a, Action::Liquidate { operators, .. } if !operators.is_empty()));
    c.traders.borrow_mut().insert(pk(10), trader(10, 5_000_000));
    let mut flagged = pos(3, -8.0, 9.0);
    flagged.is_operator = true;
    let ix = FakeIndexer {
        liquidatable: vec![LiquidatableRow { owner: pk(10), equity_usdc: -9.0, maintenance_usdc: 0.25 }],
        positions: vec![pos(1, -3.0, 2.0), pos(2, -4.0, 3.0), flagged, pos(20, -1.0, 1.0)],
        ..Default::default()
    };
    let mut k = Keeper::new(c, ix);
    let r = k.tick(0).await.unwrap();
    let want = Action::Liquidate { target: pk(10), operators: vec![pk(1)], adl: vec![pk(20)] };
    assert_eq!(r.failed.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>(), vec![want]);
    assert!(!r.sent.iter().any(|a| matches!(a, Action::Liquidate { .. })), "no operator-only fallback");
}

#[tokio::test]
async fn closes_expired_orders() {
    let mut c = chain();
    c.orders = vec![order(30, 1, Side::Long, 8.0, 9), order(31, 2, Side::Short, 8.0, 10), order(32, 3, Side::Long, 8.0, 11), order(33, 4, Side::Long, 8.0, 0)];
    let mut k = Keeper::new(c, FakeIndexer::default());
    let r = k.tick(10_000).await.unwrap();
    let closed: Vec<_> = r.sent.into_iter().filter(|a| matches!(a, Action::CancelExpiredOrder { .. })).collect();
    assert_eq!(
        closed,
        vec![
            Action::CancelExpiredOrder { owner: pk(30), order_id: 1 },
            Action::CancelExpiredOrder { owner: pk(31), order_id: 2 },
        ]
    );
}

#[tokio::test]
async fn executes_only_live_reachable_orders() {
    let mut c = chain();
    c.operators = vec![operator(1, 8.0, 0.01, 500), operator(2, 9.0, 0.01, 300)]; // op 2 stale
    c.traders.borrow_mut().insert(pk(1), trader(1, 0));
    c.orders = vec![
        order(30, 1, Side::Long, 8.02, 0),     // ask 8.01 <= 8.02: fill
        order(31, 2, Side::Long, 8.0, 0),      // ask above limit
        order(32, 3, Side::Short, 7.98, 0),    // bid 7.99 >= 7.98: fill
        order(33, 4, Side::Short, 8.5, 0),     // only stale op 2 would reach
        order(34, 5, Side::Long, 9.0, 1),      // expired
    ];
    let mut k = Keeper::new(c, FakeIndexer::default());
    let r = k.tick(10_000).await.unwrap();
    let ex: Vec<_> = r.sent.into_iter().filter(|a| matches!(a, Action::ExecuteLimitOrder { .. })).collect();
    assert_eq!(
        ex,
        vec![
            Action::ExecuteLimitOrder { owner: pk(30), order_id: 1, operators: vec![pk(1)] },
            Action::ExecuteLimitOrder { owner: pk(32), order_id: 3, operators: vec![pk(1)] },
        ]
    );
}

#[tokio::test]
async fn operator_inventory_moves_curve_for_order_reachability() {
    let mut c = chain();
    let (pda, mut op) = operator(1, 8.0, 0.0, 500);
    op.impact_per_usdc = PodI128::new(from_f64(0.1)); // 0.1 ln per USDC of inventory
    c.operators = vec![(pda, op)];
    c.traders.borrow_mut().insert(pk(1), trader(1, 1_000_000)); // long 1 USDC: center 7.9
    c.orders = vec![order(30, 1, Side::Long, 7.95, 0)];
    let mut k = Keeper::new(c, FakeIndexer::default());
    let r = k.tick(0).await.unwrap();
    assert!(r.sent.iter().any(|a| matches!(a, Action::ExecuteLimitOrder { order_id: 1, .. })));
}

#[tokio::test]
async fn failures_do_not_stop_the_tick() {
    let mut c = chain();
    c.fail.insert("crank_funding");
    c.operators = vec![operator(1, 8.0, 0.01, 500)];
    c.traders.borrow_mut().insert(pk(1), trader(1, 0));
    c.orders = vec![order(30, 1, Side::Long, 8.02, 0)];
    let ix = FakeIndexer { down: true, ..Default::default() };
    let mut k = Keeper::new(c, ix);
    let r = k.tick(0).await.unwrap();
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, Action::CrankFunding);
    assert_eq!(kinds(&k.chain), vec!["crank_mark", "execute"]);
}

fn ps(num_slots: u64, period_secs: u64) -> PerfSample {
    PerfSample { num_slots, period_secs }
}

async fn ticks_in(c: FakeChain, interval_ms: Option<u64>, ms: u64) -> usize {
    let mut k = Keeper::new(c, FakeIndexer::default());
    k.run_until(interval_ms, tokio::time::sleep(std::time::Duration::from_millis(ms))).await;
    k.chain.ticks.get()
}

#[tokio::test]
async fn loop_ticks_at_estimated_slot_time() {
    let c = FakeChain { samples: Some(vec![ps(300, 60)]), ..chain() };
    // 200ms slots: ticks at 0, 200, ..., 1000.
    let n = ticks_in(c, None, 1_050).await;
    assert!((5..=7).contains(&n), "{n}");
}

#[tokio::test]
async fn loop_falls_back_to_400ms_without_samples() {
    let n = ticks_in(chain(), None, 1_050).await;
    assert!((2..=4).contains(&n), "{n}");
}

#[tokio::test]
async fn interval_override_disables_estimation() {
    let c = FakeChain { samples: Some(vec![ps(300, 60)]), samples_delay_ms: 30_000, ..chain() };
    let n = ticks_in(c, Some(100), 1_050).await;
    assert!((9..=12).contains(&n), "{n}");
}

#[tokio::test]
async fn hung_slot_estimate_keeps_current_period() {
    let c = FakeChain { samples: Some(vec![ps(300, 60)]), samples_delay_ms: 30_000, ..chain() };
    // Estimate gives up after 250ms and keeps 400ms: ticks at ~250, 650, 1050.
    let n = ticks_in(c, None, 1_300).await;
    assert!((2..=4).contains(&n), "{n}");
}
