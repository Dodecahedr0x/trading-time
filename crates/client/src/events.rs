use base64::Engine;
use bytemuck::Pod;
pub use tradingtime_interface::events::{
    CashEvent, FundingEvent, IndexEvent, LiquidationEvent, OperatorEvent, OrderEvent, TradeEvent,
};
use tradingtime_interface::events::{TAG_CASH, TAG_FUNDING, TAG_INDEX, TAG_LIQUIDATION, TAG_OPERATOR, TAG_ORDER, TAG_TRADE};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Trade(TradeEvent),
    Index(IndexEvent),
    Funding(FundingEvent),
    Liquidation(LiquidationEvent),
    Order(OrderEvent),
    Operator(OperatorEvent),
    Cash(CashEvent),
}

fn body<T: Pod>(rest: &[u8]) -> Option<T> {
    if rest.len() != core::mem::size_of::<T>() {
        return None;
    }
    Some(bytemuck::pod_read_unaligned(rest))
}

/// Decodes `tag ++ Pod struct` as emitted by `sol_log_data`.
pub fn decode_event(bytes: &[u8]) -> Option<Event> {
    let (&tag, rest) = bytes.split_first()?;
    Some(match tag {
        TAG_TRADE => Event::Trade(body(rest)?),
        TAG_INDEX => Event::Index(body(rest)?),
        TAG_FUNDING => Event::Funding(body(rest)?),
        TAG_LIQUIDATION => Event::Liquidation(body(rest)?),
        TAG_ORDER => Event::Order(body(rest)?),
        TAG_OPERATOR => Event::Operator(body(rest)?),
        TAG_CASH => Event::Cash(body(rest)?),
        _ => return None,
    })
}

/// Parses a `Program data: <b64> [<b64>...]` log line; only the first chunk is the event.
pub fn decode_log_line(line: &str) -> Option<Event> {
    let rest = line.trim().strip_prefix("Program data:")?;
    let chunk = rest.split_whitespace().next()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(chunk).ok()?;
    decode_event(&bytes)
}
