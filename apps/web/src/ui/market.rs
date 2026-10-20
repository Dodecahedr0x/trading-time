use super::State;
use crate::vm::market::{market_bar, EXPLAIN};
use dioxus::prelude::*;

#[component]
pub fn MarketBar() -> Element {
    let s = use_context::<State>();
    let m = s.market.read().clone().unwrap_or_default();
    let b = market_bar(&m);
    let change_class = if b.change_calm { "num calm" } else { "num wild" };
    rsx! {
        div { class: "panel",
            div { class: "mbar",
                div { class: "stat",
                    span { class: "label", "Mark \u{b7} time to move 1%" }
                    span { class: "mark-readout", "data-testid": "market-mark", "{b.mark}" }
                }
                div { class: "stat",
                    span { class: "label", "7-day index" }
                    span { class: "num", "data-testid": "market-index", "{b.index}" }
                }
                div { class: "stat",
                    span { class: "label", "24h change" }
                    span { class: change_class, "{b.change}" }
                }
                div { class: "stat opt",
                    span { class: "label", "24h volume" }
                    span { class: "num", "{b.volume}" }
                }
                div { class: "stat opt",
                    span { class: "label", "Open interest" }
                    span { class: "num", "{b.open_interest}" }
                }
                div { class: "stat opt",
                    span { class: "label", "Funding" }
                    span { class: "num", "{b.funding} " span { class: "muted", "{b.funding_payer}" } }
                }
                if b.paused {
                    span { class: "label wild", "New positions paused" }
                }
            }
            p { class: "explain", "data-testid": "explain-line", "{EXPLAIN}" }
        }
    }
}
