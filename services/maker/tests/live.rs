use tradingtime_maker::feed::{FeedMode, run_feed};

async fn first_tick(mode: FeedMode) -> Option<tradingtime_maker::feed::PriceTick> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(run_feed(mode, tx));
    let t = tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await.ok().flatten();
    task.abort();
    t
}

#[tokio::test]
#[ignore = "network"]
async fn live_coinbase_ticks() {
    let t = first_tick(FeedMode::Coinbase).await.expect("coinbase tick");
    assert!(t.price > 1.0, "{t:?}");
}

#[tokio::test]
#[ignore = "network"]
async fn live_auto_ticks() {
    let t = first_tick(FeedMode::Auto).await.expect("auto tick");
    assert!(t.price > 1.0, "{t:?}");
}
