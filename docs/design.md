# Time Perp: Design Document

*Status: research complete in simulation; no on-chain code yet. Last updated 2026-10-20.*

## 1. Summary

**Idea.** A perpetual future where time and price are inverted: instead of trading SOL's
price, traders trade the **expected time for SOL to move ±1%**. Long time wins when the
market calms down; short time wins when it gets wilder. Under Brownian motion
`E[τ] = ln(1+x)² / σ²`, so the contract is economically a perp on **inverse variance**,
quoted in seconds.

**Recommendation.** Build it as:

- a **central limit order book** (market makers quote, traders take each other's sides),
- referencing a **slow realized-volatility index** (7-day EWMA of variance, expressed as
  expected time-to-move), tied to the book only through **funding**,
- with **log-time PnL**, a **~1× maximum leverage**, and **liquidation into the book down to
  the bankruptcy price, then auto-deleveraging (ADL)**, so no bad debt is possible.

**Why.** A sequence of experiments on synthetic paths and a year of real SOLUSDT 1-minute data
showed that every design quoting the *instantaneous* expected time fails: the quantity is
never observed, only estimated from noisy data, and on SOL that estimate swings 25× within
days. Whoever quotes it, whether an LP pool or market makers, must charge 3–20% spreads for
that noise or be drained by faster estimators. A slow index makes fair value forecastable
and brings costs down to vol-market levels (0.5–1.5% spreads, 27–93bp of volume to retail).

**What is still open.** Market-maker bootstrapping, oracle design for the index on-chain,
and retail outcomes: even at low cost, most leveraged retail traders lose at least half
their capital in a year on this instrument.

## 2. The product

| Term | Definition |
|---|---|
| Band | ±x around a reference price, default x = 1%. |
| Hitting time τ | Seconds until spot leaves the band; the reference then resets to spot. |
| Model price | `T(σ) = ln(1+x)² / σ²` seconds, σ per √second. 80% annualized vol ≈ 82 min. |
| Long time | Profits when T rises (volatility falls). |
| Short time | Profits when T falls (volatility rises). |
| Log-time PnL | Exposure q (USD) earns `q · Δln T`. Symmetric: doubling and halving are mirror images. |

Because `T ∝ 1/σ²`, the contract amplifies volatility moves: a vol regime change from 150%
to 50% multiplies T by 9. On real SOL over Oct 2025 – Sep 2026, a 4-hour realized index
ranged from about 600 s (≈220% vol) to 16,000 s (≈44% vol) within weeks.

**Calibration.** The formula holds on simulated Brownian paths (realized/predicted hitting
time 1.007 over 2,442 hits). On real SOL it reads 0.91: fat tails make moves arrive about
10% sooner. A production index should apply a fitted correction factor.

## 3. The simulator

Rust workspace, deterministic for a given seed.

```
crates/core   feeds, band, vol estimators, oracle, implied vol, pool, order book, agents, engines
crates/cli    headless backtests (`--venue pool|book`, many flags)
crates/app    Dioxus desktop UI (pool venue)
```

- **Feeds:** GBM, Markov regime switching (50% ↔ 150% vol), GBM with jumps, CSV replay
  (Binance klines, µs or ms timestamps). Synthetic bar highs/lows are drawn from the
  Brownian-bridge maximum so range-based estimators are unbiased.
- **Volatility estimator:** EWMA of the Parkinson range variance plus the squared open gap.
- **Agents:**
  - **Noise:** retail with heterogeneous profiles: 0.2–2 trades/day, holds of 2 h–3 days,
    1–5× leverage, stakes of 10–50%, stop-loss and take-profit, random sides.
  - **FastVol:** public-data arbitrageur with a faster vol estimate (15 min / 1 h).
  - **Informed:** knows the hidden true σ; synthetic feeds only, an upper bound.
  - **Momentum:** trend follower.
  - **Chaser:** trades the gap between the capped mark and its uncapped target.
  - **Market makers:** ladder quotes around an index forecast, inventory skew and limits,
    spreads widened by recent fair-value moves, optionally adapted daily from PnL.
- **Invariants checked every step:** cash conservation (pool), total value conservation and
  Σq = 0 (book).
- **Metrics:** house PnL; per-agent-type PnL, volume, cost in bp of volume, wiped
  (<1% capital left) and lost ≥50%; spreads; calibration; ADL volume and shortfall.

48 unit tests, including a Monte-Carlo check of the E[τ] formula.

## 4. Findings

Each step below was a response to the previous one's failure. All real-data numbers use
SOLUSDT 1-minute bars, Oct 2025 – Sep 2026 (525,600 bars), averaged over 5 agent seeds.

### 4.1 Oracle-priced LP pool: the oracle's lag and noise are both exploitable

First design: trades fill at the model price from a realized-vol estimate; an LP pool takes
the other side (GMX/Jupiter style).

- **Fast estimator (30-min half-life):** the mark is mostly estimator noise, about 18%/hour
  around the truth, and mean-reverting. An agent knowing the true σ harvested it.
- **Slower estimator (4 h):** noise falls (about 2%/hour at a 6-hour half-life) but after a
  regime switch the mark drifts
  predictably for hours. Informed and momentum traders drained the LP: **−99% to −100%**
  on regime and jump paths, while +36% on constant-vol paths.

Lesson: an oracle price is either noisy or lagged; both leak value to faster traders.

### 4.2 Price caps and worst-of pricing: solvency, at the price of spreads

- **Per-minute cap on mark moves:** removed bad debt from jumps but turned gaps into a
  public, predictable drift; a Chaser agent took $450–820k.
- **Worst-of pricing** (fill at the worse of capped mark and uncapped target): closed the
  Chaser edge, but the gap *is* the spread. After a jump it stays open for 17–70 hours.

### 4.3 Implied vol and log quoting did not fix it

- **Blending implied vol:** with even 5% noise in implied vol, the 1/σ² amplification turned
  it into about 10% mark noise that informed traders harvested (LP −40% on constant-vol
  paths). Perfect implied vol made regime switches worse, because the mark jumped faster.
- **Log-time quoting:** fixed per-position convexity (a 9× move pays 2.2× notional, not 8×)
  but barely moved LP results, and increased bad debt on single-bar gaps.

### 4.4 The real failure: liquidations leave the house one-sided

A clean test (20 zero-edge random traders, $5M pool, perfect oracle) showed random traders
winning **+$5M** on regime paths in every seed. Mechanism: a 9× regime move is 2.2 in log
terms while a 3× position is liquidated after about 0.3. Losers are cut off early; the house
alone faces winners for the rest of the move. It disappeared only at 0.5× leverage.

- **ADL** (when liquidations unbalance the pool, close the opposite side pro rata) cut the
  loss from −$5.0M to −$2.9M.
- **Net-skew cap** (refuse opens that grow |long − short| past a fraction of liquidity)
  handled the rest, because the pool effectively sells an option on any net skew it holds.
- **All mitigations together** (log quoting, ADL, 0.5% skew cap, per-minute cap, worst-of):
  LP **+12% to +50% in every run** across regime, jump and GBM paths, at 1× and 3× leverage.

### 4.5 Real SOL data confirms it, and public data is enough to exploit the pool

| Pool configuration (4h index) | LP | FastVol (public data only) |
|---|---|---|
| No mitigations | −104% | +$631k |
| All mitigations, cap 0.002 | **+38%** | +$58k |

The cost moved to traders: the worst-of gap averaged **11.2%** (cap 0.002) and still 1.3%
at cap 0.01, where the LP made only +12%.

### 4.6 Order book: a race over estimator speed

Same instrument on a CLOB, with the realized index used only for funding:

| Market-maker estimator | Spread | Outcome |
|---|---|---|
| Slower than fastest taker (1 h) | 2.3% but one-sided 98% of bars | makers wiped out; FastVol +$713k |
| As fast (15 min) | 6.3% | makers +$493k; every taker wiped out |
| Faster (3.75 min) | 20.1% | same, wider |

Makers slower than the fastest taker get picked off on stale quotes; faster makers must quote
wide because faster estimates are noisier. The venue does not change the cost of trading:
the book's 6% matched the pool's worst-of cost.

### 4.7 The slow index is what makes fair value forecastable

Make the funding index a slow EWMA and price the perp as the index expected one funding
horizon (~1 day) ahead. A 7-day average drifts only slightly per day, so the forecast is easy.

| Index half-life | Book spread (15-min makers) | Mid vs index |
|---|---|---|
| 4 hours | 11.1% | 51% |
| 1 day | 5.1% | 31% |
| 3 days | 2.4% | 14% |
| **7 days** | **1.4%** | **7%** |

A 1.4% spread in time units is roughly 0.7% in vol, about half a vol point at 70% vol,
which is in line with real volatility markets. The pool with a 7-day index needed no cap or
worst-of at all (LP +29% with log quoting, ADL and skew cap).

### 4.8 With realistic retail flow, the break-even spread follows market-making theory

With heterogeneous retail traders and competing makers, the break-even spread is set by the
ratio of uninformed to informed flow:

| Retail traders | Break-even spread | Tightest profitable spread tested | Retail cost there |
|---|---|---|---|
| 20 | 0.8–1.5% | 1.48% (makers +$54k) | 93bp |
| 60 | 0.5–0.8% | 0.80% (makers +$327k) | 77bp |
| 150 | below 0.5% | 0.52% (makers +$435k) | 27bp |

Comparison at 20 retail traders, 7-day index:

| Venue | House | Retail cost | Note |
|---|---|---|---|
| Pool, 2% skew cap | LP +$33k/yr (+3.3%) | 40bp | FastVol takes ≈4× what the LP earns |
| Book, 1.5% spread | makers +$54k | 93bp | |

Makers adapting their spread daily from a 5-day PnL average settled too tight (0.38%) within
a year: daily PnL is dominated by inventory marking, so the signal is noisy. Earlier rules
failed worse, ratcheting to 9–24% spreads or collapsing to 0.02%.

### 4.9 Leverage protects against wipe-outs, not against losses

Venue-wide maximum leverage, 7-day index:

| Venue | Max leverage | Retail wiped | Retail lost ≥50% | House |
|---|---|---|---|---|
| Book (60 retail) | 5× | 42% | 84% | +$327k |
| Book | 1× | 36% | 80% | +$207k |
| Book | 0.25× | 0% | 48% | +$172k |
| Pool (20 retail) | 5× | 3% | 46% | +$33k |
| Pool | 1× | 0% | 22% | −$118k |

- **Average retail position is already about 0.8× equity,** so caps above 2× barely bind.
- **Losses come from turnover × cost:** about 100× capital traded per year at 60–90bp.
- **The pool's LP needs leveraged retail fees and loses money at ≤2× leverage,** a
  structural conflict with protecting retail. The book stays profitable for makers at 1×.

## 5. Recommended design

### 5.1 Contract

| Parameter | Value | Rationale |
|---|---|---|
| Underlying | SOL/USD | |
| Band | ±1% | |
| Index | `T_index = c · ln(1.01)² / v`, v = EWMA of Parkinson variance on 1-min bars, half-life 7 days | §4.7 |
| Calibration c | ≈0.91, refit periodically | §2 |
| Index bounds | 60 s – 7 days | numerical safety |
| PnL | log-time: `q · Δln(price)` | symmetric payoff, §4.3 |
| Funding | continuous, `q · k · ln(mark / index)`, k = 1/day | anchors the book to the index |
| Mark price | book mid; index if one side is empty | |

### 5.2 Venue: CLOB

- Price-time priority, makers post ladders; takers send market orders with slippage limits.
- Taker fee 5bp; maker fee 0.
- Recommended launch with **designated market makers** under quoting obligations (max spread,
  minimum size) until organic flow is large enough: §4.8 shows spreads depend on retail
  volume, and §4.6 shows a thin or slow maker set collapses.

### 5.3 Risk

| Parameter | Value |
|---|---|
| Max leverage | 1× at launch (§4.9); revisit with real flow data |
| Maintenance margin | 5% of \|q\| |
| Liquidation | market order into the book with limit = bankruptcy price; remainder auto-deleveraged pro rata against opposite positions at that price |
| Liquidation fee | 1% of closed notional → insurance fund |
| Bad debt | impossible by construction (ADL at bankruptcy price); ADL shortfall is tracked |

### 5.4 What this is not

The recommended product is no longer "the expected time to the next 1% move right now". It
is "the time-to-move implied by roughly the last week of SOL volatility, expected a day
ahead". That is close to a **perpetual variance swap quoted in time units**. The instantaneous
version is not tradeable at reasonable cost.

## 6. Risks and open questions

1. **Index oracle.** Parkinson variance needs per-minute highs and lows. Pyth publishes
   prices, not OHLC bars, so a keeper must aggregate updates into bars, or the index must
   use a different estimator. The index must be manipulation-resistant; a 7-day EWMA helps,
   but bar construction is an attack surface.
2. **Market-maker bootstrapping.** Spreads depend on retail volume; early liquidity needs
   designated makers or incentives.
3. **Informed flow.** FastVol earned $300–700k a year at every book setting. Better
   estimators than mine exist (tick data, multi-venue, options-implied), so real informed
   flow may be stronger.
4. **Retail outcomes.** Even at 27bp cost, 38–50% of simulated retail traders were wiped
   out in a year at default leverage. Product risk should be disclosed; leverage and
   turnover limits are the levers.
5. **Simulation limits.**
   - One real price path (one year): seeds only vary agent behaviour.
   - Agents are scripted, not adaptive (except maker spreads).
   - All makers share one fair-value model, which likely overstates maker profits.
   - No latency or transaction ordering.
   - The pool's borrow rate (0.86%/day) is high and dominates pool retail costs.
   - The Informed agent is omniscient, an upper bound on informed flow.

## 7. Solana implementation sketch

| Account | Contents |
|---|---|
| `Market` | parameters, funding index, insurance balance, fees |
| `IndexState` | current bar (open/high/low), EWMA variance, last update slot |
| `OrderBook` | bids/asks in a slab, Phoenix/OpenBook style, ln-price ticks |
| `Trader` | cash, q, cost basis, funding snapshot, open order IDs |

| Instruction | Notes |
|---|---|
| `deposit` / `withdraw` | withdraw checks initial margin at mark |
| `place_order` / `cancel` | makers; margin-trimmed like `Book::replace_quotes` |
| `market_order` | takers; slippage limit |
| `crank_index` | permissionless; folds Pyth updates into the bar, rolls bars, updates EWMA |
| `crank_funding` | permissionless; accrues `k · ln(mark/index) · dt` into the funding index |
| `liquidate` | permissionless; book fill to bankruptcy price, then ADL over a provided counterparty list |

- **Floating point.** Fixed-point log math is needed: positions store cost as Σ size·ln(price).
  Use a lookup-table or polynomial `ln`/`exp` in Q64.64.
- **ADL.** Pro rata over *all* opposite accounts does not fit in one transaction; production
  ADL should rank by profit × leverage and take counterparties from a crank-maintained queue.
- **Reference implementation.** `crates/core/src/book.rs` (matching, accounts, funding,
  liquidation, ADL) is the logic to port; its invariants (Σq = 0, value conservation, no
  negative equity after liquidation) should become on-chain property tests.

## 8. Reproducing the results

```bash
cargo test --workspace
# real data: put Binance SOLUSDT 1m klines in data/binance/ and concatenate into one CSV
D=data/binance/SOLUSDT-1m-12mo.csv
# pool, 4h index, all mitigations
cargo run --release -p cli -- --feed csv --path $D --steps 600000 --log 1 --adl 1 --max-skew 0.005 --cap 0.002 --worst-of 1
# pool, 7d index
cargo run --release -p cli -- --feed csv --path $D --steps 600000 --index-hl 604800 --log 1 --adl 1 --max-skew 0.02 --worst-of 0
# book, 7d index, fixed spread multiplier, 60 retail traders, 1x leverage
cargo run --release -p cli -- --venue book --feed csv --path $D --steps 600000 --index-hl 604800 --mm-adapt 0 --mm-scale 1 --noise 60 --max-lev 1
# synthetic paths: --feed gbm | regime | jump, vary --seed
```

Commit history follows the sequence of §4: `6261da8` (pool and mitigations) → `fbc03f1`
(order book) → `c491a6f` (slow index) → `aedd00b` (realistic retail) → `41552d4` (competing
makers) → `1af9da8` (leverage metrics).
