//! TradingTime simulator: a perpetual on the expected time for price to move ±x%.
//!
//! Longs profit when moves take longer (volatility falls), shorts when they come
//! sooner. Trades fill at a model price `ln(1+x)²/σ̂²` against an LP pool.

pub mod agents;
pub mod band;
pub mod book;
pub mod book_sim;
pub mod engine;
pub mod feed;
pub mod implied;
pub mod metrics;
pub mod oracle;
pub mod pool;
pub mod vol;
