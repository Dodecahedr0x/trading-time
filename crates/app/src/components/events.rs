use super::{fmt_dur, fmt_usd};
use dioxus::prelude::*;
use timeperp::agents::AgentKind;
use timeperp::engine::{EventKind, Sim};

#[component]
pub fn EventLog(sim: Signal<Sim>) -> Element {
    let mut show_agents = use_signal(|| false);
    let s = sim.read();
    let lines: Vec<(usize, &'static str, String)> = s
        .events
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(i, e)| {
            let at = fmt_dur(e.t);
            let (class, text) = match &e.kind {
                EventKind::Hit { duration, predicted, up } => (
                    "hit",
                    format!(
                        "{} band hit after {} (model said {})",
                        if *up { "↑" } else { "↓" },
                        fmt_dur(*duration),
                        fmt_dur(*predicted)
                    ),
                ),
                EventKind::Liquidated { owner, agent, payout } => {
                    ("liq", format!("liquidated {agent:?}#{owner}, returned {}", fmt_usd(*payout)))
                }
                EventKind::Deleveraged { owner, agent, payout } => {
                    ("shock", format!("auto-deleveraged {agent:?}#{owner}, paid {}", fmt_usd(*payout)))
                }
                EventKind::Shock => ("shock", "volatility regime flipped".into()),
                EventKind::Opened { agent, owner, side, notional, price }
                    if show_agents() || *agent == AgentKind::User =>
                {
                    ("", format!("{agent:?}#{owner} opened {side:?} {} @ {}", fmt_usd(*notional), fmt_dur(*price)))
                }
                EventKind::Closed { agent, owner, pnl, .. } if show_agents() || *agent == AgentKind::User => {
                    ("", format!("{agent:?}#{owner} closed, PnL {}", fmt_usd(*pnl)))
                }
                EventKind::Rejected { agent: AgentKind::User, reason, .. } => ("liq", format!("rejected: {reason}")),
                _ => return None,
            };
            Some((i, class, format!("{at}  {text}")))
        })
        .take(80)
        .collect();

    rsx! {
        div { class: "panel",
            h2 { "Events" }
            label { class: "field",
                "Show agent trades"
                input {
                    r#type: "checkbox",
                    style: "width:auto",
                    checked: show_agents(),
                    onchange: move |e| show_agents.set(e.checked()),
                }
            }
            div { class: "events",
                for (i, class, text) in lines {
                    div { key: "{i}", class, "{text}" }
                }
            }
        }
    }
}
