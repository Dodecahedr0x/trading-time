use super::State;
use crate::vm::format::duration;
use crate::vm::ladder::ladder;
use crate::vm::ticket::OrderType;
use dioxus::prelude::*;

const ROWS: usize = 8;

#[component]
pub fn Quotes() -> Element {
    let mut s = use_context::<State>();
    let q = s.quotes.read().clone();
    let l = ladder(&q, ROWS);
    let quoters = q.operators.iter().filter(|o| o.fresh).count();
    let mut pick = move |secs: f64| {
        s.kind.set(OrderType::Limit);
        s.limit.set(duration(secs));
    };
    rsx! {
        section { class: "panel quotes-panel", "aria-label": "Quotes",
            div { class: "phead",
                span { class: "title", "Quotes" }
                span { class: "meta", if quoters == 1 { "1 quoter" } else { "{quoters} quoters" } }
            }
            div { class: "lhead lbl", span { "Time" } span { "Size (USD)" } span { "Total" } }
            for r in l.asks {
                div { class: "lrow ask", "data-testid": "quotes-ask-row", title: "Long time fills here \u{b7} click to set a limit",
                    onclick: move |_| pick(r.time_secs),
                    span { class: "bar", style: "width:{r.depth * 100.0}%" }
                    span { class: "calm", "{r.time}" }
                    span { "{r.size}" }
                    span { class: "muted", "{r.total}" }
                }
            }
            div { class: "lmark",
                span { class: "v", "{l.mark}" }
                span { class: "muted", "Spread " span { class: "num", "{l.spread}" } }
            }
            for r in l.bids {
                div { class: "lrow bid", "data-testid": "quotes-bid-row", title: "Short time fills here \u{b7} click to set a limit",
                    onclick: move |_| pick(r.time_secs),
                    span { class: "bar", style: "width:{r.depth * 100.0}%" }
                    span { class: "wild", "{r.time}" }
                    span { "{r.size}" }
                    span { class: "muted", "{r.total}" }
                }
            }
            if quoters == 0 {
                div { class: "lempty", "No maker is quoting right now." }
            }
            p { class: "pnote", "Long time fills at the blue quotes above the mark; Short time at the orange quotes below. Times are h:mm:ss." }
        }
    }
}
