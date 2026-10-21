//! Docs page content. Protocol numbers come from `Params::DEFAULT`, so the docs track the program.

use super::format::{duration, money};
use tradingtime_interface::Params;
use tradingtime_math::to_f64;

pub enum Block {
    /// Paragraph; `**bold**` spans are emphasised.
    P(String),
    List(Vec<String>),
    /// (formula, caption)
    Formula(&'static str, String),
    Table { head: Vec<String>, rows: Vec<Vec<String>> },
    /// Highlighted aside.
    Note(String),
}

pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub blocks: Vec<Block>,
}

const YEAR_SECS: f64 = 365.0 * 86_400.0;

fn bps(v: u16) -> String {
    let pct = v as f64 / 100.0;
    if v < 100 { format!("{v} bp ({pct}%)") } else { format!("{pct}%") }
}

fn usdc(base: u64) -> String {
    money(base as f64 / 1e6)
}

fn s(x: impl Into<String>) -> String {
    x.into()
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
}

/// Expected seconds to move ±band at annualised volatility `vol` (e.g. 0.8), with calibration.
pub fn time_at_vol(p: &Params, vol: f64) -> f64 {
    let x = (1.0 + p.band_bps as f64 / 10_000.0).ln();
    let var_per_sec = vol * vol / YEAR_SECS;
    to_f64(p.calib.get()) * x * x / var_per_sec
}

/// Log-time PnL of a `size` USDC position when time moves from `from` to `to` seconds.
pub fn pnl(size: f64, from: f64, to: f64) -> f64 {
    size * (to / from).ln()
}

/// Splits `**bold**` spans: (is_bold, text). Unclosed markers are kept as text.
pub fn spans(text: &str) -> Vec<(bool, String)> {
    let parts: Vec<&str> = text.split("**").collect();
    if parts.len().is_multiple_of(2) {
        return vec![(false, text.to_string())];
    }
    parts.iter().enumerate().filter(|(_, t)| !t.is_empty()).map(|(i, t)| (i % 2 == 1, t.to_string())).collect()
}

