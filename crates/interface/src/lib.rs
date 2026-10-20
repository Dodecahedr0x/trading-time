//! The TradingTime on-chain interface: addresses, account layouts, instruction data,
//! events and errors. Shared by the program and every off-chain component.
#![no_std]

use bytemuck::{Pod, Zeroable};
pub use tradingtime_math::{Fx, ONE, Side};

/// 7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h
pub const PROGRAM_ID: [u8; 32] = [102, 100, 218, 240, 203, 234, 153, 141, 62, 235, 66, 68, 140, 11, 240, 207, 126, 248, 81, 212, 249, 116, 36, 103, 171, 250, 129, 64, 166, 237, 137, 242];
/// EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
pub const USDC_MINT: [u8; 32] = [198, 250, 122, 243, 190, 219, 173, 58, 61, 101, 243, 106, 171, 201, 116, 49, 177, 187, 228, 194, 210, 246, 224, 228, 124, 166, 2, 3, 69, 47, 93, 97];
pub const USDC_DECIMALS: u8 = 6;
/// rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ
pub const PYTH_RECEIVER: [u8; 32] = [12, 183, 250, 187, 82, 247, 166, 72, 187, 91, 49, 125, 154, 1, 139, 144, 87, 203, 2, 71, 116, 250, 254, 1, 230, 196, 223, 152, 204, 56, 88, 129];
/// 7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE
pub const PYTH_SOL_USD_FEED: [u8; 32] = [96, 49, 71, 4, 52, 13, 237, 223, 55, 31, 212, 36, 114, 20, 143, 36, 142, 157, 26, 109, 26, 94, 178, 172, 58, 205, 139, 127, 213, 214, 178, 67];
pub const PYTH_SOL_USD_FEED_ID: [u8; 32] = [239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47, 142, 208, 198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109];
/// TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
pub const TOKEN_PROGRAM: [u8; 32] = [6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169];
/// 11111111111111111111111111111111
pub const SYSTEM_PROGRAM: [u8; 32] = [0; 32];

pub const SEED_MARKET: &[u8] = b"market";
pub const SEED_VAULT: &[u8] = b"vault";
pub const SEED_TRADER: &[u8] = b"trader";
pub const SEED_OPERATOR: &[u8] = b"operator";
pub const SEED_ORDER: &[u8] = b"order";

pub const MAX_OPERATORS_PER_IX: usize = 8;
pub const MAX_ADL_PER_IX: usize = 8;

/// Pyth PriceUpdateV2 (Borsh, Full verification) byte offsets.
pub mod pyth {
    pub const DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];
    pub const VERIFICATION_TAG: usize = 40;
    pub const VERIFICATION_FULL: u8 = 1;
    pub const FEED_ID: usize = 41;
    pub const PRICE: usize = 73;
    pub const CONF: usize = 81;
    pub const EXPONENT: usize = 89;
    pub const PUBLISH_TIME: usize = 93;
    pub const PREV_PUBLISH_TIME: usize = 101;
    pub const EMA_PRICE: usize = 109;
    pub const EMA_CONF: usize = 117;
    pub const POSTED_SLOT: usize = 125;
    pub const LEN: usize = 134;
}

/// An i128 stored as little-endian bytes so SBF and host layouts agree.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct PodI128(pub [u8; 16]);

impl PodI128 {
    pub const fn new(v: i128) -> Self {
        PodI128(v.to_le_bytes())
    }
    pub const fn get(&self) -> i128 {
        i128::from_le_bytes(self.0)
    }
    pub fn set(&mut self, v: i128) {
        self.0 = v.to_le_bytes();
    }
}

impl core::fmt::Debug for PodI128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

impl From<i128> for PodI128 {
    fn from(v: i128) -> Self {
        PodI128::new(v)
    }
}

