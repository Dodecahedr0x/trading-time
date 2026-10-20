# TradingTime task runner. `just` lists every recipe.

set shell := ["bash", "-euo", "pipefail", "-c"]

rpc := "http://127.0.0.1:8899"
api := "http://127.0.0.1:8787"
web_port := env_var_or_default("TT_WEB_PORT", "4173")
app_url := "http://127.0.0.1:" + web_port + "/?rpc=" + rpc + "&api=" + api

default:
    @just --list --unsorted

# ── build ────────────────────────────────────────────────────────────────

# Build every workspace crate (host)
build:
    cargo build --workspace

# Build the on-chain program into target/deploy
build-program:
    cargo build-sbf --manifest-path programs/tradingtime/Cargo.toml --sbf-out-dir target/deploy

# Bundle the web app into target/web/public
web-bundle:
    dx bundle --web --release -p tradingtime-web --out-dir target/web

# ── test ─────────────────────────────────────────────────────────────────

# Unit + LiteSVM tests for the whole workspace (rebuilds the program first)
test: build-program
    cargo test --workspace

# Tests for one crate, e.g. `just test-crate tradingtime-indexer`
test-crate crate:
    cargo test -p {{crate}}

# Program tests against a fresh .so
test-program: build-program
    cargo test -p tradingtime-program

# Clippy (warnings are errors)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all crates
fmt:
    cargo fmt --all

# The web app compiles for wasm32
check-wasm:
    cargo check -p tradingtime-web --target wasm32-unknown-unknown

# Install Playwright + Chromium for the browser tests (once)
e2e-install:
    pnpm -C e2e install
    pnpm -C e2e exec playwright install chromium --only-shell

# Surfpool integration tests (needs `just up`)
test-integration:
    cargo test -p tradingtime-tests -- --ignored --test-threads=1

# Browser E2E against the running stack (needs `just up`)
test-e2e: web-bundle
    pnpm -C e2e exec playwright test

# Everything: unit, lint, wasm, then a fresh stack for integration + browser tests
test-all: test lint check-wasm web-bundle
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'scripts/dev-stack.sh down >/dev/null' EXIT
    scripts/dev-stack.sh up
    pnpm -C e2e exec playwright test
    cargo test -p tradingtime-tests -- --ignored --test-threads=1

# ── local stack ──────────────────────────────────────────────────────────

# Surfpool fork + program + indexer + keeper + 2 makers (quiet market)
up:
    scripts/dev-stack.sh up

# `up` plus 4 makers, bot traders and a moving Pyth price; then serve the app
sim bots="12" tick_ms="1500": web-bundle
    BOTS={{bots}} BOT_TICK_MS={{tick_ms}} scripts/dev-stack.sh sim
    @echo "app:       {{app_url}}"
    @echo "analytics: {{app_url}}#/analytics"
    @just serve

# Stop the stack (and anything `sim` started)
down:
    scripts/dev-stack.sh down

# Which stack services are running
status:
    scripts/dev-stack.sh status

# Follow a service log: surfpool, indexer, keeper, maker1..4, bots
logs service="bots":
    tail -f target/dev-stack/{{service}}.log

# Serve the bundled web app against the local stack (Ctrl-C to stop)
serve:
    @echo "open {{app_url}}"
    TT_WEB_PORT={{web_port}} node e2e/serve.mjs

# Rebuild and serve the web app
app: web-bundle serve

# Open the app (or `just open '#/analytics'`) in the browser
open page="":
    open "{{app_url}}{{page}}"

# Indexer snapshot: market + protocol analytics summary
stats:
    @curl -s {{api}}/v1/market | python3 -m json.tool
    @curl -s {{api}}/v1/analytics/summary | python3 -m json.tool

# ── simulator (offline economics) ────────────────────────────────────────

# Headless backtest, e.g. `just backtest --feed regime --seed 7 --steps 200000`
backtest *args:
    cargo run --release -p cli -- {{args}}

# Desktop simulator UI
simulator:
    cargo run --release -p app
