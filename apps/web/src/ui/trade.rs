use super::State;
use crate::chain::{budget, ix, rpc, tx, Instruction, Pubkey};
use crate::vm::ticket::{base_units, deposit_needed, estimate_market, secs_to_ln, OrderType};
use crate::vm::Side;
use dioxus::prelude::*;
use wallet_adapter::{Cluster, SendOptions, WalletAdapter};

/// Limit orders rest for 7 days.
pub const LIMIT_EXPIRY_SECS: i64 = 7 * 86_400;
const CONFIRM_TIMEOUT_MS: u32 = 60_000;

/// Initialise the Wallet Standard listener at mount so wallets registering early are captured; drain its events
/// (the adapter's channel is bounded and blocks wallet calls when full).
pub fn init_wallet(mut s: State) {
    let Ok(adapter) = WalletAdapter::init_with_channel_capacity(64) else { return };
    let events = adapter.events();
    s.wallet.set(Some(adapter));
    spawn(async move { while events.recv().await.is_ok() {} });
}

pub async fn connect(mut s: State) -> Result<(), String> {
    let mut adapter = match s.wallet.peek().clone() {
        Some(a) => a,
        None => WalletAdapter::init().map_err(|e| e.to_string())?,
    };
    let mut wallets = adapter.wallets();
    for _ in 0..30 {
        if !wallets.is_empty() {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(100).await;
        wallets = adapter.wallets();
    }
    let wallet = wallets.into_iter().next().ok_or("No Solana wallet found")?;
    let account = adapter.connect(wallet).await.map_err(|e| e.to_string())?;
    s.owner.set(Some(account.address().to_string()));
    s.wallet.set(Some(adapter));
    super::refresh_account(s).await;
    Ok(())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn owner(s: &State) -> Result<Pubkey, String> {
    let o = s.owner.peek().clone().ok_or("Connect a wallet first")?;
    o.parse::<Pubkey>().map_err(|e| format!("bad wallet address: {e}"))
}

/// Builds create_trader (if new) + deposit (if short of margin) + the order, signs via the wallet, waits for confirmation.
pub async fn submit(s: State, side: Side, kind: OrderType, size: f64, limit_secs: Option<f64>) -> Result<(), String> {
    if size <= 0.0 {
        return Err("Enter a size".into());
    }
    let owner = owner(&s)?;
    let trader = s.trader.peek().clone();
    let quotes = s.quotes.peek().clone();
    let mut ixs: Vec<Instruction> = vec![budget::set_compute_unit_limit(budget::TRADE_CU_LIMIT)];
    if trader.is_none() {
        ixs.push(ix::create_trader(&owner));
    }
    let dep = deposit_needed(trader.as_ref(), side, kind, size);
    if dep > 0.0 {
        let mut amount = (dep * 1_000_000.0).ceil() as u64;
        let bal = base_units(*s.wallet_usdc.peek());
        if bal > 0 {
            amount = amount.min(bal);
        }
        let ata = ix::associated_token_address(&owner, &ix::usdc_mint());
        ixs.push(ix::deposit(&owner, &ata, amount));
    }
    let size_base = base_units(size);
    match kind {
        OrderType::Market => {
            let est = estimate_market(&quotes, side, size).ok_or("No market price yet")?;
            let ops: Vec<Pubkey> = quotes
                .operators
                .iter()
                .filter(|o| o.fresh)
                .filter_map(|o| o.owner.parse().ok())
                .take(tradingtime_interface::MAX_OPERATORS_PER_IX)
                .collect();
            if ops.is_empty() {
                return Err("No fresh quotes".into());
            }
            ixs.push(ix::swap(&owner, side, size_base, size_base, secs_to_ln(est.limit_secs), &ops));
        }
        OrderType::Limit => {
            let limit = limit_secs.filter(|v| *v > 0.0).ok_or("Enter a limit time")?;
            let order_id = now_ms() as u64;
            let expiry = (now_ms() / 1000.0) as i64 + LIMIT_EXPIRY_SECS;
            ixs.push(ix::place_limit_order(&owner, order_id, side, size_base, secs_to_ln(limit), expiry));
        }
    }
    send(s, &owner, &ixs).await
}

pub async fn cancel(s: State, order_id: u64) -> Result<(), String> {
    let owner = owner(&s)?;
    send(s, &owner, &[ix::cancel_limit_order(&owner, order_id)]).await
}

async fn send(s: State, owner: &Pubkey, ixs: &[Instruction]) -> Result<(), String> {
    let adapter = s.wallet.peek().clone().ok_or("Connect a wallet first")?;
    let rpc_url = s.cfg.peek().rpc_url.clone();
    let blockhash = rpc::latest_blockhash(&rpc_url).await?;
    let bytes = tx::unsigned_tx_bytes(ixs, owner, blockhash);
    let sig = adapter
        .sign_and_send_transaction(&bytes, Cluster::LocalNet, SendOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    let sig = bs58::encode(sig.to_bytes()).into_string();
    rpc::wait_confirmed(&rpc_url, &sig, CONFIRM_TIMEOUT_MS).await
}

/// Runs `fut` with the busy flag and tx-status line, then refreshes the account.
pub fn run_tx<F>(mut s: State, fut: F)
where
    F: std::future::Future<Output = Result<(), String>> + 'static,
{
    s.busy.set(true);
    s.tx_status.set(Some("Pending\u{2026}".into()));
    spawn(async move {
        let r = fut.await;
        s.tx_status.set(Some(match r {
            Ok(()) => "Confirmed".into(),
            Err(e) => format!("Failed: {e}"),
        }));
        s.busy.set(false);
        super::refresh_account(s).await;
    });
}
