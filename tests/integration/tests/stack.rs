//! End-to-end scenario against `scripts/dev-stack.sh up` (Surfpool + program + indexer + keeper + 2 makers).
//! Run: cargo test -p tradingtime-tests -- --ignored --test-threads=1

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_client::{ix, pda, rpc::Ctx, surfnet};
use tradingtime_interface::{OperatorStatus, PYTH_SOL_USD_FEED, Side, TOKEN_PROGRAM, USDC_MINT};
use tradingtime_math::Fx;
use tradingtime_tests::{Env, chain_now, chain_slot, load_env, read_keypair, wait_for};

const USDC: u64 = 1_000_000;

fn ata(owner: &Pubkey) -> Pubkey {
    let ata_program = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
    let (k, _) = Pubkey::find_program_address(
        &[owner.as_ref(), &TOKEN_PROGRAM, &USDC_MINT],
        &ata_program,
    );
    k
}

async fn api(env: &Env, path: &str) -> Result<Value> {
    let r = reqwest::get(format!("{}{path}", env.api_url)).await?;
    ensure!(r.status().is_success(), "GET {path} -> {}", r.status());
    Ok(r.json().await?)
}

async fn fresh_operators(ctx: &Ctx) -> Result<Vec<Pubkey>> {
    let market = ctx.market().await?;
    let slot = chain_slot(ctx).await?;
    let mut out = Vec::new();
    for (_, op) in ctx.operators().await? {
        if op.is_fresh(slot, market.params.curve_max_age_slots) {
            out.push(Pubkey::new_from_array(op.owner));
        }
    }
    Ok(out)
}

async fn token_balance(ctx: &Ctx, owner: &Pubkey) -> Result<u64> {
    let b = ctx.client.get_token_account_balance(&ata(owner)).await?;
    Ok(b.amount.parse()?)
}

async fn open_trader(ctx: &Ctx, deposit: u64) -> Result<()> {
    let me = ctx.payer.pubkey();
    let mut ixs = Vec::new();
    if ctx.trader(&me).await?.is_none() {
        ixs.push(ix::create_trader(&me));
    }
    ixs.push(ix::deposit(&me, &ata(&me), deposit));
    ctx.send(&ixs, &[]).await.context("create+deposit")?;
    Ok(())
}

