use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc::{self, Sender};
use tokio_tungstenite::tungstenite::Message;

pub const BINANCE_URL: &str = "wss://stream.binance.com:9443/ws/solusdt@trade";
pub const COINBASE_URL: &str = "wss://ws-feed.exchange.coinbase.com";
/// Binance silence after which `auto` quotes from Coinbase.
pub const FALLBACK_AFTER_MS: i64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Binance,
    Coinbase,
    Replay,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceTick {
    pub ts_ms: i64,
    pub price: f64,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedMode {
    Binance,
    Coinbase,
    Auto,
    Replay(String),
}

impl FromStr for FeedMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "binance" => Ok(FeedMode::Binance),
            "coinbase" => Ok(FeedMode::Coinbase),
            "auto" => Ok(FeedMode::Auto),
            _ => match s.strip_prefix("replay:") {
                Some(path) if !path.is_empty() => Ok(FeedMode::Replay(path.to_string())),
                _ => Err(format!("unknown feed {s:?}: binance|coinbase|auto|replay:<file>")),
            },
        }
    }
}

fn positive(price: f64) -> Option<f64> {
    (price.is_finite() && price > 0.0).then_some(price)
}

pub fn parse_binance(msg: &str) -> Option<PriceTick> {
    #[derive(Deserialize)]
    struct Trade<'a> {
        e: &'a str,
        p: &'a str,
        #[serde(rename = "T")]
        t: i64,
    }
    let tr: Trade = serde_json::from_str(msg).ok()?;
    if tr.e != "trade" {
        return None;
    }
    Some(PriceTick { ts_ms: tr.t, price: positive(tr.p.parse().ok()?)?, source: Source::Binance })
}

pub fn parse_coinbase(msg: &str) -> Option<PriceTick> {
    #[derive(Deserialize)]
    struct Match<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        product_id: &'a str,
        price: &'a str,
        time: &'a str,
    }
    let m: Match = serde_json::from_str(msg).ok()?;
    if !matches!(m.kind, "match" | "last_match") || m.product_id != "SOL-USD" {
        return None;
    }
    let ts_ms = chrono::DateTime::parse_from_rfc3339(m.time).ok()?.timestamp_millis();
    Some(PriceTick { ts_ms, price: positive(m.price.parse().ok()?)?, source: Source::Coinbase })
}

pub fn coinbase_subscribe() -> String {
    serde_json::json!({"type": "subscribe", "product_ids": ["SOL-USD"], "channels": ["matches"]}).to_string()
}

/// CSV rows `ts_ms,price`; a non-numeric first line is a header.
pub fn parse_replay_csv(s: &str) -> anyhow::Result<Vec<(i64, f64)>> {
    let mut rows = Vec::new();
    for (i, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (ts, price) = line.split_once(',').ok_or_else(|| anyhow::anyhow!("line {}: expected ts_ms,price", i + 1))?;
        let (ts, price) = (ts.trim(), price.trim());
        match (ts.parse::<i64>(), price.parse::<f64>()) {
            (Ok(ts), Ok(p)) if p.is_finite() && p > 0.0 => rows.push((ts, p)),
            _ if i == 0 && ts.parse::<i64>().is_err() => {}
            _ => anyhow::bail!("line {}: bad row {line:?}", i + 1),
        }
    }
    Ok(rows)
}

pub trait Clock {
    fn now_ms(&self) -> i64;
    fn sleep_until_ms(&self, ms: i64) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

pub fn unix_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        unix_ms()
    }
    async fn sleep_until_ms(&self, ms: i64) {
        let wait = ms - unix_ms();
        if wait > 0 {
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
        }
    }
}

/// Plays rows at their recorded pace starting now; ticks carry the clock's time.
pub async fn play_replay<C: Clock>(rows: Vec<(i64, f64)>, clock: C, tx: Sender<PriceTick>) {
    let Some(&(first, _)) = rows.first() else { return };
    let start = clock.now_ms();
    for (ts, price) in rows {
        let at = start + (ts - first);
        clock.sleep_until_ms(at).await;
        if tx.send(PriceTick { ts_ms: at, price, source: Source::Replay }).await.is_err() {
            return;
        }
    }
}

/// Plays `rows` repeatedly (`passes`: None = forever). Each pass is rescaled to start at the previous
/// pass's last price and starts one row-gap after it ends, so the series never jumps back.
pub async fn play_replay_looping<C: Clock>(rows: Vec<(i64, f64)>, clock: C, tx: Sender<PriceTick>, passes: Option<usize>) {
    let (Some(&(first_ts, first_px)), Some(&(last_ts, last_px))) = (rows.first(), rows.last()) else { return };
    if first_px <= 0.0 {
        return;
    }
    let gap = rows.get(1).map_or(1_000, |r| (r.0 - first_ts).max(1));
    let mut start = clock.now_ms();
    let mut scale = 1.0;
    let mut pass = 0;
    while passes.is_none_or(|n| pass < n) {
        for &(ts, price) in &rows {
            let at = start + (ts - first_ts);
            clock.sleep_until_ms(at).await;
            if tx.send(PriceTick { ts_ms: at, price: price * scale, source: Source::Replay }).await.is_err() {
                return;
            }
        }
        start += last_ts - first_ts + gap;
        scale *= last_px / first_px;
        pass += 1;
    }
}

