use pinocchio::{AccountView, Address, ProgramResult, cpi::Seed};
use tradingtime_interface::{
    self as tti, CountData, DISC_ORDER, LimitOrder, Market, Operator, PlaceLimitOrderData, PodI128, SwapData, Trader,
    TtError,
    events::{self, OrderEvent, TradeEvent},
};
use tradingtime_math::{
    ONE, Side,
    curve::{self, Curve, Route},
    position,
};

use crate::{payload, util::*};

/// Caps caller limits so routing arithmetic (headroom × 1e6) cannot overflow.
const LIMIT_CAP: i128 = ONE << 42;

pub fn clamp_limit(l: i128) -> i128 {
    l.clamp(-LIMIT_CAP, LIMIT_CAP)
}

pub struct Pairs<'a> {
    pub n: usize,
    pub op: [Option<&'a mut Operator>; 8],
    pub tr: [Option<&'a mut Trader>; 8],
}

/// Loads `n` [operator, operator_trader] pairs; settles each trader's funding when `write`.
pub fn load_pairs<'a>(accs: &'a [AccountView], n: usize, m: &Market, ts: i64, write: bool) -> R<Pairs<'a>> {
    let mut p = Pairs { n, op: [const { None }; 8], tr: [const { None }; 8] };
    for i in 0..n {
        let op = operator_mut(&accs[2 * i], false)?;
        let ta = &accs[2 * i + 1];
        ensure(key_eq(ta, &op.trader), TtError::InvalidAccount)?;
        let tr = trader_view(ta, write)?;
        if write {
            settle(tr, m, ts)?;
        }
        p.op[i] = Some(op);
        p.tr[i] = Some(tr);
    }
    Ok(p)
}

pub struct Fresh {
    pub curves: [Curve; 8],
    pub map: [u8; 8],
    pub k: usize,
}

pub fn fresh(p: &Pairs, slot: u64, max_age: u32) -> Fresh {
    let mut f = Fresh { curves: [Curve::default(); 8], map: [0; 8], k: 0 };
    for i in 0..p.n {
        let (Some(op), Some(tr)) = (p.op[i].as_deref(), p.tr[i].as_deref()) else { continue };
        if op.is_fresh(slot, max_age) {
            f.curves[f.k] = op.curve(tr.q);
            f.map[f.k] = i as u8;
            f.k += 1;
        }
    }
    f
}

/// Q24 mark: median curve center of fresh passed operators.
pub fn refresh_mark(p: &Pairs, m: &mut Market, slot: u64, ts: i64) -> R<bool> {
    let f = fresh(p, slot, m.params.curve_max_age_slots);
    if f.k == 0 {
        return Ok(false);
    }
    m.mark_ln = PodI128::new(ovf(curve::median_center(&f.curves[..f.k]))?);
    m.mark_updated_ts = ts;
    Ok(true)
}

/// Applies an exposure change at total ln notional, keeping open interest in sync.
pub fn trade(t: &mut Trader, dq: i64, ln_notional: i128, m: &mut Market, size: u64) -> R {
    oi(m, t.q, -1);
    let mut pos = t.position();
    ovf(position::apply_fill_notional(&mut pos, dq, ln_notional))?;
    t.set_position(&pos);
    oi(m, t.q, 1);
    t.volume = t.volume.saturating_add(size);
    Ok(())
}

pub struct FillCtx {
    pub side: Side,
    pub kind: u8,
    pub fee_total: u64,
    pub slot: u64,
    pub ts: i64,
}

/// Fills the taker against the routed operators and charges `fee_total` to the insurance fund (up to its
/// target share of open interest), then the treasury.
pub fn apply_route(route: &Route, f: &Fresh, taker: &mut Trader, p: &mut Pairs, m: &mut Market, c: &FillCtx) -> R {
    let fills = route.fills();
    let mut fee_left = c.fee_total;
    for (j, fill) in fills.iter().enumerate() {
        let pi = f.map[fill.idx as usize] as usize;
        let size = i64::try_from(fill.size).map_err(|_| err(TtError::MathOverflow))?;
        let dq = c.side.sign() * size;
        trade(taker, dq, fill.ln_notional, m, fill.size)?;
        let (Some(op), Some(tr)) = (p.op[pi].as_deref(), p.tr[pi].as_deref_mut()) else {
            return Err(err(TtError::InvalidAccount));
        };
        trade(tr, -dq, fill.ln_notional, m, fill.size)?;
        let fee = if j + 1 == fills.len() {
            fee_left
        } else {
            (c.fee_total as u128 * fill.size as u128 / route.total as u128) as u64
        };
        fee_left -= fee;
        emit(
            events::TAG_TRADE,
            &TradeEvent {
                trader: taker.owner,
                operator: op.owner,
                side: c.side as u8,
                kind: c.kind,
                _pad: [0; 6],
                size: fill.size,
                ln_price: PodI128::new(fill.ln_notional / fill.size as i128),
                fee,
                slot: c.slot,
                ts: c.ts,
            },
        );
    }
    add_cash(taker, -(c.fee_total as i64))?;
    let (to_insurance, to_treasury) = m.params.split_fee(c.fee_total, m.insurance, m.long_q.max(m.short_q));
    m.insurance = ovf(m.insurance.checked_add(to_insurance))?;
    m.treasury = ovf(m.treasury.checked_add(to_treasury))?;
    m.total_cash = m.total_cash.saturating_sub(c.fee_total);
    Ok(())
}

