# TradingTime

Simulator for [tradingti.me](https://tradingti.me).

A perpetual where **time and price are inverted**: you trade the expected time for
spot to move ±x% (default ±1%). Each band hit resets the reference, so the market is
perpetual. **Long time** profits when markets calm down; **short time** when they get wilder.

Under Brownian motion E[τ] = ln(1+x)² / σ², so the contract is economically a perp on
**inverse variance**. Trades fill at a model oracle (EWMA Parkinson vol → E[τ]) against
an LP pool; the realized hitting-time index is tracked to check calibration.

Two venues: an LP pool that takes the other side at the oracle price (`engine.rs`),
and a central limit order book where market makers quote and the oracle is only the
funding index (`book.rs`, `book_sim.rs`; CLI `--venue book`). `--index-hl` sets the
oracle/index half-life; a slow index (e.g. 7 days) makes fair value forecastable and
is what keeps spreads tight on real SOL data.

```
crates/core   simulator library (feeds, band, vol, oracle, pool, book, agents, engines)
crates/cli    headless backtests
crates/app    Dioxus desktop UI
```

```bash
cargo test --workspace
cargo run --release -p cli -- --feed regime --seed 7 --steps 200000   # gbm | regime | jump | csv --path f.csv
cargo run -p app
```

CSV input: `ts,open,high,low,close[,...]` (headers skipped, ms timestamps detected — Binance klines work as-is).
`data/sample.csv` is a synthetic 7-day regime-switching path (`cli --export`).

Design rationale and findings: [docs/design.md](docs/design.md).

## On-chain system

```
programs/tradingtime   Pinocchio program (log-time perp, propAMM curves, limit orders, liquidation + ADL)
crates/{math,interface,client}   shared fixed-point math, account/ix layouts, Rust client
services/indexer       Carbon + SQLite indexer, REST + WebSocket on :18787 (incl. /v1/analytics/*)
services/keeper        cranks index/funding/mark, liquidates, fills limit orders
services/maker         propAMM market maker (Binance/Coinbase feeds, or replay)
services/bots          simulated traders + Pyth price driver for local stacks
apps/web               Dioxus web app (trade + analytics), wallet via Wallet Standard
tests/integration, e2e Surfpool integration tests and Playwright browser tests
```

Tasks run through [`just`](https://github.com/casey/just) (`just` lists them). Needs Surfpool, the Solana CLI, `dx`, Node + pnpm.

```bash
just test            # workspace unit + LiteSVM tests
just test-all        # + clippy, wasm, fresh stack, integration + browser tests
just up              # quiet local stack (Surfpool fork, program, indexer, keeper, 2 makers)
just sim             # busy stack: 4 makers, 12 bot traders, moving Pyth price; serves the app
just sim 30 800      # 30 bots ticking every 800 ms
just logs bots       # follow a service log; `just status`, `just stats`, `just down`
```

`just sim` prints the app URL; the analytics view is at `#/analytics` (volume, open interest,
trader leaderboard and PnL, makers, live trades, liquidations).
