use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use carbon_core::account::AccountProcessorInputType;
use carbon_core::error::{CarbonResult, Error as CarbonError};
use carbon_core::instruction::InstructionProcessorInputType;
use carbon_core::pipeline::{Pipeline, ShutdownStrategy};
use carbon_core::processor::Processor;
use carbon_rpc_program_subscribe_datasource::{Filters as SubFilters, RpcProgramSubscribe};
use carbon_rpc_transaction_crawler_datasource::{ConnectionConfig, Filters as CrawlFilters, RetryConfig, RpcTransactionCrawler};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use tradingtime_interface::PROGRAM_ID;

use crate::api::{AppState, Notify};
use crate::decode::{Event, TtAccount, TtAccountDecoder, TtInstruction, TtInstructionDecoder, program_events};
use crate::store::{Changed, b58};
use crate::views;

pub struct TxIx {
    pub ix: TtInstruction,
    pub accounts: Vec<[u8; 32]>,
}

fn program_id_str() -> String {
    Pubkey::new_from_array(PROGRAM_ID).to_string()
}

/// Stores one program account snapshot and notifies WS subscribers.
pub fn apply_account(state: &AppState, pubkey: &[u8; 32], slot: u64, data: &[u8]) -> Result<()> {
    let changed = state.store.lock().map_err(|_| anyhow!("store poisoned"))?.upsert_account(pubkey, slot, data)?;
    match changed {
        Some(Changed::Market) => {
            state.publish(Notify::Market);
            state.publish(Notify::Quotes);
        }
        Some(Changed::Trader { owner }) => {
            state.publish(Notify::Trader(owner));
            state.publish(Notify::Quotes);
        }
        Some(Changed::Operator) => state.publish(Notify::Quotes),
        Some(Changed::Order { owner }) => state.publish(Notify::Trader(owner)),
        None => {}
    }
    Ok(())
}

/// Records instruction-only facts and every event the program logged; idempotent per (signature, seq).
pub fn apply_transaction(state: &AppState, signature: &str, slot: u64, logs: &[String], ixs: &[TxIx]) -> Result<usize> {
    let mut notes = Vec::new();
    let mut inserted = 0;
    {
        let mut store = state.store.lock().map_err(|_| anyhow!("store poisoned"))?;
        store.set_chain_slot(slot);
        for t in ixs {
            if let (TtInstruction::PlaceLimitOrder(d), Some(owner)) = (&t.ix, t.accounts.first()) {
                store.record_order_placement(owner, d.order_id, d.expiry_ts, slot)?;
            }
        }
        let pdas = store.pda_owners()?;
        let owner_of = |k: &[u8; 32]| {
            let s = b58(k);
            pdas.get(&s).cloned().unwrap_or(s)
        };
        for (seq, ev) in program_events(logs, &program_id_str()).iter().enumerate() {
            if !store.insert_event(signature, seq as u32, slot, ev)? {
                continue;
            }
            inserted += 1;
            match ev {
                Event::Trade(e) => {
                    if let Some(f) = store.fill(signature, seq as u32)? {
                        notes.push(Notify::Trade(views::fill_json(&f)));
                    }
                    notes.push(Notify::Trader(owner_of(&e.trader)));
                    notes.push(Notify::Market);
                }
                Event::Cash(e) => notes.push(Notify::Trader(owner_of(&e.owner))),
                Event::Order(e) => notes.push(Notify::Trader(owner_of(&e.owner))),
                Event::Liquidation(e) => notes.push(Notify::Trader(owner_of(&e.trader))),
                Event::Index(_) | Event::Funding(_) => notes.push(Notify::Market),
                Event::Operator(_) => notes.push(Notify::Quotes),
            }
        }
    }
    notes.dedup();
    for n in notes {
        state.publish(n);
    }
    Ok(inserted)
}

pub struct IxProcessor(pub AppState);

impl Processor<InstructionProcessorInputType<'_, TtInstruction>> for IxProcessor {
    async fn process(&mut self, data: &InstructionProcessorInputType<'_, TtInstruction>) -> CarbonResult<()> {
        let tx = &data.metadata.transaction_metadata;
        if tx.meta.status.is_err() {
            return Ok(());
        }
        let logs = tx.meta.log_messages.clone().unwrap_or_default();
        let ix = TxIx {
            ix: *data.decoded_instruction,
            accounts: data.raw_instruction.accounts.iter().map(|a| a.pubkey.to_bytes()).collect(),
        };
        apply_transaction(&self.0, &tx.signature.to_string(), tx.slot, &logs, &[ix])
            .map(|_| ())
            .map_err(|e| CarbonError::Custom(e.to_string()))
    }
}

