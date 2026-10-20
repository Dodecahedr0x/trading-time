//! Surfpool cheatcodes over JSON-RPC.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use tradingtime_interface::{PYTH_RECEIVER, PYTH_SOL_USD_FEED_ID, pyth};

/// Rent-exempt minimum for a 134-byte account.
pub const PYTH_ACCOUNT_LAMPORTS: u64 = (pyth::LEN as u64 + 128) * 3_480 * 2;

pub async fn call(url: &str, method: &str, params: Value) -> Result<Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let resp: Value = reqwest::Client::new().post(url).json(&body).send().await?.json().await?;
    if let Some(err) = resp.get("error") {
        return Err(anyhow!("{method}: {err}"));
    }
    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

pub fn set_account_params(pubkey: &Pubkey, lamports: u64, data: &[u8], owner: &Pubkey) -> Value {
    json!([
        pubkey.to_string(),
        {"lamports": lamports, "data": hex::encode(data), "owner": owner.to_string(), "executable": false, "rentEpoch": 0}
    ])
}

pub fn set_token_account_params(owner: &Pubkey, mint: &Pubkey, amount: u64) -> Value {
    json!([owner.to_string(), mint.to_string(), {"amount": amount}])
}

pub fn time_travel_params(unix_ms: i64) -> Value {
    json!([{"absoluteTimestamp": unix_ms}])
}

/// `data` is raw bytes; it is hex-encoded on the wire.
pub async fn set_account(url: &str, pubkey: &Pubkey, lamports: u64, data: &[u8], owner: &Pubkey) -> Result<()> {
    call(url, "surfnet_setAccount", set_account_params(pubkey, lamports, data, owner)).await.map(|_| ())
}

/// Sets the balance of `owner`'s associated token account for `mint`.
pub async fn set_token_account(url: &str, owner: &Pubkey, mint: &Pubkey, amount: u64) -> Result<()> {
    call(url, "surfnet_setTokenAccount", set_token_account_params(owner, mint, amount)).await.map(|_| ())
}

pub async fn time_travel_ms(url: &str, unix_ms: i64) -> Result<Value> {
    call(url, "surfnet_timeTravel", time_travel_params(unix_ms)).await
}

/// A Full-verification PriceUpdateV2 for the SOL/USD feed id.
pub fn price_update_v2(price: i64, expo: i32, publish_time: i64) -> [u8; pyth::LEN] {
    let mut d = [0u8; pyth::LEN];
    d[..8].copy_from_slice(&pyth::DISCRIMINATOR);
    d[pyth::VERIFICATION_TAG] = pyth::VERIFICATION_FULL;
    d[pyth::FEED_ID..pyth::FEED_ID + 32].copy_from_slice(&PYTH_SOL_USD_FEED_ID);
    d[pyth::PRICE..pyth::PRICE + 8].copy_from_slice(&price.to_le_bytes());
    d[pyth::EXPONENT..pyth::EXPONENT + 4].copy_from_slice(&expo.to_le_bytes());
    d[pyth::PUBLISH_TIME..pyth::PUBLISH_TIME + 8].copy_from_slice(&publish_time.to_le_bytes());
    d[pyth::PREV_PUBLISH_TIME..pyth::PREV_PUBLISH_TIME + 8].copy_from_slice(&(publish_time - 1).to_le_bytes());
    d[pyth::EMA_PRICE..pyth::EMA_PRICE + 8].copy_from_slice(&price.to_le_bytes());
    d
}

pub async fn write_pyth_price(url: &str, feed: &Pubkey, price: i64, expo: i32, publish_time: i64) -> Result<()> {
    let data = price_update_v2(price, expo, publish_time);
    set_account(url, feed, PYTH_ACCOUNT_LAMPORTS, &data, &Pubkey::new_from_array(PYTH_RECEIVER)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_update_layout() {
        let d = price_update_v2(15_000_000_000, -8, 1_700_000_000);
        assert_eq!(d.len(), 134);
        assert_eq!(hex::encode(&d[..8]), "22f123639d7ef4cd");
        assert!(d[8..40].iter().all(|&b| b == 0));
        assert_eq!(d[40], 1);
        assert_eq!(hex::encode(&d[41..73]), "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d");
        assert_eq!(i64::from_le_bytes(d[73..81].try_into().unwrap()), 15_000_000_000);
        assert_eq!(u64::from_le_bytes(d[81..89].try_into().unwrap()), 0);
        assert_eq!(i32::from_le_bytes(d[89..93].try_into().unwrap()), -8);
        assert_eq!(i64::from_le_bytes(d[93..101].try_into().unwrap()), 1_700_000_000);
        assert_eq!(i64::from_le_bytes(d[101..109].try_into().unwrap()), 1_699_999_999);
        assert_eq!(i64::from_le_bytes(d[109..117].try_into().unwrap()), 15_000_000_000);
        assert!(d[117..134].iter().all(|&b| b == 0));
    }

    #[test]
    fn cheatcode_params() {
        let a = Pubkey::new_from_array([1; 32]);
        let o = Pubkey::new_from_array(PYTH_RECEIVER);
        let p = set_account_params(&a, 5, &[0xde, 0xad], &o);
        assert_eq!(p[0], a.to_string());
        assert_eq!(p[1]["data"], "dead");
        assert_eq!(p[1]["owner"], "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");
        assert_eq!(p[1]["lamports"], 5);
        assert_eq!(p[1]["executable"], false);
        assert_eq!(p[1]["rentEpoch"], 0);
        let t = set_token_account_params(&a, &o, 7);
        assert_eq!(t[2]["amount"], 7);
        assert_eq!(time_travel_params(1_000), json!([{"absoluteTimestamp": 1000}]));
    }

    #[tokio::test]
    async fn call_surfaces_rpc_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = s.read(&mut buf).await.unwrap();
            let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"nope"}}"#;
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            s.write_all(resp.as_bytes()).await.unwrap();
        });
        let err = time_travel_ms(&url, 1).await.unwrap_err().to_string();
        assert!(err.contains("surfnet_timeTravel") && err.contains("nope"), "{err}");
    }
}
