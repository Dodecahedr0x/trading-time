use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tradingtime_indexer::{api, ingest, store::Store};

#[derive(Parser, Debug)]
#[command(name = "tradingtime-indexer")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:18899")]
    rpc: String,
    #[arg(long, default_value = "ws://127.0.0.1:18900")]
    ws: String,
    #[arg(long, default_value = "127.0.0.1:18787")]
    listen: String,
    #[arg(long, default_value = "target/dev-stack/indexer.sqlite")]
    db: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    let state = api::AppState::new(Store::open(&args.db)?);
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    tracing::info!("indexer listening on {} (db {})", args.listen, args.db.display());
    let cfg = ingest::Config { rpc_url: args.rpc, ws_url: args.ws };
    let ingest_task = tokio::spawn(ingest::run(cfg, state.clone()));
    tokio::select! {
        r = axum::serve(listener, api::router(state)) => r?,
        r = ingest_task => r??,
    }
    Ok(())
}
