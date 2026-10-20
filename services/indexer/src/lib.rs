//! TradingTime indexer: Carbon ingest → SQLite → REST + WebSocket.
pub mod api;
pub mod candles;
pub mod decode;
pub mod ingest;
pub mod store;
pub mod views;

#[cfg(test)]
mod api_tests;
