use tradingtime_interface::{Market, Operator, OperatorStatus, Params, PodI128, Trader};
use tradingtime_maker::chain::{Action, Chain};
use tradingtime_maker::feed::{self, Clock, FeedMode, PriceTick, Selector, Source};
use tradingtime_maker::forecast::Forecaster;
use tradingtime_maker::maker::{LoopOpts, bootstrap, run_with, shutdown};
use tradingtime_maker::slot::{self, PerfSample};
use tradingtime_maker::quote::{CurveQuote, QuoteParams};
use tradingtime_math::{ONE, from_f64, index, to_f64};

// Recorded from wss://stream.binance.com:9443/ws/solusdt@trade.
const BINANCE: &str = r#"{"e":"trade","E":1729418400123,"s":"SOLUSDT","t":912345678,"p":"151.23000000","q":"1.25000000","T":1729418400120,"m":true,"M":true}"#;
// Recorded from wss://ws-feed.exchange.coinbase.com matches channel.
const COINBASE: &str = r#"{"type":"match","trade_id":123456789,"maker_order_id":"ac928c66-ca53-498f-9c13-a110027a60e8","taker_order_id":"132fb6ae-456b-4654-b4e0-d681ac05cea1","side":"buy","size":"2.5","price":"151.21","product_id":"SOL-USD","sequence":50,"time":"2024-10-20T10:00:00.120Z"}"#;

