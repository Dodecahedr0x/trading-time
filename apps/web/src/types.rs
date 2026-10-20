//! Indexer JSON shapes (tree/1.md "Indexer API"). Lenient: unknown keys ignored, missing numbers default.

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SideStr {
    Long,
    Short,
}

impl<'de> Deserialize<'de> for SideStr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match &v {
            serde_json::Value::String(s) if s.eq_ignore_ascii_case("long") => Ok(SideStr::Long),
            serde_json::Value::String(s) if s.eq_ignore_ascii_case("short") => Ok(SideStr::Short),
            serde_json::Value::Number(n) if n.as_u64() == Some(0) => Ok(SideStr::Long),
            serde_json::Value::Number(n) if n.as_u64() == Some(1) => Ok(SideStr::Short),
            _ => Err(serde::de::Error::custom(format!("bad side {v}"))),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarketJson {
    pub mark_secs: f64,
    pub index_secs: f64,
    pub mark_ln: String,
    pub index_ln: String,
    pub funding_rate_per_hour: f64,
    pub long_oi_usdc: f64,
    pub short_oi_usdc: f64,
    pub volume_24h_usdc: f64,
    pub change_24h: f64,
    pub paused: bool,
    pub insurance_usdc: f64,
    pub slot: u64,
    pub updated_ts: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandleJson {
    pub t: i64,
    pub open_secs: f64,
    pub high_secs: f64,
    pub low_secs: f64,
    pub close_secs: f64,
    pub volume_usdc: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexPointJson {
    pub t: i64,
    pub index_secs: f64,
    pub mark_secs: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LevelJson {
    pub time_secs: f64,
    pub size_usdc: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OperatorJson {
    pub owner: String,
    pub fair_secs: f64,
    pub half_spread: f64,
    pub impact_per_usdc: f64,
    pub inventory_limit_usdc: f64,
    pub inventory_usdc: f64,
    pub updated_slot: u64,
    pub fresh: bool,
    pub status: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuotesJson {
    pub mark_secs: f64,
    /// Ask = where Long time fills (time above mark).
    pub asks: Vec<LevelJson>,
    pub bids: Vec<LevelJson>,
    pub operators: Vec<OperatorJson>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraderJson {
    pub owner: String,
    pub trader: String,
    pub cash_usdc: f64,
    pub q_usdc: f64,
    pub entry_secs: Option<f64>,
    pub equity_usdc: f64,
    pub upnl_usdc: f64,
    pub funding_usdc: f64,
    pub liq_secs: Option<f64>,
    /// Exposure open orders add beyond the position (netted).
    pub reserved_usdc: f64,
    pub reserved_long_usdc: f64,
    pub reserved_short_usdc: f64,
    pub is_operator: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillJson {
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub slot: u64,
    #[serde(default)]
    pub t: i64,
    pub side: SideStr,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub size_usdc: f64,
    #[serde(default)]
    pub price_secs: f64,
    #[serde(default)]
    pub fee_usdc: f64,
    #[serde(default)]
    pub operator: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderJson {
    #[serde(default)]
    pub order_id: u64,
    pub side: SideStr,
    #[serde(default)]
    pub size_usdc: f64,
    #[serde(default)]
    pub filled_usdc: f64,
    #[serde(default)]
    pub limit_secs: f64,
    #[serde(default)]
    pub expiry_t: i64,
    #[serde(default)]
    pub created_slot: u64,
    /// "open" | "filled" | "cancelled" | "expired"
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FundingJson {
    pub t: i64,
    pub amount_usdc: f64,
}

/// A WS push: `{"channel": "...", "data": ...}`.
#[derive(Clone, Debug, PartialEq)]
pub enum WsMsg {
    Market(MarketJson),
    Quotes(QuotesJson),
    Trade(FillJson),
    Trader(TraderJson),
    Other(String),
}
