use pinocchio::{
    AccountView, ProgramResult,
    cpi::{Seed, Signer},
};
use pinocchio_token::instructions::InitializeAccount3;
use tradingtime_interface::{
    self as tti, AmountData, DISC_MARKET, InitializeMarketData, Market, PodI128, SetPausedData, TtError,
    UpdateParamsData,
    events::{self, CashEvent},
};
use tradingtime_math::{Fx, ONE, fx_mul, index, ln_fx};

use crate::{payload, util::*};

const TOKEN_ACCOUNT_LEN: u64 = 165;

pub fn initialize_market(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [admin, market_acc, vault, mint, pyth, system, token] = accounts::<7>(a)?;
    let d: InitializeMarketData = payload(data)?;
    signer(admin)?;
    writable(market_acc)?;
    writable(vault)?;
    d.params.validate().map_err(err)?;
    let p = d.params;
    ensure((p.index_min_secs..=p.index_max_secs).contains(&d.initial_index_secs), TtError::ParamOutOfBounds)?;
    expect_key(mint, &USDC_MINT, TtError::InvalidAccount)?;
    ensure(key_eq(pyth, &tti::PYTH_SOL_USD_FEED), TtError::InvalidAccount)?;
    expect_key(system, &SYSTEM_PROGRAM, TtError::InvalidAccount)?;
    expect_key(token, &TOKEN_PROGRAM, TtError::InvalidAccount)?;

    let (market_key, bump) = find_pda(&[tti::SEED_MARKET]);
    expect_key(market_acc, &market_key, TtError::InvalidPda)?;
    let (vault_key, vault_bump) = find_pda(&[tti::SEED_VAULT]);
    expect_key(vault, &vault_key, TtError::InvalidPda)?;
    ensure(vault.lamports() == 0, TtError::AlreadyInitialized)?;

    let b = [bump];
    let m = create_pda::<Market>(admin, market_acc, &[Seed::from(tti::SEED_MARKET), Seed::from(&b)])?;

    let vb = [vault_bump];
    let vseeds = [Seed::from(tti::SEED_VAULT), Seed::from(&vb)];
    pinocchio_system::instructions::CreateAccount::with_minimum_balance(admin, vault, TOKEN_ACCOUNT_LEN, &TOKEN_PROGRAM, None)?
        .invoke_signed(&[Signer::from(&vseeds)])?;
    InitializeAccount3::new(vault, mint, market_acc.address()).invoke()?;

    let (_, ts) = now()?;
    m.discriminator = DISC_MARKET;
    m.admin = admin.address().to_bytes();
    m.usdc_mint = tti::USDC_MINT;
    m.pyth_feed = tti::PYTH_SOL_USD_FEED;
    m.vault = vault_key.to_bytes();
    m.params = p;
    let var0 = ovf(initial_var(p.calib.get(), p.band_bps, d.initial_index_secs))?;
    let idx = ovf(index::index_ln(var0, p.calib.get(), p.band_bps, p.index_min_secs, p.index_max_secs))?;
    m.var_ewma = PodI128::new(var0);
    m.index_ln = PodI128::new(idx);
    m.index_updated_ts = ts;
    m.funding_updated_ts = ts;
    m.bump = bump;
    m.vault_bump = vault_bump;
    Ok(())
}

/// Inverse of `index::index_ln`: the variance whose index is `secs`.
fn initial_var(calib: Fx, band_bps: u16, secs: u32) -> Option<Fx> {
    let x = ln_fx(ONE + (band_bps as i128) * ONE / 10_000)?;
    fx_mul(fx_mul(calib, x)?, x)?.checked_div(secs as i128)
}

fn admin_market<'a>(admin: &AccountView, market_acc: &'a AccountView) -> R<&'a mut Market> {
    signer(admin)?;
    let m = market_mut(market_acc, true)?;
    ensure(key_eq(admin, &m.admin), TtError::Unauthorized)?;
    Ok(m)
}

pub fn update_params(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [admin, market_acc] = accounts::<2>(a)?;
    let d: UpdateParamsData = payload(data)?;
    let m = admin_market(admin, market_acc)?;
    d.params.validate().map_err(err)?;
    m.params = d.params;
    Ok(())
}

pub fn set_paused(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [admin, market_acc] = accounts::<2>(a)?;
    let d: SetPausedData = payload(data)?;
    let m = admin_market(admin, market_acc)?;
    m.paused = (d.paused != 0) as u8;
    Ok(())
}

pub fn withdraw_treasury(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let [admin, market_acc, admin_usdc, vault, mint, token] = accounts::<6>(a)?;
    let d: AmountData = payload(data)?;
    let m = admin_market(admin, market_acc)?;
    token_accounts(m, vault, mint, token)?;
    writable(admin_usdc)?;
    ensure(d.amount > 0 && d.amount <= m.treasury, TtError::InsufficientCollateral)?;
    m.treasury -= d.amount;
    let bump = m.bump;
    transfer_out(market_acc, bump, vault, mint, admin_usdc, d.amount)?;
    let (_, ts) = now()?;
    emit(
        events::TAG_CASH,
        &CashEvent {
            owner: admin.address().to_bytes(),
            kind: events::CASH_TREASURY_WITHDRAW,
            _pad: [0; 7],
            amount: d.amount as i64,
            ts,
        },
    );
    Ok(())
}
