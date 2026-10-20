use pinocchio::{AccountView, ProgramResult, cpi::Seed};
use tradingtime_interface::{
    self as tti, DISC_OPERATOR, Operator, OperatorStatus, PodI128, TtError, UpdateCurveData,
    events::{self, OperatorEvent},
};
use tradingtime_math::{ONE, position};

use crate::{payload, util::*};

fn operator_event(op: &Operator, kind: u8, ts: i64) {
    emit(events::TAG_OPERATOR, &OperatorEvent { owner: op.owner, kind, _pad: [0; 7], bond: op.bond, ts });
}

/// The owner's operator account.
fn owned_operator<'a>(owner: &AccountView, op_acc: &'a AccountView) -> R<&'a mut Operator> {
    signer(owner)?;
    let op = operator_mut(op_acc, true)?;
    ensure(key_eq(owner, &op.owner), TtError::Unauthorized)?;
    Ok(op)
}

/// Registers a new operator, or re-activates a Closed one with a fresh bond.
pub fn register(a: &[AccountView]) -> ProgramResult {
    let [owner, trader_acc, op_acc, market_acc, owner_usdc, vault, mint, token, system] = accounts::<9>(a)?;
    signer(owner)?;
    writable(op_acc)?;
    writable(owner_usdc)?;
    expect_key(system, &SYSTEM_PROGRAM, TtError::InvalidAccount)?;
    let m = market_mut(market_acc, true)?;
    let t = owned_trader(trader_acc, owner)?;
    token_accounts(m, vault, mint, token)?;
    let (_, ts) = now()?;
    settle(t, m, ts)?;

    let owner_key = owner.address().as_array();
    let op = if t.is_operator == 0 {
        None
    } else {
        let op = operator_mut(op_acc, true)?;
        ensure(key_eq(owner, &op.owner) && key_eq(trader_acc, &op.trader), TtError::InvalidAccount)?;
        ensure(op.status == OperatorStatus::Closed as u8, TtError::AlreadyInitialized)?;
        Some(op)
    };
    let eq = ovf(position::equity(&t.position(), m.effective_mark(ts)))?;
    ensure(eq >= m.params.operator_min_capital as i64, TtError::InsufficientCollateral)?;

    let op = match op {
        Some(op) => op,
        None => {
            let (key, bump) = find_pda(&[tti::SEED_OPERATOR, owner_key]);
            expect_key(op_acc, &key, TtError::InvalidPda)?;
            let b = [bump];
            let op = create_pda::<Operator>(owner, op_acc, &[Seed::from(tti::SEED_OPERATOR), Seed::from(owner_key), Seed::from(&b)])?;
            op.discriminator = DISC_OPERATOR;
            op.owner = *owner_key;
            op.trader = trader_acc.address().to_bytes();
            op.bump = bump;
            op
        }
    };
    let bond = m.params.operator_bond;
    transfer_in(owner_usdc, mint, vault, owner, bond)?;

    op.bond = bond;
    op.fair_ln = PodI128::new(0);
    op.half_spread = PodI128::new(0);
    op.impact_per_usdc = PodI128::new(0);
    op.inventory_limit = 0;
    op.updated_slot = 0;
    op.unbond_requested_ts = 0;
    op.status = OperatorStatus::Active as u8;
    t.is_operator = 1;
    m.operator_count += 1;
    operator_event(op, events::OPERATOR_REGISTERED, ts);
    Ok(())
}

/// Bounds a quoted ln-time to ±2^20 so curve arithmetic cannot overflow.
const MAX_LN: i128 = ONE << 20;

pub fn update_curve(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [owner, op_acc] = accounts::<2>(a)?;
    let d: UpdateCurveData = payload(data)?;
    let op = owned_operator(owner, op_acc)?;
    ensure(op.status == OperatorStatus::Active as u8, TtError::OperatorNotActive)?;
    let (fair, hs, imp) = (d.fair_ln.get(), d.half_spread.get(), d.impact_per_usdc.get());
    let ok = fair.abs() <= MAX_LN && (0..=MAX_LN).contains(&hs) && (0..=MAX_LN).contains(&imp);
    ensure(ok, TtError::ParamOutOfBounds)?;
    let (slot, _) = now()?;
    op.fair_ln = d.fair_ln;
    op.half_spread = d.half_spread;
    op.impact_per_usdc = d.impact_per_usdc;
    op.inventory_limit = d.inventory_limit;
    op.updated_slot = slot;
    Ok(())
}

pub fn request_unbond(a: &[AccountView]) -> ProgramResult {
    let [owner, op_acc] = accounts::<2>(a)?;
    let op = owned_operator(owner, op_acc)?;
    ensure(op.status == OperatorStatus::Active as u8, TtError::OperatorNotActive)?;
    let (_, ts) = now()?;
    op.status = OperatorStatus::Unbonding as u8;
    op.unbond_requested_ts = ts;
    operator_event(op, events::OPERATOR_UNBOND_REQUESTED, ts);
    Ok(())
}

pub fn complete_unbond(a: &[AccountView]) -> ProgramResult {
    let [owner, op_acc, trader_acc, market_acc, owner_usdc, vault, mint, token] = accounts::<8>(a)?;
    let op = owned_operator(owner, op_acc)?;
    let m = market_mut(market_acc, true)?;
    let _ = owned_trader(trader_acc, owner)?;
    ensure(key_eq(trader_acc, &op.trader), TtError::InvalidAccount)?;
    token_accounts(m, vault, mint, token)?;
    writable(owner_usdc)?;
    ensure(op.status != OperatorStatus::Active as u8, TtError::OperatorActive)?;
    ensure(op.status == OperatorStatus::Unbonding as u8, TtError::OperatorNotActive)?;
    let (_, ts) = now()?;
    ensure(ts >= op.unbond_requested_ts + m.params.unbond_delay_secs as i64, TtError::UnbondNotReady)?;
    let bond = op.bond;
    op.bond = 0;
    op.status = OperatorStatus::Closed as u8;
    m.operator_count = m.operator_count.saturating_sub(1);
    if bond > 0 {
        let bump = m.bump;
        transfer_out(market_acc, bump, vault, mint, owner_usdc, bond)?;
    }
    let mut ev = *op;
    ev.bond = bond;
    operator_event(&ev, events::OPERATOR_CLOSED, ts);
    Ok(())
}
