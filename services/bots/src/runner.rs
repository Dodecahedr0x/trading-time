//! Drives bot wallets and the Pyth price on a Surfpool stack.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_client::{
    ix,
    rpc::Ctx,
    surfnet,
    units::{ln_to_secs, secs_to_ln, usdc},
};
use tradingtime_interface::{LimitOrder, Market, Operator, TOKEN_PROGRAM, USDC_MINT};
use tradingtime_math::{Side, from_f64};

use crate::{
    price::{PriceDriver, parse_prices},
    strategy::{Action, Observation, Persona, closing_side, decide},
};

const SOL: u64 = 1_000_000_000;
const BASE: f64 = 1_000_000.0;
const HISTORY: usize = 30;
const ATA_PROGRAM: Pubkey = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const CLOCK: Pubkey = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");

pub struct Config {
    pub rpc: String,
    pub keys_dir: PathBuf,
    pub traders: usize,
    pub seed: u64,
    pub tick_ms: u64,
    pub wallet_usdc: (u64, u64),
    /// None disables the price driver.
    pub pyth_replay: Option<PathBuf>,
    pub pyth_every_secs: u64,
    pub storm_mult: f64,
}

struct Bot {
    persona: Persona,
    ctx: Ctx,
    ata: Pubkey,
    start_equity: f64,
    rng: ChaCha8Rng,
    next_order_id: u64,
}

fn ata(owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), &TOKEN_PROGRAM, &USDC_MINT], &ATA_PROGRAM).0
}

fn usdc_base(x: f64) -> u64 {
    (x * BASE).floor().max(0.0) as u64
}

async fn chain_now(ctx: &Ctx) -> Result<i64> {
    let data = ctx.client.get_account_data(&CLOCK).await?;
    Ok(i64::from_le_bytes(data[32..40].try_into()?))
}

fn load_or_create(path: &PathBuf) -> Result<Keypair> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let bytes: Vec<u8> = serde_json::from_str(&text)?;
        return Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()));
    }
    let kp = Keypair::new();
    std::fs::write(path, serde_json::to_string(&kp.to_bytes().to_vec())?)?;
    Ok(kp)
}

async fn wallet_usdc(ctx: &Ctx, ata: &Pubkey) -> f64 {
    match ctx.client.get_token_account_balance(ata).await {
        Ok(b) => b.amount.parse::<u64>().map(|v| v as f64 / BASE).unwrap_or(0.0),
        Err(_) => 0.0,
    }
}

async fn setup(cfg: &Config, rng: &mut ChaCha8Rng) -> Result<Vec<Bot>> {
    std::fs::create_dir_all(&cfg.keys_dir)?;
    let mint = Pubkey::new_from_array(USDC_MINT);
    let mut bots = Vec::new();
    for i in 0..cfg.traders {
        let kp = load_or_create(&cfg.keys_dir.join(format!("bot{i}.json")))?;
        let owner = kp.pubkey();
        let ctx = Ctx::new(cfg.rpc.clone(), kp);
        let ata = ata(&owner);
        let persona = Persona::assign(i);
        let mut ixs = Vec::new();
        let start_equity = match ctx.trader(&owner).await? {
            Some(t) => {
                let m = ctx.market().await?;
                tradingtime_math::position::equity(&t.position(), m.effective_mark(chain_now(&ctx).await?)).map_or(0.0, usdc)
            }
            None => {
                let mut amount = rng.random_range(cfg.wallet_usdc.0..=cfg.wallet_usdc.1);
                if persona == Persona::Whale {
                    amount *= 3;
                }
                surfnet::set_account(&cfg.rpc, &owner, 100 * SOL, &[], &Pubkey::default()).await.context("fund SOL")?;
                surfnet::set_token_account(&cfg.rpc, &owner, &mint, amount * 1_000_000).await.context("fund USDC")?;
                let deposit = amount / 2;
                ixs.push(ix::create_trader(&owner));
                ixs.push(ix::deposit(&owner, &ata, deposit * 1_000_000));
                deposit as f64
            }
        };
        if !ixs.is_empty() {
            ctx.send(&ixs, &[]).await.with_context(|| format!("open bot{i}"))?;
        }
        tracing::info!(bot = i, persona = persona.name(), %owner, start_equity, "bot ready");
        bots.push(Bot {
            persona,
            ctx,
            ata,
            start_equity: start_equity.max(1.0),
            rng: ChaCha8Rng::seed_from_u64(cfg.seed.wrapping_add(i as u64 * 7919)),
            next_order_id: (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64) * 100 + i as u64,
        });
    }
    Ok(bots)
}

