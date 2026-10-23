# TradingTime: Design Document

*Status: research complete in simulation; on-chain program, services and web app implemented and running on a local Surfpool fork (§7). Last updated 2026-10-23.*

## 1. Summary

**Idea.** A perpetual future where time and price are inverted: instead of trading SOL's
price, traders trade the **expected time for SOL to move ±1%**. Long time wins when the
market calms down; short time wins when it gets wilder. Under Brownian motion
`E[τ] = ln(1+x)² / σ²`, so the contract is economically a perp on **inverse variance**,
quoted in seconds.

**Recommendation.** Build it as:

- **competing proprietary AMMs (propAMMs) behind an aggregator**: a few operators each quote
  an inventory-linked pricing curve from their own capital, refreshed every slot with
  update priority,
- referencing a **slow realized-volatility index** (7-day EWMA of variance, expressed as
  expected time-to-move), tied to the venue only through **funding**,
- with **log-time PnL**, a **~1× maximum leverage**, and **liquidation into the book down to
  the bankruptcy price, then auto-deleveraging (ADL)**, so no bad debt is possible.

**Why.** A sequence of experiments on synthetic paths and a year of real SOLUSDT 1-minute data
showed that every design quoting the *instantaneous* expected time fails: the quantity is
never observed, only estimated from noisy data, and on SOL that estimate swings 25× within
days. Whoever quotes it, whether an LP pool or market makers, must charge 3–20% spreads for
that noise or be drained by faster estimators. A slow index makes fair value forecastable
and brings costs down to vol-market levels (0.5–1.5% spreads, 27–93bp of volume to retail).
Competing propAMMs go further: 8–28bp to retail, operators still profitable, and the
public-data arbitrageur shut out (§4.10).

**What is still open.** Whether operators can reliably land curve updates before takers,
how the per-update index (§6, item 1) behaves on live Pyth data, and retail outcomes: even at low cost, most leveraged retail traders lose at least half
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

### 4.10 PropAMMs: update priority, size impact and competition

A propAMM (HumidiFi/SolFi style) is one operator's capital quoted as a curve: the marginal
price for the x-th dollar is `fair ± (half-spread + impact · x)`, and the whole curve shifts
against the operator's inventory. It refreshes every bar before takers act. Posted as a fine
ladder, several operators' curves are walked by best marginal price, which is what an
aggregator does when it splits an order. A stale mode refreshes quotes after takers act.

Real SOL, 7-day index, 60 retail traders, 1× leverage, 5 seeds:

| Setup | House | Spread | Retail cost | FastVol |
|---|---|---|---|---|
| Ladder makers ×6 (§4.8 design) | +$207k | 0.84% | 69bp | +$567k |
| Ladder makers ×6, stale | −$1.13M | 0.92% | 47bp | +$2.01M |
| propAMM ×1, 0.2% half-spread, 0.5% impact per $10k | +$717k | 0.51% | 52bp | shut out |
| propAMM ×1, stale | −$600k (wiped) | 0.50% | 39bp | +$1.16M |
| propAMM ×1, nearly flat curve (0.05% per $10k) | +$31k | 0.41% | 8bp | +$328k |
| propAMM ×1, 30-min estimator | +$177k | 0.51% | 41bp | +$399k |
| propAMM ×1, 60-min estimator | −$265k | 0.51% | 29bp | +$860k |
| propAMM ×1, 0.4% half-spread (monopoly pricing) | +$845k | 0.91% | 71bp | shut out |
| propAMM ×3, 0.05% / 0.25% per $10k | +$374k | 0.12% | 28bp | shut out |
| propAMM ×3, 0.02% / 0.1% per $10k | +$354k | 0.05% | 8bp | shut out |

Retail cost here is execution cost (spread + impact + fee against the pre-trade mark), which
separates trading costs from market moves.

1. **Update priority matters most.** Quotes one minute stale turn every maker setup from
   profit to ruin. On Solana this means cheap, prioritized curve-update transactions.
2. **The operator still needs the fastest vol estimate.** Slower estimators lose to FastVol.
3. **Size-dependent impact blocks the arbitrageur.** Its large trades stop paying. Because
   the curve shifts with inventory, splitting an order across bars does not escape the
   impact. Larger retail trades pay it too.
4. **Competition is what makes it cheap.** One operator earns most at wide settings; three
   competing operators at tight settings still earn about 20% a year on $600k each.

Retail still loses $380–700k a year in total and about 35% are wiped out, from volatility
and turnover rather than costs.

## 5. Recommended design

### 5.1 Contract

