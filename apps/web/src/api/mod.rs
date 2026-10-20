//! Indexer REST + WS client (tree/1.md "Indexer API").

use crate::types::*;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use serde::de::DeserializeOwned;

#[cfg(test)]
mod tests;

pub fn url(api: &str, path_and_query: &str) -> String {
    format!(
        "{}/{}",
        api.trim_end_matches('/'),
        path_and_query.trim_start_matches('/')
    )
}

pub fn candles_path(interval: &str, limit: u32) -> String {
    format!("/v1/candles?interval={interval}&limit={limit}")
}
pub fn index_path(limit: u32) -> String {
    format!("/v1/index?limit={limit}")
}
pub fn trader_path(owner: &str) -> String {
    format!("/v1/traders/{owner}")
}
pub fn fills_path(owner: &str, limit: u32) -> String {
    format!("/v1/traders/{owner}/fills?limit={limit}")
}
pub fn orders_path(owner: &str) -> String {
    format!("/v1/traders/{owner}/orders")
}
pub fn funding_path(owner: &str, limit: u32) -> String {
    format!("/v1/traders/{owner}/funding?limit={limit}")
}

pub fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| format!("bad JSON: {e}"))
}

pub fn check_status<T: DeserializeOwned>(status: u16, body: &str) -> Result<T, String> {
    if (200..300).contains(&status) {
        parse_json(body)
    } else {
        Err(format!("HTTP {status}: {body}"))
    }
}

pub fn trader_from(status: u16, body: &str) -> Result<Option<TraderJson>, String> {
    if status == 404 {
        Ok(None)
    } else {
        check_status(status, body).map(Some)
    }
}

async fn fetch(full_url: &str) -> Result<(u16, String), String> {
    let resp = gloo_net::http::Request::get(full_url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    Ok((status, body))
}

async fn get<T: DeserializeOwned>(api: &str, path: &str) -> Result<T, String> {
    let (status, body) = fetch(&url(api, path)).await?;
    check_status(status, &body)
}

pub async fn market(api: &str) -> Result<MarketJson, String> {
    get(api, "/v1/market").await
}
pub async fn candles(api: &str, interval: &str, limit: u32) -> Result<Vec<CandleJson>, String> {
    get(api, &candles_path(interval, limit)).await
}
pub async fn index(api: &str, limit: u32) -> Result<Vec<IndexPointJson>, String> {
    get(api, &index_path(limit)).await
}
pub async fn quotes(api: &str) -> Result<QuotesJson, String> {
    get(api, "/v1/quotes").await
}
pub async fn trader(api: &str, owner: &str) -> Result<Option<TraderJson>, String> {
    let (status, body) = fetch(&url(api, &trader_path(owner))).await?;
    trader_from(status, &body)
}
pub async fn fills(api: &str, owner: &str, limit: u32) -> Result<Vec<FillJson>, String> {
    get(api, &fills_path(owner, limit)).await
}
pub async fn orders(api: &str, owner: &str) -> Result<Vec<OrderJson>, String> {
    get(api, &orders_path(owner)).await
}
pub async fn funding(api: &str, owner: &str, limit: u32) -> Result<Vec<FundingJson>, String> {
    get(api, &funding_path(owner, limit)).await
}

pub fn subscribe_msg(channels: &[String]) -> String {
    serde_json::json!({ "subscribe": channels }).to_string()
}

/// "market"->Market, "quotes"->Quotes, "trades"->Trade, "trader:<owner>"->Trader;
/// unknown channel or undecodable data -> Other(text); non-JSON -> None.
pub fn parse_ws(text: &str) -> Option<WsMsg> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let other = || Some(WsMsg::Other(text.to_string()));
    let (Some(channel), Some(data)) = (v.get("channel").and_then(|c| c.as_str()), v.get("data")) else {
        return other();
    };
    let data = data.clone();
    let msg = match channel {
        "market" => serde_json::from_value(data).ok().map(WsMsg::Market),
        "quotes" => serde_json::from_value(data).ok().map(WsMsg::Quotes),
        "trades" => serde_json::from_value(data).ok().map(WsMsg::Trade),
        c if c.starts_with("trader:") => serde_json::from_value(data).ok().map(WsMsg::Trader),
        _ => None,
    };
    msg.or_else(other)
}

/// Spawns a reconnecting (2s backoff) subscription loop; drops out when the receiver is dropped.
pub fn connect_ws(ws_url: &str, channels: Vec<String>) -> UnboundedReceiver<WsMsg> {
    use futures::{SinkExt, StreamExt};
    use gloo_net::websocket::{futures::WebSocket, Message};

    let (tx, rx) = unbounded();
    let ws_url = ws_url.to_string();
    let sub = subscribe_msg(&channels);
    wasm_bindgen_futures::spawn_local(async move {
        while !tx.is_closed() {
            if let Ok(ws) = WebSocket::open(&ws_url) {
                let (mut sink, mut stream) = ws.split();
                if sink.send(Message::Text(sub.clone())).await.is_ok() {
                    while let Some(Ok(msg)) = stream.next().await {
                        let text = match msg {
                            Message::Text(t) => t,
                            Message::Bytes(b) => match String::from_utf8(b) {
                                Ok(t) => t,
                                Err(_) => continue,
                            },
                        };
                        if let Some(m) = parse_ws(&text)
                            && tx.unbounded_send(m).is_err() {
                                return;
                            }
                    }
                }
            }
            gloo_timers::future::TimeoutFuture::new(2_000).await;
        }
    });
    rx
}
