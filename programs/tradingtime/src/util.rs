use bytemuck::Pod;
use pinocchio::{
    AccountView, Address, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, clock::Clock},
};
use pinocchio_token::instructions::TransferChecked;
use tradingtime_interface::{
    self as tti, DISC_MARKET, DISC_OPERATOR, DISC_ORDER, DISC_TRADER, LimitOrder, Market, Operator, TtError, Trader,
};
use tradingtime_math::position::{self, Position};

pub const ID: Address = Address::new_from_array(tti::PROGRAM_ID);
pub const TOKEN_PROGRAM: Address = Address::new_from_array(tti::TOKEN_PROGRAM);
pub const SYSTEM_PROGRAM: Address = Address::new_from_array(tti::SYSTEM_PROGRAM);
pub const USDC_MINT: Address = Address::new_from_array(tti::USDC_MINT);
pub const PYTH_RECEIVER: Address = Address::new_from_array(tti::PYTH_RECEIVER);

pub type R<T = ()> = Result<T, ProgramError>;

pub fn err(e: TtError) -> ProgramError {
    ProgramError::Custom(e as u32)
}

pub fn ensure(cond: bool, e: TtError) -> R {
    if cond { Ok(()) } else { Err(err(e)) }
}

pub fn ovf<T>(v: Option<T>) -> R<T> {
    v.ok_or(err(TtError::MathOverflow))
}

pub fn now() -> R<(u64, i64)> {
    let c = Clock::get()?;
    Ok((c.slot, c.unix_timestamp))
}

pub fn accounts<const N: usize>(a: &[AccountView]) -> R<&[AccountView; N]> {
    a.get(..N).and_then(|s| s.try_into().ok()).ok_or(ProgramError::NotEnoughAccountKeys)
}

pub fn signer(a: &AccountView) -> R {
    ensure(a.is_signer(), TtError::Unauthorized)
}

pub fn writable(a: &AccountView) -> R {
    ensure(a.is_writable(), TtError::InvalidAccount)
}

pub fn key_eq(a: &AccountView, k: &[u8; 32]) -> bool {
    a.address().as_array() == k
}

pub fn expect_key(a: &AccountView, k: &Address, e: TtError) -> R {
    ensure(a.address() == k, e)
}

/// Zero-copy view of a program account. Callers must reject duplicate accounts
/// before holding two views at once.
#[allow(clippy::mut_from_ref)]
fn load<T: Pod>(a: &AccountView, disc: u64) -> R<&mut T> {
    ensure(a.owned_by(&ID), TtError::InvalidOwner)?;
    let n = core::mem::size_of::<T>();
    ensure(a.data_len() == n, TtError::InvalidAccount)?;
    // SAFETY: the runtime keeps account data alive and 8-aligned for the whole instruction.
    let bytes = unsafe { core::slice::from_raw_parts_mut(a.data_ptr() as *mut u8, n) };
    ensure(bytes[..8] == disc.to_le_bytes(), TtError::InvalidAccount)?;
    bytemuck::try_from_bytes_mut(bytes).map_err(|_| err(TtError::InvalidAccount))
}

#[allow(clippy::mut_from_ref)]
fn init<T: Pod>(a: &AccountView) -> R<&mut T> {
    let n = core::mem::size_of::<T>();
    ensure(a.owned_by(&ID) && a.data_len() == n, TtError::InvalidAccount)?;
    // SAFETY: as in `load`; the account was just created with exactly `n` zeroed bytes.
    let bytes = unsafe { core::slice::from_raw_parts_mut(a.data_ptr() as *mut u8, n) };
    bytemuck::try_from_bytes_mut(bytes).map_err(|_| err(TtError::InvalidAccount))
}

pub fn pda_eq<const N: usize>(a: &AccountView, seeds: &[&[u8]; N], bump: u8) -> R {
    let derived = Address::derive_address(seeds, Some(bump), &ID);
    ensure(a.address() == &derived, TtError::InvalidPda)
}

#[cfg(target_os = "solana")]
pub fn find_pda(seeds: &[&[u8]]) -> (Address, u8) {
    Address::find_program_address(seeds, &ID)
}

#[cfg(not(target_os = "solana"))]
pub fn find_pda(_seeds: &[&[u8]]) -> (Address, u8) {
    unimplemented!("PDA search needs the SBF runtime")
}

pub fn market_mut(a: &AccountView, write: bool) -> R<&mut Market> {
    if write {
        writable(a)?;
    }
    let m = load::<Market>(a, DISC_MARKET)?;
    pda_eq(a, &[tti::SEED_MARKET], m.bump)?;
    Ok(m)
}

pub fn trader_mut(a: &AccountView) -> R<&mut Trader> {
    trader_view(a, true)
}

pub fn trader_view(a: &AccountView, write: bool) -> R<&mut Trader> {
    if write {
        writable(a)?;
    }
    let t = load::<Trader>(a, DISC_TRADER)?;
    pda_eq(a, &[tti::SEED_TRADER, &t.owner], t.bump)?;
    Ok(t)
}

/// A trader account that must belong to `owner`.
pub fn owned_trader<'a>(a: &'a AccountView, owner: &AccountView) -> R<&'a mut Trader> {
    let t = trader_mut(a)?;
    ensure(key_eq(owner, &t.owner), TtError::Unauthorized)?;
    Ok(t)
}

