use super::State;
use crate::vm::chart::layout;
use dioxus::prelude::*;

const W: f64 = 800.0;
const H: f64 = 360.0;
const AXIS_W: f64 = 64.0;

/// (wick x, wick top, wick bottom, body x, body top, body w, body h, colour)
type Bar = (f64, f64, f64, f64, f64, f64, f64, &'static str);

#[component]
pub fn Chart() -> Element {
    let mut s = use_context::<State>();
    let candles = s.candles.read().clone();
    let iv = *s.interval.read();
    let plot_w = W - AXIS_W;
    let g = layout(&candles, plot_w, H, 5);
    let axis: Vec<(f64, f64, String)> = g.axis.iter().map(|a| (a.y, a.y + 3.0, a.label.clone())).collect();
    let bars: Vec<Bar> = g
        .bars
        .iter()
        .map(|b| {
            let c = if b.calm { "var(--calm)" } else { "var(--wild)" };
            (b.x + b.w / 2.0, b.wick_top, b.wick_bottom, b.x, b.body_top, b.w.max(1.0), b.body_h, c)
        })
        .collect();
    rsx! {
        div { class: "panel",
            div { class: "chart-head",
                span { class: "label", "Time to move 1% \u{b7} blue = moves got slower, orange = faster" }
                div { class: "seg",
                    for i in ["1h", "4h", "1d"] {
                        button {
                            class: if iv == i { "on" } else { "" },
                            onclick: move |_| {
                                s.interval.set(i);
                                spawn(super::refresh_market(s));
                            },
                            "{i}"
                        }
                    }
                }
            }
            svg { class: "chart", "data-testid": "chart", view_box: "0 0 {W} {H}",
                for (y , ty , label) in axis {
                    line { x1: "0", x2: "{plot_w}", y1: "{y}", y2: "{y}", stroke: "var(--rule)", stroke_width: "1" }
                    text { x: "{plot_w + 6.0}", y: "{ty}", "{label}" }
                }
                for (cx , wt , wb , x , bt , w , bh , c) in bars {
                    line { x1: "{cx}", x2: "{cx}", y1: "{wt}", y2: "{wb}", stroke: c, stroke_width: "1" }
                    rect { x: "{x}", y: "{bt}", width: "{w}", height: "{bh}", fill: c }
                }
                if candles.is_empty() {
                    text { x: "12", y: "24", "No candles yet" }
                }
            }
        }
    }
}
