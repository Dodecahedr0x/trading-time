#![cfg_attr(target_os = "solana", no_std)]

pub mod admin;
pub mod crank;
pub mod liquidate;
pub mod operator;
pub mod trade;
pub mod trader;
pub mod util;

pub use tradingtime_interface as interface;

use pinocchio::{AccountView, Address, ProgramResult, error::ProgramError};
use tradingtime_interface::{Ix, TtError};

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entry {
    pinocchio::program_entrypoint!(crate::process_instruction);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}

pub fn process_instruction(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    if program_id != &util::ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    let tag = data.first().and_then(|t| Ix::from_u8(*t)).ok_or(util::err(TtError::InvalidInstruction))?;
    let a: &[AccountView] = accounts;
    match tag {
        Ix::InitializeMarket => admin::initialize_market(a, data),
        Ix::UpdateParams => admin::update_params(a, data),
        Ix::SetPaused => admin::set_paused(a, data),
        Ix::CreateTrader => trader::create_trader(a),
        Ix::Deposit => trader::deposit(a, data),
        Ix::Withdraw => trader::withdraw(a, data),
        Ix::RegisterOperator => operator::register(a),
        Ix::UpdateCurve => operator::update_curve(a, data),
        Ix::RequestUnbond => operator::request_unbond(a),
        Ix::CompleteUnbond => operator::complete_unbond(a),
        Ix::Swap => trade::swap(a, data),
        Ix::PlaceLimitOrder => trade::place_limit_order(a, data),
        Ix::CancelLimitOrder => trade::cancel_limit_order(a),
        Ix::ExecuteLimitOrder => trade::execute_limit_order(a, data),
        Ix::CrankIndex => crank::crank_index(a),
        Ix::CrankFunding => crank::crank_funding(a),
        Ix::CrankMark => crank::crank_mark(a, data),
        Ix::Liquidate => liquidate::liquidate(a, data),
        Ix::WithdrawTreasury => admin::withdraw_treasury(a, data),
    }
}

pub(crate) fn payload<T: bytemuck::Pod>(data: &[u8]) -> util::R<T> {
    tradingtime_interface::read_payload(data).ok_or(util::err(TtError::InvalidInstruction))
}
