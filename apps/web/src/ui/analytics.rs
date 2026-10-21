//! Analytics page (.agent/analytics-api.md "Web"): polls the six indexer routes every 3 s while mounted.

use super::State;
use crate::api;
use crate::types::*;
use crate::vm::analytics::*;
use crate::vm::format::money_compact;
use crate::vm::{Side, TIMEFRAMES};
use dioxus::prelude::*;

const POLL_MS: u32 = 3_000;
const VOLUME_BUCKETS: u32 = 48;
const LEADERBOARD_LIMIT: u32 = 50;
const FEED_LIMIT: u32 = 30;

const VW: f64 = 800.0;
const VH: f64 = 220.0;
const V_AXIS: f64 = 64.0;
const V_FOOT: f64 = 20.0;

const HW: f64 = 400.0;
const HH: f64 = 150.0;
const H_FOOT: f64 = 20.0;
const H_BINS: usize = 12;

#[derive(Clone, Default, PartialEq)]
struct Data {
    summary: Option<AnalyticsSummaryJson>,
    volume: Vec<VolumeBucketJson>,
    leaders: Vec<LeaderboardRowJson>,
    makers: Vec<MakerRowJson>,
    trades: Vec<AnalyticsTradeJson>,
    liqs: Vec<LiquidationJson>,
    error: Option<String>,
    now: i64,
}

#[derive(Clone, Copy, PartialEq)]
struct Controls {
    interval: Signal<&'static str>,
    sort: Signal<&'static str>,
    makers: Signal<bool>,
    data: Signal<Data>,
}

fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

async fn refresh(s: State, mut c: Controls) {
    let api_url = s.cfg.peek().api_url.clone();
    let (iv, sort, mk) = (*c.interval.peek(), *c.sort.peek(), *c.makers.peek());
    let (summary, volume, leaders, makers, trades, liqs) = futures::join!(
        api::analytics_summary(&api_url),
        api::analytics_volume(&api_url, iv, VOLUME_BUCKETS),
        api::leaderboard(&api_url, sort, LEADERBOARD_LIMIT, mk),
        api::analytics_makers(&api_url),
        api::analytics_trades(&api_url, FEED_LIMIT),
        api::liquidations(&api_url, FEED_LIMIT),
    );
    let mut d = c.data.peek().clone();
    let mut err: Option<String> = None;
    fn take<T>(r: Result<T, String>, slot: &mut T, err: &mut Option<String>) {
        match r {
            Ok(v) => *slot = v,
            Err(e) => {
                err.get_or_insert(e);
            }
        }
    }
    let mut sum = d.summary.take();
    take(summary.map(Some), &mut sum, &mut err);
    d.summary = sum;
    take(volume, &mut d.volume, &mut err);
    take(leaders, &mut d.leaders, &mut err);
    take(makers, &mut d.makers, &mut err);
    take(trades, &mut d.trades, &mut err);
    take(liqs, &mut d.liqs, &mut err);
    d.error = err;
    d.now = now_secs();
    c.data.set(d);
}

fn copy(text: &str) {
    if let Ok(js) = serde_json::to_string(text) {
        let _ = document::eval(&format!("navigator.clipboard && navigator.clipboard.writeText({js})"));
    }
}

fn side_cls(side: Side) -> &'static str {
    match side {
        Side::Long => "side calm",
        Side::Short => "side wild",
    }
}

fn pnl_cls(gain: bool) -> &'static str {
    if gain { "gain" } else { "loss" }
}

#[component]
pub fn Analytics() -> Element {
    let s = use_context::<State>();
    let c = Controls {
        interval: use_signal(|| "5m"),
        sort: use_signal(|| "pnl"),
        makers: use_signal(|| false),
        data: use_signal(Data::default),
    };
    use_future(move || async move {
        loop {
            refresh(s, c).await;
            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
        }
    });
    let d = c.data.read().clone();
    rsx! {
        div { class: "analytics", "data-testid": "analytics-page",
            div { class: "kpis",
                for k in kpis(d.summary.as_ref()) {
                    div { class: "panel kpi",
                        span { class: "label", "{k.label}" }
                        span { class: "kpi-v num", "data-testid": k.id, "{k.value}" }
                        span { class: "kpi-sub muted", "{k.sub}" }
                    }
                }
            }
            if let Some(e) = d.error.clone() {
                div { class: "panel an-err loss", "Indexer: {e}" }
            }
            div { class: "an-grid",
                div { class: "an-col",
                    VolumeChart { c, buckets: d.volume.clone() }
                    Leaderboard { c, rows: d.leaders.clone(), now: d.now }
                    Makers { rows: d.makers.clone() }
                }
                div { class: "an-col",
                    PnlDistribution { pnls: trader_pnls(&d.leaders) }
                    TradesFeed { rows: d.trades.clone(), now: d.now }
                    Liquidations { rows: d.liqs.clone(), now: d.now }
                }
            }
        }
    }
}

