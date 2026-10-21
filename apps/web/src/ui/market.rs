use super::State;
use crate::vm::market::market_bar;
use dioxus::prelude::*;

#[component]
pub fn MarketBar() -> Element {
    let s = use_context::<State>();
    let m = s.market.read().clone().unwrap_or_default();
    let b = market_bar(&m);
    let change_class = if b.change_calm { "num calm" } else { "num wild" };
    rsx! {
        section { class: "panel mkt", "aria-label": "Market",
            div { class: "mbar",
                div { class: "stat",
                    span { class: "title", "SOL \u{b7} time to move 1%" }
                    span { class: "sub", "Perpetual \u{b7} 1\u{d7} max \u{b7} settles via funding" }
                }
                div { class: "stat",
                    span { class: "lbl", "Mark" }
                    span { class: "mark-readout", "data-testid": "market-mark", "{b.mark}" }
                }
                div { class: "stat",
                    span { class: "lbl", "7-day index" }
                    span { class: "num", "data-testid": "market-index", "{b.index}" }
                }
                div { class: "stat",
                    span { class: "lbl", "24h change" }
                    span { class: change_class, "{b.change}" }
                }
                div { class: "stat opt",
                    span { class: "lbl", "SOL volatility" }
                    span { class: "num", "{b.volatility}" }
                }
                div { class: "stat opt",
                    span { class: "lbl", "24h volume" }
                    span { class: "num", "{b.volume}" }
                }
                div { class: "stat opt",
                    span { class: "lbl", "Open interest" }
                    span { class: "num", "{b.open_interest}" }
                }
                div { class: "stat opt",
                    span { class: "lbl", "Funding / hour" }
                    span { class: "num", "{b.funding_text}" }
                }
                if b.paused {
                    span { class: "lbl wild", "New positions paused" }
                }
            }
            p { class: "explain", "data-testid": "explain-line",
                "You trade how long SOL takes to move 1% up or down, not its price. "
                strong { class: "calm", "Long time" }
                " gains when moves get slower (calmer market). "
                strong { class: "wild", "Short time" }
                " gains when moves get faster (wilder market)."
            }
        }
    }
}