pub fn operator_mut(a: &AccountView, write: bool) -> R<&mut Operator> {
    if write {
        writable(a)?;
    }
    let o = load::<Operator>(a, DISC_OPERATOR)?;
    pda_eq(a, &[tti::SEED_OPERATOR, &o.owner], o.bump)?;
    Ok(o)
}

pub fn order_mut(a: &AccountView) -> R<&mut LimitOrder> {
    writable(a)?;
    let o = load::<LimitOrder>(a, DISC_ORDER)?;
    let id = o.order_id.to_le_bytes();
    pda_eq(a, &[tti::SEED_ORDER, &o.owner, &id], o.bump)?;
    Ok(o)
}

/// Creates a program-owned PDA account of `size_of::<T>()` bytes funded by `payer`.
pub fn create_pda<'a, T: Pod>(payer: &AccountView, target: &'a AccountView, seeds: &[Seed]) -> R<&'a mut T> {
    ensure(target.lamports() == 0 && target.data_len() == 0, TtError::AlreadyInitialized)?;
    pinocchio_system::instructions::CreateAccount::with_minimum_balance(
        payer,
        target,
        core::mem::size_of::<T>() as u64,
        &ID,
        None,
    )?
    .invoke_signed(&[Signer::from(seeds)])?;
    init::<T>(target)
}

/// Rejects any repeated address in `keys`.
pub fn distinct(keys: &[&Address]) -> R {
    for i in 0..keys.len() {
        for j in i + 1..keys.len() {
            ensure(keys[i] != keys[j], TtError::InvalidAccount)?;
        }
    }
    Ok(())
}

pub fn token_accounts(market: &Market, vault: &AccountView, mint: &AccountView, token_program: &AccountView) -> R {
    ensure(key_eq(vault, &market.vault), TtError::InvalidAccount)?;
    writable(vault)?;
    expect_key(mint, &USDC_MINT, TtError::InvalidAccount)?;
    ensure(key_eq(mint, &market.usdc_mint), TtError::InvalidAccount)?;
    expect_key(token_program, &TOKEN_PROGRAM, TtError::InvalidAccount)
}

pub fn transfer_in(from: &AccountView, mint: &AccountView, vault: &AccountView, authority: &AccountView, amount: u64) -> ProgramResult {
    TransferChecked::<&AccountView>::new(from, mint, vault, authority, amount, tti::USDC_DECIMALS).invoke()
}

/// Transfer out of the vault, signed by the market PDA.
pub fn transfer_out(
    market_acc: &AccountView,
    market_bump: u8,
    vault: &AccountView,
    mint: &AccountView,
    to: &AccountView,
    amount: u64,
) -> ProgramResult {
    let bump = [market_bump];
    let seeds = [Seed::from(tti::SEED_MARKET), Seed::from(&bump)];
    TransferChecked::<&AccountView>::new(vault, mint, to, market_acc, amount, tti::USDC_DECIMALS)
        .invoke_signed(&[Signer::from(&seeds)])
}

pub fn emit<T: Pod>(tag: u8, ev: &T) {
    let mut buf = [0u8; 192];
    let n = tti::events::encode(tag, ev, &mut buf);
    log_data(&buf[..n]);
}

#[cfg(target_os = "solana")]
fn log_data(data: &[u8]) {
    let parts: [&[u8]; 1] = [data];
    // SAFETY: `parts` is a valid slice of byte slices for the duration of the call.
    unsafe { pinocchio::syscalls::sol_log_data(parts.as_ptr() as *const u8, 1) };
}

#[cfg(not(target_os = "solana"))]
fn log_data(_data: &[u8]) {}

/// Settles funding into the trader's cash, logging a CashEvent when cash moved.
pub fn settle(t: &mut Trader, market: &Market, ts: i64) -> R<Position> {
    let mut p = t.position();
    let moved = ovf(position::settle_funding(&mut p, market.funding_index.get()))?;
    t.set_position(&p);
    if moved != 0 {
        emit(
            tti::events::TAG_CASH,
            &tti::events::CashEvent { owner: t.owner, kind: tti::events::CASH_FUNDING, _pad: [0; 7], amount: moved, ts },
        );
    }
    Ok(p)
}

/// Removes (`sign = -1`) or adds (`sign = 1`) an exposure to the market's open interest.
pub fn oi(m: &mut Market, q: i64, sign: i64) {
    let a = q.unsigned_abs();
    let slot = if q > 0 { &mut m.long_q } else { &mut m.short_q };
    *slot = if sign > 0 { slot.saturating_add(a) } else { slot.saturating_sub(a) };
}

pub fn ceil_bps(x: u64, bps: u16) -> u64 {
    ((x as u128 * bps as u128).div_ceil(10_000)) as u64
}

pub fn add_cash(t: &mut Trader, d: i64) -> R {
    t.cash = ovf(t.cash.checked_add(d))?;
    Ok(())
}

/// The position netted with open limit orders must fit within max leverage of equity at `mark`.
pub fn check_initial_margin(t: &Trader, mark: i128, max_leverage_bps: u16) -> R {
    let eq = ovf(position::equity(&t.position(), mark))?;
    let need = t.margin_need();
    ensure(eq > 0 && need <= position::max_exposure(eq, max_leverage_bps) as u128, TtError::InsufficientMargin)
}
