use super::{trade, State};
use crate::vm::analytics::Page;
use crate::vm::format::{money, short_addr};
use dioxus::prelude::*;

const PAGES: [(Page, &str, &str, &str); 3] = [
    (Page::Trade, "#/", "nav-trade", "Trade"),
    (Page::Analytics, "#/analytics", "nav-analytics", "Analytics"),
    (Page::Docs, "#/docs", "nav-docs", "Docs"),
];

#[component]
fn Links(testids: bool) -> Element {
    let s = use_context::<State>();
    let page = *s.page.read();
    rsx! {
        for (p, href, testid, label) in PAGES {
            a {
                class: if page == p { "navlink on" } else { "navlink" },
                href,
                "data-testid": if testids { testid } else { "" },
                "aria-current": if page == p { "page" } else { "false" },
                onclick: move |e| {
                    e.prevent_default();
                    super::go(s, p);
                },
                "{label}"
            }
        }
    }
}

#[component]
pub fn Nav() -> Element {
    let mut s = use_context::<State>();
    let owner = s.owner.read().clone();
    let light = *s.light.read();
    let equity = s.trader.read().as_ref().map(|t| money(t.equity_usdc));
    rsx! {
        nav { class: "nav", "aria-label": "Main",
            div { class: "left",
                span { class: "wordmark", "TradingTime" }
                div { class: "navlinks", Links { testids: true } }
            }
            div { class: "right",
                if let Some(e) = equity {
                    span { class: "num equity", title: "Account equity", "{e}" }
                }
                button { class: "btn ghost theme", onclick: move |_| s.light.set(!light), if light { "Dark" } else { "Light" } }
                if let Some(o) = owner {
                    span { class: "num addr", "data-testid": "wallet-address", "data-address": "{o}", title: "{o}", "{short_addr(&o)}" }
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
}

/// Phone bottom navigation (hidden on desktop by CSS).
#[component]
pub fn TabBar() -> Element {
    rsx! {
        nav { class: "tabbar", "aria-label": "Sections", Links { testids: false } }
    }
}