#[component]
fn VolumeChart(c: Controls, buckets: Vec<VolumeBucketJson>) -> Element {
    let mut interval = c.interval;
    let s = use_context::<State>();
    let iv = *interval.read();
    let plot_w = VW - V_AXIS;
    let g = volume_layout(&buckets, plot_w, VH);
    let total: f64 = buckets.iter().map(|b| b.volume_usdc).sum();
    rsx! {
        div { class: "panel",
            div { class: "an-head",
                span { class: "label", "Volume \u{b7} " span { class: "calm", "long" } " / " span { class: "wild", "short" } }
                span { class: "muted num", "{money_compact(total)} in window" }
                span { class: "grow" }
                select {
                    "data-testid": "volume-interval",
                    "aria-label": "Volume interval",
                    value: iv,
                    onchange: move |e| {
                        if let Some((v, _)) = TIMEFRAMES.iter().find(|(v, _)| *v == e.value()) {
                            interval.set(v);
                            spawn(refresh(s, c));
                        }
                    },
                    for (i, label) in TIMEFRAMES {
                        option { value: i, selected: iv == i, "{label}" }
                    }
                }
            }
            svg { class: "an-chart", "data-testid": "volume-chart", view_box: "0 0 {VW} {VH + V_FOOT}",
                for t in g.y_ticks {
                    line { x1: "0", x2: "{plot_w}", y1: "{t.pos}", y2: "{t.pos}", stroke: "var(--rule)", stroke_width: "1" }
                    text { x: "{plot_w + 6.0}", y: "{(t.pos + 4.0).max(11.0)}", "{t.label}" }
                }
                for b in g.bars {
                    g {
                        title { "{b.title}" }
                        rect { x: "{b.x}", y: "{b.long_y}", width: "{b.w.max(1.0)}", height: "{b.long_h}", fill: "var(--calm)" }
                        rect { x: "{b.x}", y: "{b.short_y}", width: "{b.w.max(1.0)}", height: "{b.short_h}", fill: "var(--wild)" }
                    }
                }
                line { x1: "0", x2: "{plot_w}", y1: "{VH}", y2: "{VH}", stroke: "var(--rule)", stroke_width: "1" }
                for t in g.x_ticks {
                    text { x: "{t.pos}", y: "{VH + 15.0}", text_anchor: "middle", "{t.label}" }
                }
                if buckets.is_empty() {
                    text { x: "12", y: "24", "No volume yet" }
                }
            }
        }
    }
}

#[component]
fn Owner(addr: String, short: String) -> Element {
    let a = addr.clone();
    rsx! {
        button { class: "addr num", title: "{addr}", "aria-label": "Copy {addr}", onclick: move |_| copy(&a), "{short}" }
    }
}

#[component]
fn Leaderboard(c: Controls, rows: Vec<LeaderboardRowJson>, now: i64) -> Element {
    let s = use_context::<State>();
    let (mut sort, mut makers) = (c.sort, c.makers);
    let srt = *sort.read();
    let mk = *makers.read();
    let rows = leaderboard_rows(&rows, now);
    let none = now > 0 && rows.is_empty();
    rsx! {
        div { class: "panel",
            div { class: "an-head",
                span { class: "label", "Leaderboard" }
                span { class: "muted", "click an address to copy" }
                span { class: "grow" }
                select {
                    "data-testid": "leaderboard-sort",
                    "aria-label": "Sort leaderboard by",
                    value: srt,
                    onchange: move |e| {
                        if let Some((v, _)) = SORTS.iter().find(|(v, _)| *v == e.value()) {
                            sort.set(v);
                            spawn(refresh(s, c));
                        }
                    },
                    for (v , label) in SORTS {
                        option { value: v, selected: srt == v, "{label}" }
                    }
                }
                button {
                    class: if mk { "toggle on" } else { "toggle" },
                    "data-testid": "leaderboard-makers",
                    "aria-pressed": "{mk}",
                    onclick: move |_| {
                        makers.set(!mk);
                        spawn(refresh(s, c));
                    },
                    "Makers"
                }
            }
            div { class: "scroll",
                table {
                    thead { tr { th { "#" } th { "Account" } th { "Equity" } th { "PnL" } th { "PnL %" } th { "Position" } th { "Volume" } th { "Trades" } th { "Last trade" } } }
                    tbody {
                        for r in rows {
                            tr { "data-testid": "leaderboard-row",
                                td { class: "muted", "{r.rank}" }
                                td {
                                    Owner { addr: r.owner.clone(), short: r.owner_short.clone() }
                                    if r.maker { span { class: "badge", "maker" } }
                                }
                                td { "{r.equity}" }
                                td { class: pnl_cls(r.gain), "{r.pnl}" }
                                td { class: pnl_cls(r.gain), "{r.pnl_pct}" }
                                td { class: r.position_side.map(side_cls).unwrap_or("muted"), "{r.position}" }
                                td { "{r.volume}" } td { "{r.trades}" }
                                td { class: "muted", "{r.last_trade}" }
                            }
                        }
                    }
                }
                if none { div { class: "empty", "No traders yet" } }
            }
        }
    }
}

