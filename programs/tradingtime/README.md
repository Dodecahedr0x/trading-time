# tradingtime-program

Pinocchio (`no_std`, zero-copy) TradingTime program. Contracts: `crates/interface`, math: `crates/math`.

## Build

```sh
cargo build-sbf --manifest-path programs/tradingtime/Cargo.toml --sbf-out-dir target/deploy
cp programs/tradingtime/program-keypair.json target/deploy/tradingtime_program-keypair.json
```

`cargo build-sbf` generates a random `target/deploy/tradingtime_program-keypair.json` when none exists;
the copy step replaces it with the pinned key so deploys land at `tradingtime_interface::PROGRAM_ID`
(`7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h`).

## Test

```sh
cargo test -p tradingtime-program                          # LiteSVM, loads target/deploy/tradingtime_program.so
cargo test -p tradingtime-program compute_units -- --nocapture   # CU table per instruction
```

## Compute units

Swap/ExecuteLimitOrder with 2 curves use ~160k CU; with 8 curves ~530k CU, so clients passing more
than two operators must prepend `SetComputeUnitLimit` (e.g. 1_400_000).
