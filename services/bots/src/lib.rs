//! Simulated protocol activity for local stacks: trader personas and a Pyth price driver.

pub mod price;
pub mod strategy;

#[cfg(feature = "rpc")]
pub mod runner;
