# TradingTime

A perpetual where **time and price are inverted** ([tradingti.me](https://tradingti.me)).
You trade the expected time for SOL to move ±1%, quoted in time ("1h 30m"). **Long time**
profits when markets calm down and moves take longer; **short time** profits when they get
wilder. Under Brownian motion E[τ] = ln(1+x)² / σ², so the contract is economically a
perpetual on **inverse variance**.

This repository holds the research simulator and the full on-chain system: a Solana program,
off-chain services and a web app. Design rationale and findings: [docs/design.md](docs/design.md).

## Layout

```
programs/tradingtime            Pinocchio program (log-time perp, propAMM curves, limit orders, liquidation + ADL)
crates/math                     fixed-point log-time math shared by the program, services, web app and simulator
crates/interface, crates/client account/instruction layouts and events; Rust client
services/indexer                Carbon + SQLite indexer, REST + WebSocket (incl. /v1/analytics/*)
services/keeper                 cranks index/funding/mark, liquidates with ADL, fills and expires limit orders
services/maker                  propAMM market maker (Binance/Coinbase feeds, or replay)
services/bots                   simulated traders + Pyth price driver for local stacks
apps/web                        Dioxus web app: Trade, Analytics and Docs pages, Wallet Standard wallets
tests/integration, e2e          Surfpool integration tests, Playwright browser tests
crates/core, cli, app           research simulator (library, headless backtests, desktop UI)
```

## Quick start

Needs Rust (pinned in `rust-toolchain.toml`), [`just`](https://github.com/casey/just),
[Surfpool](https://surfpool.run), the Solana CLI, `dx` (Dioxus CLI), Node and pnpm.

```bash
just e2e-install     # once: Playwright + Chromium for the browser tests
just sim             # local stack with 4 makers, 12 bot traders and a moving Pyth price; serves the app
```

`just sim` prints the app URL. Open `#/analytics` for protocol activity and trader PnL, and
`#/docs` for how the product works. Stop everything with `just down`.

| Command | |
|---|---|
| `just test` | workspace unit and LiteSVM program tests |
| `just test-all` | plus clippy, wasm build, a fresh stack, Surfpool integration and browser tests |
| `just up` | quiet local stack: Surfpool fork, program, indexer, keeper, 2 makers |
| `just sim [bots] [tick_ms]` | busy stack, e.g. `just sim 30 800` |
| `just status`, `just logs <service>`, `just stats` | service health, logs (`bots`, `keeper`, `maker1`…), indexer summary |
| `just backtest …`, `just simulator` | research simulator |

Local ports (override with the environment variables):

| Service | Port | Variable |
|---|---|---|
| Surfpool RPC / WebSocket | 18899 / 18900 | `TT_RPC_PORT`, `TT_WS_PORT` |
| Indexer API | 18787 | `TT_API_PORT` |
| Web app | 14173 | `TT_WEB_PORT` |

The local stack deploys with `programs/tradingtime/program-keypair.json`, which is not committed;
generate one with `solana-keygen new -o programs/tradingtime/program-keypair.json` and update
`PROGRAM_ID` in `crates/interface` to match, or use the keypair shared with you.

## Protocol at a glance

| | |
|---|---|
| Index | 7-day EWMA of SOL variance from every Pyth SOL/USD update, expressed as time to move 1% |
| Mark | median centre of the makers' fresh pricing curves |
| PnL | `q · ln(T_exit / T_entry)`: doubling and halving are mirror images |
| Funding | continuous, `q · ln(mark / index)` per day |
| Leverage | 1× maximum; maintenance margin 5% |
| Liquidation | against maker curves down to the bankruptcy price, then auto-deleveraging in-profit traders |
| Fees | 2bp taker, into the insurance fund until it holds 5% of open interest, then the treasury; 1bp keeper fee on limit fills; 1% liquidation penalty to insurance |
| Makers | permissionless propAMM operators: 1,000 USDC bond, 10,000 USDC minimum capital, curves stale after 10 slots |

Exact values live in `Params::DEFAULT` (`crates/interface`); the in-app Docs page reads them
from there.

## Research simulator

The simulator compares venues for the product: an LP pool at an oracle price (`engine.rs`), a
central limit order book (`book.rs`, `book_sim.rs`) and competing propAMMs. `--index-hl` sets the
index half-life; a slow index (e.g. 7 days) makes fair value forecastable and is what keeps
spreads tight on real SOL data.

```bash
cargo run --release -p cli -- --feed regime --seed 7 --steps 200000   # gbm | regime | jump | csv --path f.csv
cargo run -p app
```

CSV input: `ts,open,high,low,close[,...]` (headers skipped, ms timestamps detected; Binance klines
work as-is). `data/sample.csv` is a synthetic 7-day regime-switching path (`cli --export`).
