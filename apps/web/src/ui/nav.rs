use super::{trade, State};
use crate::vm::format::short_addr;
use dioxus::prelude::*;

#[component]
pub fn Nav() -> Element {
    let mut s = use_context::<State>();
    let owner = s.owner.read().clone();
    let light = *s.light.read();
    rsx! {
        div { class: "panel nav",
            span { class: "wordmark", "TradingTime" }
            span { class: "market-name", "SOL \u{b7} time to move 1%" }
            span { class: "grow" }
            button { class: "theme", onclick: move |_| s.light.set(!light), if light { "Dark" } else { "Light" } }
            if let Some(o) = owner {
                span { class: "num", "data-testid": "wallet-address", "data-address": "{o}", title: "{o}", "{short_addr(&o)}" }
            } else {
                button {
                    class: "connect",
                    "data-testid": "connect-wallet",
                    onclick: move |_| {
                        spawn(async move {
                            if let Err(e) = trade::connect(s).await {
                                s.tx_status.set(Some(format!("Failed: {e}")));
                            }
                        });
                    },
                    "Connect wallet"
                }
            }
        }
    }
}
