pub const MINUS: char = '\u{2212}';
pub const DASH: &str = "\u{2014}";

fn group(int: u64) -> String {
    let s = int.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Magnitude with cents: (negative, "1,284.50"). Rounds to cents first so -0.004 is not negative.
fn cents(v: f64) -> (bool, String) {
    let c = (v.abs() * 100.0).round() as u64;
    let neg = v < 0.0 && c > 0;
    (neg, format!("{}.{:02}", group(c / 100), c % 100))
}

/// "$1,284.50"; negatives "−$1,284.50".
pub fn money(v: f64) -> String {
    let (neg, s) = cents(v);
    if neg { format!("{MINUS}${s}") } else { format!("${s}") }
}

/// Always signed: "+$1,284.50" / "−$3.20" / "$0.00".
pub fn money_signed(v: f64) -> String {
    let (neg, s) = cents(v);
    if neg {
        format!("{MINUS}${s}")
    } else if s == "0.00" {
        format!("${s}")
    } else {
        format!("+${s}")
    }
}

/// "$1,284" (whole dollars, for button labels and sizes).
pub fn money_whole(v: f64) -> String {
    let w = v.abs().round() as u64;
    if v < 0.0 && w > 0 { format!("{MINUS}${}", group(w)) } else { format!("${}", group(w)) }
}

/// "$1.25M", "$350.0K", "$812".
pub fn money_compact(v: f64) -> String {
    let a = v.abs();
    let sign = if v < 0.0 && a >= 0.5 { MINUS.to_string() } else { String::new() };
    if (a / 100.0).round() >= 10_000.0 {
        format!("{sign}${:.2}M", a / 1_000_000.0)
    } else if a.round() >= 1_000.0 {
        format!("{sign}${:.1}K", a / 1_000.0)
    } else {
        format!("{sign}${}", a.round() as u64)
    }
}

/// Largest two units: "45s", "45m", "12m 30s", "1h 30m", "2h", "2.4d".
pub fn duration(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return DASH.to_string();
    }
    let s = secs.round() as u64;
    if s >= 86_400 {
        let d = secs / 86_400.0;
        let t = format!("{d:.1}");
        return format!("{}d", t.strip_suffix(".0").unwrap_or(&t));
    }
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    match (h, m, sec) {
        (0, 0, sec) => format!("{sec}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, sec) => format!("{m}m {sec}s"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// Quote ladder "h:mm:ss".
pub fn hms(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return DASH.to_string();
    }
    let s = secs.round() as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Fraction to signed percent with `dp` decimals: 0.024 -> "+2.40%".
pub fn pct_signed(frac: f64, dp: usize) -> String {
    let p = frac * 100.0;
    let s = format!("{:.*}", dp, p.abs());
    let zero = s.chars().all(|c| c == '0' || c == '.');
    if zero {
        format!("{s}%")
    } else if p < 0.0 {
        format!("{MINUS}{s}%")
    } else {
        format!("+{s}%")
    }
}

/// Unix seconds -> "10-20 14:03" (UTC).
pub fn timestamp(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:02}-{:02} {:02}:{:02}", m, d, rem / 3600, (rem % 3600) / 60)
}

/// Shortened base58 "7thn…kz4h".
pub fn short_addr(addr: &str) -> String {
    if addr.len() <= 10 {
        return addr.to_string();
    }
    format!("{}\u{2026}{}", &addr[..4], &addr[addr.len() - 4..])
}

/// Parses a user-entered dollar amount ("5,000", "$12.5") into a non-negative value.
pub fn parse_amount(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| *c != ',' && *c != '$' && *c != ' ').collect();
    let v: f64 = t.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// Parses a user-entered duration: "1h 30m", "90m", "1:30:00", "5400" (seconds), "2.4d".
pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains(':') {
        let mut total = 0.0;
        for part in s.split(':') {
            total = total * 60.0 + part.trim().parse::<f64>().ok()?;
        }
        return (total > 0.0).then_some(total);
    }
    if let Ok(v) = s.parse::<f64>() {
        return (v > 0.0 && v.is_finite()).then_some(v);
    }
    let mut total = 0.0;
    let mut num = String::new();
    let mut any = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else if c.is_whitespace() {
            continue;
        } else {
            let unit = match c {
                'd' => 86_400.0,
                'h' => 3_600.0,
                'm' => 60.0,
                's' => 1.0,
                _ => return None,
            };
            total += num.parse::<f64>().ok()? * unit;
            num.clear();
            any = true;
        }
    }
    if !num.is_empty() || !any {
        return None;
    }
    (total > 0.0).then_some(total)
}
