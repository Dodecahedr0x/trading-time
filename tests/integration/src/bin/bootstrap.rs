//! Bootstraps a TradingTime market on a running surfnet and writes target/dev-stack/env.json.
//!
//! bootstrap --keys <dir> [--rpc URL] [--ws URL] [--api URL]

use anyhow::{Context, Result};
use clap::Parser;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_client::{ix, pda, program_id, surfnet};
use tradingtime_interface::{PYTH_SOL_USD_FEED, Params, USDC_MINT};
use tradingtime_tests::{Env, chain_now, env_path, read_keypair};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    keys: String,
    #[arg(long, default_value = "http://127.0.0.1:8899")]
    rpc: String,
    #[arg(long, default_value = "ws://127.0.0.1:8900")]
    ws: String,
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    api: String,
}

const SOL: u64 = 1_000_000_000;
const USDC: u64 = 1_000_000;
const PRICE_EXPO: i32 = -8;
const SOL_PRICE: i64 = 18_000_000_000; // $180.00
const INITIAL_INDEX_SECS: u32 = 5_400;

#[tokio::main]
async fn main() -> Result<()> {
    let a = Args::parse();
    let key = |n: &str| format!("{}/{n}.json", a.keys);
    let names_traders = ["trader1", "trader2"];
    let names_makers = ["maker1", "maker2"];
    let all = ["admin", "keeper", "trader1", "trader2", "maker1", "maker2", "e2e"];

    let mint = Pubkey::new_from_array(USDC_MINT);
    let feed = Pubkey::new_from_array(PYTH_SOL_USD_FEED);
    let system = Pubkey::default();
    for n in all {
        let pk = read_keypair(&key(n))?.pubkey();
        surfnet::set_account(&a.rpc, &pk, 1_000 * SOL, &[], &system).await.with_context(|| format!("fund SOL {n}"))?;
        let usdc = if n == "admin" || n == "keeper" { 1_000 } else { 200_000 };
        surfnet::set_token_account(&a.rpc, &pk, &mint, usdc * USDC).await.with_context(|| format!("fund USDC {n}"))?;
    }

    let admin = tradingtime_client::rpc::Ctx::new(a.rpc.clone(), read_keypair(&key("admin"))?);
    let now = chain_now(&admin).await?;
    surfnet::write_pyth_price(&a.rpc, &feed, SOL_PRICE, PRICE_EXPO, now - 30).await?;

    let params = Params::DEFAULT;
    if admin.client.get_account(&pda::market().0).await.is_err() {
        let init = ix::initialize_market(&admin.payer.pubkey(), &mint, &feed, params, INITIAL_INDEX_SECS);
        admin.send(&[init], &[]).await.context("initialize_market")?;
    }
    admin.send(&[ix::crank_index(&feed)], &[]).await.context("crank_index (record)")?;
    let market = admin.market().await?;
    let index_secs = tradingtime_math::to_f64(market.index_ln.get()).exp();
    eprintln!("market initialized; index {index_secs:.0}s; program {}", program_id());

    let env = Env {
        rpc_url: a.rpc.clone(),
        ws_url: a.ws.clone(),
        api_url: a.api.clone(),
        program_id: program_id().to_string(),
        market: pda::market().0.to_string(),
        usdc_mint: mint.to_string(),
        admin_keypair: key("admin"),
        keeper_keypair: key("keeper"),
        trader_keypairs: names_traders.iter().map(|n| key(n)).collect(),
        maker_keypairs: names_makers.iter().map(|n| key(n)).collect(),
        e2e_keypair: key("e2e"),
    };
    std::fs::write(env_path(), serde_json::to_string_pretty(&env)?)?;
    println!("{}", env_path().display());
    Ok(())
}
