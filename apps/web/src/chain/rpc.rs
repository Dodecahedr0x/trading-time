//! Solana JSON-RPC over `gloo_net` for the browser.
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use tradingtime_interface::TtError;

const POLL_MS: u32 = 500;

fn body(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string()
}

/// Parses an RPC response and returns its `result`, or the RPC error message.
fn result(json: &str) -> Result<Value, String> {
    let mut v: Value = serde_json::from_str(json).map_err(|e| format!("bad rpc json: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(err.get("message").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| err.to_string()));
    }
    match v.get_mut("result") {
        Some(r) => Ok(r.take()),
        None => Err("rpc response without result".into()),
    }
}

async fn post(rpc_url: &str, body: String) -> Result<String, String> {
    let resp = gloo_net::http::Request::post(rpc_url)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.text().await.map_err(|e| e.to_string())
}

pub fn latest_blockhash_body() -> String {
    body("getLatestBlockhash", json!([{"commitment": "confirmed"}]))
}

pub fn parse_latest_blockhash(json: &str) -> Result<[u8; 32], String> {
    let r = result(json)?;
    let s = r["value"]["blockhash"].as_str().ok_or("missing blockhash")?;
    let bytes = bs58::decode(s).into_vec().map_err(|e| e.to_string())?;
    bytes.try_into().map_err(|_| "blockhash is not 32 bytes".to_string())
}

pub async fn latest_blockhash(rpc_url: &str) -> Result<[u8; 32], String> {
    parse_latest_blockhash(&post(rpc_url, latest_blockhash_body()).await?)
}

pub fn signature_status_body(sig: &str) -> String {
    body("getSignatureStatuses", json!([[sig], {"searchTransactionHistory": true}]))
}

/// Ok(None) = not yet confirmed/unknown; Ok(Some(Ok(()))) = confirmed|finalized w/o err; Ok(Some(Err(reason))) = landed with err.
pub fn parse_signature_status(json: &str) -> Result<Option<Result<(), String>>, String> {
    let r = result(json)?;
    let st = &r["value"][0];
    if st.is_null() {
        return Ok(None);
    }
    if !st["err"].is_null() {
        return Ok(Some(Err(error_reason(&st["err"]))));
    }
    match st["confirmationStatus"].as_str() {
        Some("confirmed") | Some("finalized") => Ok(Some(Ok(()))),
        _ => Ok(None),
    }
}

/// `{"InstructionError":[i,{"Custom":code}]}` with a known code -> the `TtError` name; anything else -> compact JSON.
pub fn error_reason(err: &Value) -> String {
    err["InstructionError"][1]["Custom"]
        .as_u64()
        .and_then(|c| u32::try_from(c).ok())
        .and_then(TtError::from_code)
        .map(|e| format!("{e:?}"))
        .unwrap_or_else(|| err.to_string())
}

/// Transient RPC failures keep polling; only a landed error or the timeout ends early.
pub async fn wait_confirmed(rpc_url: &str, signature: &str, timeout_ms: u32) -> Result<(), String> {
    let mut waited = 0u32;
    loop {
        if let Ok(text) = post(rpc_url, signature_status_body(signature)).await
            && let Ok(Some(done)) = parse_signature_status(&text) {
                return done;
            }
        if waited >= timeout_ms {
            return Err("timeout".into());
        }
        gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
        waited = waited.saturating_add(POLL_MS);
    }
}

pub fn token_balance_body(token_account: &Pubkey) -> String {
    body("getTokenAccountBalance", json!([token_account.to_string(), {"commitment": "confirmed"}]))
}

pub async fn token_balance(rpc_url: &str, token_account: &Pubkey) -> Result<u64, String> {
    parse_token_balance(&post(rpc_url, token_balance_body(token_account)).await?)
}

/// Missing token account -> Ok(0).
pub fn parse_token_balance(json: &str) -> Result<u64, String> {
    match result(json) {
        Ok(r) => r["value"]["amount"]
            .as_str()
            .ok_or("missing amount")?
            .parse()
            .map_err(|e| format!("bad amount: {e}")),
        Err(e) if e.contains("could not find account") => Ok(0),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bodies() {
        let v: Value = serde_json::from_str(&latest_blockhash_body()).unwrap();
        assert_eq!(v, json!({"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[{"commitment":"confirmed"}]}));
        let v: Value = serde_json::from_str(&signature_status_body("abc")).unwrap();
        assert_eq!(v, json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[["abc"],{"searchTransactionHistory":true}]}));
        let pk = Pubkey::new_from_array([1; 32]);
        let v: Value = serde_json::from_str(&token_balance_body(&pk)).unwrap();
        assert_eq!(v["method"], "getTokenAccountBalance");
        assert_eq!(v["params"][0], pk.to_string());
    }

    #[test]
    fn latest_blockhash_parse() {
        let bh = Pubkey::new_from_array([5; 32]).to_string();
        let ok = format!(
            r#"{{"jsonrpc":"2.0","result":{{"context":{{"slot":2792}},"value":{{"blockhash":"{bh}","lastValidBlockHeight":3090}}}},"id":1}}"#
        );
        assert_eq!(parse_latest_blockhash(&ok), Ok([5; 32]));
        let err = r#"{"jsonrpc":"2.0","error":{"code":-32005,"message":"Node is behind"},"id":1}"#;
        assert!(parse_latest_blockhash(err).unwrap_err().contains("Node is behind"));
        assert!(parse_latest_blockhash(r#"{"result":{"value":{"blockhash":"abc"}}}"#).is_err());
        assert!(parse_latest_blockhash("not json").is_err());
    }

    #[test]
    fn signature_status_parse() {
        let unknown = r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[null]},"id":1}"#;
        assert_eq!(parse_signature_status(unknown), Ok(None));
        let processed = r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":0,"err":null,"status":{"Ok":null},"confirmationStatus":"processed"}]},"id":1}"#;
        assert_eq!(parse_signature_status(processed), Ok(None));
        let confirmed = r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":10,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"}]},"id":1}"#;
        assert_eq!(parse_signature_status(confirmed), Ok(Some(Ok(()))));
        let finalized = r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":null,"err":null,"status":{"Ok":null},"confirmationStatus":"finalized"}]},"id":1}"#;
        assert_eq!(parse_signature_status(finalized), Ok(Some(Ok(()))));
        let failed = r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":0,"err":{"InstructionError":[0,{"Custom":16}]},"status":{"Err":{"InstructionError":[0,{"Custom":16}]}},"confirmationStatus":"processed"}]},"id":1}"#;
        assert_eq!(parse_signature_status(failed), Ok(Some(Err("MinFillNotMet".into()))));
        let rpc_err = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid param"},"id":1}"#;
        assert!(parse_signature_status(rpc_err).is_err());
    }

    #[test]
    fn error_reasons() {
        assert_eq!(error_reason(&json!({"InstructionError":[0,{"Custom":10}]})), "InsufficientMargin");
        assert_eq!(error_reason(&json!({"InstructionError":[1,{"Custom":25}]})), "WrongSide");
        assert_eq!(error_reason(&json!({"InstructionError":[0,{"Custom":999}]})), r#"{"InstructionError":[0,{"Custom":999}]}"#);
        assert_eq!(error_reason(&json!({"InstructionError":[0,"InvalidAccountData"]})), r#"{"InstructionError":[0,"InvalidAccountData"]}"#);
        assert_eq!(error_reason(&json!("AccountNotFound")), r#""AccountNotFound""#);
        assert_eq!(error_reason(&json!({"InsufficientFundsForRent":{"account_index":0}})), r#"{"InsufficientFundsForRent":{"account_index":0}}"#);
    }

    #[test]
    fn token_balance_parse() {
        let ok = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1114},"value":{"amount":"9864","decimals":6,"uiAmount":0.009864,"uiAmountString":"0.009864"}},"id":1}"#;
        assert_eq!(parse_token_balance(ok), Ok(9864));
        let missing = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid param: could not find account"},"id":1}"#;
        assert_eq!(parse_token_balance(missing), Ok(0));
        let other = r#"{"jsonrpc":"2.0","error":{"code":-32005,"message":"Node is behind"},"id":1}"#;
        assert!(parse_token_balance(other).is_err());
        assert!(parse_token_balance(r#"{"result":{"value":{"amount":"x"}}}"#).is_err());
    }
}
