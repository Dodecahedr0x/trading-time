use crate::chain::{Action, Chain};
use crate::feed::{Feed, FeedMode, unix_ms};
use crate::forecast::{DEFAULT_HALF_LIFE_SECS, Forecaster, implied_var};
use crate::quote::QuoteParams;
pub use crate::slot::ESTIMATE_TIMEOUT_MS;
use crate::slot::{DEFAULT_SLOT_MS, ESTIMATE_REFRESH_MS, FIRST_MEASURE_MS, measured_slot_ms, slot_ms_or_default};
use std::time::Duration;
use tradingtime_interface::OperatorStatus;

/// Creates the trader, deposits and registers the operator, whichever are missing.
pub async fn bootstrap<C: Chain>(c: &C, deposit_usdc: u64) -> anyhow::Result<()> {
    let trader = c.trader().await?;
    let operator = c.operator().await?;
    let mut actions = Vec::new();
    if trader.is_none() {
        actions.push(Action::CreateTrader);
    }
    let status = operator.and_then(|o| OperatorStatus::from_u8(o.status));
    match status {
        None | Some(OperatorStatus::Closed) => {
            if deposit_usdc > 0 {
                actions.push(Action::Deposit(deposit_usdc.saturating_mul(1_000_000)));
            }
            actions.push(Action::RegisterOperator);
        }
        Some(OperatorStatus::Unbonding) => tracing::warn!("operator is unbonding; curve updates may be rejected"),
        Some(OperatorStatus::Active) => {}
    }
    if !actions.is_empty() {
        tracing::info!("bootstrap: {actions:?}");
        c.send(&actions).await?;
    }
    Ok(())
}

pub async fn shutdown<C: Chain>(c: &C, unbond: bool) -> anyhow::Result<()> {
    if unbond {
        tracing::info!("requesting unbond");
        c.send(&[Action::RequestUnbond]).await?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct LoopOpts {
    pub feed: FeedMode,
    pub quote: QuoteParams,
    pub deposit_usdc: u64,
    /// Fixed tick period; None estimates it from recent slot times.
    pub slot_ms: Option<u64>,
    pub market_refresh_ms: i64,
    pub unbond_on_exit: bool,
}

async fn estimate<C: Chain>(chain: &C, current_ms: u64) -> u64 {
    let Ok(samples) = tokio::time::timeout(Duration::from_millis(ESTIMATE_TIMEOUT_MS), chain.perf_samples()).await else {
        tracing::warn!("perf samples: timed out after {ESTIMATE_TIMEOUT_MS}ms; keeping {current_ms}ms");
        return current_ms;
    };
    let ms = slot_ms_or_default(samples.inspect_err(|e| tracing::warn!("perf samples: {e:#}")));
    tracing::debug!(slot_ms = ms, "slot time estimate");
    ms
}

/// Current `(slot, unix_ms)`, bounded like `estimate`.
async fn slot_reading<C: Chain>(chain: &C) -> Option<(u64, i64)> {
    match tokio::time::timeout(Duration::from_millis(ESTIMATE_TIMEOUT_MS), chain.slot()).await {
        Ok(Ok(s)) => Some((s, unix_ms())),
        _ => None,
    }
}

fn ticker(period_ms: u64) -> tokio::time::Interval {
    let p = Duration::from_millis(period_ms.max(1));
    let mut t = tokio::time::interval_at(tokio::time::Instant::now() + p, p);
    t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    t
}

/// Bootstrap, then send `update_curve` every slot (fixed or estimated period) until `stop` resolves, then `shutdown`.
pub async fn run_with<C: Chain>(chain: &C, opts: LoopOpts, stop: impl Future<Output = ()>) -> anyhow::Result<()> {
    bootstrap(chain, opts.deposit_usdc).await?;
    let mut market = chain.market().await?;
    let mut market_at = unix_ms();
    let mut fc = Forecaster::new(DEFAULT_HALF_LIFE_SECS);
    let mut feed = Feed::spawn(opts.feed.clone());
    let mut period = match opts.slot_ms {
        Some(ms) => ms,
        None => estimate(chain, DEFAULT_SLOT_MS).await,
    };
    let mut estimated_at = unix_ms();
    let mut slot_mark = if opts.slot_ms.is_none() { slot_reading(chain).await } else { None };
    let mut measured = false;
    let mut timer = tokio::time::interval(Duration::from_millis(period.max(1)));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut feed_open = true;
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            t = feed.next(), if feed_open => match t {
                Some(t) => fc.on_tick(&t),
                None => feed_open = false,
            },
            _ = timer.tick() => {
                let now = unix_ms();
                if now - market_at >= opts.market_refresh_ms {
                    match chain.market().await {
                        Ok(m) => market = m,
                        Err(e) => tracing::warn!("market fetch: {e:#}"),
                    }
                    market_at = now;
                }
                let refresh = if measured { ESTIMATE_REFRESH_MS } else { FIRST_MEASURE_MS };
                if opts.slot_ms.is_none() && now - estimated_at >= refresh {
                    let reading = slot_reading(chain).await;
                    let from_slots = slot_mark.zip(reading).and_then(|(a, b)| measured_slot_ms(a, b));
                    if reading.is_some() {
                        slot_mark = reading;
                    }
                    measured |= from_slots.is_some();
                    let ms = match from_slots {
                        Some(ms) => ms,
                        None => estimate(chain, period).await,
                    };
                    if ms != period {
                        period = ms;
                        timer = ticker(period);
                    }
                    estimated_at = now;
                }
                if fc.var().is_none() && market.index_ln.get() != 0 {
                    // Prior: the vol the on-chain index implies, so fair starts at the index.
                    if let Some(v) = implied_var(&market) {
                        fc.seed(v);
                    }
                }
                let Some(fair) = fc.fair_ln(&market) else {
                    tracing::warn!("fair_ln overflow; skipping quote");
                    continue;
                };
                let q = opts.quote.quote(fair);
                match chain.send(&[Action::UpdateCurve(q)]).await {
                    Ok(()) => tracing::debug!(fair_secs = tradingtime_math::to_f64(fair).exp(), "curve updated"),
                    Err(e) => tracing::warn!("update_curve: {e:#}"),
                }
            }
        }
    }
    drop(feed);
    shutdown(chain, opts.unbond_on_exit).await
}
