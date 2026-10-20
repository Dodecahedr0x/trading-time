mod chart;
mod controls;
mod events;
mod trade;

pub use chart::Charts;
pub use controls::{Controls, Params};
pub use events::EventLog;
pub use trade::TradePanel;

/// Formats seconds as a compact duration.
pub fn fmt_dur(secs: f64) -> String {
    let s = secs.abs();
    let sign = if secs < 0.0 { "-" } else { "" };
    if s >= 86_400.0 {
        format!("{sign}{:.1}d", s / 86_400.0)
    } else if s >= 3_600.0 {
        format!("{sign}{}h {:02}m", (s / 3_600.0) as u64, ((s % 3_600.0) / 60.0) as u64)
    } else if s >= 60.0 {
        format!("{sign}{:.0}m", s / 60.0)
    } else {
        format!("{sign}{s:.0}s")
    }
}

pub fn fmt_usd(v: f64) -> String {
    if v.abs() >= 1e6 {
        format!("${:.2}M", v / 1e6)
    } else if v.abs() >= 1e4 {
        format!("${:.1}k", v / 1e3)
    } else {
        format!("${v:.2}")
    }
}
