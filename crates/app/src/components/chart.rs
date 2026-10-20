use dioxus::prelude::*;
use timeperp::engine::Sim;

const W: f64 = 600.0;
const H: f64 = 170.0;

#[derive(Clone, PartialEq)]
struct Series {
    label: &'static str,
    color: &'static str,
    points: Vec<(f64, f64)>,
}

/// Line chart over shared x/y extents. Gaps (NaN) split a series into segments.
#[component]
fn Chart(title: &'static str, unit: &'static str, series: Vec<Series>) -> Element {
    let finite = || series.iter().flat_map(|s| s.points.iter()).filter(|p| p.1.is_finite());
    let (x0, x1) = finite().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (mut y0, mut y1) = finite().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    if y0 > y1 {
        (y0, y1) = (0.0, 1.0);
    }
    let pad = ((y1 - y0) * 0.08).max(y1.abs() * 1e-6).max(1e-9);
    let (y0, y1) = (y0 - pad, y1 + pad);
    let sx = |x: f64| if x1 > x0 { (x - x0) / (x1 - x0) * W } else { 0.0 };
    let sy = |y: f64| H - (y - y0) / (y1 - y0) * H;

    let paths: Vec<(String, &str)> = series
        .iter()
        .map(|s| {
            let mut d = String::new();
            let mut pen_down = false;
            for &(x, y) in &s.points {
                if y.is_finite() {
                    d.push_str(&format!("{}{:.1},{:.1} ", if pen_down { "L" } else { "M" }, sx(x), sy(y)));
                    pen_down = true;
                } else {
                    pen_down = false;
                }
            }
            (d, s.color)
        })
        .collect();

    rsx! {
        div { class: "panel chart",
            h2 { "{title}" }
            svg {
                view_box: "0 0 {W} {H}",
                preserve_aspect_ratio: "none",
                for (d, color) in paths {
                    path { d: "{d}", fill: "none", stroke: color, stroke_width: "1.5", vector_effect: "non-scaling-stroke" }
                }
            }
            div { class: "legend",
                for s in series.iter() {
                    span { style: "--c: {s.color}", "{s.label}" }
                }
                span { class: "muted", style: "--c: transparent", "{y0:.1} – {y1:.1} {unit}" }
            }
        }
    }
}

#[component]
pub fn Charts(sim: Signal<Sim>) -> Element {
    let s = sim.read();
    let h = &s.history;
    let mins = |v: f64| v / 60.0;
    let line = |label, color, f: &dyn Fn(&timeperp::engine::Snapshot) -> f64| Series {
        label,
        color,
        points: h.iter().map(|p| (p.t, f(p))).collect(),
    };

    let price = vec![
        line("spot", "#d7dce5", &|p| p.price),
        line("upper", "#3ecf8e", &|p| p.upper),
        line("lower", "#ff6b6b", &|p| p.lower),
    ];
    let time = vec![
        line("model (traded)", "#6aa9ff", &|p| mins(p.model)),
        line("uncapped target", "#6aa9ff66", &|p| mins(p.target)),
        line("implied-vol E[τ]", "#5fd3d3", &|p| p.implied_model.map_or(f64::NAN, mins)),
        line("true E[τ]", "#f5b84a", &|p| p.true_model.map_or(f64::NAN, mins)),
        line("realized index", "#b48cff", &|p| p.index.map_or(f64::NAN, mins)),
    ];
    let lp = vec![line("LP equity", "#3ecf8e", &|p| p.lp_equity / 1e3)];
    let oi = vec![line("long OI", "#3ecf8e", &|p| p.long_oi / 1e3), line("short OI", "#ff6b6b", &|p| p.short_oi / 1e3)];

    rsx! {
        div { class: "charts",
            Chart { title: "Spot & ±band", unit: "", series: price }
            Chart { title: "Time to move ±band", unit: "min", series: time }
            Chart { title: "LP equity", unit: "k$", series: lp }
            Chart { title: "Open interest", unit: "k$", series: oi }
        }
    }
}
