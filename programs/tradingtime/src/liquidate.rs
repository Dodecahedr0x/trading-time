use pinocchio::{AccountView, ProgramResult};
use tradingtime_interface::{
    LiquidateData, MAX_ADL_PER_IX, PodI128, Trader, TtError,
    events::{self, LiquidationEvent, TradeEvent},
};
use tradingtime_math::{
    Side,
    curve::{self, Route},
    position,
};

use crate::{
    payload,
    trade::{FillCtx, apply_route, clamp_limit, distinct_accounts, fresh, load_pairs, refresh_mark, trade},
    util::*,
};

fn upnl(t: &Trader, mark: i128) -> R<i128> {
    ovf((t.q as i128).checked_mul(mark).and_then(|v| v.checked_sub(t.cost.get())))
}

#[inline(never)]
pub fn liquidate(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let d: LiquidateData = payload(data)?;
    let (n, n_adl) = (d.n_ops as usize, d.n_adl as usize);
    ensure(n <= tradingtime_interface::MAX_OPERATORS_PER_IX && n_adl <= MAX_ADL_PER_IX, TtError::TooManyAccounts)?;
    let [liquidator, target_acc, market_acc] = accounts::<3>(a)?;
    let pair_accs = a.get(3..3 + 2 * n).ok_or(err(TtError::InvalidInstruction))?;
    let adl_accs = a.get(3 + 2 * n..3 + 2 * n + n_adl).ok_or(err(TtError::InvalidInstruction))?;
    signer(liquidator)?;
    distinct_accounts(&[target_acc, market_acc], &a[3..3 + 2 * n + n_adl])?;
    let m = market_mut(market_acc, true)?;
    let t = trader_mut(target_acc)?;
    let (slot, ts) = now()?;
    settle(t, m, ts)?;
    let mut p = load_pairs(pair_accs, n, m, ts, true)?;
    refresh_mark(&p, m, slot, ts)?;
    let mark = m.effective_mark(ts);

    let q0 = t.q;
    ensure(q0 != 0, TtError::NotLiquidatable)?;
    let eq = ovf(position::equity(&t.position(), mark))?;
    ensure(eq < position::maintenance(q0, m.params.maintenance_bps), TtError::NotLiquidatable)?;

    let mut insurance_used = 0u64;
    if eq < 0 {
        let used = (eq.unsigned_abs()).min(m.insurance);
        cover(t, m, used)?;
        insurance_used += used;
    }
    let bk = ovf(position::bankruptcy_ln(&t.position()))?;
    let side = if q0 > 0 { Side::Short } else { Side::Long };
    let size = q0.unsigned_abs();

    let f = fresh(&p, slot, m.params.curve_max_age_slots);
    let route = if f.k > 0 {
        ovf(curve::route(&f.curves[..f.k], side, size, clamp_limit(bk)))?
    } else {
        Route::default()
    };
    if route.total > 0 {
        let ctx = FillCtx { side, kind: events::TRADE_LIQUIDATION, fee_total: 0, slot, ts };
        apply_route(&route, &f, t, &mut p, m, &ctx)?;
    }

    let remaining = size - route.total;
    if remaining > 0 && n_adl > 0 {
        adl(adl_accs, t, m, side, remaining, bk, mark, slot, ts)?;
    }
    let closed = size - t.q.unsigned_abs();
    ensure(closed > 0, TtError::NoFreshCurves)?;

    let eq_after = ovf(position::equity(&t.position(), m.effective_mark(ts)))?;
    let penalty = ceil_bps(closed, m.params.liq_fee_bps).min(eq_after.max(0) as u64);
    add_cash(t, -(penalty as i64))?;
    m.insurance = ovf(m.insurance.checked_add(penalty))?;
    m.total_cash = m.total_cash.saturating_sub(penalty);
    if t.q == 0 && t.cash < 0 {
        let used = t.cash.unsigned_abs().min(m.insurance);
        cover(t, m, used)?;
        insurance_used += used;
    }
    refresh_mark(&p, m, slot, ts)?;
    emit(
        events::TAG_LIQUIDATION,
        &LiquidationEvent {
            trader: t.owner,
            liquidator: liquidator.address().to_bytes(),
            size: closed,
            bankruptcy_ln: PodI128::new(bk),
            penalty,
            insurance_used,
            ts,
        },
    );
    Ok(())
}

fn cover(t: &mut Trader, m: &mut tradingtime_interface::Market, used: u64) -> R {
    add_cash(t, used as i64)?;
    m.insurance -= used;
    m.total_cash = ovf(m.total_cash.checked_add(used))?;
    Ok(())
}

/// Closes `remaining` of the target pro rata against opposite-side, in-profit traders at `bk`.
#[allow(clippy::too_many_arguments)]
fn adl(
    accs: &[AccountView],
    t: &mut Trader,
    m: &mut tradingtime_interface::Market,
    side: Side,
    remaining: u64,
    bk: i128,
    mark: i128,
    slot: u64,
    ts: i64,
) -> R {
    let mut cps: [Option<&mut Trader>; MAX_ADL_PER_IX] = [const { None }; MAX_ADL_PER_IX];
    let mut total: u128 = 0;
    let want_sign = -t.q.signum();
    for (i, acc) in accs.iter().enumerate() {
        let c = trader_mut(acc)?;
        settle(c, m, ts)?;
        let ok = c.is_operator == 0 && c.q.signum() == want_sign && upnl(c, mark)? > 0;
        ensure(ok, TtError::AdlInvalidCounterparty)?;
        total += c.q.unsigned_abs() as u128;
        cps[i] = Some(c);
    }
    let mut left = remaining;
    let count = accs.len();
    for (i, slot_c) in cps.iter_mut().take(count).enumerate() {
        let Some(c) = slot_c.as_deref_mut() else { continue };
        let cap = c.q.unsigned_abs();
        let share = if total <= remaining as u128 {
            cap
        } else if i + 1 == count {
            left
        } else {
            (remaining as u128 * cap as u128 / total) as u64
        };
        let alloc = share.min(cap).min(left);
        if alloc == 0 {
            continue;
        }
        left -= alloc;
        let size = i64::try_from(alloc).map_err(|_| err(TtError::MathOverflow))?;
        let dq = side.sign() * size;
        let notional = ovf((alloc as i128).checked_mul(bk))?;
        trade(t, dq, notional, m, alloc)?;
        trade(c, -dq, notional, m, alloc)?;
        emit(
            events::TAG_TRADE,
            &TradeEvent {
                trader: t.owner,
                operator: c.owner,
                side: side as u8,
                kind: events::TRADE_ADL,
                _pad: [0; 6],
                size: alloc,
                ln_price: PodI128::new(bk),
                fee: 0,
                slot,
                ts,
            },
        );
    }
    Ok(())
}
