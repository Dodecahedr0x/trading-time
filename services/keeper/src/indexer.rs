//! Indexer REST client for the keeper's routes (`/v1/liquidatable`, `/v1/positions`, `/v1/orders/open`).

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de::Error as _};
use solana_pubkey::Pubkey;
use tradingtime_interface::Side;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LiquidatableRow {
    #[serde(deserialize_with = "de_pubkey")]
    pub owner: Pubkey,
    #[serde(default)]
    pub equity_usdc: f64,
    #[serde(default)]
    pub maintenance_usdc: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PositionRow {
    #[serde(deserialize_with = "de_pubkey")]
    pub owner: Pubkey,
    pub q_usdc: f64,
    pub upnl_usdc: f64,
    #[serde(default)]
    pub is_operator: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct OpenOrderRow {
    #[serde(deserialize_with = "de_pubkey")]
    pub owner: Pubkey,
    pub order_id: u64,
    #[serde(deserialize_with = "de_side")]
    pub side: Side,
    pub size_usdc: f64,
    #[serde(default)]
    pub filled_usdc: f64,
    #[serde(default)]
    pub limit_secs: f64,
    #[serde(default)]
    pub expiry_t: i64,
}

fn de_pubkey<'de, D: Deserializer<'de>>(d: D) -> Result<Pubkey, D::Error> {
    let s = String::deserialize(d)?;
    s.parse().map_err(|_| D::Error::custom(format!("bad pubkey {s}")))
}

fn de_side<'de, D: Deserializer<'de>>(d: D) -> Result<Side, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) if s.eq_ignore_ascii_case("long") => Ok(Side::Long),
        serde_json::Value::String(s) if s.eq_ignore_ascii_case("short") => Ok(Side::Short),
        serde_json::Value::Number(n) => n.as_u64().and_then(|v| Side::from_u8(v as u8)).ok_or_else(|| D::Error::custom("bad side")),
        v => Err(D::Error::custom(format!("bad side {v}"))),
    }
}

pub fn side_param(side: Side) -> &'static str {
    match side {
        Side::Long => "long",
        Side::Short => "short",
    }
}

#[allow(async_fn_in_trait)]
pub trait Indexer {
    async fn liquidatable(&self) -> Result<Vec<LiquidatableRow>>;
    async fn positions(&self, side: Side, in_profit: bool) -> Result<Vec<PositionRow>>;
    async fn open_orders(&self) -> Result<Vec<OpenOrderRow>>;
}

pub struct HttpIndexer {
    base: String,
    http: reqwest::Client,
}

pub const POSITIONS_LIMIT: u32 = 64;

impl HttpIndexer {
    pub fn new(base: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: reqwest::Client::new() }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.get(&url).send().await.with_context(|| format!("GET {url}"))?;
        let resp = resp.error_for_status().with_context(|| format!("GET {url}"))?;
        resp.json().await.with_context(|| format!("decode {url}"))
    }
}

impl Indexer for HttpIndexer {
    async fn liquidatable(&self) -> Result<Vec<LiquidatableRow>> {
        self.get("/v1/liquidatable").await
    }

    async fn positions(&self, side: Side, in_profit: bool) -> Result<Vec<PositionRow>> {
        self.get(&format!("/v1/positions?side={}&in_profit={in_profit}&limit={POSITIONS_LIMIT}", side_param(side))).await
    }

    async fn open_orders(&self) -> Result<Vec<OpenOrderRow>> {
        self.get("/v1/orders/open").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const OWNER: &str = "11111111111111111111111111111112";

    #[test]
    fn parses_pinned_rows() {
        let l: Vec<LiquidatableRow> = serde_json::from_str(&format!(
            r#"[{{"owner":"{OWNER}","trader":"{OWNER}","equity_usdc":1.5,"maintenance_usdc":2.0}}]"#
        ))
        .unwrap();
        assert_eq!(l[0].owner.to_string(), OWNER);
        assert_eq!(l[0].maintenance_usdc, 2.0);

        let p: Vec<PositionRow> = serde_json::from_str(&format!(
            r#"[{{"owner":"{OWNER}","trader":"{OWNER}","q_usdc":-10.0,"upnl_usdc":3.0,"equity_usdc":9.0}}]"#
        ))
        .unwrap();
        assert_eq!(p[0].q_usdc, -10.0);

        let o: Vec<OpenOrderRow> = serde_json::from_str(&format!(
            r#"[{{"owner":"{OWNER}","order_id":7,"side":"short","size_usdc":5.0,"filled_usdc":0.0,"limit_secs":3600.0,"expiry_t":0}},
                {{"owner":"{OWNER}","order_id":8,"side":0,"size_usdc":5.0}}]"#
        ))
        .unwrap();
        assert_eq!(o[0].side, Side::Short);
        assert_eq!(o[1].side, Side::Long);
        assert_eq!(o[1].order_id, 8);
    }

    #[test]
    fn rejects_bad_pubkey() {
        assert!(serde_json::from_str::<Vec<PositionRow>>(r#"[{"owner":"nope","q_usdc":1,"upnl_usdc":1}]"#).is_err());
    }

    /// One-shot HTTP server returning `body`; yields the request line it saw.
    async fn serve_once(body: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).await.unwrap();
            req
        });
        (format!("http://{addr}/"), h)
    }

    #[tokio::test]
    async fn http_positions_hits_pinned_route() {
        let (base, h) = serve_once(format!(r#"[{{"owner":"{OWNER}","q_usdc":-1.0,"upnl_usdc":2.0}}]"#)).await;
        let rows = HttpIndexer::new(&base).positions(Side::Short, true).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(h.await.unwrap(), "GET /v1/positions?side=short&in_profit=true&limit=64 HTTP/1.1");
    }

    #[tokio::test]
    async fn http_liquidatable_and_orders_routes() {
        let (base, h) = serve_once("[]".into()).await;
        assert!(HttpIndexer::new(&base).liquidatable().await.unwrap().is_empty());
        assert_eq!(h.await.unwrap(), "GET /v1/liquidatable HTTP/1.1");
        let (base, h) = serve_once("[]".into()).await;
        assert!(HttpIndexer::new(&base).open_orders().await.unwrap().is_empty());
        assert_eq!(h.await.unwrap(), "GET /v1/orders/open HTTP/1.1");
    }
}
