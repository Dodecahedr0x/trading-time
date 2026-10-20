# Time Perp

A perpetual where **time and price are inverted**: you trade the expected time for
spot to move ±x% (default ±1%). Each band hit resets the reference, so the market is
perpetual. **Long time** profits when markets calm down; **short time** when they get wilder.

Under Brownian motion E[τ] = ln(1+x)² / σ², so the contract is economically a perp on
**inverse variance**. Trades fill at a model oracle (EWMA Parkinson vol → E[τ]) against
an LP pool; the realized hitting-time index is tracked to check calibration.

Two venues: an LP pool that takes the other side at the oracle price (`engine.rs`),
and a central limit order book where market makers quote and the oracle is only the
funding index (`book.rs`, `book_sim.rs`; CLI `--venue book`).

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
