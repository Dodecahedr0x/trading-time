//! Off-chain client for the TradingTime program: PDAs, instruction builders,
//! account/event decoders and display units. Feature `rpc` adds RPC and Surfpool helpers.

pub use tradingtime_interface as interface;
pub use tradingtime_math as math;

use solana_pubkey::Pubkey;
use tradingtime_math::Fx;

pub mod accounts;
pub mod events;
pub mod ix;
pub mod pda;
pub mod slot;
pub mod units;

#[cfg(feature = "rpc")]
pub mod rpc;
#[cfg(feature = "rpc")]
pub mod surfnet;

pub fn program_id() -> Pubkey {
    Pubkey::new_from_array(interface::PROGRAM_ID)
}

/// An operator's quoted curve, as sent by UpdateCurve.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CurveQuote {
    pub fair_ln: Fx,
    pub half_spread: Fx,
    pub impact_per_usdc: Fx,
    pub inventory_limit: u64,
}