pub fn sections() -> Vec<Section> {
    let p = Params::DEFAULT;
    let band = p.band_bps as f64 / 100.0;
    let calib = to_f64(p.calib.get());
    let half_life_days = p.index_half_life_secs as f64 / 86_400.0;
    let funding_days = p.funding_period_secs as f64 / 86_400.0;
    let max_lev = p.max_leverage_bps as f64 / 10_000.0;
    let ex_size = 1_000.0;
    let ex_entry = 5_400.0;

    vec![
        Section {
            id: "overview",
            title: "What you trade",
            blocks: vec![
                Block::P(format!(
                    "TradingTime is a perpetual future on **how long SOL takes to move {band}%**, up or down. \
                     The price is quoted in time: a market at **1h 30m** expects SOL to need about an hour and a half \
                     to move {band}% from where it is now."
                )),
                Block::P(s(
                    "You never trade SOL's price. You trade its calm. When the market quietens, moves take longer \
                     and the price rises. When it gets wild, moves come faster and the price falls.",
                )),
                Block::Table {
                    head: row(&["You buy", "You win when", "You lose when"]),
                    rows: vec![
                        row(&["Long time", "SOL calms down (moves get slower, time rises)", "SOL gets wilder"]),
                        row(&["Short time", "SOL gets wilder (moves get faster, time falls)", "SOL calms down"]),
                    ],
                },
                Block::Note(s(
                    "The market never expires. Every position stays open until you close it or it is liquidated; \
                     funding keeps the price anchored to the index.",
                )),
            ],
        },
        Section {
            id: "price",
            title: "Reading the price",
            blocks: vec![
                Block::P(format!(
                    "For a random walk, the expected time to move ±{band}% is set by volatility σ. TradingTime applies a \
                     calibration factor c = {calib:.2}, fitted on real SOL data (fat tails make moves arrive about 10% sooner \
                     than the textbook formula)."
                )),
                Block::Formula("T = c · ln(1 + x)² / σ²", format!("x = {band}%, σ = volatility per √second")),
                Block::P(s(
                    "Because time goes as 1/σ², it amplifies volatility moves: when volatility halves, time quadruples.",
                )),
                Block::Table {
                    head: row(&["Annualised volatility", "Time to move 1%"]),
                    rows: [0.4, 0.6, 0.8, 1.0, 1.5, 2.0]
                        .iter()
                        .map(|v| vec![format!("{:.0}%", v * 100.0), duration(time_at_vol(&p, *v))])
                        .collect(),
                },
            ],
        },
        Section {
            id: "pnl",
            title: "Profit and loss",
            blocks: vec![
                Block::P(s(
                    "Positions are sized in USDC. PnL is **logarithmic in time**: a position of size q earns q times the \
                     log change of the price. Doubling and halving are mirror images.",
                )),
                Block::Formula("PnL = q · ln(T_exit / T_entry)", s("q in USDC, positive for long time, negative for short time")),
                Block::P(format!(
                    "Example: long time {} at {}.",
                    money(ex_size),
                    duration(ex_entry)
                )),
                Block::Table {
                    head: row(&["Time moves to", "Change", "Long time PnL", "Short time PnL"]),
                    rows: [2.0, 1.5, 1.1, 0.9, 0.67, 0.5]
                        .iter()
                        .map(|m| {
                            let v = pnl(ex_size, ex_entry, ex_entry * m);
                            vec![
                                duration(ex_entry * m),
                                format!("×{m}"),
                                format!("{}{}", if v >= 0.0 { "+" } else { "−" }, money(v.abs())),
                                format!("{}{}", if v <= 0.0 { "+" } else { "−" }, money(v.abs())),
                            ]
                        })
                        .collect(),
                },
                Block::Note(format!(
                    "At {max_lev}× leverage a long time position loses its whole margin if time falls to about 37% of \
                     entry (ln 0.37 ≈ −1). The ticket shows the exact level as \"Closed out at\"."
                )),
            ],
        },
        Section {
            id: "index",
            title: "The index",
            blocks: vec![
                Block::P(format!(
                    "The index is the protocol's reference time. It is computed on-chain from the **Pyth SOL/USD** price: \
                     every new Pyth update adds its squared log return to an exponentially weighted average of variance \
                     with a **{half_life_days:.0}-day half-life**, and the index is the time-to-move that variance implies."
                )),
                Block::Formula("index = c · ln(1 + x)² / EWMA(r² / Δt)", format!("half-life {half_life_days:.0} days")),
                Block::List(vec![
                    format!("Bounded between {} and {}.", duration(p.index_min_secs as f64), duration(p.index_max_secs as f64)),
                    format!("Pyth prices older than {} s are rejected.", p.pyth_max_staleness_secs),
                    s("Anyone can crank it; the keeper does so on every Pyth update."),
                ]),
                Block::P(s(
                    "A slow index is deliberate: it makes fair value forecastable, which keeps makers' spreads tight. \
                     The market price can move ahead of it; funding pulls the two together.",
                )),
            ],
        },
        Section {
            id: "mark",
            title: "Mark price",
            blocks: vec![
                Block::P(format!(
                    "The mark is the **median centre of the makers' fresh curves**. It values positions, drives funding, and \
                     decides liquidations. If no curve has been fresh for {} s, the mark falls back to the index.",
                    p.mark_stale_secs
                )),
                Block::P(s("The order book on the trade page shows the aggregated curves: asks fill long time, bids fill short time.")),
            ],
        },
        Section {
            id: "funding",
            title: "Funding",
            blocks: vec![
                Block::P(format!(
                    "Funding is paid continuously between longs and shorts. When the mark is above the index, longs pay \
                     shorts; when it is below, shorts pay longs. The full gap is paid over {funding_days:.0} day."
                )),
                Block::Formula("funding per day = q · ln(mark / index)", format!("period {funding_days:.0} day, settled on every account touch")),
                Block::P(format!(
                    "Example: mark 10% above the index, long {}: you pay about {} per day.",
                    money(ex_size),
                    money(ex_size * 1.1f64.ln())
                )),
            ],
        },
        Section {
            id: "makers",
            title: "Makers and routing",
            blocks: vec![
                Block::P(s(
                    "Liquidity comes from **competing makers** (propAMMs). Each posts a pricing curve backed by its own \
                     capital and refreshes it every slot from its own volatility forecast.",
                )),
                Block::Formula("price(size) = centre ± (half_spread + impact · size)", s("centre = fair − impact · inventory")),
                Block::List(vec![
                    format!(
                        "Your market order is split across up to {} makers in small chunks, each chunk going to the best price.",
                        tradingtime_interface::MAX_OPERATORS_PER_IX
                    ),
                    format!("A curve older than {} slots is ignored.", p.curve_max_age_slots),
                    format!(
                        "Anyone can become a maker: post a {} bond with at least {} of capital. Unbonding takes {}.",
                        usdc(p.operator_bond),
                        usdc(p.operator_min_capital),
                        duration(p.unbond_delay_secs as f64)
                    ),
                    s("Makers take the other side of trades but are never auto-deleveraged."),
                ]),
            ],
        },
        Section {
            id: "orders",
            title: "Orders",
            blocks: vec![
                Block::P(s(
                    "**Market orders** fill immediately across the makers' curves. They are all-or-nothing, with a \
                     slippage guard just past the worst visible level.",
                )),
                Block::P(format!(
                    "**Limit orders** rest on-chain until the market reaches your time. Keepers fill them (partially \
                     if needed) for a {} fee. An order expires after 7 days in the app; you can cancel any time, and \
                     anyone can close an expired order (its rent goes back to you).",
                    bps(p.keeper_fee_bps)
                )),
                Block::P(s(
                    "Resting orders reserve margin only for the exposure they could add: an order that reduces your \
                     position needs no extra collateral until it would flip it.",
                )),
            ],
        },
        Section {
            id: "margin",
            title: "Margin and liquidation",
            blocks: vec![
                Block::Table {
                    head: row(&["Parameter", "Value"]),
                    rows: vec![
                        vec![s("Maximum leverage"), format!("{max_lev}×")],
                        vec![s("Maintenance margin"), format!("{} of position", bps(p.maintenance_bps))],
                        vec![s("Liquidation fee"), format!("{} of closed size, to the insurance fund", bps(p.liq_fee_bps))],
                        vec![s("Collateral"), s("USDC")],
                    ],
                },
                Block::P(s(
                    "When equity falls below maintenance, anyone can liquidate the position. The protocol closes it \
                     against the makers' curves down to the **bankruptcy price** (where your equity would be zero).",
                )),
                Block::P(s(
                    "If the curves cannot absorb it there, the rest is **auto-deleveraged (ADL)**: closed at the \
                     bankruptcy price against traders on the other side who are in profit, pro rata. The insurance fund \
                     covers any negative equity first. Losses never exceed your deposit.",
                )),
            ],
        },
        Section {
            id: "fees",
            title: "Fees",
            blocks: vec![
                Block::Table {
                    head: row(&["Fee", "Rate", "Goes to"]),
                    rows: vec![
                        vec![s("Taker"), bps(p.taker_fee_bps), s("Insurance fund, then treasury")],
                        vec![s("Limit order fill"), bps(p.keeper_fee_bps), s("The keeper that filled it")],
                        vec![s("Liquidation"), bps(p.liq_fee_bps), s("Insurance fund")],
                    ],
                },
                Block::P(s("Makers earn their spread; there is no separate maker fee or rebate.")),
            ],
        },
        Section {
            id: "account",
            title: "Your account",
            blocks: vec![
                Block::List(vec![
                    s("Connect any Solana wallet (Wallet Standard). One trading account per wallet."),
                    s("The first trade creates your account and deposits the USDC it needs, in one transaction."),
                    s("Withdraw any USDC not needed as margin for open positions and orders."),
                ]),
            ],
        },
        Section {
            id: "risks",
            title: "Risks",
            blocks: vec![
                Block::List(vec![
                    s("Time amplifies volatility: a move from 150% to 50% volatility multiplies time by 9."),
                    s("Volatility regimes change fast on SOL. In simulation, most leveraged retail traders lost at least half their capital in a year."),
                    s("If you are in profit when someone on the other side is liquidated through ADL, part of your position can be closed at their bankruptcy price."),
                    s("The index depends on the Pyth SOL/USD feed and on keepers cranking it."),
                ]),
                Block::Note(s("This is experimental software. Only trade what you can afford to lose.")),
            ],
        },
        Section {
            id: "glossary",
            title: "Glossary",
            blocks: vec![Block::Table {
                head: row(&["Term", "Meaning"]),
                rows: vec![
                    row(&["Time to move", "Expected seconds for SOL to move ±1%: the quoted price."]),
                    row(&["Long time", "Position that gains when markets calm down."]),
                    row(&["Short time", "Position that gains when markets get wilder."]),
                    row(&["Index", "7-day realised-volatility reference time from Pyth."]),
                    row(&["Mark", "Median of the makers' fresh curve centres."]),
                    row(&["Funding", "Continuous payment that pulls the mark towards the index."]),
                    row(&["Bankruptcy price", "Time at which a position's equity would be zero."]),
                    row(&["ADL", "Auto-deleveraging: closing in-profit opposite positions at the bankruptcy price."]),
                    row(&["Maker / operator", "Bonded liquidity provider quoting a pricing curve."]),
                    row(&["Keeper", "Permissionless bot that cranks the index, funding and mark, fills limit orders and liquidates."]),
                ],
            }],
        },
        Section {
            id: "developers",
            title: "For developers",
            blocks: vec![
                Block::P(s("Everything shown in the app is served by the indexer's public REST and WebSocket API:")),
                Block::Table {
                    head: row(&["Route", "Returns"]),
                    rows: vec![
                        row(&["GET /v1/market", "mark, index, funding, open interest, 24h volume"]),
                        row(&["GET /v1/candles?interval=1m…1d", "OHLC of the mark in seconds"]),
                        row(&["GET /v1/quotes", "aggregated maker ladder"]),
                        row(&["GET /v1/traders/{owner}", "account, position, equity, liquidation level"]),
                        row(&["GET /v1/traders/{owner}/fills | orders | funding", "account history"]),
                        row(&["GET /v1/analytics/summary | volume | leaderboard | makers | trades | liquidations", "protocol analytics"]),
                        row(&["WS /v1/ws", "subscribe to market, quotes, trades, trader:{owner}"]),
                    ],
                },
                Block::P(s("Run a full local stack with bot traders and makers: `just sim`. See the repository README.")),
            ],
        },
    ]
}