pub fn grew(q0: i64, q1: i64) -> bool {
    q1.unsigned_abs() > q0.unsigned_abs()
}

fn n_ops(n: u8) -> R<usize> {
    ensure(n as usize <= tti::MAX_OPERATORS_PER_IX, TtError::TooManyAccounts)?;
    Ok(n as usize)
}

/// Rejects repeated addresses among the program-owned accounts of an instruction.
pub fn distinct_accounts(fixed: &[&AccountView], rest: &[AccountView]) -> R {
    let total = fixed.len() + rest.len();
    ensure(total <= 32, TtError::TooManyAccounts)?;
    let mut ks: [&Address; 32] = [fixed[0].address(); 32];
    for (i, acc) in fixed.iter().copied().chain(rest.iter()).enumerate() {
        ks[i] = acc.address();
    }
    distinct(&ks[..total])
}

#[inline(never)]
pub fn swap(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let d: SwapData = payload(data)?;
    let side = Side::from_u8(d.side).ok_or(err(TtError::InvalidInstruction))?;
    ensure(d.size > 0, TtError::InvalidInstruction)?;
    let n = n_ops(d.n_ops)?;
    let [owner, trader_acc, market_acc] = accounts::<3>(a)?;
    let pair_accs = a.get(3..3 + 2 * n).ok_or(err(TtError::InvalidInstruction))?;
    signer(owner)?;
    distinct_accounts(&[trader_acc, market_acc], pair_accs)?;
    let m = market_mut(market_acc, true)?;
    let t = owned_trader(trader_acc, owner)?;
    let (slot, ts) = now()?;
    settle(t, m, ts)?;
    let mut p = load_pairs(pair_accs, n, m, ts, true)?;
    let f = fresh(&p, slot, m.params.curve_max_age_slots);
    ensure(f.k > 0, TtError::NoFreshCurves)?;
    let route = ovf(curve::route(&f.curves[..f.k], side, d.size, clamp_limit(d.limit_ln.get())))?;
    ensure(route.total > 0 && route.total >= d.min_fill, TtError::MinFillNotMet)?;
    let q0 = t.q;
    let ctx = FillCtx {
        side,
        kind: events::TRADE_SWAP,
        fee_total: ceil_bps(route.total, m.params.taker_fee_bps),
        slot,
        ts,
    };
    apply_route(&route, &f, t, &mut p, m, &ctx)?;
    refresh_mark(&p, m, slot, ts)?;
    if grew(q0, t.q) {
        ensure(m.paused == 0, TtError::Paused)?;
        check_initial_margin(t, m.effective_mark(ts), m.params.max_leverage_bps)?;
    }
    Ok(())
}

fn order_event(o: &LimitOrder, kind: u8, ts: i64) {
    emit(
        events::TAG_ORDER,
        &OrderEvent {
            owner: o.owner,
            order_id: o.order_id,
            kind,
            side: o.side,
            _pad: [0; 6],
            size: o.size,
            filled: o.filled,
            limit_ln: o.limit_ln,
            ts,
        },
    );
}

pub fn place_limit_order(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [owner, trader_acc, order_acc, market_acc, system] = accounts::<5>(a)?;
    let d: PlaceLimitOrderData = payload(data)?;
    let side = Side::from_u8(d.side).ok_or(err(TtError::InvalidInstruction))?;
    ensure(d.size > 0 && d.size <= i64::MAX as u64, TtError::InvalidInstruction)?;
    signer(owner)?;
    writable(order_acc)?;
    expect_key(system, &SYSTEM_PROGRAM, TtError::InvalidAccount)?;
    let m = market_mut(market_acc, true)?;
    let t = owned_trader(trader_acc, owner)?;
    let (slot, ts) = now()?;
    ensure(d.expiry_ts > ts, TtError::OrderExpired)?;
    settle(t, m, ts)?;
    let q1 = t.q.saturating_add(side.sign() * d.size as i64);
    ensure(m.paused == 0 || !grew(t.q, q1), TtError::Paused)?;

    let owner_key = owner.address().as_array();
    let id = d.order_id.to_le_bytes();
    let (key, bump) = find_pda(&[tti::SEED_ORDER, owner_key, &id]);
    expect_key(order_acc, &key, TtError::InvalidPda)?;
    let b = [bump];
    let o = create_pda::<LimitOrder>(
        owner,
        order_acc,
        &[Seed::from(tti::SEED_ORDER), Seed::from(owner_key), Seed::from(&id), Seed::from(&b)],
    )?;
    o.discriminator = DISC_ORDER;
    o.owner = *owner_key;
    o.order_id = d.order_id;
    o.side = d.side;
    o.bump = bump;
    o.size = d.size;
    o.limit_ln = d.limit_ln;
    o.expiry_ts = d.expiry_ts;
    o.reserved = d.size;
    o.created_slot = slot;

    let r = t.reserved_mut(side);
    *r = ovf(r.checked_add(d.size))?;
    t.open_orders += 1;
    m.next_order_seq += 1;
    check_initial_margin(t, m.effective_mark(ts), m.params.max_leverage_bps)?;
    order_event(o, events::ORDER_PLACED, ts);
    Ok(())
}

