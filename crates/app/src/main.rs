//! Desktop UI for the time-perp simulator.

mod components;

use components::{Charts, Controls, EventLog, Params, TradePanel};
use dioxus::prelude::*;
use std::time::Duration;
use timeperp::engine::{AgentMix, Sim, SimConfig};
use timeperp::feed::{CsvFeed, Model, PriceFeed, SynthFeed};
use timeperp::oracle::sigma_per_sec;

const CSS: &str = include_str!("style.css");

fn main() {
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(dioxus::desktop::WindowBuilder::new().with_title("Time Perp Simulator"))
                .with_menu(None),
        )
        .launch(App);
}

fn build(p: &Params) -> Result<Sim, String> {
    let feed: Box<dyn PriceFeed> = match p.feed.as_str() {
        "gbm" => Box::new(SynthFeed::new(Model::Gbm { sigma: sigma_per_sec(p.calm_vol / 100.0) }, p.seed, 150.0, 60.0)),
        "jump" => Box::new(SynthFeed::new(
            Model::Jump { sigma: sigma_per_sec(p.calm_vol / 100.0), lambda: 1.0 / 86_400.0, jump_sigma: 0.04 },
            p.seed,
            150.0,
            60.0,
        )),
        "csv" => Box::new(CsvFeed::load(&p.csv_path)?),
        _ => Box::new(SynthFeed::new(
            Model::Regime {
                sigmas: vec![sigma_per_sec(p.calm_vol / 100.0), sigma_per_sec(p.hot_vol / 100.0)],
                switch_prob: p.switches_per_day / 1_440.0,
            },
            p.seed,
            150.0,
            60.0,
        )),
    };
    let cfg = SimConfig {
        band_x: p.band_pct / 100.0,
        ewma_half_life: p.half_life_h * 3_600.0,
        seed: p.seed,
        agents: AgentMix { noise: p.noise, informed: p.informed, momentum: p.momentum, chaser: p.chaser, fast: p.fast },
        max_move_per_min: (p.cap_pct_per_hour > 0.0).then(|| (1.0 + p.cap_pct_per_hour / 100.0).ln() / 60.0),
        iv_weight: p.iv_weight,
        pool: timeperp::pool::PoolConfig { log_quote: p.log_quote, adl: p.adl, ..Default::default() },
        ..Default::default()
    };
    Sim::new(cfg, feed)
}

#[component]
fn App() -> Element {
    let params = use_signal(Params::default);
    let mut sim = use_signal(|| build(&Params::default()).expect("default sim builds"));
    let mut error = use_signal(|| None::<String>);
    let running = use_signal(|| false);
    let speed = use_signal(|| 10u32);

    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if running() {
                let mut s = sim.write();
                for _ in 0..speed() {
                    if !s.step() {
                        break;
                    }
                }
            }
        }
    });

    let reset = move |_| match build(&params.read()) {
        Ok(s) => {
            sim.set(s);
            error.set(None);
        }
        Err(e) => error.set(Some(e)),
    };

    rsx! {
        style { {CSS} }
        div { class: "layout",
            Controls { params, running, speed, sim, on_reset: reset }
            main {
                if let Some(e) = error() {
                    div { class: "error", "{e}" }
                }
                Charts { sim }
                div { class: "bottom",
                    TradePanel { sim }
                    EventLog { sim }
                }
            }
        }
    }
}
