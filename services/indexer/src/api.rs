use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use tokio::sync::broadcast;
use tradingtime_math::Side;

use crate::store::Store;
use crate::views::{self, ViewError, ViewResult};

/// What changed; WS connections render the matching channel from the store.
#[derive(Clone, Debug, PartialEq)]
pub enum Notify {
    Market,
    Quotes,
    Trade(Value),
    Trader(String),
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub notify: broadcast::Sender<Notify>,
}

impl AppState {
    pub fn new(store: Store) -> AppState {
        AppState { store: Arc::new(Mutex::new(store)), notify: broadcast::channel(1024).0 }
    }

    pub fn with_store<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        f(&self.store.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn publish(&self, n: Notify) {
        let _ = self.notify.send(n);
    }
}

pub struct ApiError(ViewError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match self.0 {
            ViewError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ViewError::NotFound => (StatusCode::NOT_FOUND, "unknown trader".into()),
            ViewError::NotSynced => (StatusCode::SERVICE_UNAVAILABLE, "not synced".into()),
            ViewError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (code, Json(json!({ "error": msg }))).into_response()
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

fn respond(r: ViewResult) -> ApiResult {
    r.map(Json).map_err(ApiError)
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(ViewError::BadRequest(msg.into()))
}

fn limit(q: &HashMap<String, String>, default: usize, max: usize) -> Result<usize, ApiError> {
    match q.get("limit") {
        None => Ok(default),
        Some(s) => match s.parse::<usize>() {
            Ok(n) if n >= 1 && n <= max => Ok(n),
            _ => Err(bad(format!("limit must be 1..={max}"))),
        },
    }
}

fn owner(s: &str) -> Result<String, ApiError> {
    s.parse::<Pubkey>().map(|p| p.to_string()).map_err(|_| bad("invalid owner pubkey"))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/market", get(market))
        .route("/v1/candles", get(candles))
        .route("/v1/index", get(index))
        .route("/v1/quotes", get(quotes))
        .route("/v1/operators", get(operators))
        .route("/v1/traders/{owner}", get(trader))
        .route("/v1/traders/{owner}/fills", get(fills))
        .route("/v1/traders/{owner}/orders", get(trader_orders))
        .route("/v1/traders/{owner}/funding", get(funding))
        .route("/v1/positions", get(positions))
        .route("/v1/liquidatable", get(liquidatable))
        .route("/v1/orders/open", get(open_orders))
        .route("/v1/ws", get(ws))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

async fn health(State(s): State<AppState>) -> Json<Value> {
    Json(s.with_store(views::health))
}

async fn market(State(s): State<AppState>) -> ApiResult {
    respond(s.with_store(views::market_view))
}

async fn candles(State(s): State<AppState>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let n = limit(&q, 200, 500)?;
    let iv = q.get("interval").map(String::as_str).unwrap_or("1h");
    respond(s.with_store(|st| views::candles_view(st, iv, n)))
}

async fn index(State(s): State<AppState>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let n = limit(&q, 200, 1000)?;
    respond(s.with_store(|st| views::index_view(st, n)))
}

async fn quotes(State(s): State<AppState>) -> ApiResult {
    respond(s.with_store(views::quotes_view))
}

async fn operators(State(s): State<AppState>) -> ApiResult {
    respond(s.with_store(views::operators_view))
}

async fn trader(State(s): State<AppState>, Path(o): Path<String>) -> ApiResult {
    let o = owner(&o)?;
    respond(s.with_store(|st| views::trader_view(st, &o)))
}

async fn fills(State(s): State<AppState>, Path(o): Path<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let o = owner(&o)?;
    let n = limit(&q, 100, 1000)?;
    respond(s.with_store(|st| views::fills_view(st, &o, n)))
}

async fn trader_orders(State(s): State<AppState>, Path(o): Path<String>) -> ApiResult {
    let o = owner(&o)?;
    respond(s.with_store(|st| views::trader_orders_view(st, &o)))
}

async fn funding(State(s): State<AppState>, Path(o): Path<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let o = owner(&o)?;
    let n = limit(&q, 100, 1000)?;
    respond(s.with_store(|st| views::funding_view(st, &o, n)))
}

async fn positions(State(s): State<AppState>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let side = match q.get("side").map(String::as_str) {
        None => None,
        Some("long") => Some(Side::Long),
        Some("short") => Some(Side::Short),
        Some(x) => return Err(bad(format!("bad side {x}"))),
    };
    let in_profit = match q.get("in_profit").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(x) => return Err(bad(format!("bad in_profit {x}"))),
    };
    let n = limit(&q, 100, 1000)?;
    respond(s.with_store(|st| views::positions_view(st, side, in_profit, n)))
}

async fn liquidatable(State(s): State<AppState>) -> ApiResult {
    respond(s.with_store(views::liquidatable_view))
}

async fn open_orders(State(s): State<AppState>) -> ApiResult {
    respond(s.with_store(views::open_orders_view))
}

async fn ws(State(s): State<AppState>, up: WebSocketUpgrade) -> Response {
    up.on_upgrade(move |socket| ws_session(s, socket))
}

fn render(s: &AppState, channel: &str) -> Option<Value> {
    let data = match channel {
        "market" => s.with_store(views::market_view),
        "quotes" => s.with_store(views::quotes_view),
        c => {
            let o = c.strip_prefix("trader:")?;
            s.with_store(|st| views::trader_view(st, o))
        }
    }
    .ok()?;
    Some(json!({ "channel": channel, "data": data }))
}

async fn send(socket: &mut WebSocket, v: Value) -> bool {
    socket.send(Message::Text(v.to_string().into())).await.is_ok()
}

async fn ws_session(s: AppState, mut socket: WebSocket) {
    let mut rx = s.notify.subscribe();
    let mut subs: HashSet<String> = HashSet::new();
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return };
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    if !send(&mut socket, json!({ "error": "invalid json" })).await { return }
                    continue;
                };
                let Some(list) = v.get("subscribe").and_then(Value::as_array) else { continue };
                for ch in list.iter().filter_map(Value::as_str) {
                    if subs.insert(ch.to_string())
                        && let Some(snapshot) = render(&s, ch)
                        && !send(&mut socket, snapshot).await
                    {
                        return;
                    }
                }
            }
            n = rx.recv() => {
                let n = match n {
                    Ok(n) => n,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                };
                let out = match n {
                    Notify::Market => subs.contains("market").then(|| render(&s, "market")).flatten(),
                    Notify::Quotes => subs.contains("quotes").then(|| render(&s, "quotes")).flatten(),
                    Notify::Trader(o) => {
                        let ch = format!("trader:{o}");
                        subs.contains(&ch).then(|| render(&s, &ch)).flatten()
                    }
                    Notify::Trade(fill) => subs.contains("trades").then(|| json!({ "channel": "trades", "data": fill })),
                };
                if let Some(v) = out && !send(&mut socket, v).await {
                    return;
                }
            }
        }
    }
}
