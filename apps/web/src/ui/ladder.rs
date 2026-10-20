use super::State;
use crate::vm::format::duration;
use crate::vm::ladder::ladder;
use crate::vm::ticket::OrderType;
use dioxus::prelude::*;

const ROWS: usize = 8;

#[component]
pub fn Quotes() -> Element {
    let mut s = use_context::<State>();
    let l = ladder(&s.quotes.read(), ROWS);
    let mut pick = move |secs: f64| {
        s.kind.set(OrderType::Limit);
        s.limit.set(duration(secs));
    };
    rsx! {
        div { class: "panel",
            div { class: "line", span { class: "label", "Quotes" } span { class: "label", "Spread {l.spread}" } }
            div { class: "ladder",
                div { class: "lrow label", span { "Time" } span { "Size" } span { "Total" } }
                for r in l.asks {
                    div { class: "lrow ask", "data-testid": "quotes-ask-row", title: "Long time fills here",
                        onclick: move |_| pick(r.time_secs),
                        div { class: "bar", style: "width:{r.depth * 100.0}%" }
                        span { class: "calm", "{r.time}" }
                        span { "{r.size}" }
                        span { class: "muted", "{r.total}" }
                    }
                }
                div { class: "lmark", span { class: "label", "Mark" } span { class: "v", "{l.mark}" } }
                for r in l.bids {
                    div { class: "lrow bid", "data-testid": "quotes-bid-row", title: "Short time fills here",
                        onclick: move |_| pick(r.time_secs),
                        div { class: "bar", style: "width:{r.depth * 100.0}%" }
                        span { class: "wild", "{r.time}" }
                        span { "{r.size}" }
                        span { class: "muted", "{r.total}" }
                    }
                }
            }
        }
    }
}
