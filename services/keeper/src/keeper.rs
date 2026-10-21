use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use solana_pubkey::Pubkey;
use tradingtime_client::slot::{
    DEFAULT_SLOT_MS, ESTIMATE_REFRESH_MS, ESTIMATE_TIMEOUT_MS, FIRST_MEASURE_MS, measured_slot_ms, slot_ms_or_default,
};
use tradingtime_interface::Side;

use crate::chain::{Action, Chain};
use crate::decide::{self, FUNDING_EVERY_MS, MARK_EVERY_MS};
use crate::indexer::{Indexer, PositionRow};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Slot time from perf samples, bounded by ESTIMATE_TIMEOUT_MS; keeps `current_ms` on timeout, 400ms on error.
/// Current `(slot, unix_ms)`, bounded like `estimate`.
async fn slot_reading<C: Chain>(chain: &C) -> Option<(u64, i64)> {
    match tokio::time::timeout(Duration::from_millis(ESTIMATE_TIMEOUT_MS), chain.slot()).await {
        Ok(Ok(s)) => Some((s, now_ms())),
        _ => None,
    }
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

fn ticker(period_ms: u64, first: tokio::time::Instant) -> tokio::time::Interval {
    let mut t = tokio::time::interval_at(first, Duration::from_millis(period_ms.max(1)));
    t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    t
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub sent: Vec<Action>,
    pub failed: Vec<(Action, String)>,
}

pub struct Keeper<C, I> {
    pub chain: C,
    pub indexer: I,
    last_funding_ms: Option<i64>,
    last_mark_ms: Option<i64>,
}

impl<C: Chain, I: Indexer> Keeper<C, I> {
    pub fn new(chain: C, indexer: I) -> Self {
        Self { chain, indexer, last_funding_ms: None, last_mark_ms: None }
    }

    /// The keeper fee is paid into the keeper's Trader account; create it if missing. Returns true if created.
    pub async fn ensure_trader(&self) -> Result<bool> {
        if self.chain.trader(&self.chain.keeper()).await?.is_some() {
            return Ok(false);
        }
        self.chain.send(&Action::CreateTrader).await?;
        Ok(true)
    }

    async fn send(&self, action: Action, report: &mut TickReport) -> bool {
        match self.chain.send(&action).await {
            Ok(()) => {
                tracing::info!(?action, "sent");
                report.sent.push(action);
                true
            }
            Err(e) => {
                tracing::warn!(?action, error = %e, "send failed");
                report.failed.push((action, format!("{e:#}")));
                false
            }
        }
    }

    /// Ticks every `interval_ms`, or at the estimated slot time (re-estimated every 60s) when None, until `stop`.
    pub async fn run_until(&mut self, interval_ms: Option<u64>, stop: impl Future<Output = ()>) {
        let mut period = match interval_ms {
            Some(ms) => ms,
            None => estimate(&self.chain, DEFAULT_SLOT_MS).await,
        };
        let mut estimated_at = Instant::now();
        let mut slot_mark = if interval_ms.is_none() { slot_reading(&self.chain).await } else { None };
        let mut measured = false;
        let mut timer = ticker(period, tokio::time::Instant::now());
        tokio::pin!(stop);
        loop {
            tokio::select! {
                _ = &mut stop => break,
                _ = timer.tick() => {
                    let refresh = if measured { ESTIMATE_REFRESH_MS } else { FIRST_MEASURE_MS };
                    if interval_ms.is_none() && estimated_at.elapsed() >= Duration::from_millis(refresh as u64) {
                        let reading = slot_reading(&self.chain).await;
                        let from_slots = slot_mark.zip(reading).and_then(|(a, b)| measured_slot_ms(a, b));
                        if reading.is_some() {
                            slot_mark = reading;
                        }
                        measured |= from_slots.is_some();
                        let ms = match from_slots {
                            Some(ms) => ms,
                            None => estimate(&self.chain, period).await,
                        };
                        if ms != period {
                            period = ms;
                            timer = ticker(period, tokio::time::Instant::now() + Duration::from_millis(period.max(1)));
                        }
                        estimated_at = Instant::now();
                    }
                    if let Err(e) = self.tick(now_ms()).await {
                        tracing::warn!(error = %format!("{e:#}"), "tick");
                    }
                }
            }
        }
    }

    /// One pass of the loop. Individual action and indexer failures are logged and skipped;
    /// only failing to read the market, slot or operators aborts the tick.
    pub async fn tick(&mut self, now_ms: i64) -> Result<TickReport> {
        let mut report = TickReport::default();
        let market = self.chain.market().await?;
        let slot = self.chain.slot().await?;
        let operators = self.chain.operators().await?;
        let fresh_ops = decide::fresh_operators(&operators, slot, &market.params);

        let feed = Pubkey::new_from_array(market.pyth_feed);
        match self.chain.pyth_publish_time(&feed).await {
            Ok(t) if decide::should_crank_index(&market, t) => {
                self.send(Action::CrankIndex { pyth_feed: feed }, &mut report).await;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "read pyth feed"),
        }

        if decide::due(self.last_funding_ms, now_ms, FUNDING_EVERY_MS) {
            self.last_funding_ms = Some(now_ms);
            self.send(Action::CrankFunding, &mut report).await;
        }

        if !fresh_ops.is_empty() && decide::due(self.last_mark_ms, now_ms, MARK_EVERY_MS) {
            self.last_mark_ms = Some(now_ms);
            self.send(Action::CrankMark { operators: fresh_ops.clone() }, &mut report).await;
        }

        let all_ops: Vec<Pubkey> = operators.iter().map(|(_, o)| Pubkey::new_from_array(o.owner)).collect();
        if let Err(e) = self.liquidate_all(&fresh_ops, &all_ops, &mut report).await {
            tracing::warn!(error = %e, "liquidation pass");
        }
        if let Err(e) = self.execute_orders(&operators, slot, &market.params, now_ms / 1000, &mut report).await {
            tracing::warn!(error = %e, "limit order pass");
        }
        Ok(report)
    }

    /// Makers are never ADL counterparties: every operator owner (`all_ops`) is excluded, fresh or not.
    async fn liquidate_all(&self, fresh_ops: &[Pubkey], all_ops: &[Pubkey], report: &mut TickReport) -> Result<()> {
        let rows = self.indexer.liquidatable().await?;
        let mut adl_pools: HashMap<Side, Vec<PositionRow>> = HashMap::new();
        for row in rows {
            let q = match self.chain.trader(&row.owner).await {
                Ok(Some(t)) => t.q,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(owner = %row.owner, error = %e, "read target trader");
                    continue;
                }
            };
            if q == 0 {
                continue;
            }
            let side = if q > 0 { Side::Long } else { Side::Short };
            let counter = decide::opposite(side);
            if let std::collections::hash_map::Entry::Vacant(v) = adl_pools.entry(counter) {
                v.insert(self.indexer.positions(counter, true).await.unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "read adl positions");
                    Vec::new()
                }));
            }
            let mut exclude = all_ops.to_vec();
            exclude.push(row.owner);
            let adl = decide::adl_candidates(side, &adl_pools[&counter], &exclude);
            self.send(Action::Liquidate { target: row.owner, operators: fresh_ops.to_vec(), adl }, report).await;
        }
        Ok(())
    }

    async fn execute_orders(
        &self,
        operators: &[(Pubkey, tradingtime_interface::Operator)],
        slot: u64,
        params: &tradingtime_interface::Params,
        now_secs: i64,
        report: &mut TickReport,
    ) -> Result<()> {
        let orders = self.chain.orders().await?;
        for (_, order) in orders.iter().filter(|(_, o)| decide::order_expired(o, now_secs)) {
            let action = Action::CancelExpiredOrder { owner: Pubkey::new_from_array(order.owner), order_id: order.order_id };
            self.send(action, report).await;
        }
        let live: Vec<_> = orders.iter().filter(|(_, o)| decide::order_live(o, now_secs)).collect();
        if live.is_empty() {
            return Ok(());
        }
        let fresh = decide::fresh(operators, slot, params);
        let mut curves = Vec::with_capacity(fresh.len());
        for op in &fresh {
            let owner = Pubkey::new_from_array(op.owner);
            let q = self.chain.trader(&owner).await?.map_or(0, |t| t.q);
            curves.push(op.curve(q));
        }
        let owners: Vec<Pubkey> = fresh.iter().map(|o| Pubkey::new_from_array(o.owner)).collect();
        for (_, order) in live {
            if decide::limit_reachable(order, &curves) {
                let action = Action::ExecuteLimitOrder {
                    owner: Pubkey::new_from_array(order.owner),
                    order_id: order.order_id,
                    operators: owners.clone(),
                };
                self.send(action, report).await;
            }
        }
        Ok(())
    }
}