pub const DISC_MARKET: u64 = 0x5454_4d41_524b_4554;
pub const DISC_TRADER: u64 = 0x5454_5452_4144_4552;
pub const DISC_OPERATOR: u64 = 0x5454_4f50_4552_4154;
pub const DISC_ORDER: u64 = 0x5454_4f52_4445_5231;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Params {
    pub taker_fee_bps: u16,
    pub liq_fee_bps: u16,
    pub maintenance_bps: u16,
    pub max_leverage_bps: u16,
    pub keeper_fee_bps: u16,
    pub band_bps: u16,
    pub _pad0: u32,
    pub index_half_life_secs: u32,
    pub funding_period_secs: u32,
    pub index_min_secs: u32,
    pub index_max_secs: u32,
    pub pyth_max_staleness_secs: u32,
    pub curve_max_age_slots: u32,
    pub mark_stale_secs: u32,
    pub unbond_delay_secs: u32,
    pub operator_bond: u64,
    pub operator_min_capital: u64,
    /// Fx calibration factor applied to the Brownian formula.
    pub calib: PodI128,
}

impl Params {
    pub const DEFAULT: Params = Params {
        taker_fee_bps: 5,
        liq_fee_bps: 100,
        maintenance_bps: 500,
        max_leverage_bps: 10_000,
        keeper_fee_bps: 1,
        band_bps: 100,
        _pad0: 0,
        index_half_life_secs: 604_800,
        funding_period_secs: 86_400,
        index_min_secs: 60,
        index_max_secs: 604_800,
        pyth_max_staleness_secs: 60,
        curve_max_age_slots: 10,
        mark_stale_secs: 60,
        unbond_delay_secs: 86_400,
        operator_bond: 1_000_000_000,
        operator_min_capital: 10_000_000_000,
        calib: PodI128::new(256_142_228_806_697),
    };