| Parameter | Value | Rationale |
|---|---|---|
| Underlying | SOL/USD | |
| Band | ±1% | |
| Index | `T_index = c · ln(1.01)² / v`, v = EWMA of variance, half-life 7 days. Simulated on 1-min Parkinson bars; built on-chain as r²/Δt of every Pyth update (§7) | §4.7 |
| Calibration c | ≈0.91, refit periodically | §2 |
| Index bounds | 60 s – 7 days | numerical safety |
| PnL | log-time: `q · Δln(price)` | symmetric payoff, §4.3 |
| Funding | continuous, `q · k · ln(mark / index)`, k = 1/day | anchors the book to the index |
| Mark price | median centre of fresh operator curves (built); falls back to the index after 60 s without fresh curves | robust to one stale or outlying operator |

### 5.2 Venue: competing propAMMs

- **Operators:** at least three, each quoting its own capital as an inventory-linked curve,
  `fair ± (half-spread + impact · size)`, with the center shifted by `impact · inventory`.
- **Updates:** every slot, before takers, with priority (§4.10). Operators price from a fast
  vol estimate (15 minutes or faster on 1-minute bars).
- **Routing:** an aggregator splits each taker order across curves by best marginal price,
  with a slippage limit. Taker fee 2bp (built; 5bp in simulation), paid into the insurance fund until
  it holds 5% of open interest, then to the treasury (§7.4).
- **Starting parameters:** half-spread 0.02–0.05%, impact 0.1–0.25% per $10k, inventory limit
  about 10% of operator capital. Competition, not the protocol, should set the final values.
- **Fallback:** a plain order book with designated makers (§4.8) if operators cannot get
  reliable update priority.

### 5.3 Risk

| Parameter | Value |
|---|---|
| Max leverage | 1× at launch (§4.9); revisit with real flow data |
| Maintenance margin | 5% of \|q\| |
| Liquidation | market order across operator curves with limit = bankruptcy price; remainder auto-deleveraged pro rata against opposite positions at that price |
| Liquidation fee | 1% of closed notional → insurance fund |
| Insurance fund | liquidation fees, plus taker fees until it holds 5% of open interest; covers negative equity before ADL |
| ADL counterparties | in-profit opposite traders, pro rata at the bankruptcy price; operators are excluded, so a position only operators could absorb stays open until traders take the other side |
| Bad debt | impossible by construction (ADL at bankruptcy price); ADL shortfall is tracked |

### 5.4 What this is not

The recommended product is no longer "the expected time to the next 1% move right now". It
is "the time-to-move implied by roughly the last week of SOL volatility, expected a day
ahead". That is close to a **perpetual variance swap quoted in time units**. The instantaneous
version is not tradeable at reasonable cost.

## 6. Risks and open questions

1. **Index oracle.** Parkinson variance needs per-minute highs and lows, which Pyth does not
   publish. The built index folds every Pyth update into the EWMA as r²/Δt instead (close-to-close
   on irregular steps), rejecting prices older than 60 s. It is noisier than Parkinson per sample
   but has no bar construction to attack; the 7-day half-life does the smoothing. A fresh market
   takes its starting index as an `InitializeMarket` argument rather than waiting days to converge.
2. **Update priority.** The propAMM result depends on operators landing curve updates before
   takers in every slot. Stale quotes cost operators everything in simulation.
3. **Operator competition.** One operator maximizes profit at wide spreads (0.91%, 71bp to
   retail). Retail costs are only low if several operators compete for aggregator flow.
4. **Informed flow.** On ladder books FastVol earned $300–700k a year; propAMM impact shut it
   out. Real arbitrageurs are smarter (tick data, multi-venue, options-implied vol, watching
   flow), so the protection from impact may be weaker in practice.
5. **Retail outcomes.** Even at 27bp cost, 38–50% of simulated retail traders were wiped
   out in a year at default leverage. Product risk should be disclosed; leverage and
   turnover limits are the levers.
6. **Simulation limits.**
   - One real price path (one year): seeds only vary agent behaviour.
   - Agents are scripted, not adaptive (except maker spreads).
   - All makers share one fair-value model, which likely overstates maker profits.
   - Latency is modelled only as whole-bar staleness (`--maker-lag`), not per slot.
   - PropAMM parameters are fixed per run, not set by adaptive competition.
   - Execution cost is measured against the mark, which falls back to the index when one
     side is empty, so single-operator runs are noisier.
   - The pool's borrow rate (0.86%/day) is high and dominates pool retail costs.
   - The Informed agent is omniscient, an upper bound on informed flow.

## 7. Solana implementation

Built and tested on a local Surfpool mainnet fork (real USDC and the Pyth SOL/USD feed account).

### 7.1 Components