async fn swap(ctx: &Ctx, side: Side, size: u64) -> Result<()> {
    let ops = wait_for(30, "fresh operators", || async { Ok(Some(fresh_operators(ctx).await?).filter(|o| !o.is_empty())) }).await?;
    let limit: Fx = match side {
        Side::Long => i128::MAX / 4,
        Side::Short => i128::MIN / 4,
    };
    ctx.send(&[ix::swap(&ctx.payer.pubkey(), side, size, size, limit, &ops)], &[]).await.context("swap")?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs scripts/dev-stack.sh up"]
async fn full_lifecycle_on_surfpool() -> Result<()> {
    let env = load_env()?;
    let t1 = Ctx::new(env.rpc_url.clone(), read_keypair(&env.trader_keypairs[0])?);
    let t2 = Ctx::new(env.rpc_url.clone(), read_keypair(&env.trader_keypairs[1])?);
    let admin = Ctx::new(env.rpc_url.clone(), read_keypair(&env.admin_keypair)?);

    // 1. Both makers registered and quoting fresh curves.
    let ops = wait_for(120, "two fresh operators", || async {
        let f = fresh_operators(&admin).await?;
        Ok((f.len() >= 2).then_some(f))
    })
    .await?;
    for (_, op) in admin.operators().await? {
        ensure!(op.status == OperatorStatus::Active as u8 && op.bond == 1_000 * USDC, "operator bonded and active");
    }

    let ops_q = |ops: Vec<Pubkey>| {
        let admin = &admin;
        async move {
            let mut s = 0i64;
            for o in &ops {
                s += admin.trader(o).await?.context("op trader")?.q;
            }
            anyhow::Ok(s)
        }
    };
    let op_q0 = ops_q(ops.clone()).await?;

    // 2. Trader 1 buys 5,000 of long time at ~1x; trader 2 buys 3,000 of short time.
    open_trader(&t1, 5_200 * USDC).await?;
    open_trader(&t2, 20_000 * USDC).await?;
    swap(&t1, Side::Long, 5_000 * USDC).await?;
    swap(&t2, Side::Short, 3_000 * USDC).await?;
    let tr1 = t1.trader(&t1.payer.pubkey()).await?.context("t1")?;
    let tr2 = t2.trader(&t2.payer.pubkey()).await?.context("t2")?;
    ensure!(tr1.q == 5_000 * USDC as i64 && tr2.q == -(3_000 * USDC as i64), "positions {} {}", tr1.q, tr2.q);
    let op_q = ops_q(ops.clone()).await? - op_q0;
    ensure!(op_q == -2_000 * USDC as i64, "operators took the other side: {op_q}");

    // Indexer agrees with chain.
    let t1_key = t1.payer.pubkey().to_string();
    let view = wait_for(60, "indexer trader view", || async {
        let v = api(&env, &format!("/v1/traders/{t1_key}")).await?;
        Ok((v["q_usdc"].as_f64() == Some(5_000.0)).then_some(v))
    })
    .await?;
    ensure!(view["equity_usdc"].as_f64().unwrap() > 0.0);
    let fills = api(&env, &format!("/v1/traders/{t1_key}/fills?limit=10")).await?;
    ensure!(fills.as_array().is_some_and(|f| f.iter().any(|x| x["kind"] == "swap" && x["side"] == "long")), "fills {fills}");
    let quotes = api(&env, "/v1/quotes").await?;
    ensure!(quotes["operators"].as_array().is_some_and(|o| o.len() >= 2), "quotes {quotes}");
    let market = api(&env, "/v1/market").await?;
    ensure!(market["mark_secs"].as_f64().unwrap() > 60.0, "market {market}");

    // 3. A resting limit order that is reachable now gets filled by the keeper.
    let m = t2.market().await?;
    let now = chain_now(&t2).await?;
    let me2 = t2.payer.pubkey();
    let order_id = 42u64;
    t2.send(
        &[ix::place_limit_order(&me2, order_id, Side::Long, 1_000 * USDC, m.effective_mark(now) + tradingtime_math::ONE / 20, now + 3_600)],
        &[],
    )
    .await
    .context("place_limit_order")?;
    wait_for(60, "keeper fills the limit order", || async {
        let filled = match t2.client.get_account(&pda::order(&me2, order_id).0).await {
            Ok(acc) => tradingtime_client::accounts::read_order(&acc.data)?.filled == 1_000 * USDC,
            Err(_) => true,
        };
        Ok(filled.then_some(()))
    })
    .await?;
    ensure!(t2.trader(&me2).await?.unwrap().q == -2_000 * (USDC as i64), "limit fill moved q");

    // 4. Funding accrues (keeper cranks every 60s).
    let f0 = admin.market().await?.funding_updated_ts;
    wait_for(150, "funding crank", || async { Ok((admin.market().await?.funding_updated_ts > f0).then_some(())) }).await?;

    // 5. Volatility shock: a 10x variance sample drops the index; long time at ~1x is liquidated.
    let feed = Pubkey::new_from_array(PYTH_SOL_USD_FEED);
    let m = admin.market().await?;
    let now = chain_now(&admin).await?;
    let dt = (now - m.last_publish_time).max(1);
    let var = tradingtime_math::to_f64(m.var_ewma.get());
    let alpha = 1.0 - 2f64.powf(-(dt as f64) / m.params.index_half_life_secs as f64);
    let r = (10.0 * var * dt as f64 / alpha).sqrt();
    let price = (m.last_pyth_price as f64 * r.exp()).round() as i64;
    surfnet::write_pyth_price(&env.rpc_url, &feed, price, m.last_pyth_expo, now).await?;
    wait_for(60, "index falls", || async {
        let m2 = admin.market().await?;
        Ok((m2.index_ln.get() < m.index_ln.get() - tradingtime_math::ONE).then_some(()))
    })
    .await?;
    // Makers are never ADL counterparties, so with only makers left on the other side t1 may stay partly open.
    let q1_before = 5_000 * USDC as i64;
    wait_for(120, "keeper liquidates trader 1", || async {
        Ok((t1.trader(&t1.payer.pubkey()).await?.unwrap().q < q1_before).then_some(()))
    })
    .await?;
    let me2_key = me2.to_string();
    let adl_fills = wait_for(60, "indexer adl fill against trader 2", || async {
        let f = api(&env, &format!("/v1/traders/{t1_key}/fills?limit=50")).await?;
        let adl: Vec<Value> = f.as_array().into_iter().flatten().filter(|x| x["kind"] == "adl").cloned().collect();
        Ok(adl.iter().any(|x| x["operator"] == me2_key.as_str()).then_some(adl))
    })
    .await?;
    let adl_size: f64 = adl_fills.iter().map(|x| x["size_usdc"].as_f64().unwrap_or(0.0)).sum();
    ensure!(adl_size > 0.0, "adl fills carry size: {adl_fills:?}");
    let makers: Vec<String> = admin.operators().await?.iter().map(|(_, op)| Pubkey::new_from_array(op.owner).to_string()).collect();
    ensure!(
        adl_fills.iter().all(|x| x["operator"].as_str().is_some_and(|o| !makers.iter().any(|m| m == o))),
        "no ADL fill names a maker: {adl_fills:?}"
    );
    let q2_after = t2.trader(&me2).await?.unwrap().q;
    ensure!(q2_after > -2_000 * (USDC as i64), "trader 2 was deleveraged: {q2_after}");

    // 6. Trader 2 (short time, a winner; possibly ADL'd already) closes and withdraws everything.
    let q2 = t2.trader(&me2).await?.unwrap().q;
    ensure!(q2 <= 0, "trader 2 still short or flat: {q2}");
    if q2 < 0 {
        swap(&t2, Side::Long, q2.unsigned_abs()).await?;
    }
    let tr2 = t2.trader(&me2).await?.unwrap();
    ensure!(tr2.q == 0 && tr2.cash > 20_000 * USDC as i64, "short time profited: cash {}", tr2.cash);
    let before = token_balance(&t2, &me2).await?;
    t2.send(&[ix::withdraw(&me2, &ata(&me2), tr2.cash as u64, false)], &[]).await.context("withdraw")?;
    let after = token_balance(&t2, &me2).await?;
    ensure!(after - before == tr2.cash as u64, "withdrew {}", after - before);

    // Vault holds exactly what the books say.
    let m = admin.market().await?;
    let vault = admin.client.get_token_account_balance(&pda::vault().0).await?.amount.parse::<u64>()?;
    let mut bonds = 0;
    for (_, op) in admin.operators().await? {
        bonds += op.bond;
    }
    ensure!(vault >= m.insurance + m.treasury + bonds, "vault {vault} covers insurance+treasury+bonds");
    Ok(())
}

#[tokio::test]
#[ignore = "needs scripts/dev-stack.sh up"]
async fn indexer_routes_and_ws_on_live_stack() -> Result<()> {
    use futures::{SinkExt, StreamExt};
    let env = load_env()?;
    ensure!(api(&env, "/v1/health").await?["slot"].as_u64().unwrap() > 0);
    for path in ["/v1/market", "/v1/quotes", "/v1/operators", "/v1/candles?interval=1h&limit=10", "/v1/index?limit=10", "/v1/orders/open", "/v1/liquidatable", "/v1/positions?side=long&limit=5"] {
        api(&env, path).await.with_context(|| path.to_string())?;
    }
    let r = reqwest::get(format!("{}/v1/traders/{}", env.api_url, Keypair::new().pubkey())).await?;
    ensure!(r.status() == 404, "unknown trader -> {}", r.status());
    let r = reqwest::get(format!("{}/v1/candles?interval=7m", env.api_url)).await?;
    ensure!(r.status() == 400, "bad interval -> {}", r.status());

    let ws_url = env.api_url.replace("http", "ws") + "/v1/ws";
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(r#"{"subscribe":["market","quotes"]}"#.into())).await?;
    // The subscribe snapshot must agree with REST for each channel.
    let mut seen = std::collections::HashSet::new();
    while seen.len() < 2 {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(30), ws.next()).await?.context("ws closed")??;
        let v: Value = serde_json::from_str(msg.to_text()?)?;
        match v["channel"].as_str() {
            Some("market") => {
                let rest = api(&env, "/v1/market").await?;
                let (a, b) = (v["data"]["index_ln"].as_str(), rest["index_ln"].as_str());
                ensure!(a.is_some() && a == b, "ws market index {a:?} vs rest {b:?}");
                seen.insert("market");
            }
            Some("quotes") => {
                let rest = api(&env, "/v1/quotes").await?;
                let owners = |x: &Value| -> std::collections::BTreeSet<String> {
                    x["operators"].as_array().into_iter().flatten().filter_map(|o| o["owner"].as_str().map(String::from)).collect()
                };
                ensure!(!owners(&v["data"]).is_empty() && owners(&v["data"]) == owners(&rest), "ws quotes {v} vs rest {rest}");
                seen.insert("quotes");
            }
            _ => {}
        }
    }
    Ok(())
}