/// Section id from a docs hash such as `#/docs/funding`.
pub fn anchor(hash: &str) -> Option<String> {
    let h = hash.trim_start_matches('#').trim_start_matches('/');
    let rest = h.strip_prefix("docs")?.trim_start_matches('/');
    let id = rest.split(['?', '/']).next().unwrap_or("");
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_unique_and_non_empty() {
        let s = sections();
        let mut ids: Vec<&str> = s.iter().map(|x| x.id).collect();
        assert!(s.iter().all(|x| !x.blocks.is_empty()));
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), s.len());
    }

    #[test]
    fn numbers_follow_params() {
        let p = Params::DEFAULT;
        // 80% vol ≈ 1h 14m with the 0.91 calibration.
        let t = time_at_vol(&p, 0.8);
        assert!((t - 4_440.0).abs() < 60.0, "{t}");
        assert!((pnl(1_000.0, 5_400.0, 10_800.0) - 693.147).abs() < 0.01);
        assert!((pnl(1_000.0, 5_400.0, 2_700.0) + 693.147).abs() < 0.01);
        assert_eq!(bps(5), "5 bp (0.05%)");
        assert_eq!(bps(100), "1%");
        let fees = sections().into_iter().find(|s| s.id == "fees").unwrap();
        let Block::Table { rows, .. } = &fees.blocks[0] else { panic!() };
        assert_eq!(rows[0][1], bps(p.taker_fee_bps));
    }

    #[test]
    fn bold_spans_and_anchors() {
        assert_eq!(spans("a **b** c"), vec![(false, "a ".into()), (true, "b".into()), (false, " c".into())]);
        assert_eq!(spans("no **close"), vec![(false, "no **close".into())]);
        assert_eq!(anchor("#/docs/funding"), Some("funding".into()));
        assert_eq!(anchor("#/docs"), None);
        assert_eq!(anchor("#/analytics"), None);
    }
}