#[test]
fn parses_binance_trade() {
    let t = feed::parse_binance(BINANCE).unwrap();
    assert_eq!(t, PriceTick { ts_ms: 1_729_418_400_120, price: 151.23, source: Source::Binance });
    assert!(feed::parse_binance(r#"{"result":null,"id":1}"#).is_none());
}

#[test]
fn parses_coinbase_match() {
    let t = feed::parse_coinbase(COINBASE).unwrap();
    assert_eq!(t, PriceTick { ts_ms: 1_729_418_400_120, price: 151.21, source: Source::Coinbase });
    let last = COINBASE.replace(r#""type":"match""#, r#""type":"last_match""#);
    assert_eq!(feed::parse_coinbase(&last).unwrap().price, 151.21);
    assert!(feed::parse_coinbase(r#"{"type":"subscriptions","channels":[]}"#).is_none());
    assert!(feed::parse_coinbase(&COINBASE.replace("SOL-USD", "BTC-USD")).is_none());
}

#[test]
fn coinbase_subscribe_message() {
    let v: serde_json::Value = serde_json::from_str(&feed::coinbase_subscribe()).unwrap();
    assert_eq!(v, serde_json::json!({"type":"subscribe","product_ids":["SOL-USD"],"channels":["matches"]}));
}

#[test]
fn feed_mode_parses() {
    assert_eq!("binance".parse::<FeedMode>().unwrap(), FeedMode::Binance);
    assert_eq!("coinbase".parse::<FeedMode>().unwrap(), FeedMode::Coinbase);
    assert_eq!("auto".parse::<FeedMode>().unwrap(), FeedMode::Auto);
    assert_eq!("replay:a/b.csv".parse::<FeedMode>().unwrap(), FeedMode::Replay("a/b.csv".into()));
    assert!("kraken".parse::<FeedMode>().is_err());
}

fn tick(src: Source, ts_ms: i64) -> PriceTick {
    PriceTick { ts_ms, price: 150.0, source: src }
}

#[test]
fn auto_switches_after_five_seconds_and_back() {
    let mut s = Selector::new();
    assert_eq!(s.active(), Source::Binance);
    assert!(s.on_tick(tick(Source::Binance, 1_000), 1_000).is_some());
    assert!(s.on_tick(tick(Source::Coinbase, 2_000), 2_000).is_none());
    // 4.999s of silence: still Binance.
    assert!(s.on_tick(tick(Source::Coinbase, 5_999), 5_999).is_none());
    assert_eq!(s.active(), Source::Binance);
    // 5s of silence: switch.
    s.on_timer(6_000);
    assert_eq!(s.active(), Source::Coinbase);
    assert!(s.on_tick(tick(Source::Coinbase, 6_100), 6_100).is_some());
    // Binance resumes: switch back, Coinbase ignored again.
    assert!(s.on_tick(tick(Source::Binance, 7_000), 7_000).is_some());
    assert_eq!(s.active(), Source::Binance);
    assert!(s.on_tick(tick(Source::Coinbase, 7_100), 7_100).is_none());
}

#[test]
fn auto_switches_on_coinbase_tick_after_silence_without_timer() {
    let mut s = Selector::new();
    // Never saw Binance: silence counted from start.
    s.start(0);
    assert!(s.on_tick(tick(Source::Coinbase, 4_000), 4_000).is_none());
    assert!(s.on_tick(tick(Source::Coinbase, 5_000), 5_000).is_some());
    assert_eq!(s.active(), Source::Coinbase);
}

#[test]
fn replay_csv_parses() {
    let rows = feed::parse_replay_csv("ts_ms,price\n1000,150.5\n\n2500,151\n").unwrap();
    assert_eq!(rows, vec![(1000, 150.5), (2500, 151.0)]);
    assert!(feed::parse_replay_csv("1000,abc\n").is_err());
}

#[derive(Clone)]
struct FakeClock(std::sync::Arc<std::sync::Mutex<i64>>);

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        *self.0.lock().unwrap()
    }
    async fn sleep_until_ms(&self, ms: i64) {
        let mut t = self.0.lock().unwrap();
        *t = (*t).max(ms);
    }
}

#[tokio::test]
async fn replay_plays_at_file_pace_from_clock_start() {
    let clock = FakeClock(std::sync::Arc::new(std::sync::Mutex::new(1_000_000)));
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    feed::play_replay(vec![(5_000, 100.0), (5_400, 101.0), (7_000, 99.0)], clock.clone(), tx).await;
    let mut got = vec![];
    while let Some(t) = rx.recv().await {
        got.push(t);
    }
    let ts: Vec<i64> = got.iter().map(|t| t.ts_ms).collect();
    assert_eq!(ts, vec![1_000_000, 1_000_400, 1_002_000]);
    assert!(got.iter().all(|t| t.source == Source::Replay));
    assert_eq!(clock.now_ms(), 1_002_000);
}

#[tokio::test]
async fn replay_loops_continuing_from_last_price() {
    let clock = FakeClock(std::sync::Arc::new(std::sync::Mutex::new(0)));
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    feed::play_replay_looping(vec![(0, 100.0), (1_000, 110.0)], clock.clone(), tx, Some(3)).await;
    let mut got = vec![];
    while let Some(t) = rx.recv().await {
        got.push((t.ts_ms, (t.price * 1e6).round() / 1e6));
    }
    assert_eq!(got, vec![(0, 100.0), (1_000, 110.0), (2_000, 110.0), (3_000, 121.0), (4_000, 121.0), (5_000, 133.1)]);
}

fn market() -> Market {
    let mut m: Market = bytemuck::Zeroable::zeroed();
    m.params = Params::DEFAULT;
    m
}

fn var_from_index(p: &Params, index_ln: f64) -> f64 {
    let x = (1.0 + p.band_bps as f64 / 10_000.0).ln();
    to_f64(p.calib.get()) * x * x / index_ln.exp()
}

#[test]
fn forecaster_ewma_matches_hand_computation() {
    let mut f = Forecaster::new(900);
    f.seed(from_f64(1e-8));
    // Two ticks in second 0: the second-close is the last one (100.0).
    for (ts, p) in [(0, 99.0), (500, 100.0), (1_200, 101.0), (3_100, 100.0), (4_000, 100.0)] {
        f.on_tick(&PriceTick { ts_ms: ts, price: p, source: Source::Replay });
    }
    // Closed seconds: 0 -> 100, 1 -> 101 (dt 1), 3 -> 100 (dt 2); second 4 still open.
    let a = |dt: f64| 1.0 - 0.5f64.powf(dt / 900.0);
    let r1 = (101.0f64 / 100.0).ln();
    let r2 = (100.0f64 / 101.0).ln();
    let mut v = 1e-8;
    v += a(1.0) * (r1 * r1 - v);
    v += a(2.0) * (r2 * r2 / 2.0 - v);
    let got = to_f64(f.var().unwrap());
    assert!((got - v).abs() / v < 1e-6, "got {got}, want {v}");
}

#[test]
fn forecaster_unseeded_without_returns_has_no_var() {
    let mut f = Forecaster::new(900);
    f.on_tick(&PriceTick { ts_ms: 0, price: 100.0, source: Source::Replay });
    assert!(f.var().is_none());
}

#[test]
fn fair_equals_index_when_vol_matches_index() {
    let mut m = market();
    let p = m.params;
    let idx = (3_600.0f64).ln();
    m.index_ln = PodI128::new(from_f64(idx));
    let mut f = Forecaster::new(900);
    f.seed(from_f64(var_from_index(&p, idx)));
    let fair = to_f64(f.fair_ln(&m).unwrap());
    assert!((fair - idx).abs() < 1e-6, "fair {fair} idx {idx}");
}

#[test]
fn fair_blends_toward_fast_vol() {
    let mut m = market();
    let p = m.params;
    let idx = (3_600.0f64).ln();
    m.index_ln = PodI128::new(from_f64(idx));
    let var_index = var_from_index(&p, idx);
    let var_now = 4.0 * var_index;
    let mut f = Forecaster::new(900);
    f.seed(from_f64(var_now));
    let w = 0.5f64.powf(p.funding_period_secs as f64 / p.index_half_life_secs as f64);
    let blend = var_now + (var_index - var_now) * w;
    let x = (1.0 + p.band_bps as f64 / 10_000.0).ln();
    let want = (to_f64(p.calib.get()) * x * x / blend).ln();
    let fair = to_f64(f.fair_ln(&m).unwrap());
    assert!((fair - want).abs() < 1e-6, "fair {fair} want {want}");
    let onchain = to_f64(index::index_ln(from_f64(blend), p.calib.get(), p.band_bps, p.index_min_secs, p.index_max_secs).unwrap());
    assert!((fair - onchain).abs() < 1e-6);
    assert!(fair < idx);
}

#[test]
fn fair_unseeded_uses_index() {
    let mut m = market();
    let idx = (7_200.0f64).ln();
    m.index_ln = PodI128::new(from_f64(idx));
    let f = Forecaster::new(900);
    assert!((to_f64(f.fair_ln(&m).unwrap()) - idx).abs() < 1e-6);
}

#[test]
fn quote_conversion() {
    let qp = QuoteParams { half_spread_bps: 2.0, impact_bps_per_10k: 10.0, inventory_limit_usdc: 20_000 };
    let q = qp.quote(5 * ONE);
    assert_eq!(q.fair_ln, 5 * ONE);
    assert_eq!(q.half_spread, 2 * ONE / 10_000);
    assert_eq!(q.impact_per_usdc, 10 * ONE / 100_000_000);
    assert_eq!(q.inventory_limit, 20_000_000_000);
    assert!((to_f64(q.half_spread) - 0.0002).abs() < 1e-12);
    // 10bp per 10k USDC: 10k whole USDC of size moves the curve by 0.001 in ln.
    assert!((to_f64(q.impact_per_usdc) * 10_000.0 - 0.001).abs() < 1e-9);
}

fn ps(num_slots: u64, period_secs: u64) -> PerfSample {
    PerfSample { num_slots, period_secs }
}

#[test]
fn slot_ms_from_performance_samples() {
    assert_eq!(slot::estimate_slot_ms(&[ps(150, 60)]), 400);
    assert_eq!(slot::estimate_slot_ms(&[ps(300, 60)]), 200);
    // Weighted over all samples: 450 slots in 120s.
    assert_eq!(slot::estimate_slot_ms(&[ps(150, 60), ps(300, 60)]), 266);
    assert_eq!(slot::estimate_slot_ms(&[ps(60_000, 60)]), slot::MIN_SLOT_MS);
    assert_eq!(slot::estimate_slot_ms(&[ps(10, 60)]), slot::MAX_SLOT_MS);
    assert_eq!(slot::estimate_slot_ms(&[]), slot::DEFAULT_SLOT_MS);
    assert_eq!(slot::estimate_slot_ms(&[ps(0, 60)]), slot::DEFAULT_SLOT_MS);
    assert_eq!(slot::slot_ms_or_default(Err(anyhow::anyhow!("method not found"))), 400);
    assert_eq!(slot::slot_ms_or_default(Ok(vec![ps(300, 60)])), 200);
    assert_eq!((slot::MIN_SLOT_MS, slot::MAX_SLOT_MS, slot::DEFAULT_SLOT_MS), (50, 2_000, 400));
}

#[derive(Default)]
struct FakeChain {
    trader: Option<Trader>,
    operator: Option<Operator>,
    sent: std::sync::Mutex<Vec<Vec<Action>>>,
    /// None: the RPC rejects getRecentPerformanceSamples.
    samples: Option<Vec<PerfSample>>,
    /// Simulates a hung getRecentPerformanceSamples.
    samples_delay_ms: u64,
}

impl Chain for FakeChain {
    async fn market(&self) -> anyhow::Result<Market> {
        let mut m = market();
        m.index_ln = PodI128::new(from_f64(3_600f64.ln()));
        Ok(m)
    }
    async fn trader(&self) -> anyhow::Result<Option<Trader>> {
        Ok(self.trader)
    }
    async fn operator(&self) -> anyhow::Result<Option<Operator>> {
        Ok(self.operator)
    }
    async fn send(&self, actions: &[Action]) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(actions.to_vec());
        Ok(())
    }
    async fn slot(&self) -> anyhow::Result<u64> {
        Err(anyhow::anyhow!("no slot clock in tests"))
    }
    async fn perf_samples(&self) -> anyhow::Result<Vec<PerfSample>> {
        tokio::time::sleep(std::time::Duration::from_millis(self.samples_delay_ms)).await;
        self.samples.clone().ok_or_else(|| anyhow::anyhow!("method not found"))
    }
}

#[tokio::test]
async fn bootstrap_creates_everything_when_missing() {
    let c = FakeChain::default();
    bootstrap(&c, 50_000).await.unwrap();
    let sent = c.sent.lock().unwrap();
    assert_eq!(*sent, vec![vec![Action::CreateTrader, Action::Deposit(50_000_000_000), Action::RegisterOperator]]);
}

#[tokio::test]
async fn bootstrap_noop_when_active_operator() {
    let mut op: Operator = bytemuck::Zeroable::zeroed();
    op.status = OperatorStatus::Active as u8;
    let mut tr: Trader = bytemuck::Zeroable::zeroed();
    tr.is_operator = 1;
    let c = FakeChain { trader: Some(tr), operator: Some(op), ..Default::default() };
    bootstrap(&c, 50_000).await.unwrap();
    assert!(c.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bootstrap_reregisters_closed_operator() {
    let mut op: Operator = bytemuck::Zeroable::zeroed();
    op.status = OperatorStatus::Closed as u8;
    let tr: Trader = bytemuck::Zeroable::zeroed();
    let c = FakeChain { trader: Some(tr), operator: Some(op), ..Default::default() };
    bootstrap(&c, 0).await.unwrap();
    assert_eq!(*c.sent.lock().unwrap(), vec![vec![Action::RegisterOperator]]);
}

#[tokio::test]
async fn shutdown_unbonds_only_when_asked() {
    let c = FakeChain::default();
    shutdown(&c, false).await.unwrap();
    assert!(c.sent.lock().unwrap().is_empty());
    shutdown(&c, true).await.unwrap();
    assert_eq!(*c.sent.lock().unwrap(), vec![vec![Action::RequestUnbond]]);
}

#[tokio::test]
async fn loop_quotes_from_replay_then_unbonds() {
    let path = std::env::temp_dir().join(format!("tt-maker-replay-{}.csv", std::process::id()));
    let mut csv = String::from("ts_ms,price\n");
    for i in 0..8 {
        csv += &format!("{},{}\n", 1_000 * i, 150.0 * (1.0 + 0.01 * (i % 2) as f64));
    }
    std::fs::write(&path, csv).unwrap();
    let qp = QuoteParams { half_spread_bps: 2.0, impact_bps_per_10k: 10.0, inventory_limit_usdc: 20_000 };
    let opts = LoopOpts {
        feed: FeedMode::Replay(path.to_string_lossy().into_owned()),
        quote: qp,
        deposit_usdc: 50_000,
        slot_ms: Some(100),
        market_refresh_ms: 60_000,
        unbond_on_exit: true,
    };
    let c = FakeChain::default();
    run_with(&c, opts, tokio::time::sleep(std::time::Duration::from_millis(5_500))).await.unwrap();
    std::fs::remove_file(&path).ok();
    let sent = c.sent.lock().unwrap();
    assert_eq!(sent[0], vec![Action::CreateTrader, Action::Deposit(50_000_000_000), Action::RegisterOperator]);
    assert_eq!(sent.last().unwrap(), &vec![Action::RequestUnbond]);
    let fairs: Vec<i128> = sent[1..sent.len() - 1]
        .iter()
        .map(|a| match a[..] {
            [Action::UpdateCurve(q)] => {
                assert_eq!(q, qp.quote(q.fair_ln));
                q.fair_ln
            }
            _ => panic!("unexpected {a:?}"),
        })
        .collect();
    // First quote: fast vol seeded from the index, so fair = index.
    assert!((to_f64(fairs[0]) - 3_600f64.ln()).abs() < 1e-6);
    // 1% moves every second are far above the 1h-index vol: fair time shortens.
    assert!(to_f64(*fairs.last().unwrap()) < 3_600f64.ln() - 0.01, "{fairs:?}");
    // One update per 100ms tick over 5.5s.
    assert!((45..=60).contains(&fairs.len()), "{}", fairs.len());
}

/// Runs the loop on a flat replay feed for `ms` and returns the UpdateCurve count.
async fn flat_updates(c: &FakeChain, slot_ms: Option<u64>, ms: u64) -> Vec<CurveQuote> {
    let path = std::env::temp_dir().join(format!("tt-maker-flat-{}-{slot_ms:?}-{ms}.csv", std::process::id()));
    std::fs::write(&path, "ts_ms,price\n0,150\n").unwrap();
    let opts = LoopOpts {
        feed: FeedMode::Replay(path.to_string_lossy().into_owned()),
        quote: QuoteParams { half_spread_bps: 2.0, impact_bps_per_10k: 10.0, inventory_limit_usdc: 20_000 },
        deposit_usdc: 0,
        slot_ms,
        market_refresh_ms: 60_000,
        unbond_on_exit: false,
    };
    run_with(c, opts, tokio::time::sleep(std::time::Duration::from_millis(ms))).await.unwrap();
    std::fs::remove_file(&path).ok();
    let sent = c.sent.lock().unwrap();
    sent.iter()
        .filter_map(|a| match a[..] {
            [Action::UpdateCurve(q)] => Some(q),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn loop_sends_every_tick_even_when_unchanged() {
    let c = FakeChain { samples: Some(vec![ps(300, 60)]), ..Default::default() };
    let qs = flat_updates(&c, None, 1_050).await;
    // 200ms estimated slots: ticks at 0, 200, ..., 1000.
    assert!((5..=7).contains(&qs.len()), "{}", qs.len());
    assert!(qs.windows(2).all(|w| w[0] == w[1]), "{qs:?}");
}

#[tokio::test]
async fn loop_falls_back_to_400ms_without_samples() {
    let c = FakeChain::default();
    let qs = flat_updates(&c, None, 1_050).await;
    // Ticks at 0, 400, 800.
    assert!((2..=4).contains(&qs.len()), "{}", qs.len());
}

#[tokio::test]
async fn hung_slot_estimate_does_not_stall_quoting() {
    let c = FakeChain { samples: Some(vec![ps(300, 60)]), samples_delay_ms: 30_000, ..Default::default() };
    let qs = flat_updates(&c, None, 1_300).await;
    // Estimate gives up after 250ms and keeps the 400ms fallback: ticks at ~250, 650, 1050.
    assert!((2..=4).contains(&qs.len()), "{}", qs.len());
}

#[tokio::test]
async fn slot_ms_override_disables_estimation() {
    let c = FakeChain { samples: Some(vec![ps(150, 60)]), ..Default::default() };
    let qs = flat_updates(&c, Some(100), 1_050).await;
    assert!((9..=12).contains(&qs.len()), "{}", qs.len());
}

#[tokio::test]
async fn feed_spawn_replay_loops() {
    use tradingtime_maker::feed::Feed;
    let path = std::env::temp_dir().join(format!("tt-maker-feed-{}.csv", std::process::id()));
    std::fs::write(&path, "ts_ms,price\n0,150\n50,151\n").unwrap();
    let mut f = Feed::spawn(FeedMode::Replay(path.to_string_lossy().into_owned()));
    let a = f.next().await.unwrap();
    let b = f.next().await.unwrap();
    assert_eq!((a.price, b.price, b.ts_ms - a.ts_ms), (150.0, 151.0, 50));
    let c = f.next().await.expect("replay loops instead of ending");
    assert_eq!((c.price, c.ts_ms - b.ts_ms), (151.0, 50));
    std::fs::remove_file(&path).ok();
}
