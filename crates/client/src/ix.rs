//! Instruction builders; account order follows the pinned Wire table.

use bytemuck::Pod;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use tradingtime_interface::{
    self as tti, AmountData, CountData, InitializeMarketData, Ix, LiquidateData, Params, PlaceLimitOrderData,
    PodI128, SetPausedData, Side, SwapData, UpdateCurveData,
};
use tradingtime_math::Fx;

use crate::{CurveQuote, pda, program_id};

fn w(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn ws(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, true)
}
fn r(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn rs(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, true)
}

fn market() -> Pubkey {
    pda::market().0
}
fn vault() -> Pubkey {
    pda::vault().0
}
fn trader(owner: &Pubkey) -> Pubkey {
    pda::trader(owner).0
}
fn operator(owner: &Pubkey) -> Pubkey {
    pda::operator(owner).0
}
fn usdc_mint() -> Pubkey {
    Pubkey::new_from_array(tti::USDC_MINT)
}
fn token_program() -> Pubkey {
    Pubkey::new_from_array(tti::TOKEN_PROGRAM)
}
fn system_program() -> Pubkey {
    Pubkey::new_from_array(tti::SYSTEM_PROGRAM)
}

fn data<T: Pod>(tag: Ix, payload: &T) -> Vec<u8> {
    let mut out = vec![0u8; 1 + core::mem::size_of::<T>()];
    tti::ix_data(tag, payload, &mut out);
    out
}

fn build(accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction { program_id: program_id(), accounts, data }
}

/// [operator(w), operator_trader(w)] for each operator owner.
fn op_pairs(operators: &[Pubkey]) -> impl Iterator<Item = AccountMeta> + '_ {
    operators.iter().flat_map(|o| [w(operator(o)), w(trader(o))])
}

pub fn initialize_market(
    admin: &Pubkey,
    usdc_mint: &Pubkey,
    pyth_feed: &Pubkey,
    params: Params,
    initial_index_secs: u32,
) -> Instruction {
    build(
        vec![ws(*admin), w(market()), w(vault()), r(*usdc_mint), r(*pyth_feed), r(system_program()), r(token_program())],
        data(Ix::InitializeMarket, &InitializeMarketData { params, initial_index_secs, _pad0: 0 }),
    )
}

pub fn update_params(admin: &Pubkey, params: Params) -> Instruction {
    build(vec![rs(*admin), w(market())], data(Ix::UpdateParams, &tti::UpdateParamsData { params }))
}

pub fn set_paused(admin: &Pubkey, paused: bool) -> Instruction {
    build(vec![rs(*admin), w(market())], data(Ix::SetPaused, &SetPausedData { paused: paused as u8 }))
}

pub fn create_trader(owner: &Pubkey) -> Instruction {
    build(vec![ws(*owner), w(trader(owner)), r(market()), r(system_program())], vec![Ix::CreateTrader as u8])
}

fn cash_accounts(owner: &Pubkey, owner_usdc: &Pubkey) -> Vec<AccountMeta> {
    vec![rs(*owner), w(trader(owner)), w(market()), w(*owner_usdc), w(vault()), r(usdc_mint()), r(token_program())]
}

pub fn deposit(owner: &Pubkey, owner_usdc: &Pubkey, amount: u64) -> Instruction {
    build(cash_accounts(owner, owner_usdc), data(Ix::Deposit, &AmountData { amount }))
}

/// `is_operator` must mirror `Trader.is_operator`; it appends the operator account.
pub fn withdraw(owner: &Pubkey, owner_usdc: &Pubkey, amount: u64, is_operator: bool) -> Instruction {
    let mut accounts = cash_accounts(owner, owner_usdc);
    if is_operator {
        accounts.push(r(operator(owner)));
    }
    build(accounts, data(Ix::Withdraw, &AmountData { amount }))
}

pub fn register_operator(owner: &Pubkey, owner_usdc: &Pubkey) -> Instruction {
    build(
        vec![
            ws(*owner),
            w(trader(owner)),
            w(operator(owner)),
            w(market()),
            w(*owner_usdc),
            w(vault()),
            r(usdc_mint()),
            r(token_program()),
            r(system_program()),
        ],
        vec![Ix::RegisterOperator as u8],
    )
}

pub fn update_curve(owner: &Pubkey, quote: CurveQuote) -> Instruction {
    build(
        vec![rs(*owner), w(operator(owner))],
        data(
            Ix::UpdateCurve,
            &UpdateCurveData {
                fair_ln: PodI128::new(quote.fair_ln),
                half_spread: PodI128::new(quote.half_spread),
                impact_per_usdc: PodI128::new(quote.impact_per_usdc),
                inventory_limit: quote.inventory_limit,
            },
        ),
    )
}

pub fn request_unbond(owner: &Pubkey) -> Instruction {
    build(vec![rs(*owner), w(operator(owner))], vec![Ix::RequestUnbond as u8])
}

