use dioxus::prelude::*;
use tradingtime::engine::Sim;

#[derive(Clone, PartialEq)]
pub struct Params {
    pub feed: String,
    pub csv_path: String,
    pub seed: u64,
    pub band_pct: f64,
    pub half_life_h: f64,
    /// Max model-price move per hour in percent; 0 = uncapped.
    pub cap_pct_per_hour: f64,
    /// Implied-vol weight in the oracle, 0..1.
    pub iv_weight: f64,
    /// Settle PnL on ln(time) instead of time.
    pub log_quote: bool,
    pub adl: bool,
    /// Annualized vol in percent; also the GBM/jump diffusion vol.
    pub calm_vol: f64,
    pub hot_vol: f64,
    pub switches_per_day: f64,
    pub noise: usize,
    pub informed: usize,
    pub momentum: usize,
    pub chaser: usize,
    pub fast: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            feed: "regime".into(),
            csv_path: "data/sample.csv".into(),
            seed: 1,
            band_pct: 1.0,
            half_life_h: 4.0,
            cap_pct_per_hour: 0.0,
            iv_weight: 0.0,
            log_quote: false,
            adl: false,
            calm_vol: 50.0,
            hot_vol: 150.0,
            switches_per_day: 1.0,
            noise: 20,
            informed: 2,
            momentum: 3,
            chaser: 2,
            fast: 2,
        }
    }
}

#[component]
fn NumField(label: &'static str, value: f64, onchange: EventHandler<f64>) -> Element {
    rsx! {
        label { class: "field",
            "{label}"
            input {
                r#type: "number",
                step: "any",
                value: "{value}",
                onchange: move |e| {
                    if let Ok(v) = e.value().parse() {
                        onchange.call(v)
                    }
                },
            }
        }
    }
}

#[component]
pub fn Controls(
    params: Signal<Params>,
    running: Signal<bool>,
    speed: Signal<u32>,
    sim: Signal<Sim>,
    on_reset: EventHandler<()>,
) -> Element {
    let p = params();
    let feed = p.feed.clone();
    rsx! {
        aside {
            h1 { "TradingTime" }
            div { class: "tagline", "Trade the time for price to move ±x%. Long = calmer, short = wilder." }

            h2 { "Run" }
            div { class: "row",
                button {
                    class: "primary",
                    onclick: move |_| running.toggle(),
                    if running() { "Pause" } else { "Play" }
                }
                button { onclick: move |_| { sim.write().step(); }, "Step" }
                button { onclick: move |_| { running.set(false); on_reset.call(()) }, "Reset" }
            }
            label { class: "field",
                "Bars / tick"
                select {
                    value: "{speed}",
                    onchange: move |e| speed.set(e.value().parse().unwrap_or(10)),
                    for n in [1u32, 5, 10, 30, 100, 500] {
                        option { value: "{n}", selected: speed() == n, "{n}" }
                    }
                }
            }
            div { class: "row",
                button {
                    title: "Flip the volatility regime (regime feed only)",
                    onclick: move |_| sim.write().shock(),
                    "⚡ Shock regime"
                }
            }

            h2 { "Feed (applies on reset)" }
            label { class: "field",
                "Source"
                select {
                    value: "{feed}",
                    onchange: move |e| params.write().feed = e.value(),
                    for (v, l) in [("regime", "Regime switch"), ("gbm", "GBM"), ("jump", "GBM + jumps"), ("csv", "CSV file")] {
                        option { value: v, selected: feed == v, "{l}" }
                    }
                }
            }
            if feed == "csv" {
                input {
                    class: "wide",
                    value: "{p.csv_path}",
                    placeholder: "path/to/bars.csv",
                    onchange: move |e| params.write().csv_path = e.value(),
                }
            }
            NumField { label: "Seed", value: p.seed as f64, onchange: move |v: f64| params.write().seed = v as u64 }
            NumField { label: "Vol (calm) %", value: p.calm_vol, onchange: move |v| params.write().calm_vol = v }
            if feed == "regime" {
                NumField { label: "Vol (hot) %", value: p.hot_vol, onchange: move |v| params.write().hot_vol = v }
                NumField { label: "Switches / day", value: p.switches_per_day, onchange: move |v| params.write().switches_per_day = v }
            }

            h2 { "Market" }
            NumField { label: "Band ±%", value: p.band_pct, onchange: move |v| params.write().band_pct = v }
            NumField { label: "EWMA half-life h", value: p.half_life_h, onchange: move |v| params.write().half_life_h = v }
            label { class: "field",
                "Log-time PnL"
                input {
                    r#type: "checkbox",
                    style: "width:auto",
                    checked: p.log_quote,
                    onchange: move |e| params.write().log_quote = e.checked(),
                }
            }
            label { class: "field",
                "Auto-deleverage"
                input {
                    r#type: "checkbox",
                    style: "width:auto",
                    checked: p.adl,
                    onchange: move |e| params.write().adl = e.checked(),
                }
            }
            NumField { label: "Implied vol weight", value: p.iv_weight, onchange: move |v: f64| params.write().iv_weight = v.clamp(0.0, 1.0) }
            NumField { label: "Max move %/h (0=off)", value: p.cap_pct_per_hour, onchange: move |v| params.write().cap_pct_per_hour = v }

            h2 { "Agents" }
            NumField { label: "Noise", value: p.noise as f64, onchange: move |v: f64| params.write().noise = v as usize }
            NumField { label: "Informed", value: p.informed as f64, onchange: move |v: f64| params.write().informed = v as usize }
            NumField { label: "Momentum", value: p.momentum as f64, onchange: move |v: f64| params.write().momentum = v as usize }
            NumField { label: "Chaser", value: p.chaser as f64, onchange: move |v: f64| params.write().chaser = v as usize }
            NumField { label: "Fast-vol arb", value: p.fast as f64, onchange: move |v: f64| params.write().fast = v as usize }
        }
    }
}