pub struct AccountProcessor(pub AppState);

impl Processor<AccountProcessorInputType<'_, TtAccount>> for AccountProcessor {
    async fn process(&mut self, data: &AccountProcessorInputType<'_, TtAccount>) -> CarbonResult<()> {
        apply_account(&self.0, &data.metadata.pubkey.to_bytes(), data.metadata.slot, &data.raw_account.data)
            .map_err(|e| CarbonError::Custom(e.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,
    pub ws_url: String,
}

pub fn pipeline(cfg: &Config, state: &AppState) -> CarbonResult<Pipeline> {
    let program = Pubkey::new_from_array(PROGRAM_ID);
    let crawl = ConnectionConfig::new(100, Duration::from_millis(500), 5, RetryConfig::default(), None, None, false);
    Pipeline::builder()
        .datasource(RpcTransactionCrawler::new(
            cfg.rpc_url.clone(),
            program,
            crawl,
            CrawlFilters::new(None, None, None),
            None,
        ))
        .datasource(RpcProgramSubscribe::new(cfg.ws_url.clone(), SubFilters::new(program, None)))
        .instruction(TtInstructionDecoder, IxProcessor(state.clone()))
        .account(TtAccountDecoder, AccountProcessor(state.clone()))
        .shutdown_strategy(ShutdownStrategy::ProcessPending)
        .build()
}

async fn rpc(http: &reqwest::Client, url: &str, method: &str, params: Value) -> Result<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let v: Value = http.post(url).json(&body).send().await?.json().await?;
    if let Some(e) = v.get("error") {
        return Err(anyhow!("{method}: {e}"));
    }
    v.get("result").cloned().context("missing result")
}

fn b64(v: &Value) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(v.get(0)?.as_str()?).ok()
}

const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";

/// Slot + chain clock, then every known program account by address (`getMultipleAccounts`, fast).
pub async fn poll_once(http: &reqwest::Client, url: &str, state: &AppState) -> Result<()> {
    let slot = rpc(http, url, "getSlot", json!([{ "commitment": "confirmed" }])).await?.as_u64().unwrap_or(0);
    let clock = rpc(http, url, "getAccountInfo", json!([CLOCK_SYSVAR, { "encoding": "base64" }])).await?;
    let ts = clock
        .pointer("/value/data")
        .and_then(b64)
        .and_then(|d| d.get(32..40).map(|b| i64::from_le_bytes(b.try_into().unwrap_or_default())));
    if let Some(ts) = ts {
        state.store.lock().map_err(|_| anyhow!("store poisoned"))?.set_chain(slot, ts);
    }
    let known = state.store.lock().map_err(|_| anyhow!("store poisoned"))?.known_pubkeys()?;
    for chunk in known.chunks(100) {
        let res = rpc(http, url, "getMultipleAccounts", json!([chunk, { "encoding": "base64", "commitment": "confirmed" }])).await?;
        let ctx_slot = res.pointer("/context/slot").and_then(Value::as_u64).unwrap_or(slot);
        let values = res.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
        for (key, acc) in chunk.iter().zip(values) {
            let (Ok(pk), Some(data)) = (key.parse::<Pubkey>(), acc.pointer("/data").and_then(b64)) else { continue };
            apply_account(state, &pk.to_bytes(), ctx_slot, &data)?;
        }
    }
    Ok(())
}

/// All program accounts (`getProgramAccounts`): discovers new accounts. Slow on a forked validator, so it runs
/// on its own schedule and never delays the per-second refresh.
pub async fn discover(http: &reqwest::Client, url: &str, state: &AppState) -> Result<usize> {
    let program = Pubkey::new_from_array(PROGRAM_ID).to_string();
    let res = rpc(http, url, "getProgramAccounts", json!([program, { "encoding": "base64", "withContext": true }])).await?;
    let ctx_slot = res.pointer("/context/slot").and_then(Value::as_u64).unwrap_or(0);
    let list = res.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
    for a in &list {
        let (Some(pk), Some(data)) = (
            a.get("pubkey").and_then(Value::as_str).and_then(|s| s.parse::<Pubkey>().ok()),
            a.pointer("/account/data").and_then(b64),
        ) else {
            continue;
        };
        apply_account(state, &pk.to_bytes(), ctx_slot, &data)?;
    }
    Ok(list.len())
}

