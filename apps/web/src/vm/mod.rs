//! Host-testable view-model: formatting and derived display rows.

pub mod analytics;
pub mod chart;
pub mod format;
pub mod ladder;
pub mod market;
pub mod payoff;
pub mod rows;
pub mod ticket;

#[cfg(test)]
mod tests;

pub use tradingtime_interface::Side;

pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Long => "long",
        Side::Short => "short",
    }
}
