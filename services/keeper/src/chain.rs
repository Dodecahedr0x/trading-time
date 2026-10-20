use anyhow::Result;
use solana_pubkey::Pubkey;
use tradingtime_client::slot::PerfSample;
use tradingtime_interface::{LimitOrder, Market, Operator, Trader};

/// Keeper transactions; the RPC impl maps each to `tradingtime_client::ix` builders.
/// Operator and ADL lists hold owners, as the ix builders expect.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    CreateTrader,
    CrankIndex { pyth_feed: Pubkey },
    CrankFunding,
    CrankMark { operators: Vec<Pubkey> },
    Liquidate { target: Pubkey, operators: Vec<Pubkey>, adl: Vec<Pubkey> },
    ExecuteLimitOrder { owner: Pubkey, order_id: u64, operators: Vec<Pubkey> },
    /// Closes another owner's expired order; rent goes back to the owner.
    CancelExpiredOrder { owner: Pubkey, order_id: u64 },
}

#[allow(async_fn_in_trait)]
pub trait Chain {
    fn keeper(&self) -> Pubkey;
    async fn market(&self) -> Result<Market>;
    async fn slot(&self) -> Result<u64>;
    async fn pyth_publish_time(&self, feed: &Pubkey) -> Result<i64>;
    /// (operator PDA, account)
    async fn operators(&self) -> Result<Vec<(Pubkey, Operator)>>;
    /// (order PDA, account)
    async fn orders(&self) -> Result<Vec<(Pubkey, LimitOrder)>>;
    async fn trader(&self, owner: &Pubkey) -> Result<Option<Trader>>;
    async fn send(&self, action: &Action) -> Result<()>;
    /// `getRecentPerformanceSamples`, newest first.
    async fn perf_samples(&self) -> Result<Vec<PerfSample>>;
}

/// PriceUpdateV2 publish_time, validated against the pinned layout.
pub fn parse_pyth_publish_time(data: &[u8]) -> Option<i64> {
    use tradingtime_interface::pyth;
    if data.len() < pyth::PUBLISH_TIME + 8 || data[..8] != pyth::DISCRIMINATOR {
        return None;
    }
    Some(i64::from_le_bytes(data[pyth::PUBLISH_TIME..pyth::PUBLISH_TIME + 8].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradingtime_interface::pyth;

    #[test]
    fn pyth_publish_time_layout() {
        let mut d = vec![0u8; pyth::LEN];
        d[..8].copy_from_slice(&pyth::DISCRIMINATOR);
        d[pyth::PUBLISH_TIME..pyth::PUBLISH_TIME + 8].copy_from_slice(&1_700_000_123i64.to_le_bytes());
        assert_eq!(parse_pyth_publish_time(&d), Some(1_700_000_123));
        assert_eq!(parse_pyth_publish_time(&d[..50]), None);
        d[0] ^= 1;
        assert_eq!(parse_pyth_publish_time(&d), None);
    }
}