/// Runs the Carbon pipeline (restarting on failure) and the slot/clock/account poller forever.
pub async fn run(cfg: Config, state: AppState) -> Result<()> {
    let poll_state = state.clone();
    let rpc_url = cfg.rpc_url.clone();
    // Makers re-quote every slot and curves go stale after 10 slots (~4 s), so known accounts are re-read every
    // second; programSubscribe alone lags on Surfpool and would show live makers as stale.
    let poller = tokio::spawn(async move {
        let http = reqwest::Client::new();
        loop {
            if let Err(e) = poll_once(&http, &rpc_url, &poll_state).await {
                tracing::warn!("poll: {e:#}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let discover_state = state.clone();
    let rpc_url = cfg.rpc_url.clone();
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        loop {
            if let Err(e) = discover(&http, &rpc_url, &discover_state).await {
                tracing::warn!("discover: {e:#}");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    let cfg = Arc::new(cfg);
    loop {
        match pipeline(&cfg, &state) {
            Ok(mut p) => {
                if let Err(e) = p.run().await {
                    tracing::warn!("carbon pipeline stopped: {e}");
                }
            }
            Err(e) => tracing::warn!("carbon pipeline build: {e}"),
        }
        if poller.is_finished() {
            return Err(anyhow!("poller exited"));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::tests::{order_ev, trade_ev, trader};
    use bytemuck::bytes_of;
    use tradingtime_interface::events::*;
    use tradingtime_interface::{PlaceLimitOrderData, PodI128};

    fn line<T: bytemuck::Pod>(tag: u8, ev: &T) -> String {
        let mut buf = vec![0u8; 1 + size_of::<T>()];
        encode(tag, ev, &mut buf);
        format!("Program data: {}", base64::engine::general_purpose::STANDARD.encode(buf))
    }

    #[tokio::test]
    async fn transaction_events_and_accounts_reach_store_and_subscribers() {
        let state = AppState::new(Store::open_in_memory().unwrap());
        let mut rx = state.notify.subscribe();
        let owner = [7; 32];
        apply_account(&state, &[2; 32], 5, bytes_of(&trader(owner, 10, 0, 1.0))).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Notify::Trader(b58(&owner)));
        assert_eq!(rx.recv().await.unwrap(), Notify::Quotes);

        let me = program_id_str();
        let logs = vec![
            format!("Program {me} invoke [1]"),
            line(TAG_ORDER, &order_ev(owner, 3, ORDER_PLACED, 0, 100)),
            line(TAG_TRADE, &trade_ev([2; 32], 0, 1_000_000, 3600.0, 101)),
            format!("Program {me} success"),
        ];
        let place = PlaceLimitOrderData { side: 0, _pad: [0; 7], order_id: 3, size: 1, expiry_ts: 999, limit_ln: PodI128::new(0) };
        let ixs = [TxIx { ix: TtInstruction::PlaceLimitOrder(place), accounts: vec![owner, [2; 32]] }];
        assert_eq!(apply_transaction(&state, "sig1", 60, &logs, &ixs).unwrap(), 2);
        assert_eq!(apply_transaction(&state, "sig1", 60, &logs, &ixs).unwrap(), 0);

        assert_eq!(rx.recv().await.unwrap(), Notify::Trader(b58(&owner)));
        match rx.recv().await.unwrap() {
            Notify::Trade(f) => {
                assert_eq!(f["signature"], "sig1");
                assert_eq!(f["side"], "long");
                assert_eq!(f["size_usdc"], 1.0);
            }
            other => panic!("expected trade, got {other:?}"),
        }
        let store = state.store.lock().unwrap();
        assert_eq!(store.open_orders().unwrap()[0].expiry_ts, Some(999));
        assert_eq!(store.fills_for(&b58(&owner), 10).unwrap().len(), 1);
        assert_eq!(store.slot(), 60);
    }
}
