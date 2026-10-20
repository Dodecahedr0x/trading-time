//! Zero-copy account decoders. `decode_*` borrow (buffer must be 8-byte aligned, as RPC
//! `Vec<u8>` data is in practice); `read_*` copy and accept any alignment.

use bytemuck::Pod;
use tradingtime_interface::{DISC_MARKET, DISC_OPERATOR, DISC_ORDER, DISC_TRADER, LimitOrder, Market, Operator, Trader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    WrongDiscriminator,
    Misaligned,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            DecodeError::TooShort => "account data too short",
            DecodeError::WrongDiscriminator => "wrong account discriminator",
            DecodeError::Misaligned => "account data misaligned",
        };
        f.write_str(s)
    }
}

impl std::error::Error for DecodeError {}

fn prefix<T: Pod>(data: &[u8], disc: u64) -> Result<&[u8], DecodeError> {
    let n = core::mem::size_of::<T>();
    if data.len() < n {
        return Err(DecodeError::TooShort);
    }
    if data[..8] != disc.to_le_bytes() {
        return Err(DecodeError::WrongDiscriminator);
    }
    Ok(&data[..n])
}

fn decode<T: Pod>(data: &[u8], disc: u64) -> Result<&T, DecodeError> {
    bytemuck::try_from_bytes(prefix::<T>(data, disc)?).map_err(|_| DecodeError::Misaligned)
}

fn read<T: Pod>(data: &[u8], disc: u64) -> Result<T, DecodeError> {
    Ok(bytemuck::pod_read_unaligned(prefix::<T>(data, disc)?))
}

pub fn decode_market(data: &[u8]) -> Result<&Market, DecodeError> {
    decode(data, DISC_MARKET)
}
pub fn decode_trader(data: &[u8]) -> Result<&Trader, DecodeError> {
    decode(data, DISC_TRADER)
}
pub fn decode_operator(data: &[u8]) -> Result<&Operator, DecodeError> {
    decode(data, DISC_OPERATOR)
}
pub fn decode_order(data: &[u8]) -> Result<&LimitOrder, DecodeError> {
    decode(data, DISC_ORDER)
}

pub fn read_market(data: &[u8]) -> Result<Market, DecodeError> {
    read(data, DISC_MARKET)
}
pub fn read_trader(data: &[u8]) -> Result<Trader, DecodeError> {
    read(data, DISC_TRADER)
}
pub fn read_operator(data: &[u8]) -> Result<Operator, DecodeError> {
    read(data, DISC_OPERATOR)
}
pub fn read_order(data: &[u8]) -> Result<LimitOrder, DecodeError> {
    read(data, DISC_ORDER)
}