/// Refunds the order's rent to `to` and closes it.
fn close_order(order_acc: &AccountView, to: &AccountView) -> R {
    let (mut oa, mut ta) = (*order_acc, *to);
    ta.set_lamports(ovf(ta.lamports().checked_add(oa.lamports()))?);
    oa.set_lamports(0);
    oa.close()
}

fn release(t: &mut Trader, o: &LimitOrder, amount: u64) {
    if let Some(side) = Side::from_u8(o.side) {
        let r = t.reserved_mut(side);
        *r = r.saturating_sub(amount);
    }
}

/// The owner cancels any order; once expired, any signer may close it, passing the owner (w) as a 4th account
/// to receive the rent.
pub fn cancel_limit_order(a: &[AccountView]) -> ProgramResult {
    let [signer_acc, trader_acc, order_acc] = accounts::<3>(a)?;
    signer(signer_acc)?;
    let o = order_mut(order_acc)?;
    let (_, ts) = now()?;
    let rent_to = if key_eq(signer_acc, &o.owner) {
        signer_acc
    } else {
        ensure(o.expiry_ts > 0 && o.expiry_ts <= ts, TtError::Unauthorized)?;
        let owner = a.get(3).ok_or(err(TtError::InvalidAccount))?;
        ensure(key_eq(owner, &o.owner), TtError::InvalidAccount)?;
        owner
    };
    writable(rent_to)?;
    let t = trader_mut(trader_acc)?;
    ensure(t.owner == o.owner, TtError::Unauthorized)?;
    release(t, o, o.reserved);
    t.open_orders = t.open_orders.saturating_sub(1);
    order_event(o, events::ORDER_CANCELLED, ts);
    close_order(order_acc, rent_to)
}

#[inline(never)]
pub fn execute_limit_order(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let d: CountData = payload(data)?;
    let n = n_ops(d.n_ops)?;
    let [keeper, keeper_trader_acc, order_acc, order_owner, trader_acc, market_acc] = accounts::<6>(a)?;
    let pair_accs = a.get(6..6 + 2 * n).ok_or(err(TtError::InvalidInstruction))?;
    signer(keeper)?;
    writable(order_owner)?;
    distinct_accounts(&[keeper_trader_acc, order_acc, trader_acc, market_acc], pair_accs)?;
    let m = market_mut(market_acc, true)?;
    let kt = owned_trader(keeper_trader_acc, keeper)?;
    let o = order_mut(order_acc)?;
    ensure(key_eq(order_owner, &o.owner), TtError::InvalidAccount)?;
    let t = owned_trader(trader_acc, order_owner)?;
    let (slot, ts) = now()?;
    ensure(ts <= o.expiry_ts, TtError::OrderExpired)?;
    settle(t, m, ts)?;
    settle(kt, m, ts)?;
    let mut p = load_pairs(pair_accs, n, m, ts, true)?;
    let f = fresh(&p, slot, m.params.curve_max_age_slots);
    ensure(f.k > 0, TtError::NoFreshCurves)?;
    let side = Side::from_u8(o.side).ok_or(err(TtError::InvalidAccount))?;
    let remaining = o.size - o.filled;
    let route = ovf(curve::route(&f.curves[..f.k], side, remaining, clamp_limit(o.limit_ln.get())))?;
    ensure(route.total > 0, TtError::LimitNotReached)?;
    let q0 = t.q;
    let ctx = FillCtx {
        side,
        kind: events::TRADE_LIMIT,
        fee_total: ceil_bps(route.total, m.params.taker_fee_bps),
        slot,
        ts,
    };
    apply_route(&route, &f, t, &mut p, m, &ctx)?;
    let keeper_fee = ceil_bps(route.total, m.params.keeper_fee_bps) as i64;
    add_cash(t, -keeper_fee)?;
    add_cash(kt, keeper_fee)?;
    refresh_mark(&p, m, slot, ts)?;

    o.filled += route.total;
    let freed = route.total.min(o.reserved);
    o.reserved -= freed;
    release(t, o, freed);
    if grew(q0, t.q) {
        ensure(m.paused == 0, TtError::Paused)?;
        check_initial_margin(t, m.effective_mark(ts), m.params.max_leverage_bps)?;
    }
    if o.filled >= o.size {
        t.open_orders = t.open_orders.saturating_sub(1);
        release(t, o, o.reserved);
        order_event(o, events::ORDER_FILLED, ts);
        close_order(order_acc, order_owner)?;
    } else {
        order_event(o, events::ORDER_PARTIAL, ts);
    }
    Ok(())
}
