use super::{trade, State};
use crate::vm::format::pct_signed;
use crate::vm::rows::*;
use crate::vm::ticket::OrderType;
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
        Side::Long => "l side calm",
        Side::Short => "l side wild",
    }
}

#[component]
pub fn Account() -> Element {
    let s = use_context::<State>();
    let mut tab = use_signal(|| Tab::Positions);
    let t = *tab.read();
    let mark = s.market.read().as_ref().map(|m| m.mark_secs).unwrap_or(0.0);
    let trader = s.trader.read().clone();
    let positions = position_rows(trader.as_ref(), mark);
    let roe = trader.as_ref().filter(|t| t.q_usdc != 0.0).map(|t| pct_signed(t.upnl_usdc / t.q_usdc.abs(), 2)).unwrap_or_default();
    let q = trader.as_ref().map(|t| t.q_usdc).unwrap_or(0.0);
    let orders = order_rows(&s.orders.read());
    let fills = fill_rows(&s.fills.read());
    let funding = funding_rows(&s.funding.read());
    let busy = *s.busy.read();
    let cls = |x: Tab| if t == x { "on" } else { "" };
    let close = move |_| {
        let side = if q > 0.0 { Side::Short } else { Side::Long };
        trade::run_tx(s, trade::submit(s, side, OrderType::Market, q.abs(), None));
    };
    rsx! {
        section { class: "panel acct", "aria-label": "Account",
            div { class: "tabs", role: "tablist",
                button { class: cls(Tab::Positions), role: "tab", "data-testid": "tab-positions", onclick: move |_| tab.set(Tab::Positions), "Positions ({positions.len()})" }
                button { class: cls(Tab::Orders), role: "tab", "data-testid": "tab-orders", onclick: move |_| tab.set(Tab::Orders), "Open orders ({orders.len()})" }
                button { class: cls(Tab::Trades), role: "tab", onclick: move |_| tab.set(Tab::Trades), "Trade history" }
                button { class: cls(Tab::Funding), role: "tab", onclick: move |_| tab.set(Tab::Funding), "Funding history" }
            }
            div { class: "scroll",
                match t {
                    Tab::Positions => rsx! {
                        if positions.is_empty() {
                            p { class: "empty", "No open position. Pick Long time or Short time in the ticket to open one." }
                        } else {
                            table { class: "positions",
                                thead { tr {
                                    th { class: "l", "Market" } th { class: "l", "Side" } th { "Size" } th { "Entry" } th { "Mark" }
                                    th { "Closed out at" } th { "Funding" } th { "PnL (return)" } th { "Close" }
                                } }
                                tbody {
                                    for r in positions {
                                        tr { "data-testid": "positions-row",
                                            td { class: "l sans", "SOL time to move 1%" }
                                            td { class: side_cls(r.side), "{r.side_label}" }
                                            td { "{r.size}" } td { "{r.entry}" } td { "{r.mark}" } td { "{r.liq}" }
                                            td { class: if r.funding_positive { "gain" } else { "loss" }, "{r.funding}" }
                                            td { class: if r.upnl_positive { "gain" } else { "loss" }, "{r.upnl} ({roe})" }
                                            td { button { class: "btn sm", disabled: busy, onclick: close, "Market close" } }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Tab::Orders => rsx! {
                        if orders.is_empty() {
                            p { class: "empty", "No open orders. Limit orders you place appear here until they fill or you cancel them." }
                        } else {
                            table {
                                thead { tr { th { class: "l", "Side" } th { "Size" } th { "Filled" } th { "Limit" } th { "Expires" } th { "" } } }
                                tbody {
                                    for r in orders {
                                        tr { "data-testid": "orders-row",
                                            td { class: side_cls(r.side), "{r.side_label}" }
                                            td { "{r.size}" } td { "{r.filled}" } td { "{r.limit}" } td { "{r.expiry}" }
                                            td { button { class: "btn sm", onclick: move |_| trade::run_tx(s, trade::cancel(s, r.order_id)), "Cancel" } }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Tab::Trades => rsx! {
                        if fills.is_empty() {
                            p { class: "empty", "No trades yet." }
                        } else {
                            table {
                                thead { tr { th { class: "l", "Time" } th { class: "l", "Action" } th { "Size" } th { "Fill" } th { "Fee" } } }
                                tbody {
                                    for r in fills {
                                        tr {
                                            td { class: "l", "{r.time}" }
                                            td { class: side_cls(r.side), "{r.kind} \u{b7} {r.side_label}" }
                                            td { "{r.size}" } td { "{r.price}" } td { "{r.fee}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Tab::Funding => rsx! {
                        if funding.is_empty() {
                            p { class: "empty", "Funding is settled continuously while you hold a position; payments appear here." }
                        } else {
                            table {
                                thead { tr { th { class: "l", "Time" } th { "Amount" } } }
                                tbody {
                                    for r in funding {
                                        tr { td { class: "l", "{r.time}" } td { class: if r.positive { "gain" } else { "loss" }, "{r.amount}" } }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}