pub fn complete_unbond(owner: &Pubkey, owner_usdc: &Pubkey) -> Instruction {
    build(
        vec![
            rs(*owner),
            w(operator(owner)),
            w(trader(owner)),
            w(market()),
            w(*owner_usdc),
            w(vault()),
            r(usdc_mint()),
            r(token_program()),
        ],
        vec![Ix::CompleteUnbond as u8],
    )
}

/// `operators` are operator owner keys; at most `MAX_OPERATORS_PER_IX`.
pub fn swap(owner: &Pubkey, side: Side, size: u64, min_fill: u64, limit_ln: Fx, operators: &[Pubkey]) -> Instruction {
    let mut accounts = vec![rs(*owner), w(trader(owner)), w(market())];
    accounts.extend(op_pairs(operators));
    build(
        accounts,
        data(
            Ix::Swap,
            &SwapData {
                side: side as u8,
                n_ops: operators.len() as u8,
                _pad: [0; 6],
                size,
                min_fill,
                limit_ln: PodI128::new(limit_ln),
            },
        ),
    )
}

pub fn place_limit_order(
    owner: &Pubkey,
    order_id: u64,
    side: Side,
    size: u64,
    limit_ln: Fx,
    expiry_ts: i64,
) -> Instruction {
    build(
        vec![ws(*owner), w(trader(owner)), w(pda::order(owner, order_id).0), w(market()), r(system_program())],
        data(
            Ix::PlaceLimitOrder,
            &PlaceLimitOrderData {
                side: side as u8,
                _pad: [0; 7],
                order_id,
                size,
                expiry_ts,
                limit_ln: PodI128::new(limit_ln),
            },
        ),
    )
}

pub fn cancel_limit_order(owner: &Pubkey, order_id: u64) -> Instruction {
    build(
        vec![ws(*owner), w(trader(owner)), w(pda::order(owner, order_id).0)],
        vec![Ix::CancelLimitOrder as u8],
    )
}

/// Closes another owner's expired order; its rent goes to `order_owner`.
pub fn cancel_expired_order(signer: &Pubkey, order_owner: &Pubkey, order_id: u64) -> Instruction {
    build(
        vec![rs(*signer), w(trader(order_owner)), w(pda::order(order_owner, order_id).0), w(*order_owner)],
        vec![Ix::CancelLimitOrder as u8],
    )
}

pub fn execute_limit_order(keeper: &Pubkey, order_owner: &Pubkey, order_id: u64, operators: &[Pubkey]) -> Instruction {
    let mut accounts = vec![
        rs(*keeper),
        w(trader(keeper)),
        w(pda::order(order_owner, order_id).0),
        w(*order_owner),
        w(trader(order_owner)),
        w(market()),
    ];
    accounts.extend(op_pairs(operators));
    build(accounts, data(Ix::ExecuteLimitOrder, &CountData { n_ops: operators.len() as u8, _pad: [0; 7] }))
}

pub fn crank_index(pyth_feed: &Pubkey) -> Instruction {
    build(vec![w(market()), r(*pyth_feed)], vec![Ix::CrankIndex as u8])
}

pub fn crank_funding() -> Instruction {
    build(vec![w(market())], vec![Ix::CrankFunding as u8])
}

pub fn crank_mark(operators: &[Pubkey]) -> Instruction {
    let mut accounts = vec![w(market())];
    accounts.extend(op_pairs(operators));
    build(accounts, data(Ix::CrankMark, &CountData { n_ops: operators.len() as u8, _pad: [0; 7] }))
}

/// `adl_owners` are owner keys of opposite-side in-profit traders; at most `MAX_ADL_PER_IX`.
pub fn liquidate(liquidator: &Pubkey, target_owner: &Pubkey, operators: &[Pubkey], adl_owners: &[Pubkey]) -> Instruction {
    let mut accounts = vec![rs(*liquidator), w(trader(target_owner)), w(market())];
    accounts.extend(op_pairs(operators));
    accounts.extend(adl_owners.iter().map(|o| w(trader(o))));
    build(
        accounts,
        data(
            Ix::Liquidate,
            &LiquidateData { n_ops: operators.len() as u8, n_adl: adl_owners.len() as u8, _pad: [0; 6] },
        ),
    )
}

pub fn withdraw_treasury(admin: &Pubkey, admin_usdc: &Pubkey, amount: u64) -> Instruction {
    build(
        vec![rs(*admin), w(market()), w(*admin_usdc), w(vault()), r(usdc_mint()), r(token_program())],
        data(Ix::WithdrawTreasury, &AmountData { amount }),
    )
}

pub const COMPUTE_BUDGET_PROGRAM: Pubkey = Pubkey::from_str_const("ComputeBudget111111111111111111111111111111");
/// Compute units requested for transactions that touch several operator curves (A97).
pub const DEFAULT_COMPUTE_UNITS: u32 = 1_400_000;

/// ComputeBudget SetComputeUnitLimit: data = [2] ++ units (u32 LE).
pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction { program_id: COMPUTE_BUDGET_PROGRAM, accounts: vec![], data }
}