/// `auto` mode: Binance while it ticks, Coinbase after `FALLBACK_AFTER_MS` of Binance silence.
/// Silence is measured on the local clock, never on exchange timestamps.
#[derive(Clone, Debug)]
pub struct Selector {
    active: Source,
    last_binance_ms: Option<i64>,
    start_ms: Option<i64>,
}

impl Default for Selector {
    fn default() -> Self {
        Self::new()
    }
}

impl Selector {
    pub fn new() -> Self {
        Selector { active: Source::Binance, last_binance_ms: None, start_ms: None }
    }

    pub fn start(&mut self, now_ms: i64) {
        self.start_ms = Some(now_ms);
    }

    pub fn active(&self) -> Source {
        self.active
    }

    pub fn on_tick(&mut self, t: PriceTick, now_ms: i64) -> Option<PriceTick> {
        match t.source {
            Source::Binance => {
                self.last_binance_ms = Some(now_ms);
                self.active = Source::Binance;
                Some(t)
            }
            Source::Coinbase => {
                self.on_timer(now_ms);
                (self.active == Source::Coinbase).then_some(t)
            }
            Source::Replay => Some(t),
        }
    }

    pub fn on_timer(&mut self, now_ms: i64) {
        let since = self.last_binance_ms.or(self.start_ms).unwrap_or_else(|| *self.start_ms.insert(now_ms));
        if self.active == Source::Binance && now_ms - since >= FALLBACK_AFTER_MS {
            tracing::warn!("no Binance tick for {}ms, quoting from Coinbase", now_ms - since);
            self.active = Source::Coinbase;
        }
    }
}

async fn ws_loop(url: &'static str, subscribe: Option<String>, parse: fn(&str) -> Option<PriceTick>, tx: Sender<PriceTick>) {
    while !tx.is_closed() {
        match tokio_tungstenite::connect_async(url).await {
            Ok((mut ws, _)) => {
                tracing::info!("connected {url}");
                if let Some(sub) = &subscribe
                    && let Err(e) = ws.send(Message::Text(sub.clone().into())).await
                {
                    tracing::warn!("{url}: subscribe failed: {e}");
                    continue;
                }
                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Some(t) = parse(&text)
                                && tx.send(t).await.is_err()
                            {
                                return;
                            }
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("{url}: {e}");
                            break;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("connect {url}: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Streams ticks for `mode` into `tx` until `tx` closes (a replay file loops forever).
pub async fn run_feed(mode: FeedMode, tx: Sender<PriceTick>) -> anyhow::Result<()> {
    match mode {
        FeedMode::Binance => ws_loop(BINANCE_URL, None, parse_binance, tx).await,
        FeedMode::Coinbase => ws_loop(COINBASE_URL, Some(coinbase_subscribe()), parse_coinbase, tx).await,
        FeedMode::Replay(path) => {
            let rows = parse_replay_csv(&std::fs::read_to_string(&path)?)?;
            tracing::info!("replaying {} ticks from {path}", rows.len());
            play_replay_looping(rows, SystemClock, tx, None).await;
            tracing::info!("replay {path} finished");
        }
        FeedMode::Auto => {
            let (itx, mut irx) = mpsc::channel(1024);
            let b = tokio::spawn(ws_loop(BINANCE_URL, None, parse_binance, itx.clone()));
            let c = tokio::spawn(ws_loop(COINBASE_URL, Some(coinbase_subscribe()), parse_coinbase, itx));
            let mut sel = Selector::new();
            sel.start(unix_ms());
            let mut timer = tokio::time::interval(Duration::from_millis(500));
            loop {
                tokio::select! {
                    t = irx.recv() => {
                        let Some(t) = t else { break };
                        let was = sel.active();
                        if let Some(t) = sel.on_tick(t, unix_ms()) {
                            if was != sel.active() {
                                tracing::info!("feed source now {:?}", sel.active());
                            }
                            if tx.send(t).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ = timer.tick() => sel.on_timer(unix_ms()),
                    _ = tx.closed() => break,
                }
            }
            b.abort();
            c.abort();
        }
    }
    Ok(())
}

/// A running price feed: `run_feed(mode)` on a background task, read with `next()`.
/// Dropping it stops the task.
pub struct Feed {
    rx: mpsc::Receiver<PriceTick>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Feed {
    pub fn spawn(mode: FeedMode) -> Self {
        let (tx, rx) = mpsc::channel(4096);
        Feed { rx, task: tokio::spawn(run_feed(mode, tx)) }
    }

    /// Next tick; `None` once the feed has ended (e.g. replay finished or file unreadable).
    pub async fn next(&mut self) -> Option<PriceTick> {
        self.rx.recv().await
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.task.abort();
    }
}