| Path | Role |
|---|---|
| `programs/tradingtime` | Pinocchio program, `no_std`, zero-copy accounts |
| `crates/math` | fixed-point log-time math (Fx = i128, 48 fractional bits): ln/exp, index, positions, funding, curve routing; shared by the program, services, web app and simulator |
| `crates/interface`, `crates/client` | account and instruction layouts, events; Rust client and builders |
| `services/keeper` | permissionless crank: index on every new Pyth price, funding, mark, liquidations with ADL, limit-order fills, closing expired orders |
| `services/maker` | propAMM operator: fair value from a 15-minute EWMA of Binance (Coinbase fallback) trades, curve update every slot |
| `services/indexer` | Carbon + SQLite; REST and WebSocket API, including protocol analytics |
| `services/bots` | local-stack traders (noise, momentum, mean-revert, whale, limit, informed) and a Pyth price driver with calm/storm regimes |
| `apps/web` | Dioxus web app (trade, analytics, docs), Wallet Standard wallets |

### 7.2 Accounts and instructions

| Account | Size | Contents |
|---|---|---|
| `Market` | 384 B | parameters, index (variance EWMA, last Pyth price), mark, funding index, open interest, insurance, treasury |
| `Trader` | 120 B | cash, q, Σ size·ln(price) cost, funding snapshot, long/short limit reservations |
| `Operator` | 160 B | bond, curve (fair, half-spread, impact, inventory limit), last update slot, status |
| `LimitOrder` | 112 B | side, size, filled, limit, expiry, reservation |

Instructions: `initialize_market`, `update_params`, `set_paused`, `create_trader`, `deposit`,
`withdraw`, `register_operator`, `update_curve`, `request_unbond`, `complete_unbond`, `swap`,
`place_limit_order`, `cancel_limit_order`, `execute_limit_order`, `crank_index`,
`crank_funding`, `crank_mark`, `liquidate`, `withdraw_treasury`.

### 7.3 Departures from the sketch

- **No separate index account.** The index state lives in `Market`, and `crank_index` folds each
  new Pyth price directly (§6, item 1).
- **Routing on-chain.** `swap` takes up to 8 operators and routes in 64 chunks by best marginal
  price; an 8-operator swap needs about 530k compute units, so clients request 1.4M.
- **Mark** is the median centre of fresh curves (§5.1), refreshed by trades and `crank_mark`.
- **ADL counterparties** are supplied by the caller (the keeper ranks in-profit opposite traders)
  and closed pro rata; operators are never auto-deleveraged.
- **Operators** are permissionless: 1,000 USDC bond, 10,000 USDC minimum capital, curves stale
  after 10 slots, 1-day unbonding, and they can re-register after unbonding.
- **Limit orders** reserve margin net of the position (a reducing order needs no extra
  collateral), are filled by any keeper for a 1bp fee, and anyone can close an expired one.

### 7.4 Fees and the insurance fund

The local stack showed retail losses are dominated by turnover costs, not by the counterparty.
In an early run, bots churned $27M on $1M of deposits in an hour; traders lost $21.8k, of which
$13.6k was fees and $8.2k went to makers, and only 2 of 14 traders ended in profit. Fees were also
routed entirely to the treasury, leaving the insurance fund empty.

The built protocol therefore:

- charges a **2bp** taker fee (down from 5bp);
- pays fees into the **insurance fund until it holds 5% of open interest** (`insurance_target_bps`),
  and only then to the treasury.

Safety rests on maintenance margin, liquidation to the bankruptcy price and ADL, not on fees; the
insurance fund absorbs negative equity before anyone is auto-deleveraged. With realistic bot
behaviour (positions held minutes to hours, 2–4× deposits of turnover per hour), a 30-minute run
cost traders $133 in fees, all to insurance, and 8 of 14 traders were in profit.

## 8. Reproducing the results

On-chain system (needs Surfpool, the Solana CLI, `dx`, Node and pnpm; see the README):

```bash
just test-all        # unit, LiteSVM, clippy, wasm, then Surfpool integration and browser tests
just sim             # local stack with 4 makers, 12 bot traders and a moving Pyth price
```

Simulator:

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
# competing propAMMs, 7d index, 60 retail traders, 1x leverage (add --maker-lag 1 for stale quotes)
cargo run --release -p cli -- --venue book --feed csv --path $D --steps 600000 --index-hl 604800 --mm 0 --prop 3 --prop-h 0.0002 --prop-k 0.001 --noise 60 --max-lev 1
# synthetic paths: --feed gbm | regime | jump, vary --seed
```

Commit history follows the sequence of §4: `6261da8` (pool and mitigations) → `fbc03f1`
(order book) → `c491a6f` (slow index) → `aedd00b` (realistic retail) → `41552d4` (competing
makers) → `1af9da8` (leverage metrics) → `a2e1e1e` (propAMMs, stale quotes, execution cost).
