use super::format::{duration, money_signed, money_whole, DASH};
use super::{side_word, Side};

#[derive(Clone, Debug, PartialEq)]
pub struct Payoff {
    /// "You are buying long time"
    pub headline: String,
    pub sentence: String,
    /// "+$3,465.74"
    pub if_double: String,
    /// "−$3,465.74"
    pub if_half: String,
    /// The doubling outcome is a gain (Long); halving is then a loss, and vice versa.
    pub if_double_gain: bool,
    /// Duration at which the position is closed out (liquidated), e.g. "34m 50s"; "—" when none.
    pub closed_out: String,
}

/// PnL of signed exposure `q_usdc` when time moves from `from_secs` to `to_secs` (log-quoted).
pub fn pnl(q_usdc: f64, from_secs: f64, to_secs: f64) -> f64 {
    q_usdc * (to_secs / from_secs).ln()
}

/// Duration at which equity hits maintenance: entry·exp((|q|·m − equity)/q). None when q == 0 or never.
pub fn liquidation_secs(q_usdc: f64, entry_secs: f64, equity_usdc: f64, maintenance_bps: u16) -> Option<f64> {
    if q_usdc == 0.0 || entry_secs <= 0.0 {
        return None;
    }
    let m = maintenance_bps as f64 / 10_000.0;
    let x = (q_usdc.abs() * m - equity_usdc) / q_usdc;
    let v = entry_secs * x.exp();
    (v.is_finite() && v > 0.0).then_some(v)
}

/// The "You are buying" box for a new order of `size_usdc` at `entry_secs`, given resulting position
/// `q_after` and `equity_after` (USDC) for the closed-out level.
pub fn payoff(side: Side, size_usdc: f64, entry_secs: f64, q_after: f64, equity_after: f64, maintenance_bps: u16) -> Payoff {
    let q = match side {
        Side::Long => size_usdc,
        Side::Short => -size_usdc,
    };
    let d = duration(entry_secs);
    let sentence = match side {
        Side::Long => format!(
            "You gain if SOL takes longer than {d} to move 1% (calmer market), and lose if its moves get faster."
        ),
        Side::Short => format!(
            "You gain if SOL moves 1% faster than every {d} (wilder market), and lose if its moves get slower."
        ),
    };
    let ok = entry_secs > 0.0 && size_usdc > 0.0;
    Payoff {
        headline: format!("You are buying {} time", side_word(side)),
        sentence,
        if_double: if ok { money_signed(pnl(q, entry_secs, entry_secs * 2.0)) } else { DASH.into() },
        if_double_gain: q >= 0.0,
        if_half: if ok { money_signed(pnl(q, entry_secs, entry_secs / 2.0)) } else { DASH.into() },
        closed_out: liquidation_secs(q_after, entry_secs, equity_after, maintenance_bps)
            .map(duration)
            .unwrap_or_else(|| DASH.into()),
    }
}

/// "Buy long time · $5,000"
pub fn buy_label(side: Side, size_usdc: f64) -> String {
    format!("Buy {} time \u{b7} {}", side_word(side), money_whole(size_usdc))
}
