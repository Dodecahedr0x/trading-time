#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub rpc_url: String,
    pub api_url: String,
}

pub const DEFAULT_RPC: &str = "http://127.0.0.1:18899";
pub const DEFAULT_API: &str = "http://127.0.0.1:18787";

impl Config {
    /// `query` is `location.search` ("?rpc=...&api=..."), possibly empty.
    pub fn from_query(query: &str) -> Config {
        let mut rpc = option_env!("TT_RPC_URL").unwrap_or(DEFAULT_RPC).to_string();
        let mut api = option_env!("TT_API_URL").unwrap_or(DEFAULT_API).to_string();
        for pair in query.trim_start_matches('?').split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v);
            if v.is_empty() {
                continue;
            }
            match k {
                "rpc" => rpc = v,
                "api" => api = v,
                _ => {}
            }
        }
        Config { rpc_url: rpc.trim_end_matches('/').to_string(), api_url: api.trim_end_matches('/').to_string() }
    }

    pub fn load() -> Config {
        let search = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        Config::from_query(&search)
    }

    /// `http://host:18787` -> `ws://host:18787/v1/ws`.
    pub fn ws_url(&self) -> String {
        let base = if let Some(rest) = self.api_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.api_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            self.api_url.clone()
        };
        format!("{base}/v1/ws")
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_overrides_defaults() {
        let c = Config::from_query("?rpc=http%3A%2F%2Flocalhost%3A9999&api=http://h:1/");
        assert_eq!(c.rpc_url, "http://localhost:9999");
        assert_eq!(c.api_url, "http://h:1");
        assert_eq!(c.ws_url(), "ws://h:1/v1/ws");
        let d = Config::from_query("");
        assert!(d.rpc_url.ends_with(":18899") || option_env!("TT_RPC_URL").is_some());
    }
}
