//! Shared harness for the Surfpool integration tests and the dev-stack bootstrap.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use std::path::{Path, PathBuf};
use tradingtime_client::rpc::Ctx;

/// `target/dev-stack/env.json`, written by the bootstrap binary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Env {
    pub rpc_url: String,
    pub ws_url: String,
    pub api_url: String,
    pub program_id: String,
    pub market: String,
    pub usdc_mint: String,
    pub admin_keypair: String,
    pub keeper_keypair: String,
    pub trader_keypairs: Vec<String>,
    pub maker_keypairs: Vec<String>,
    pub e2e_keypair: String,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

pub fn env_path() -> PathBuf {
    repo_root().join("target/dev-stack/env.json")
}

pub fn load_env() -> Result<Env> {
    let p = env_path();
    let text = std::fs::read_to_string(&p).with_context(|| format!("{} (run scripts/dev-stack.sh up)", p.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// Reads a Solana CLI keypair file (JSON array of 64 bytes).
pub fn read_keypair(path: &str) -> Result<Keypair> {
    let bytes: Vec<u8> = serde_json::from_str(&std::fs::read_to_string(path).with_context(|| path.to_string())?)?;
    Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow::anyhow!("{path}: {e}"))
}

pub fn ctx(env: &Env, keypair_path: &str) -> Result<Ctx> {
    Ok(Ctx::new(env.rpc_url.clone(), read_keypair(keypair_path)?))
}

/// Chain unix timestamp from the Clock sysvar.
pub async fn chain_now(ctx: &Ctx) -> Result<i64> {
    let clock = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");
    let data = ctx.client.get_account_data(&clock).await?;
    Ok(i64::from_le_bytes(data[32..40].try_into()?))
}

pub async fn chain_slot(ctx: &Ctx) -> Result<u64> {
    Ok(ctx.client.get_slot().await?)
}

/// Polls `f` until it returns Some, or fails after `secs`.
pub async fn wait_for<T, F, Fut>(secs: u64, what: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>>>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if let Ok(Some(v)) = f().await {
            return Ok(v);
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("timed out after {secs}s waiting for {what}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_round_trips() {
        let e = Env {
            rpc_url: "http://127.0.0.1:18899".into(),
            ws_url: "ws://127.0.0.1:18900".into(),
            api_url: "http://127.0.0.1:18787".into(),
            program_id: "p".into(),
            market: "m".into(),
            usdc_mint: "u".into(),
            admin_keypair: "a".into(),
            keeper_keypair: "k".into(),
            trader_keypairs: vec!["t1".into()],
            maker_keypairs: vec!["m1".into()],
            e2e_keypair: "e".into(),
        };
        let back: Env = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.trader_keypairs, e.trader_keypairs);
    }
}