#[component]
fn Makers(rows: Vec<MakerRowJson>) -> Element {
    let rows = maker_rows(&rows);
    rsx! {
        div { class: "panel",
            div { class: "an-head", span { class: "label", "Makers" } }
            div { class: "scroll",
                table {
                    thead { tr { th { "Maker" } th { "Status" } th { "Bond" } th { "Equity" } th { "PnL" } th { "Inventory" } th { "Volume" } th { "Fills" } th { "Half spread" } th { "Fair" } } }
                    tbody {
                        for r in rows {
                            tr { "data-testid": "makers-row",
                                td { Owner { addr: r.owner.clone(), short: r.owner_short.clone() } }
                                td { class: "side",
                                    span { class: if r.fresh { "dot gain" } else { "dot muted" }, title: if r.fresh { "quotes fresh" } else { "quotes stale" }, "\u{25cf} " }
                                    "{r.status}"
                                }
                                td { "{r.bond}" } td { "{r.equity}" }
                                td { class: pnl_cls(r.gain), "{r.pnl}" }
                                td { "{r.inventory}" } td { "{r.volume}" } td { "{r.fills}" } td { "{r.spread}" } td { "{r.fair}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PnlDistribution(pnls: Vec<f64>) -> Element {
    let bins = pnl_histogram(&pnls, H_BINS, HW, HH);
    let edge = bins.last().map(|b| b.hi).unwrap_or(0.0);
    let (lo, hi) = (money_compact(-edge), money_compact(edge));
    let profit = pnls.iter().filter(|p| **p > 0.0).count();
    rsx! {
        div { class: "panel",
            div { class: "an-head",
                span { class: "label", "Trader PnL distribution" }
                span { class: "grow" }
                span { class: "muted num", "{profit}/{pnls.len()} in profit" }
            }
            svg { class: "an-hist", "data-testid": "pnl-distribution", view_box: "0 0 {HW} {HH + H_FOOT}",
                for b in bins.iter() {
                    rect { x: "{b.x}", y: "{b.y}", width: "{b.w}", height: "{b.h}", fill: if b.gain { "var(--gain)" } else { "var(--loss)" },
                        title { "{money_compact(b.lo)} to {money_compact(b.hi)}: {b.count}" }
                    }
                }
                line { x1: "{HW / 2.0}", x2: "{HW / 2.0}", y1: "0", y2: "{HH}", stroke: "var(--muted)", stroke_width: "1", stroke_dasharray: "3 3" }
                line { x1: "0", x2: "{HW}", y1: "{HH}", y2: "{HH}", stroke: "var(--rule)", stroke_width: "1" }
                if bins.is_empty() {
                    text { x: "12", y: "24", "No traders yet" }
                } else {
                    text { x: "0", y: "{HH + 15.0}", "{lo}" }
                    text { x: "{HW / 2.0}", y: "{HH + 15.0}", text_anchor: "middle", "$0" }
                    text { x: "{HW}", y: "{HH + 15.0}", text_anchor: "end", "{hi}" }
                }
            }
        }
    }
}

#[component]
fn TradesFeed(rows: Vec<AnalyticsTradeJson>, now: i64) -> Element {
    let rows = trade_rows(&rows, now);
    let none = now > 0 && rows.is_empty();
    rsx! {
        div { class: "panel",
            div { class: "an-head", span { class: "label", "Live trades" } span { class: "grow" } span { class: "live-dot gain", "\u{25cf} live" } }
            div { class: "scroll feed",
                table {
                    thead { tr { th { "Time" } th { "Trader" } th { "Side" } th { "Size" } th { "Price" } } }
                    tbody {
                        for r in rows {
                            tr { "data-testid": "analytics-trade-row", title: "{r.kind} \u{b7} fee {r.fee}",
                                td { class: "muted", "{r.time}" }
                                td { Owner { addr: r.trader.clone(), short: r.trader_short.clone() } }
                                td { class: side_cls(r.side), "{r.side_label}" }
                                td { "{r.size}" } td { "{r.price}" }
                            }
                        }
                    }
                }
                if none { div { class: "empty", "No trades yet" } }
            }
        }
    }
}

#[component]
fn Liquidations(rows: Vec<LiquidationJson>, now: i64) -> Element {
    let rows = liquidation_rows(&rows, now);
    let none = now > 0 && rows.is_empty();
    rsx! {
        div { class: "panel",
            div { class: "an-head", span { class: "label", "Liquidations" } }
            div { class: "scroll feed",
                table {
                    thead { tr { th { "Time" } th { "Trader" } th { "Size" } th { "Bankrupt at" } th { "Penalty" } } }
                    tbody {
                        for r in rows {
                            tr { "data-testid": "liquidation-row",
                                title: "liquidator {r.liquidator} \u{b7} insurance used {r.insurance}",
                                td { class: "muted", "{r.time}" }
                                td { Owner { addr: r.trader.clone(), short: r.trader_short.clone() } }
                                td { "{r.size}" } td { "{r.bankruptcy}" } td { class: "loss", "{r.penalty}" }
                            }
                        }
                    }
                }
                if none { div { class: "empty", "No liquidations" } }
            }
        }
    }
}
