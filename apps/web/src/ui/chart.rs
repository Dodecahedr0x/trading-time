use super::State;
use crate::vm::chart::plot;
use crate::vm::{timeframe_secs, TIMEFRAMES};
use dioxus::prelude::*;

const W: f64 = 760.0;
const H: f64 = 380.0;
const PLOT_W: f64 = 672.0;
const PLOT_TOP: f64 = 8.0;
const PLOT_H: f64 = 344.0;

#[component]
pub fn Chart() -> Element {
    let mut s = use_context::<State>();
    let candles = s.candles.read().clone();
    let index: Vec<(i64, f64)> = s.index_points.read().iter().map(|p| (p.t, p.index_secs)).collect();
    let iv = *s.interval.read();
    let p = plot(&candles, &index, timeframe_secs(iv), PLOT_W, PLOT_H);
    let label = |y: f64| y + PLOT_TOP;
    rsx! {
        section { class: "panel chart-panel", "aria-label": "Chart",
            div { class: "phead",
                div { class: "chips", role: "group", "aria-label": "Candle length",
                    for (v, text) in TIMEFRAMES {
                        button {
                            class: if iv == v { "chip on" } else { "chip" },
                            "aria-pressed": if iv == v { "true" } else { "false" },
                            onclick: move |_| {
                                s.interval.set(v);
                                spawn(super::refresh_market(s));
                            },
                            "{text}"
                        }
                    }
                }
                div { class: "legend",
                    span { i { class: "sw calm" } "Moves got slower" }
                    span { i { class: "sw wild" } "Moves got faster" }
                    span { i { class: "sw dash" } "7-day index" }
                }
            }
            svg { class: "chart", "data-testid": "chart", view_box: "0 0 {W} {H}", role: "img",
                "aria-label": "Candles of SOL time to move 1% with the 7-day index",
                for a in p.grid.iter() {
                    line { class: "grid", x1: "8", x2: "{PLOT_W + 8.0}", y1: "{label(a.y)}", y2: "{label(a.y)}" }
                    text { x: "{PLOT_W + 18.0}", y: "{label(a.y) + 4.0}", "{a.label}" }
                }
                for (x, t) in p.times.iter() {
                    text { x: "{x + 8.0}", y: "372", text_anchor: "middle", "{t}" }
                }
                g { transform: "translate(8 {PLOT_TOP})",
                    if !p.index_path.is_empty() {
                        path { class: "idx", d: "{p.index_path}" }
                    }
                    for b in p.bars.iter() {
                        line {
                            x1: "{b.x + b.w / 2.0}", x2: "{b.x + b.w / 2.0}", y1: "{b.wick_top}", y2: "{b.wick_bottom}",
                            stroke: if b.calm { "var(--calm)" } else { "var(--wild)" }, stroke_width: "1",
                        }
                        rect {
                            x: "{b.x}", y: "{b.body_top}", width: "{b.w}", height: "{b.body_h}",
                            fill: if b.calm { "var(--calm)" } else { "var(--wild)" },
                        }
                    }
                }
                if let Some(y) = p.now_y {
                    line { class: "now", x1: "8", x2: "{PLOT_W + 8.0}", y1: "{label(y)}", y2: "{label(y)}" }
                    rect { class: "now-tag", x: "{PLOT_W + 10.0}", y: "{label(y) - 10.0}", width: "76", height: "20", rx: "2" }
                    text { class: "now-text", x: "{PLOT_W + 48.0}", y: "{label(y) + 4.0}", text_anchor: "middle", "{p.now_label}" }
                }
                if candles.is_empty() {
                    text { x: "16", y: "40", "No trades yet on this timeframe" }
                }
            }
        }
    }
}
