use clap::Parser;
use tradingtime_bots::runner::{Config, run};

/// Simulated traders (and a Pyth price driver) for a local TradingTime stack.
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8899")]
    rpc: String,
    /// Bot keypairs are created here (bot<i>.json) and reused on restart.
    #[arg(long)]
    keys_dir: std::path::PathBuf,
    #[arg(long, default_value_t = 8)]
    traders: usize,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long, default_value_t = 2_000)]
    tick_ms: u64,
    #[arg(long, default_value_t = 20_000)]
    min_wallet_usdc: u64,
    #[arg(long, default_value_t = 200_000)]
    max_wallet_usdc: u64,
    /// `ts_ms,price` CSV looped as the Pyth SOL/USD price; omit to leave Pyth alone.
    #[arg(long)]
    pyth_replay: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 10)]
    pyth_every_secs: u64,
    /// Log-return multiplier during volatile regimes.
    #[arg(long, default_value_t = 4.0)]
    storm_mult: f64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let a = Args::parse();
    run(Config {
        rpc: a.rpc,
        keys_dir: a.keys_dir,
        traders: a.traders,
        seed: a.seed,
        tick_ms: a.tick_ms,
        wallet_usdc: (a.min_wallet_usdc, a.max_wallet_usdc.max(a.min_wallet_usdc)),
        pyth_replay: a.pyth_replay,
        pyth_every_secs: a.pyth_every_secs,
        storm_mult: a.storm_mult,
    })
    .await
}
