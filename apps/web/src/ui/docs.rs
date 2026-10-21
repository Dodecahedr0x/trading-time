use crate::vm::docs::{anchor, sections, spans, Block};
use dioxus::prelude::*;

/// Scrolls the section into view and records it in the hash without re-routing. False if it is not rendered yet.
fn show(id: &str, smooth: bool) -> bool {
    let Some(w) = web_sys::window() else { return false };
    let Some(el) = w.document().and_then(|d| d.get_element_by_id(&format!("doc-{id}"))) else { return false };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_behavior(if smooth { web_sys::ScrollBehavior::Smooth } else { web_sys::ScrollBehavior::Instant });
    opts.set_block(web_sys::ScrollLogicalPosition::Start);
    el.scroll_into_view_with_scroll_into_view_options(&opts);
    if let Ok(h) = w.history() {
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&format!("#/docs/{id}")));
    }
    true
}

#[component]
fn Text(text: String) -> Element {
    rsx! {
        for (bold, t) in spans(&text) {
            if bold { strong { "{t}" } } else { "{t}" }
        }
    }
}

#[component]
pub fn Docs() -> Element {
    let secs = sections();
    let toc: Vec<(&'static str, &'static str)> = secs.iter().map(|s| (s.id, s.title)).collect();
    let ids: Vec<&'static str> = toc.iter().map(|t| t.0).collect();
    let linked = web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .and_then(|h| anchor(&h))
        .and_then(|a| ids.iter().copied().find(|id| *id == a));
    let mut active = use_signal(|| linked.unwrap_or(ids[0]));
    // Deep link: jump once the section is in the DOM (layout can lag the first render on a cold load).
    let mut settled = use_signal(|| linked.is_none());
    use_effect(move || {
        if let Some(id) = linked {
            spawn(async move {
                for _ in 0..40 {
                    gloo_timers::future::TimeoutFuture::new(50).await;
                    if show(id, false) {
                        break;
                    }
                }
                gloo_timers::future::TimeoutFuture::new(200).await;
                settled.set(true);
            });
        }
    });
    // Highlight the last section whose heading has reached the top of the viewport.
    use_future(move || {
        let ids = ids.clone();
        async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(250).await;
                if !*settled.peek() {
                    continue;
                }
                let Some(w) = web_sys::window() else { continue };
                let Some(doc) = w.document() else { continue };
                let line = 120.0;
                let current = ids
                    .iter()
                    .copied()
                    .rev()
                    .find(|id| {
                        doc.get_element_by_id(&format!("doc-{id}"))
                            .is_some_and(|el| el.get_bounding_client_rect().top() <= line)
                    })
                    .unwrap_or(ids[0]);
                if *active.peek() != current {
                    active.set(current);
                }
            }
        }
    });
    rsx! {
        div { class: "docs", "data-testid": "docs-page",
            nav { class: "panel docs-toc", "data-testid": "docs-toc",
                div { class: "label", "Docs" }
                for (id, title) in toc {
                    a {
                        class: if *active.read() == id { "on" } else { "" },
                        href: "#/docs/{id}",
                        onclick: move |e| {
                            e.prevent_default();
                            active.set(id);
                            show(id, true);
                        },
                        "{title}"
                    }
                }
            }
            article { class: "panel docs-body",
                h1 { "TradingTime docs" }
                p { class: "muted", "How the time-to-move perpetual works, from the first trade to liquidation." }
                for sec in secs {
                    section { id: "doc-{sec.id}", class: "docs-section", "data-testid": "docs-section",
                        h2 { "{sec.title}" }
                        for block in sec.blocks {
                            match block {
                                Block::P(t) => rsx! { p { Text { text: t } } },
                                Block::List(items) => rsx! {
                                    ul { for it in items { li { Text { text: it } } } }
                                },
                                Block::Formula(f, cap) => rsx! {
                                    div { class: "formula", span { class: "num", "{f}" } span { class: "muted", "{cap}" } }
                                },
                                Block::Table { head, rows } => rsx! {
                                    table { class: "docs-table",
                                        thead { tr { for h in head { th { "{h}" } } } }
                                        tbody { for r in rows { tr { for c in r { td { "{c}" } } } } }
                                    }
                                },
                                Block::Note(t) => rsx! { div { class: "note", Text { text: t } } },
                            }
                        }
                    }
                }
            }
        }
    }
}
