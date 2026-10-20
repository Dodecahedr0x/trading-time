use pinocchio::{AccountView, ProgramResult, cpi::Seed};
use tradingtime_interface::{
    self as tti, AmountData, DISC_TRADER, OperatorStatus, TtError, Trader,
    events::{self, CashEvent},
};
use tradingtime_math::position;

use crate::{payload, util::*};

pub fn create_trader(a: &[AccountView]) -> ProgramResult {
    let [owner, trader_acc, market_acc, system] = accounts::<4>(a)?;
    signer(owner)?;
    writable(trader_acc)?;
    expect_key(system, &SYSTEM_PROGRAM, TtError::InvalidAccount)?;
    let m = market_mut(market_acc, false)?;
    let owner_key = owner.address().as_array();
    let (key, bump) = find_pda(&[tti::SEED_TRADER, owner_key]);
    expect_key(trader_acc, &key, TtError::InvalidPda)?;
    let b = [bump];
    let t = create_pda::<Trader>(owner, trader_acc, &[Seed::from(tti::SEED_TRADER), Seed::from(owner_key), Seed::from(&b)])?;
    t.discriminator = DISC_TRADER;
    t.owner = *owner_key;
    t.funding_snapshot = m.funding_index;
    t.bump = bump;
    Ok(())
}

fn cash_event(owner: &AccountView, kind: u8, amount: u64, ts: i64) {
    emit(
        events::TAG_CASH,
        &CashEvent { owner: owner.address().to_bytes(), kind, _pad: [0; 7], amount: amount as i64, ts },
    );
}

pub fn deposit(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [owner, trader_acc, market_acc, owner_usdc, vault, mint, token] = accounts::<7>(a)?;
    let d: AmountData = payload(data)?;
    ensure(d.amount > 0 && d.amount <= i64::MAX as u64, TtError::InvalidInstruction)?;
    signer(owner)?;
    let m = market_mut(market_acc, true)?;
    let t = owned_trader(trader_acc, owner)?;
    token_accounts(m, vault, mint, token)?;
    writable(owner_usdc)?;
    let (_, ts) = now()?;
    settle(t, m, ts)?;
    transfer_in(owner_usdc, mint, vault, owner, d.amount)?;
    add_cash(t, d.amount as i64)?;
    m.total_cash = ovf(m.total_cash.checked_add(d.amount))?;
    cash_event(owner, events::CASH_DEPOSIT, d.amount, ts);
    Ok(())
}

pub fn withdraw(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [owner, trader_acc, market_acc, owner_usdc, vault, mint, token] = accounts::<7>(a)?;
    let d: AmountData = payload(data)?;
    ensure(d.amount > 0 && d.amount <= i64::MAX as u64, TtError::InvalidInstruction)?;
    signer(owner)?;
    let m = market_mut(market_acc, true)?;
    let t = owned_trader(trader_acc, owner)?;
    token_accounts(m, vault, mint, token)?;
    writable(owner_usdc)?;
    if t.is_operator != 0 {
        let op_acc = a.get(7).ok_or(err(TtError::OperatorActive))?;
        let op = operator_mut(op_acc, false)?;
        ensure(key_eq(trader_acc, &op.trader), TtError::InvalidAccount)?;
        ensure(op.status == OperatorStatus::Closed as u8, TtError::OperatorActive)?;
    }
    let (_, ts) = now()?;
    settle(t, m, ts)?;
    ensure(t.cash >= d.amount as i64, TtError::InsufficientCollateral)?;
    add_cash(t, -(d.amount as i64))?;
    let eq = ovf(position::equity(&t.position(), m.effective_mark(ts)))?;
    let need = t.margin_need();
    let ok = eq >= 0
        && need <= position::max_exposure(eq, m.params.max_leverage_bps) as u128
        && eq >= position::maintenance(t.q, m.params.maintenance_bps);
    ensure(ok, TtError::InsufficientCollateral)?;
    m.total_cash = m.total_cash.saturating_sub(d.amount);
    let bump = m.bump;
    transfer_out(market_acc, bump, vault, mint, owner_usdc, d.amount)?;
    cash_event(owner, events::CASH_WITHDRAW, d.amount, ts);
    Ok(())
}
