mod account;
mod chart;
mod ladder;
mod market;
mod nav;
mod style;
mod ticket;
pub mod trade;

use crate::config::Config;
use crate::types::*;
use crate::vm::ticket::OrderType;
use crate::vm::Side;
use crate::{api, chain};
use dioxus::prelude::*;
use futures::StreamExt;
use wallet_adapter::WalletAdapter;

#[derive(Clone, Copy)]
pub struct State {
    pub cfg: Signal<Config>,
    pub market: Signal<Option<MarketJson>>,
    pub quotes: Signal<QuotesJson>,
    pub candles: Signal<Vec<CandleJson>>,
    pub interval: Signal<&'static str>,
    pub owner: Signal<Option<String>>,
    pub wallet: Signal<Option<WalletAdapter>>,
    pub trader: Signal<Option<TraderJson>>,
    pub orders: Signal<Vec<OrderJson>>,
    pub fills: Signal<Vec<FillJson>>,
    pub funding: Signal<Vec<FundingJson>>,
    pub wallet_usdc: Signal<f64>,
    pub tx_status: Signal<Option<String>>,
    pub busy: Signal<bool>,
    pub light: Signal<bool>,
    pub side: Signal<Side>,
    pub kind: Signal<OrderType>,
    pub size: Signal<String>,
    pub limit: Signal<String>,
}

impl State {
    fn new() -> State {
        State {
            cfg: Signal::new(Config::load()),
            market: Signal::new(None),
            quotes: Signal::new(QuotesJson::default()),
            candles: Signal::new(vec![]),
            interval: Signal::new("1h"),
            owner: Signal::new(None),
            wallet: Signal::new(None),
            trader: Signal::new(None),
            orders: Signal::new(vec![]),
            fills: Signal::new(vec![]),
            funding: Signal::new(vec![]),
            wallet_usdc: Signal::new(0.0),
            tx_status: Signal::new(None),
            busy: Signal::new(false),
            light: Signal::new(false),
            side: Signal::new(Side::Long),
            kind: Signal::new(OrderType::Market),
            size: Signal::new("5,000".into()),
            limit: Signal::new(String::new()),
        }
    }
}

pub async fn refresh_market(mut s: State) {
    let api_url = s.cfg.peek().api_url.clone();
    if let Ok(m) = api::market(&api_url).await {
        s.market.set(Some(m));
    }
    if let Ok(q) = api::quotes(&api_url).await {
        s.quotes.set(q);
    }
    let iv = *s.interval.peek();
    if let Ok(c) = api::candles(&api_url, iv, 120).await {
        s.candles.set(c);
    }
}

pub async fn refresh_account(mut s: State) {
    let Some(owner) = s.owner.peek().clone() else { return };
    let (api_url, rpc_url) = {
        let c = s.cfg.peek();
        (c.api_url.clone(), c.rpc_url.clone())
    };
    if let Ok(t) = api::trader(&api_url, &owner).await {
        s.trader.set(t);
    }
    if let Ok(o) = api::orders(&api_url, &owner).await {
        s.orders.set(o);
    }
    if let Ok(f) = api::fills(&api_url, &owner, 50).await {
        s.fills.set(f);
    }
    if let Ok(f) = api::funding(&api_url, &owner, 50).await {
        s.funding.set(f);
    }
    if let Ok(pk) = owner.parse::<chain::Pubkey>() {
        let ata = chain::ix::associated_token_address(&pk, &chain::ix::usdc_mint());
        if let Ok(b) = chain::rpc::token_balance(&rpc_url, &ata).await {
            s.wallet_usdc.set(b as f64 / 1_000_000.0);
        }
    }
}

#[component]
pub fn App() -> Element {
    let s = use_context_provider(State::new);
    use_hook(move || trade::init_wallet(s));
    use_future(move || async move {
        loop {
            refresh_market(s).await;
            refresh_account(s).await;
            gloo_timers::future::TimeoutFuture::new(2_000).await;
        }
    });
    use_future(move || async move {
        let mut s = s;
        let ws = s.cfg.peek().ws_url();
        let mut rx = api::connect_ws(&ws, vec!["market".into(), "quotes".into()]);
        while let Some(m) = rx.next().await {
            match m {
                WsMsg::Market(m) => s.market.set(Some(m)),
                WsMsg::Quotes(q) => s.quotes.set(q),
                _ => {}
            }
        }
    });
    let class = if *s.light.read() { "tt light" } else { "tt" };
    rsx! {
        document::Link { rel: "stylesheet", href: style::FONTS }
        style { {style::CSS} }
        div { class,
            nav::Nav {}
            market::MarketBar {}
            div { class: "main",
                chart::Chart {}
                ladder::Quotes {}
                div { class: "panel ticket-panel", ticket::Ticket {} }
            }
            div { class: "panel acct", account::Account {} }
        }
    }
}
