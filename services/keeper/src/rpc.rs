use anyhow::{Context, Result};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_client::slot::{PerfSample, SAMPLE_LIMIT};
use tradingtime_client::{
    ix,
    rpc::{Ctx, Discovered},
};
use tradingtime_interface::{LimitOrder, Market, Operator, Trader};

use crate::chain::{Action, Chain, parse_pyth_publish_time};

pub fn to_instruction(keeper: &Pubkey, action: &Action) -> solana_instruction::Instruction {
    match action {
        Action::CreateTrader => ix::create_trader(keeper),
        Action::CrankIndex { pyth_feed } => ix::crank_index(pyth_feed),
        Action::CrankFunding => ix::crank_funding(),
        Action::CrankMark { operators } => ix::crank_mark(operators),
        Action::Liquidate { target, operators, adl } => ix::liquidate(keeper, target, operators, adl),
        Action::ExecuteLimitOrder { owner, order_id, operators } => {
            ix::execute_limit_order(keeper, owner, *order_id, operators)
        }
        Action::CancelExpiredOrder { owner, order_id } => ix::cancel_expired_order(keeper, owner, *order_id),
    }
}

/// Re-scan period for operator and order addresses (each scan is slow on a forked validator).
const DISCOVERY_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

pub struct RpcChain {
    pub ctx: Ctx,
    discovered: tokio::sync::OnceCell<Discovered>,
}

impl RpcChain {
    pub fn new(rpc_url: &str, payer: Keypair) -> Self {
        Self { ctx: Ctx::new(rpc_url, payer), discovered: tokio::sync::OnceCell::new() }
    }

    async fn discovered(&self) -> Result<&Discovered> {
        self.discovered.get_or_try_init(|| self.ctx.discover(DISCOVERY_EVERY)).await
    }
}

impl Chain for RpcChain {
    fn keeper(&self) -> Pubkey {
        self.ctx.payer.pubkey()
    }
    async fn market(&self) -> Result<Market> {
        self.ctx.market().await
    }
    async fn slot(&self) -> Result<u64> {
        Ok(self.ctx.client.get_slot().await?)
    }
    async fn pyth_publish_time(&self, feed: &Pubkey) -> Result<i64> {
        let data = self.ctx.client.get_account_data(feed).await.context("fetch pyth feed")?;
        parse_pyth_publish_time(&data).context("not a PriceUpdateV2 account")
    }
    async fn operators(&self) -> Result<Vec<(Pubkey, Operator)>> {
        self.ctx.operators_in(self.discovered().await?).await
    }
    async fn orders(&self) -> Result<Vec<(Pubkey, LimitOrder)>> {
        self.ctx.orders_in(self.discovered().await?).await
    }
    async fn trader(&self, owner: &Pubkey) -> Result<Option<Trader>> {
        self.ctx.trader(owner).await
    }
    async fn send(&self, action: &Action) -> Result<()> {
        let sig = self.ctx.send(&[to_instruction(&self.keeper(), action)], &[]).await?;
        tracing::debug!(%sig, "confirmed");
        Ok(())
    }
    async fn perf_samples(&self) -> Result<Vec<PerfSample>> {
        let samples = self.ctx.client.get_recent_performance_samples(Some(SAMPLE_LIMIT)).await?;
        Ok(samples
            .into_iter()
            .map(|s| PerfSample { num_slots: s.num_slots, period_secs: u64::from(s.sample_period_secs) })
            .collect())
    }
}

/// Solana CLI keypair file: JSON array of 64 bytes.
pub fn read_keypair(path: &str) -> Result<Keypair> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read keypair {path}"))?;
    let bytes: Vec<u8> = serde_json::from_str(&raw).with_context(|| format!("parse keypair {path}"))?;
    Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow::anyhow!("invalid keypair {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradingtime_client::{pda, program_id};

    #[test]
    fn actions_map_to_pinned_instructions() {
        let k = Pubkey::new_from_array([1; 32]);
        let o = Pubkey::new_from_array([2; 32]);
        let t = Pubkey::new_from_array([3; 32]);
        let a = Pubkey::new_from_array([4; 32]);

        let i = to_instruction(&k, &Action::CreateTrader);
        assert_eq!((i.program_id, i.data[0], i.accounts[1].pubkey), (program_id(), 3, pda::trader(&k).0));

        let i = to_instruction(&k, &Action::CrankFunding);
        assert_eq!(i.data, vec![15]);

        let i = to_instruction(&k, &Action::CrankMark { operators: vec![o] });
        assert_eq!((i.data[0], i.data[1], i.accounts.len()), (16, 1, 3));

        let i = to_instruction(&k, &Action::Liquidate { target: t, operators: vec![o], adl: vec![a] });
        assert_eq!((i.data[0], i.data[1], i.data[2]), (17, 1, 1));
        assert!(i.accounts[0].is_signer && i.accounts[0].pubkey == k);
        assert_eq!(i.accounts[1].pubkey, pda::trader(&t).0);
        assert_eq!(i.accounts.last().unwrap().pubkey, pda::trader(&a).0);

        let i = to_instruction(&k, &Action::ExecuteLimitOrder { owner: t, order_id: 9, operators: vec![o] });
        assert_eq!(i.data[0], 13);
        assert_eq!(i.accounts[1].pubkey, pda::trader(&k).0);
        assert_eq!(i.accounts[2].pubkey, pda::order(&t, 9).0);

        let i = to_instruction(&k, &Action::CancelExpiredOrder { owner: t, order_id: 9 });
        assert_eq!((i.data[0], i.accounts.len()), (12, 4));
        assert!(i.accounts[0].is_signer && i.accounts[0].pubkey == k);
        assert_eq!((i.accounts[2].pubkey, i.accounts[3].pubkey), (pda::order(&t, 9).0, t));

        let f = Pubkey::new_from_array([5; 32]);
        let i = to_instruction(&k, &Action::CrankIndex { pyth_feed: f });
        assert_eq!((i.data[0], i.accounts[1].pubkey), (14, f));
    }

    #[test]
    fn reads_cli_keypair_file() {
        let kp = Keypair::new();
        let path = std::env::temp_dir().join(format!("tt-keeper-kp-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_string(&kp.to_bytes().to_vec()).unwrap()).unwrap();
        let got = read_keypair(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(got.pubkey(), kp.pubkey());
        assert!(read_keypair("/nonexistent/kp.json").is_err());
    }
}