/// Fresh operator owners, tightest spread first, capped at the per-instruction maximum.
fn fresh_ops(ops: &[(Pubkey, Operator)], slot: u64, m: &Market) -> Vec<Pubkey> {
    let mut f: Vec<&Operator> = ops.iter().map(|(_, o)| o).filter(|o| o.is_fresh(slot, m.params.curve_max_age_slots)).collect();
    f.sort_by_key(|o| o.half_spread.get());
    f.into_iter().take(tradingtime_interface::MAX_OPERATORS_PER_IX).map(|o| Pubkey::new_from_array(o.owner)).collect()
}

struct Tick {
    market: Market,
    now: i64,
    ops: Vec<Pubkey>,
    orders: Vec<(Pubkey, LimitOrder)>,
    history: Vec<f64>,
}

fn slippage_limit(mark_ln: i128, side: Side, pct: f64) -> i128 {
    let d = from_f64((1.0 + pct).ln());
    match side {
        Side::Long => mark_ln + d,
        Side::Short => mark_ln - d,
    }
}

async fn act(bot: &mut Bot, t: &Tick) -> Result<Option<String>> {
    let owner = bot.ctx.payer.pubkey();
    let Some(tr) = bot.ctx.trader(&owner).await? else { return Ok(None) };
    let mark = t.market.effective_mark(t.now);
    let equity = tradingtime_math::position::equity(&tr.position(), mark).map_or(0.0, usdc);
    let mine: Vec<&LimitOrder> = t.orders.iter().map(|(_, o)| o).filter(|o| o.owner == owner.to_bytes()).collect();
    let obs = Observation {
        mark_secs: ln_to_secs(mark),
        index_secs: ln_to_secs(t.market.index_ln.get()),
        mark_history: t.history.clone(),
        q_usdc: usdc(tr.q),
        equity_usdc: equity,
        reserved_usdc: usdc((tr.margin_need() - tr.q.unsigned_abs() as u128) as i128),
        open_orders: mine.len(),
        wallet_usdc: wallet_usdc(&bot.ctx, &bot.ata).await,
        has_quotes: !t.ops.is_empty(),
    };
    let Some(action) = decide(bot.persona, &obs, bot.start_equity, &mut bot.rng) else { return Ok(None) };
    let ixs = match &action {
        Action::Market { side, size_usdc } => {
            let size = usdc_base(*size_usdc);
            vec![ix::swap(&owner, *side, size, size, slippage_limit(mark, *side, 0.05), &t.ops)]
        }
        Action::Close => {
            let size = tr.q.unsigned_abs();
            let side = closing_side(tr.q as f64);
            vec![ix::swap(&owner, side, size, size, slippage_limit(mark, side, 0.10), &t.ops)]
        }
        Action::Limit { side, size_usdc, limit_secs, ttl_secs } => {
            bot.next_order_id += 1;
            vec![ix::place_limit_order(&owner, bot.next_order_id, *side, usdc_base(*size_usdc), secs_to_ln(*limit_secs), t.now + ttl_secs)]
        }
        Action::CancelOrders => mine.iter().map(|o| ix::cancel_limit_order(&owner, o.order_id)).collect(),
        Action::Deposit { usdc } => vec![ix::deposit(&owner, &bot.ata, usdc_base(*usdc))],
        Action::Withdraw { usdc } => vec![ix::withdraw(&owner, &bot.ata, usdc_base(*usdc), false)],
    };
    if ixs.is_empty() {
        return Ok(None);
    }
    bot.ctx.send(&ixs, &[]).await?;
    Ok(Some(format!("{action:?}")))
}

