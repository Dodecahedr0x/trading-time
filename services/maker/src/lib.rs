pub mod chain;
pub mod feed;
pub mod forecast;
pub mod maker;
pub mod quote;
#[cfg(feature = "rpc")]
pub mod rpc;
pub mod slot;

use clap::Parser;
use feed::FeedMode;
use maker::LoopOpts;
use quote::QuoteParams;

#[derive(Parser, Clone, Debug)]
#[command(name = "tradingtime-maker", about = "TradingTime propAMM maker")]
pub struct Config {
    #[arg(long, default_value = "http://127.0.0.1:8899")]
    pub rpc: String,
    #[arg(long)]
    pub keypair: String,
    /// USDC deposited before registering the operator (whole USDC).
    #[arg(long, default_value_t = 0)]
    pub deposit_usdc: u64,
    #[arg(long, default_value_t = 2.0)]
    pub half_spread_bps: f64,
    #[arg(long = "impact-bps-per-10k", default_value_t = 10.0)]
    pub impact_bps_per_10k: f64,
    #[arg(long, default_value_t = 20_000)]
    pub inventory_limit_usdc: u64,
    /// binance | coinbase | auto | replay:<file>
    #[arg(long, default_value = "auto")]
    pub feed: FeedMode,
    /// Fixed quote period; omit to track the estimated slot time from getRecentPerformanceSamples.
    #[arg(long)]
    pub slot_ms: Option<u64>,
    #[arg(long, default_value_t = 5_000)]
    pub market_refresh_ms: i64,
    #[arg(long)]
    pub unbond_on_exit: bool,
}

impl Config {
    pub fn loop_opts(&self) -> LoopOpts {
        LoopOpts {
            feed: self.feed.clone(),
            quote: QuoteParams {
                half_spread_bps: self.half_spread_bps,
                impact_bps_per_10k: self.impact_bps_per_10k,
                inventory_limit_usdc: self.inventory_limit_usdc,
            },
            deposit_usdc: self.deposit_usdc,
            slot_ms: self.slot_ms,
            market_refresh_ms: self.market_refresh_ms,
            unbond_on_exit: self.unbond_on_exit,
        }
    }
}

/// Runs the maker against the RPC in `cfg` until Ctrl-C / SIGTERM.
#[cfg(feature = "rpc")]
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    let payer = solana_keypair::read_keypair_file(&cfg.keypair)
        .map_err(|e| anyhow::anyhow!("read keypair {}: {e}", cfg.keypair))?;
    let chain = rpc::RpcChain::new(&cfg.rpc, payer);
    tracing::info!("maker {} on {}", chain.owner(), cfg.rpc);
    maker::run_with(&chain, cfg.loop_opts(), stop_signal()).await
}

#[cfg(feature = "rpc")]
async fn stop_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
