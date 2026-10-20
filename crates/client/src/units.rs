use tradingtime_math::{Fx, from_f64, to_f64};

/// ln-time (Fx) to seconds.
pub fn ln_to_secs(ln: Fx) -> f64 {
    to_f64(ln).exp()
}

/// Seconds to ln-time (Fx); `secs` must be > 0.
pub fn secs_to_ln(secs: f64) -> Fx {
    from_f64(secs.ln())
}

/// USDC base units (6 decimals) to whole USDC.
pub fn usdc(base_units: impl Into<i128>) -> f64 {
    base_units.into() as f64 / 1e6
}

/// Human duration using the largest two units ("1h 30m", "45m", "2m 30s"); a day or more is
/// shown as fractional days ("2.4d").
pub fn format_duration(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "—".to_string();
    }
    if secs >= 86_400.0 {
        return format!("{:.1}d", secs / 86_400.0);
    }
    let s = secs.round() as u64;
    let two = |big: u64, bu: &str, small: u64, su: &str| {
        if small == 0 { format!("{big}{bu}") } else { format!("{big}{bu} {small}{su}") }
    };
    if s >= 3_600 {
        two(s / 3_600, "h", (s % 3_600) / 60, "m")
    } else if s >= 60 {
        two(s / 60, "m", s % 60, "s")
    } else {
        format!("{s}s")
    }
}
