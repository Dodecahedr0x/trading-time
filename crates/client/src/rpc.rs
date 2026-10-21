use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
pub use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig, UiAccountEncoding};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_commitment_config::CommitmentConfig;
use solana_instruction::Instruction;
pub use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
pub use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::Transaction;
use tradingtime_interface::{DISC_OPERATOR, DISC_ORDER, LimitOrder, Market, Operator, Trader};

use crate::{accounts, pda, program_id};

pub struct Ctx {
    pub client: RpcClient,
    pub payer: Keypair,
}

/// Operator and order addresses found by a background `getProgramAccounts` scan, read back by address with
/// `getMultipleAccounts`. A forked validator (Surfpool) takes 10–20 s per scan, which would otherwise stall
/// every caller that polls operators or orders each slot.
#[derive(Clone, Default)]
pub struct Discovered {
    operators: Arc<Mutex<Vec<Pubkey>>>,
    orders: Arc<Mutex<Vec<Pubkey>>>,
}

async fn scan_keys(client: &RpcClient, disc: u64) -> Result<Vec<Pubkey>> {
    Ok(program_accounts(client, disc).await?.into_iter().map(|(k, _)| k).collect())
}

async fn scan(client: &RpcClient, d: &Discovered) -> Result<()> {
    let ops = scan_keys(client, DISC_OPERATOR).await?;
    let orders = scan_keys(client, DISC_ORDER).await?;
    *d.operators.lock().map_err(|_| anyhow::anyhow!("discovery poisoned"))? = ops;
    *d.orders.lock().map_err(|_| anyhow::anyhow!("discovery poisoned"))? = orders;
    Ok(())
}

async fn program_accounts(client: &RpcClient, disc: u64) -> Result<Vec<(Pubkey, Vec<u8>)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, disc.to_le_bytes().to_vec()))]),
        account_config: RpcAccountInfoConfig { encoding: Some(UiAccountEncoding::Base64), ..RpcAccountInfoConfig::default() },
        ..RpcProgramAccountsConfig::default()
    };
    let raw = client.get_program_ui_accounts_with_config(&program_id(), config).await?;
    Ok(raw.into_iter().filter_map(|(k, a)| a.data.decode().map(|d| (k, d))).collect())
}

impl Ctx {
    /// Confirmed commitment.
    pub fn new(url: impl Into<String>, payer: Keypair) -> Self {
        Ctx { client: RpcClient::new_with_commitment(url.into(), CommitmentConfig::confirmed()), payer }
    }

    /// Legacy transaction paid and signed by `payer`, co-signed by `extra_signers`.
    pub async fn send(&self, ixs: &[Instruction], extra_signers: &[&Keypair]) -> Result<Signature> {
        let blockhash = self.client.get_latest_blockhash().await.context("get_latest_blockhash")?;
        let payer = self.payer.pubkey();
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend(extra_signers.iter().copied().filter(|k| k.pubkey() != payer));
        // Multi-operator swaps exceed the default 200k compute units (A97).
        let mut all = Vec::with_capacity(ixs.len() + 1);
        if !ixs.iter().any(|ix| ix.program_id == crate::ix::COMPUTE_BUDGET_PROGRAM) {
            all.push(crate::ix::set_compute_unit_limit(crate::ix::DEFAULT_COMPUTE_UNITS));
        }
        all.extend_from_slice(ixs);
        let tx = Transaction::new_signed_with_payer(&all, Some(&payer), &signers, blockhash);
        Ok(self.client.send_and_confirm_transaction(&tx).await?)
    }

    /// Like `send` but returns once the node accepts the transaction (preflight passes), without waiting for
    /// confirmation. For high-frequency updates such as curve quotes.
    pub async fn send_unconfirmed(&self, ixs: &[Instruction]) -> Result<Signature> {
        let blockhash = self.client.get_latest_blockhash().await.context("get_latest_blockhash")?;
        let payer = self.payer.pubkey();
        let tx = Transaction::new_signed_with_payer(ixs, Some(&payer), &[&self.payer], blockhash);
        Ok(self.client.send_transaction(&tx).await?)
    }

    pub async fn market(&self) -> Result<Market> {
        let data = self.client.get_account_data(&pda::market().0).await.context("fetch market")?;
        Ok(accounts::read_market(&data)?)
    }

    pub async fn trader(&self, owner: &Pubkey) -> Result<Option<Trader>> {
        let acc = self
            .client
            .get_account_with_commitment(&pda::trader(owner).0, self.client.commitment())
            .await?
            .value;
        Ok(match acc {
            Some(a) => Some(accounts::read_trader(&a.data)?),
            None => None,
        })
    }

    /// Scans once now, then re-scans every `every` in the background; read with `operators_in` / `orders_in`.
    pub async fn discover(&self, every: std::time::Duration) -> Result<Discovered> {
        let d = Discovered::default();
        scan(&self.client, &d).await?;
        let (url, commitment, bg) = (self.client.url(), self.client.commitment(), d.clone());
        tokio::spawn(async move {
            let client = RpcClient::new_with_commitment(url, commitment);
            loop {
                tokio::time::sleep(every).await;
                // A failed scan keeps the previous addresses.
                let _ = scan(&client, &bg).await;
            }
        });
        Ok(d)
    }

    pub async fn operators_in(&self, d: &Discovered) -> Result<Vec<(Pubkey, Operator)>> {
        let keys = d.operators.lock().map_err(|_| anyhow::anyhow!("discovery poisoned"))?.clone();
        self.read_many(&keys, accounts::read_operator).await
    }

    pub async fn orders_in(&self, d: &Discovered) -> Result<Vec<(Pubkey, LimitOrder)>> {
        let keys = d.orders.lock().map_err(|_| anyhow::anyhow!("discovery poisoned"))?.clone();
        self.read_many(&keys, accounts::read_order).await
    }

    /// Accounts by address; closed or undecodable ones are skipped.
    async fn read_many<T>(&self, keys: &[Pubkey], read: fn(&[u8]) -> Result<T, accounts::DecodeError>) -> Result<Vec<(Pubkey, T)>> {
        let mut out = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(100) {
            let accs = self.client.get_multiple_accounts(chunk).await?;
            for (k, a) in chunk.iter().zip(accs) {
                if let Some(v) = a.and_then(|a| read(&a.data).ok()) {
                    out.push((*k, v));
                }
            }
        }
        Ok(out)
    }

    pub async fn operators(&self) -> Result<Vec<(Pubkey, Operator)>> {
        self.by_discriminator(DISC_OPERATOR, accounts::read_operator).await
    }

    pub async fn orders(&self) -> Result<Vec<(Pubkey, LimitOrder)>> {
        self.by_discriminator(DISC_ORDER, accounts::read_order).await
    }

    async fn by_discriminator<T>(
        &self,
        disc: u64,
        read: fn(&[u8]) -> Result<T, accounts::DecodeError>,
    ) -> Result<Vec<(Pubkey, T)>> {
        program_accounts(&self.client, disc).await?.into_iter().map(|(k, d)| Ok((k, read(&d)?))).collect()
    }
}
