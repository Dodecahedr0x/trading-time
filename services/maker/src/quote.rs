use tradingtime_math::{Fx, ONE};

/// Mirrors `tradingtime_client::CurveQuote`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveQuote {
    pub fair_ln: Fx,
    pub half_spread: Fx,
    pub impact_per_usdc: Fx,
    pub inventory_limit: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuoteParams {
    pub half_spread_bps: f64,
    /// ln-price moved by 10,000 USDC of size, in bps.
    pub impact_bps_per_10k: f64,
    pub inventory_limit_usdc: u64,
}

impl QuoteParams {
    pub fn quote(&self, fair_ln: Fx) -> CurveQuote {
        CurveQuote {
            fair_ln,
            half_spread: (self.half_spread_bps.max(0.0) * ONE as f64 / 1e4) as Fx,
            impact_per_usdc: (self.impact_bps_per_10k.max(0.0) * ONE as f64 / 1e8) as Fx,
            inventory_limit: self.inventory_limit_usdc.saturating_mul(1_000_000),
        }
    }
}