async fn drive_price(cfg: &Config, path: PathBuf, seed: u64) -> Result<()> {
    let ctx = Ctx::new(cfg.rpc.clone(), Keypair::new());
    let m = ctx.market().await?;
    let feed = Pubkey::new_from_array(m.pyth_feed);
    let expo = if m.last_pyth_expo == 0 { -8 } else { m.last_pyth_expo };
    let scale = 10f64.powi(-expo);
    let start = if m.last_pyth_price > 0 { m.last_pyth_price as f64 / scale } else { 180.0 };
    let prices = parse_prices(&std::fs::read_to_string(&path).with_context(|| path.display().to_string())?);
    let mut d = PriceDriver::new(&prices, start).context("replay file needs at least two prices")?;
    d.storm_mult = cfg.storm_mult;
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5eed);
    let mut timer = tokio::time::interval(Duration::from_secs(cfg.pyth_every_secs.max(1)));
    loop {
        timer.tick().await;
        let p = d.step(cfg.pyth_every_secs as usize, &mut rng);
        let now = match chain_now(&ctx).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("clock: {e:#}");
                continue;
            }
        };
        match surfnet::write_pyth_price(&cfg.rpc, &feed, (p * scale).round() as i64, expo, now).await {
            Ok(()) => tracing::info!(price = format!("{p:.4}"), storm = d.storm(), "pyth update"),
            Err(e) => tracing::warn!("pyth write: {e:#}"),
        }
    }
}

pub async fn run(cfg: Config) -> Result<()> {
    let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);
    let mut bots = setup(&cfg, &mut rng).await?;
    let pyth = cfg.pyth_replay.clone().map(|p| {
        let cfg = Config { rpc: cfg.rpc.clone(), keys_dir: cfg.keys_dir.clone(), pyth_replay: None, ..cfg };
        let seed = cfg.seed;
        tokio::spawn(async move {
            if let Err(e) = drive_price(&cfg, p, seed).await {
                tracing::error!("price driver stopped: {e:#}");
            }
        })
    });
    let reader = Ctx::new(cfg.rpc.clone(), Keypair::new());
    let mut history: Vec<f64> = Vec::new();
    let mut timer = tokio::time::interval(Duration::from_millis(cfg.tick_ms.max(50)));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (mut sent, mut failed) = (0u64, 0u64);
    let mut last_report = std::time::Instant::now();
    loop {
        timer.tick().await;
        let snapshot = async {
            let market = reader.market().await?;
            let slot = reader.client.get_slot().await?;
            let now = chain_now(&reader).await?;
            let ops = fresh_ops(&reader.operators().await?, slot, &market);
            let orders = reader.orders().await?;
            anyhow::Ok((market, now, ops, orders))
        };
        let (market, now, ops, orders) = match snapshot.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("snapshot: {e:#}");
                continue;
            }
        };
        history.push(ln_to_secs(market.effective_mark(now)));
        if history.len() > HISTORY {
            history.remove(0);
        }
        let tick = Tick { market, now, ops, orders, history: history.clone() };
        for (i, bot) in bots.iter_mut().enumerate() {
            match act(bot, &tick).await {
                Ok(Some(what)) => {
                    sent += 1;
                    tracing::info!(bot = i, persona = bot.persona.name(), "{what}");
                }
                Ok(None) => {}
                Err(e) => {
                    failed += 1;
                    tracing::debug!(bot = i, persona = bot.persona.name(), "tx failed: {e:#}");
                }
            }
        }
        if last_report.elapsed() >= Duration::from_secs(30) {
            tracing::info!(sent, failed, mark_secs = format!("{:.0}", history.last().copied().unwrap_or(0.0)), "activity");
            last_report = std::time::Instant::now();
        }
        if pyth.as_ref().is_some_and(|h| h.is_finished()) {
            anyhow::bail!("price driver exited");
        }
    }
}