    /// Hard bounds enforced by InitializeMarket and UpdateParams.
    pub fn validate(&self) -> Result<(), TtError> {
        let calib = self.calib.get();
        let ok = self.taker_fee_bps <= 100
            && self.liq_fee_bps <= 500
            && (100..=2_000).contains(&self.maintenance_bps)
            && (1_000..=50_000).contains(&self.max_leverage_bps)
            && self.keeper_fee_bps <= 20
            && (10..=1_000).contains(&self.band_bps)
            && (3_600..=2_592_000).contains(&self.index_half_life_secs)
            && (3_600..=604_800).contains(&self.funding_period_secs)
            && self.index_min_secs >= 1
            && self.index_min_secs < self.index_max_secs
            && self.index_max_secs <= 31_536_000
            && (10..=600).contains(&self.pyth_max_staleness_secs)
            && (1..=150).contains(&self.curve_max_age_slots)
            && (5..=3_600).contains(&self.mark_stale_secs)
            && self.unbond_delay_secs <= 604_800
            && calib > ONE / 4
            && calib < ONE * 4;
        if ok { Ok(()) } else { Err(TtError::ParamOutOfBounds) }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Market {
    pub discriminator: u64,
    pub admin: [u8; 32],
    pub usdc_mint: [u8; 32],
    pub pyth_feed: [u8; 32],
    pub vault: [u8; 32],
    pub params: Params,
    /// Fx variance of log price per second.
    pub var_ewma: PodI128,
    pub last_pyth_price: i64,
    pub last_pyth_expo: i32,
    pub _pad0: u32,
    pub last_publish_time: i64,
    pub index_ln: PodI128,
    pub index_updated_ts: i64,
    pub mark_ln: PodI128,
    pub mark_updated_ts: i64,
    /// Fx cumulative funding per unit of exposure.
    pub funding_index: PodI128,
    pub funding_updated_ts: i64,
    pub long_q: u64,
    pub short_q: u64,
    pub insurance: u64,
    pub treasury: u64,
    pub total_cash: u64,
    pub next_order_seq: u64,
    pub operator_count: u32,
    pub paused: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub _pad1: u8,
}

impl Market {
    /// The mark to use at `now`: the curve-median mark while fresh, else the index.
    pub fn effective_mark(&self, now: i64) -> Fx {
        if self.mark_updated_ts > 0 && now - self.mark_updated_ts <= self.params.mark_stale_secs as i64 {
            self.mark_ln.get()
        } else {
            self.index_ln.get()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Trader {
    pub discriminator: u64,
    pub owner: [u8; 32],
    pub cash: i64,
    pub q: i64,
    pub cost: PodI128,
    pub funding_snapshot: PodI128,
    /// Remaining size of open long limit orders.
    pub reserved_long: u64,
    /// Remaining size of open short limit orders.
    pub reserved_short: u64,
    pub volume: u64,
    pub open_orders: u32,
    pub is_operator: u8,
    pub bump: u8,
    pub _pad: [u8; 2],
}

impl Trader {
    pub fn position(&self) -> tradingtime_math::position::Position {
        tradingtime_math::position::Position {
            cash: self.cash,
            q: self.q,
            cost: self.cost.get(),
            funding_snapshot: self.funding_snapshot.get(),
        }
    }

    pub fn set_position(&mut self, p: &tradingtime_math::position::Position) {
        self.cash = p.cash;
        self.q = p.q;
        self.cost.set(p.cost);
        self.funding_snapshot.set(p.funding_snapshot);
    }

    pub fn reserved_mut(&mut self, side: Side) -> &mut u64 {
        match side {
            Side::Long => &mut self.reserved_long,
            Side::Short => &mut self.reserved_short,
        }
    }

    /// Exposure that must fit within max leverage: the position with open limit orders netted against it.
    pub fn margin_need(&self) -> u128 {
        margin_need(self.q, self.reserved_long, self.reserved_short)
    }
}

/// max(|q + R_long|, |q − R_short|): the largest exposure the position can reach if either side's open orders fill.
pub fn margin_need(q: i64, reserved_long: u64, reserved_short: u64) -> u128 {
    let q = q as i128;
    let up = (q + reserved_long as i128).unsigned_abs();
    let down = (q - reserved_short as i128).unsigned_abs();
    up.max(down)
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorStatus {
    Active = 0,
    Unbonding = 1,
    Closed = 2,
}

impl OperatorStatus {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Active),
            1 => Some(Self::Unbonding),
            2 => Some(Self::Closed),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Operator {
    pub discriminator: u64,
    pub owner: [u8; 32],
    pub trader: [u8; 32],
    pub bond: u64,
    pub fair_ln: PodI128,
    pub half_spread: PodI128,
    /// Fx ln per whole USDC of size.
    pub impact_per_usdc: PodI128,
    pub inventory_limit: u64,
    pub updated_slot: u64,
    pub unbond_requested_ts: i64,
    pub status: u8,
    pub bump: u8,
    pub _pad: [u8; 6],
}

impl Operator {
    pub fn curve(&self, operator_q: i64) -> tradingtime_math::curve::Curve {
        tradingtime_math::curve::Curve {
            fair_ln: self.fair_ln.get(),
            half_spread: self.half_spread.get(),
            impact_per_usdc: self.impact_per_usdc.get(),
            inventory_limit: self.inventory_limit,
            q: operator_q,
        }
    }

    pub fn is_fresh(&self, slot: u64, max_age_slots: u32) -> bool {
        self.status == OperatorStatus::Active as u8 && slot.saturating_sub(self.updated_slot) <= max_age_slots as u64
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LimitOrder {
    pub discriminator: u64,
    pub owner: [u8; 32],
    pub order_id: u64,
    pub side: u8,
    pub bump: u8,
    pub _pad: [u8; 6],
    pub size: u64,
    pub filled: u64,
    pub limit_ln: PodI128,
    pub expiry_ts: i64,
    pub reserved: u64,
    pub created_slot: u64,
}

/// Instruction tags (first byte of instruction data).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ix {
    InitializeMarket = 0,
    UpdateParams = 1,
    SetPaused = 2,
    CreateTrader = 3,
    Deposit = 4,
    Withdraw = 5,
    RegisterOperator = 6,
    UpdateCurve = 7,
    RequestUnbond = 8,
    CompleteUnbond = 9,
    Swap = 10,
    PlaceLimitOrder = 11,
    CancelLimitOrder = 12,
    ExecuteLimitOrder = 13,
    CrankIndex = 14,
    CrankFunding = 15,
    CrankMark = 16,
    Liquidate = 17,
    WithdrawTreasury = 18,
}

impl Ix {
    pub fn from_u8(v: u8) -> Option<Ix> {
        if v <= 18 {
            // Safety: repr(u8) with contiguous discriminants 0..=18.
            Some(unsafe { core::mem::transmute::<u8, Ix>(v) })
        } else {
            None
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InitializeMarketData {
    pub params: Params,
    /// Starting index in seconds; must lie in [index_min_secs, index_max_secs].
    pub initial_index_secs: u32,
    pub _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct UpdateParamsData {
    pub params: Params,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SetPausedData {
    pub paused: u8,
}

/// Deposit, Withdraw, WithdrawTreasury.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct AmountData {
    pub amount: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct UpdateCurveData {
    pub fair_ln: PodI128,
    pub half_spread: PodI128,
    pub impact_per_usdc: PodI128,
    pub inventory_limit: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SwapData {
    pub side: u8,
    pub n_ops: u8,
    pub _pad: [u8; 6],
    pub size: u64,
    pub min_fill: u64,
    pub limit_ln: PodI128,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PlaceLimitOrderData {
    pub side: u8,
    pub _pad: [u8; 7],
    pub order_id: u64,
    pub size: u64,
    pub expiry_ts: i64,
    pub limit_ln: PodI128,
}

/// ExecuteLimitOrder, CrankMark.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CountData {
    pub n_ops: u8,
    pub _pad: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LiquidateData {
    pub n_ops: u8,
    pub n_adl: u8,
    pub _pad: [u8; 6],
}

/// Instruction data = tag byte followed by the Pod payload.
pub fn ix_data<T: Pod>(tag: Ix, payload: &T, out: &mut [u8]) -> usize {
    let bytes = bytemuck::bytes_of(payload);
    out[0] = tag as u8;
    out[1..1 + bytes.len()].copy_from_slice(bytes);
    1 + bytes.len()
}

/// Reads the payload that follows the tag byte (the buffer may be unaligned).
pub fn read_payload<T: Pod>(data: &[u8]) -> Option<T> {
    let n = core::mem::size_of::<T>();
    if data.len() < 1 + n {
        return None;
    }
    bytemuck::try_pod_read_unaligned(&data[1..1 + n]).ok()
}

pub mod events {
    use super::*;

    pub const TAG_TRADE: u8 = 1;
    pub const TAG_INDEX: u8 = 2;
    pub const TAG_FUNDING: u8 = 3;
    pub const TAG_LIQUIDATION: u8 = 4;
    pub const TAG_ORDER: u8 = 5;
    pub const TAG_OPERATOR: u8 = 6;
    pub const TAG_CASH: u8 = 7;

    pub const TRADE_SWAP: u8 = 0;
    pub const TRADE_LIMIT: u8 = 1;
    pub const TRADE_LIQUIDATION: u8 = 2;
    pub const TRADE_ADL: u8 = 3;

    pub const ORDER_PLACED: u8 = 0;
    pub const ORDER_CANCELLED: u8 = 1;
    pub const ORDER_FILLED: u8 = 2;
    pub const ORDER_PARTIAL: u8 = 3;

    pub const OPERATOR_REGISTERED: u8 = 0;
    pub const OPERATOR_UNBOND_REQUESTED: u8 = 1;
    pub const OPERATOR_CLOSED: u8 = 2;

    pub const CASH_DEPOSIT: u8 = 0;
    pub const CASH_WITHDRAW: u8 = 1;
    pub const CASH_FUNDING: u8 = 2;
    pub const CASH_TREASURY_WITHDRAW: u8 = 3;

    /// One per operator fill (or per ADL counterparty).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct TradeEvent {
        pub trader: [u8; 32],
        pub operator: [u8; 32],
        pub side: u8,
        pub kind: u8,
        pub _pad: [u8; 6],
        pub size: u64,
        /// Average ln price of this fill.
        pub ln_price: PodI128,
        pub fee: u64,
        pub slot: u64,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct IndexEvent {
        pub index_ln: PodI128,
        pub var_ewma: PodI128,
        pub pyth_price: i64,
        pub pyth_expo: i32,
        pub _pad: u32,
        pub publish_time: i64,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct FundingEvent {
        pub funding_index: PodI128,
        pub mark_ln: PodI128,
        pub index_ln: PodI128,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct LiquidationEvent {
        pub trader: [u8; 32],
        pub liquidator: [u8; 32],
        pub size: u64,
        pub bankruptcy_ln: PodI128,
        pub penalty: u64,
        pub insurance_used: u64,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct OrderEvent {
        pub owner: [u8; 32],
        pub order_id: u64,
        pub kind: u8,
        pub side: u8,
        pub _pad: [u8; 6],
        pub size: u64,
        pub filled: u64,
        pub limit_ln: PodI128,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct OperatorEvent {
        pub owner: [u8; 32],
        pub kind: u8,
        pub _pad: [u8; 7],
        pub bond: u64,
        pub ts: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    pub struct CashEvent {
        pub owner: [u8; 32],
        pub kind: u8,
        pub _pad: [u8; 7],
        pub amount: i64,
        pub ts: i64,
    }

    /// Writes `tag ++ bytes_of(event)` into `out`; returns the length.
    pub fn encode<T: Pod>(tag: u8, event: &T, out: &mut [u8]) -> usize {
        let b = bytemuck::bytes_of(event);
        out[0] = tag;
        out[1..1 + b.len()].copy_from_slice(b);
        1 + b.len()
    }
}

/// Program errors, surfaced as `ProgramError::Custom(code)`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtError {
    InvalidInstruction = 1,
    InvalidAccount = 2,
    InvalidOwner = 3,
    InvalidPda = 4,
    Unauthorized = 5,
    ParamOutOfBounds = 6,
    Paused = 7,
    MathOverflow = 8,
    InsufficientCollateral = 9,
    InsufficientMargin = 10,
    NotLiquidatable = 11,
    PythInvalid = 12,
    PythStale = 13,
    StaleOrSamePrice = 14,
    NoFreshCurves = 15,
    MinFillNotMet = 16,
    LimitNotReached = 17,
    OrderExpired = 18,
    OperatorNotActive = 19,
    OperatorActive = 20,
    UnbondNotReady = 21,
    TooManyAccounts = 22,
    AdlInvalidCounterparty = 23,
    AlreadyInitialized = 24,
    WrongSide = 25,
}

impl TtError {
    pub fn from_code(code: u32) -> Option<TtError> {
        if (1..=25).contains(&code) {
            // Safety: repr(u32) with contiguous discriminants 1..=25.
            Some(unsafe { core::mem::transmute::<u32, TtError>(code) })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::mem::size_of;

    #[test]
    fn layouts_are_stable() {
        assert_eq!(size_of::<Params>(), 80);
        assert_eq!(size_of::<Market>(), 384);
        assert_eq!(size_of::<Trader>(), 120);
        assert_eq!(size_of::<Operator>(), 160);
        assert_eq!(size_of::<LimitOrder>(), 112);
        assert_eq!(size_of::<UpdateCurveData>(), 56);
        assert_eq!(size_of::<SwapData>(), 40);
        assert_eq!(size_of::<PlaceLimitOrderData>(), 48);
        assert_eq!(size_of::<events::TradeEvent>(), 120);
        assert_eq!(core::mem::align_of::<Market>(), 8);
    }

    #[test]
    fn margin_need_nets_reducing_orders() {
        assert_eq!(margin_need(1_000, 0, 1_000), 1_000);
        assert_eq!(margin_need(1_000, 0, 1_500), 1_000);
        assert_eq!(margin_need(1_000, 0, 2_500), 1_500);
        assert_eq!(margin_need(1_000, 200, 0), 1_200);
        assert_eq!(margin_need(-300, 500, 100), 400);
        assert_eq!(margin_need(0, 7, 9), 9);
    }

    #[test]
    fn defaults_are_in_bounds_and_tags_round_trip() {
        assert_eq!(Params::DEFAULT.validate(), Ok(()));
        let mut p = Params::DEFAULT;
        p.max_leverage_bps = 60_000;
        assert_eq!(p.validate(), Err(TtError::ParamOutOfBounds));
        for t in 0..=18u8 {
            assert_eq!(Ix::from_u8(t).unwrap() as u8, t);
        }
        assert_eq!(Ix::from_u8(19), None);
        assert_eq!(TtError::from_code(25), Some(TtError::WrongSide));
        assert_eq!(TtError::from_code(0), None);
    }

    #[test]
    fn payload_round_trip_unaligned() {
        let d = SwapData { side: 1, n_ops: 3, _pad: [0; 6], size: 7, min_fill: 2, limit_ln: PodI128::new(-5) };
        let mut buf = [0u8; 64];
        let n = ix_data(Ix::Swap, &d, &mut buf);
        assert_eq!(n, 41);
        assert_eq!(buf[0], Ix::Swap as u8);
        assert_eq!(read_payload::<SwapData>(&buf[..n]), Some(d));
        assert_eq!(read_payload::<SwapData>(&buf[..10]), None);
    }
}
