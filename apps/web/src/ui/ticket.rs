use super::{trade, State};
use crate::vm::format::{duration, money, parse_amount, parse_duration, DASH};
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
    let dep = deposit_needed(trader.as_ref(), side, kind, size);
    let equity_after = trader.as_ref().map(|t| t.equity_usdc).unwrap_or(0.0) + dep - fee;
    let p = payoff(side, size, entry, q_after(trader.as_ref(), side, size), equity_after, MAINTENANCE_BPS);
    let label = buy_label(side, size);
    let (double_cls, half_cls) = if p.if_double_gain { ("v gain", "v loss") } else { ("v loss", "v gain") };
    let busy = *s.busy.read();
    let status = s.tx_status.read().clone();
    let (long_cls, short_cls) = match side {
        Side::Long => ("long on", "short"),
        Side::Short => ("long", "short on"),
    };
    let buy_cls = match side {
        Side::Long => "buy long",
        Side::Short => "buy short",
    };
    let funding = s
        .market
        .read()
        .as_ref()
        .map(|m| crate::vm::format::pct_signed(m.funding_rate_per_hour, 4) + "/h")
        .unwrap_or_else(|| DASH.into());
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
        div { class: "ticket",
            div { class: "tabs",
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
            div { class: "sides",
                button { class: long_cls, "data-testid": "side-long", onclick: move |_| s.side.set(Side::Long), "Long time" }
                button { class: short_cls, "data-testid": "side-short", onclick: move |_| s.side.set(Side::Short), "Short time" }
            }
            if kind == OrderType::Limit {
                label { class: "label", "Limit time to move 1%" }
                input { "data-testid": "limit-input", value: "{s.limit}", placeholder: "1h 30m",
                    oninput: move |e| s.limit.set(e.value()) }
            }
            div { class: "line", span { class: "label", "Size (USDC)" } span { class: "label", "Max {money(max)}" } }
            input { "data-testid": "size-input", value: "{s.size}", inputmode: "decimal",
                oninput: move |e| s.size.set(e.value()) }
            div { class: "presets",
                for pct in [25u32, 50, 75, 100] {
                    button { onclick: move |_| s.size.set(format!("{}", preset(max, pct))), "{pct}%" }
                }
            }
            div { class: "buying", "data-testid": "you-are-buying",
                span { class: "head", "{p.headline}" }
                span { "{p.sentence}" }
                div { class: "line", span { "If time doubles" } span { class: double_cls, "{p.if_double}" } }
                div { class: "line", span { "If time halves" } span { class: half_cls, "{p.if_half}" } }
                div { class: "line", span { "Closed out at" } span { class: "v", "{p.closed_out}" } }
            }
            div { class: "line", span { class: "muted", "Est. entry" } span { class: "v", "{duration(entry)}" } }
            div { class: "line", span { class: "muted", "Fee (5 bp)" } span { class: "v", "{money(fee)}" } }
            if dep > 0.0 {
                div { class: "line", span { class: "muted", "Deposit first" } span { class: "v", "{money(dep)}" } }
            }
            div { class: "line", span { class: "muted", "Funding" } span { class: "v", "{funding}" } }
            button { class: buy_cls, "data-testid": "buy-button", disabled: busy || size <= 0.0, onclick: on_buy, "{label}" }
            span { class: status_cls, "data-testid": "tx-status", {status.unwrap_or_default()} }
        }
    }
}
