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
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, disc.to_le_bytes().to_vec()))]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                ..RpcAccountInfoConfig::default()
            },
            ..RpcProgramAccountsConfig::default()
        };
        let raw = self.client.get_program_ui_accounts_with_config(&program_id(), config).await?;
        raw.into_iter()
            .map(|(k, ui)| {
                let data = ui.data.decode().context("undecodable account data")?;
                Ok((k, read(&data)?))
            })
            .collect()
    }
}
