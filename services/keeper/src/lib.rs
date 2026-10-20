pub mod chain;
pub mod decide;
pub mod indexer;
pub mod keeper;
pub mod rpc;

use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use crate::indexer::HttpIndexer;
use crate::keeper::Keeper;
use crate::rpc::{RpcChain, read_keypair};

#[derive(Clone, Debug, Parser)]
#[command(name = "tradingtime-keeper", about = "Permissionless TradingTime keeper")]
pub struct Config {
    #[arg(long, default_value = "http://127.0.0.1:8899")]
    pub rpc: String,
    /// Accepted for stack uniformity; the keeper polls RPC.
    #[arg(long, default_value = "ws://127.0.0.1:8900")]
    pub ws: String,
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    pub api: String,
    #[arg(long)]
    pub keypair: String,
    /// Fixed tick period; omit to tick at the slot time estimated from getRecentPerformanceSamples.
    #[arg(long)]
    pub interval_ms: Option<u64>,
}

/// Runs forever; returns only on a startup error.
pub async fn run(cfg: Config) -> Result<()> {
    let payer = read_keypair(&cfg.keypair)?;
    let keeper = Keeper::new(RpcChain::new(&cfg.rpc, payer), HttpIndexer::new(&cfg.api));
    tracing::info!(keeper = %chain::Chain::keeper(&keeper.chain), rpc = %cfg.rpc, api = %cfg.api, "keeper starting");
    loop {
        match keeper.ensure_trader().await {
            Ok(created) => {
                if created {
                    tracing::info!("created keeper trader account");
                }
                break;
            }
            Err(e) => tracing::warn!(error = %e, "ensure keeper trader; retrying"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let mut keeper = keeper;
    keeper.run_until(cfg.interval_ms, std::future::pending()).await;
    Ok(())
}
