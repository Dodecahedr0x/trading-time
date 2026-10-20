use super::{trade, State};
use crate::vm::rows::*;
use crate::vm::Side;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Positions,
    Orders,
    Trades,
    Funding,
}

fn side_cls(side: Side) -> &'static str {
    match side {
        Side::Long => "side calm",
        Side::Short => "side wild",
    }
}

#[component]
pub fn Account() -> Element {
    let s = use_context::<State>();
    let mut tab = use_signal(|| Tab::Positions);
    let t = *tab.read();
    let mark = s.market.read().as_ref().map(|m| m.mark_secs).unwrap_or(0.0);
    let positions = position_rows(s.trader.read().as_ref(), mark);
    let orders = order_rows(&s.orders.read());
    let fills = fill_rows(&s.fills.read());
    let funding = funding_rows(&s.funding.read());
    let cls = |x: Tab| if t == x { "on" } else { "" };
    rsx! {
        div { class: "tabs",
            button { class: cls(Tab::Positions), "data-testid": "tab-positions", onclick: move |_| tab.set(Tab::Positions), "Positions ({positions.len()})" }
            button { class: cls(Tab::Orders), "data-testid": "tab-orders", onclick: move |_| tab.set(Tab::Orders), "Open orders ({orders.len()})" }
            button { class: cls(Tab::Trades), onclick: move |_| tab.set(Tab::Trades), "Trade history" }
            button { class: cls(Tab::Funding), onclick: move |_| tab.set(Tab::Funding), "Funding history" }
        }
        match t {
            Tab::Positions => rsx! {
                table {
                    thead { tr { th { "Side" } th { "Size" } th { "Entry" } th { "Mark" } th { "Unrealised" } th { "Equity" } th { "Closed out at" } th { "Funding" } } }
                    tbody {
                        for r in positions {
                            tr { "data-testid": "positions-row",
                                td { class: side_cls(r.side), "{r.side_label}" }
                                td { "{r.size}" } td { "{r.entry}" } td { "{r.mark}" }
                                td { class: if r.upnl_positive { "gain" } else { "loss" }, "{r.upnl}" }
                                td { "{r.equity}" } td { "{r.liq}" }
                                td { class: if r.funding_positive { "gain" } else { "loss" }, "{r.funding}" }
                            }
                        }
                    }
                }
                if s.trader.read().as_ref().is_none_or(|t| t.q_usdc == 0.0) { div { class: "empty", "No open position" } }
            },
            Tab::Orders => rsx! {
                table {
                    thead { tr { th { "Side" } th { "Size" } th { "Filled" } th { "Limit" } th { "Expires" } th { "" } } }
                    tbody {
                        for r in orders {
                            tr { "data-testid": "orders-row",
                                td { class: side_cls(r.side), "{r.side_label}" }
                                td { "{r.size}" } td { "{r.filled}" } td { "{r.limit}" } td { "{r.expiry}" }
                                td { button { onclick: move |_| trade::run_tx(s, trade::cancel(s, r.order_id)), "Cancel" } }
                            }
                        }
                    }
                }
            },
            Tab::Trades => rsx! {
                table {
                    thead { tr { th { "Time" } th { "Side" } th { "Kind" } th { "Size" } th { "Price" } th { "Fee" } } }
                    tbody {
                        for r in fills {
                            tr { td { "{r.time}" } td { class: side_cls(r.side), "{r.side_label}" } td { "{r.kind}" } td { "{r.size}" } td { "{r.price}" } td { "{r.fee}" } }
                        }
                    }
                }
            },
            Tab::Funding => rsx! {
                table {
                    thead { tr { th { "Time" } th { "Amount" } } }
                    tbody {
                        for r in funding {
                            tr { td { "{r.time}" } td { class: if r.positive { "gain" } else { "loss" }, "{r.amount}" } }
                        }
                    }
                }
            },
        }
    }
}
