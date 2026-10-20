#!/usr/bin/env bash
# TradingTime local stack on a Surfpool mainnet fork.
#   scripts/dev-stack.sh up    start surfpool, deploy, bootstrap, indexer, keeper, two makers
#   scripts/dev-stack.sh down  stop everything started by `up`
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="$ROOT/target/dev-stack"
KEYS="$DIR/keys"
PIDS="$DIR/pids"
RPC="http://127.0.0.1:8899"
WS="ws://127.0.0.1:8900"
API="http://127.0.0.1:8787"
BIN="$ROOT/target/dev-stack/bin"
export CARGO_TARGET_DIR="$ROOT/target/n1.8"

rpc_ok() { curl -s -m 2 -X POST "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getSlot"}' | grep -q '"result"'; }
wait_until() { local what="$1" n="$2"; shift 2; for _ in $(seq 1 "$n"); do "$@" && return 0; sleep 1; done; echo "timed out waiting for $what" >&2; return 1; }
start() { local name="$1"; shift; "$@" >"$DIR/$name.log" 2>&1 & echo "$! $name" >>"$PIDS"; }

down() {
  if [ -f "$PIDS" ]; then
    while read -r pid name; do kill "$pid" 2>/dev/null && echo "stopped $name" || true; done <"$PIDS"
    sleep 1
    while read -r pid _; do kill -9 "$pid" 2>/dev/null || true; done <"$PIDS"
    rm -f "$PIDS"
  fi
}

up() {
  down
  mkdir -p "$DIR" "$KEYS" "$BIN"
  rm -f "$DIR/indexer.sqlite"*
  for k in admin keeper trader1 trader2 maker1 maker2 e2e; do
    [ -f "$KEYS/$k.json" ] || solana-keygen new --no-bip39-passphrase --silent -o "$KEYS/$k.json" >/dev/null
  done

  echo "building binaries"
  cargo build-sbf --manifest-path "$ROOT/programs/tradingtime/Cargo.toml" --sbf-out-dir "$ROOT/target/deploy" >"$DIR/build-sbf.log" 2>&1
  cp "$ROOT/programs/tradingtime/program-keypair.json" "$ROOT/target/deploy/tradingtime_program-keypair.json"
  cargo build --release -p tradingtime-indexer -p tradingtime-keeper -p tradingtime-maker -p tradingtime-tests --bins >"$DIR/build.log" 2>&1
  for b in tradingtime-indexer tradingtime-keeper tradingtime-maker bootstrap; do cp "$CARGO_TARGET_DIR/release/$b" "$BIN/"; done

  echo "starting surfpool"
  start surfpool surfpool start --ci --network mainnet --no-deploy -y -p 8899 -w 8900
  wait_until surfpool 60 rpc_ok

  echo "deploying program"
  ADMIN="$(solana-keygen pubkey "$KEYS/admin.json")"
  curl -s -X POST "$RPC" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"surfnet_setAccount\",\"params\":[\"$ADMIN\",{\"lamports\":100000000000000}]}" >/dev/null
  solana program deploy "$ROOT/target/deploy/tradingtime_program.so" \
    --program-id "$ROOT/programs/tradingtime/program-keypair.json" \
    --keypair "$KEYS/admin.json" -u "$RPC" >"$DIR/deploy.log" 2>&1

  echo "bootstrapping market"
  "$BIN/bootstrap" --keys "$KEYS" --rpc "$RPC" --ws "$WS" --api "$API" >"$DIR/bootstrap.log" 2>&1

  start indexer "$BIN/tradingtime-indexer" --rpc "$RPC" --ws "$WS" --listen 127.0.0.1:8787 --db "$DIR/indexer.sqlite"
  start keeper "$BIN/tradingtime-keeper" --rpc "$RPC" --ws "$WS" --api "$API" --keypair "$KEYS/keeper.json"
  start maker1 "$BIN/tradingtime-maker" --rpc "$RPC" --keypair "$KEYS/maker1.json" --deposit-usdc 50000 \
    --half-spread-bps 2 --impact-bps-per-10k 10 --inventory-limit-usdc 20000 --feed "replay:$ROOT/tests/fixtures/sol-ticks.csv"
  start maker2 "$BIN/tradingtime-maker" --rpc "$RPC" --keypair "$KEYS/maker2.json" --deposit-usdc 50000 \
    --half-spread-bps 3 --impact-bps-per-10k 15 --inventory-limit-usdc 20000 --feed "replay:$ROOT/tests/fixtures/sol-ticks.csv"
  wait_until indexer 60 curl -sf "$API/v1/health" -o /dev/null
  echo "stack up: $DIR/env.json"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
