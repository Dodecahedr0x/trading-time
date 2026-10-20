use super::{fmt_dur, fmt_usd};
use dioxus::prelude::*;
use tradingtime::agents::Action;
use tradingtime::engine::{Sim, USER};
use tradingtime::pool::Side;

#[component]
fn Stat(k: &'static str, v: String) -> Element {
    rsx! {
        div { class: "stat",
            div { class: "k", "{k}" }
            div { class: "v", "{v}" }
        }
    }
}

fn signed_class(v: f64) -> &'static str {
    if v >= 0.0 { "pos" } else { "neg" }
}

#[component]
pub fn TradePanel(sim: Signal<Sim>) -> Element {
    let mut collateral = use_signal(|| 1_000.0f64);
    let mut leverage = use_signal(|| 2.0f64);
    let mut msg = use_signal(String::new);

    let mut open = move |side: Side| {
        let r = sim.write().execute(USER, Action::Open { side, collateral: collateral(), leverage: leverage() });
        msg.set(r.err().unwrap_or_default());
    };

    let s = sim.read();
    let sigma_ann = s.sigma_hat() * (365.0f64 * 86_400.0).sqrt() * 100.0;
    let funding_hr = s.pool.funding_rate() * 3_600.0 * 100.0;
    let worth = s.net_worth(USER);
    let pnl = worth - s.slots[USER].initial;
    let elapsed = s.t - s.band.ref_time();
    let rows: Vec<_> = s
        .pool
        .positions
        .values()
        .filter(|p| p.owner == USER)
        .map(|p| {
            let upnl = p.pnl(s.model) - s.pool.carry_owed(p);
            (p.id, p.side, p.entry, upnl, s.pool.equity(p, s.model), s.pool.liq_price(p))
        })
        .collect();

    rsx! {
        div { class: "panel",
            h2 { "Market" }
            div { class: "stats",
                Stat { k: "Model E[τ] (mark)", v: fmt_dur(s.model) }
                Stat { k: "True E[τ]", v: s.true_model().map_or("—".into(), fmt_dur) }
                Stat { k: "Realized index", v: s.band.realized_index().map_or("—".into(), fmt_dur) }
                Stat { k: "σ̂ (annualized)", v: format!("{sigma_ann:.0}%") }
                Stat { k: "Since band reset", v: fmt_dur(elapsed) }
                Stat { k: "Funding (longs pay)", v: format!("{funding_hr:+.4}%/h") }
                Stat { k: "LP equity", v: fmt_usd(s.pool.lp_equity(s.model)) }
                Stat { k: "Calibration", v: format!("{:.2}× (n={})", s.calibration.ratio(), s.calibration.n) }
                Stat { k: "Sim time", v: fmt_dur(s.t) }
            }

            h2 { "Your account" }
            div { class: "stats",
                Stat { k: "Wallet", v: fmt_usd(s.slots[USER].wallet) }
                Stat { k: "Net worth", v: fmt_usd(worth) }
                div { class: "stat",
                    div { class: "k", "PnL" }
                    div { class: "v {signed_class(pnl)}", "{fmt_usd(pnl)}" }
                }
            }
            div { class: "row",
                label { class: "field",
                    "Collateral $"
                    input {
                        r#type: "number",
                        value: "{collateral}",
                        onchange: move |e| collateral.set(e.value().parse().unwrap_or(collateral())),
                    }
                }
                label { class: "field",
                    "Leverage"
                    input {
                        r#type: "number",
                        step: "0.5",
                        value: "{leverage}",
                        onchange: move |e| leverage.set(e.value().parse().unwrap_or(leverage())),
                    }
                }
            }
            div { class: "row",
                button { class: "long", onclick: move |_| open(Side::Long), "Long time (calmer)" }
                button { class: "short", onclick: move |_| open(Side::Short), "Short time (wilder)" }
            }
            if !msg().is_empty() {
                div { class: "neg", "{msg}" }
            }

            if !rows.is_empty() {
                table {
                    tr {
                        th { "Side" }
                        th { "Entry" }
                        th { "Liq at" }
                        th { "uPnL" }
                        th { "Equity" }
                        th {}
                    }
                    for (id, side, entry, upnl, equity, liq) in rows {
                        tr { key: "{id}",
                            td { class: if side == Side::Long { "pos" } else { "neg" }, "{side:?}" }
                            td { "{fmt_dur(entry)}" }
                            td { "{fmt_dur(liq)}" }
                            td { class: signed_class(upnl), "{fmt_usd(upnl)}" }
                            td { "{fmt_usd(equity)}" }
                            td {
                                button {
                                    onclick: move |_| {
                                        let r = sim.write().execute(USER, Action::Close { id });
                                        msg.set(r.err().unwrap_or_default());
                                    },
                                    "Close"
                                }
                            }
                        }
                    }
                }
            }
            div { class: "hint",
                "Prices are seconds of expected time for spot to leave the ±band. "
                "PnL per unit = Δ time. Fills at the model price ± skew impact, 0.1% fee each way."
            }
        }
    }
}
