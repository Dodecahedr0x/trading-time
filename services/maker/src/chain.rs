use crate::quote::CurveQuote;
use crate::slot::PerfSample;
use tradingtime_interface::{Market, Operator, Trader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { CreateTrader, Deposit(u64), RegisterOperator, UpdateCurve(CurveQuote), RequestUnbond }

pub trait Chain {
    fn market(&self) -> impl Future<Output = anyhow::Result<Market>> + Send;
    fn trader(&self) -> impl Future<Output = anyhow::Result<Option<Trader>>> + Send;
    fn operator(&self) -> impl Future<Output = anyhow::Result<Option<Operator>>> + Send;
    fn send(&self, actions: &[Action]) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// Current slot (measures the real slot time).
    fn slot(&self) -> impl Future<Output = anyhow::Result<u64>> + Send;
    /// `getRecentPerformanceSamples`, newest first.
    fn perf_samples(&self) -> impl Future<Output = anyhow::Result<Vec<PerfSample>>> + Send;
}
