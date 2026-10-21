use super::{trade, State};
use crate::vm::format::{duration, hms, money, money_whole, parse_amount, parse_duration, DASH};
use crate::vm::payoff::{buy_label, payoff};
use crate::vm::ticket::*;
use crate::vm::Side;
use dioxus::prelude::*;

#[component]
pub fn Ticket() -> Element {
    let mut s = use_context::<State>();
    let side = *s.side.read();
    let kind = *s.kind.read();
    let size = parse_amount(&s.size.read()).unwrap_or(0.0);
    let limit_secs = parse_duration(&s.limit.read());
    let trader = s.trader.read().clone();
    let quotes = s.quotes.read().clone();
    let mark = s.market.read().as_ref().map(|m| m.mark_secs).filter(|v| *v > 0.0).unwrap_or(quotes.mark_secs);
    let wallet_usdc = *s.wallet_usdc.read();
    let max = max_size(trader.as_ref(), wallet_usdc, side);
    let entry = match kind {
        OrderType::Market => estimate_market(&quotes, side, size).map(|e| e.entry_secs),
        OrderType::Limit => limit_secs,
    }
    .unwrap_or(mark);
    let fee = size * TAKER_FEE_BPS as f64 / 10_000.0;
    let fee_bp = TAKER_FEE_BPS as f64;
    let impact_bp = if mark > 0.0 && entry > 0.0 && kind == OrderType::Market { (entry / mark).ln().abs() * 1e4 } else { 0.0 };
    let dep = deposit_needed(trader.as_ref(), side, kind, size);
    let equity_after = trader.as_ref().map(|t| t.equity_usdc).unwrap_or(0.0) + dep - fee;
    let p = payoff(side, size, entry, q_after(trader.as_ref(), side, size), equity_after, MAINTENANCE_BPS);
    let label = buy_label(side, size);
    let busy = *s.busy.read();
    let status = s.tx_status.read().clone();
    let calm = side == Side::Long;
    let tone = if calm { "long" } else { "short" };
    let side_name = if calm { "Long time" } else { "Short time" };
    let at = if entry > 0.0 { duration(entry) } else { DASH.into() };
    let sentence = format!(
        "{} of {side_name} at {at}. You gain if SOL\u{2019}s 1% moves take {} than {at}, as measured by the 7-day index.",
        money_whole(size),
        if calm { "longer" } else { "less time" },
    );
    let (twice, half) = (duration(entry * 2.0), duration(entry / 2.0));
    let (up_label, up, down_label, down) = if calm {
        (format!("If it slows to {twice}"), p.if_double.clone(), format!("If it speeds up to {half}"), p.if_half.clone())
    } else {
        (format!("If it speeds up to {half}"), p.if_half.clone(), format!("If it slows to {twice}"), p.if_double.clone())
    };
    let status_cls = match status.as_deref() {
        Some("Confirmed") => "txs gain",
        Some(t) if t.starts_with("Failed") => "txs loss",
        _ => "txs muted",
    };
    let on_buy = move |_| {
        let limit = parse_duration(&s.limit.peek());
        let size = parse_amount(&s.size.peek()).unwrap_or(0.0);
        trade::run_tx(s, async move {
            if s.owner.peek().is_none() {
                trade::connect(s).await?;
            }
            trade::submit(s, side, kind, size, limit).await
        });
    };
    rsx! {
        section { class: "panel ticket-panel", "aria-label": "Order ticket",
            div { class: "ticket",
                div { class: "tabs", role: "group", "aria-label": "Order type",
                    button { class: if kind == OrderType::Market { "on" } else { "" }, "data-testid": "order-type-market",
                        onclick: move |_| s.kind.set(OrderType::Market), "Market" }
                    button { class: if kind == OrderType::Limit { "on" } else { "" }, "data-testid": "order-type-limit",
                        onclick: move |_| {
                            s.kind.set(OrderType::Limit);
                            if s.limit.peek().is_empty() && mark > 0.0 {
                                s.limit.set(duration(mark));
                            }
                        },
                        "Limit" }
                }
                div { class: "sides", role: "group", "aria-label": "What you are buying",
                    button { class: if calm { "long on" } else { "long" }, "data-testid": "side-long", "aria-pressed": "{calm}",
                        onclick: move |_| s.side.set(Side::Long),
                        span { class: "s", "Long time" } span { class: "h", "moves get slower" } }
                    button { class: if calm { "short" } else { "short on" }, "data-testid": "side-short", "aria-pressed": "{!calm}",
                        onclick: move |_| s.side.set(Side::Short),
                        span { class: "s", "Short time" } span { class: "h", "moves get faster" } }
                }
                div { class: "avail", span { class: "muted", "Available" } span { class: "num", "{money(max)}" } }
                if kind == OrderType::Limit {
                    div { class: "field",
                        label { class: "lbl", r#for: "tt-limit", "Limit time" }
                        input { id: "tt-limit", "data-testid": "limit-input", value: "{s.limit}", placeholder: "1h 30m",
                            oninput: move |e| s.limit.set(e.value()) }
                    }
                }
                div { class: "field",
                    label { class: "lbl", r#for: "tt-size", "Size (USD)" }
                    input { id: "tt-size", "data-testid": "size-input", value: "{s.size}", inputmode: "decimal",
                        oninput: move |e| s.size.set(e.value()) }
                    div { class: "presets",
                        for pct in [25u32, 50, 75, 100] {
                            button { onclick: move |_| s.size.set(format!("{}", preset(max, pct))), "{pct}%" }
                        }
                    }
                }
                div { class: "buying {tone}", "data-testid": "you-are-buying",
                    span { class: "lbl", "You are buying" }
                    p { "{sentence}" }
                    dl { class: "kv",
                        dt { "{up_label}" } dd { class: "gain", "{up}" }
                        dt { "{down_label}" } dd { class: "loss", "{down}" }
                        dt { "Closed out at" } dd { "{p.closed_out}" }
                    }
                }
                dl { class: "kv",
                    dt { if kind == OrderType::Market { "Average fill" } else { "Limit" } }
                    dd { if entry > 0.0 { "{hms(entry)}" } else { "{DASH}" } }
                    if kind == OrderType::Market {
                        dt { "Spread and size impact" } dd { "{impact_bp:.1}bp" }
                    }
                    dt { "Fee" } dd { "{money(fee)}" }
                    if kind == OrderType::Market {
                        dt { "Total cost" } dd { "{impact_bp + fee_bp:.1}bp" }
                    }
                    if dep > 0.0 {
                        dt { "Deposit first" } dd { "{money(dep)}" }
                    }
                    dt { "Leverage" } dd { "1\u{d7}" }
                }
                button { class: "buy {tone}", "data-testid": "buy-button", disabled: busy || size <= 0.0, onclick: on_buy, "{label}" }
                span { class: status_cls, "data-testid": "tx-status", {status.unwrap_or_default()} }
            }
        }
    }
}
