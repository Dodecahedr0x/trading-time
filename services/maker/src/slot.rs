pub use tradingtime_client::slot::*;

pub fn slot_ms_or_default(r: anyhow::Result<Vec<PerfSample>>) -> u64 {
    tradingtime_client::slot::slot_ms_or_default(r)
}
