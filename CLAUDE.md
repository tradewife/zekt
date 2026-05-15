# Zekt — Coding Guidelines

## What This Is
A **liquidity-aware momentum scalper** for Flash Trade (Solana perps DEX). The edge is market-structure arbitrage: find illiquid markets where a single dominant LP provides most of the depth, detect when that LP is being consumed in one direction, and ride the momentum. Rust binary, single crate, standalone workspace. Targets Solana mainnet via Flash Trade's public REST API.

## Build & Run
```bash
cargo build --release                          # Build
./target/release/zekt --dry-run                # Preview mode
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL  # Live
./scripts/run-zekt.sh --dry-run --market BTC   # Via script
```

## Architecture
```
main.rs         CLI (clap) — parses args, loads config, starts engine or dry-run
config.rs       TOML config (Flash, strategy, risk sections)
flash_api.rs    REST client for Flash Trade API (prices, positions, tx builder)
executor.rs     Solana keypair loading + tx sign/submit/confirm via RPC
signal.rs       Momentum detector — price velocity, consecutive moves, exit signals
risk.rs         Risk manager — SL/TP/trailing, circuit breaker, trade journal
engine.rs       Main loop — poll price → detect → preview → build tx → sign → monitor
```

## Key Dependencies
- `solana-sdk` / `solana-client` — Keypair, Transaction, RPC
- `reqwest` — HTTP client for Flash Trade API
- `tokio` — Async runtime
- `clap` — CLI argument parsing
- `chrono` — Timestamps
- `serde` / `serde_json` / `toml` — Serialization
- `base64` / `bs58` / `bincode` — Encoding

## Flash Trade API
Base URL: `https://flashapi.trade`
- Public, no auth required
- Transaction builder returns base64-encoded unsigned Solana transactions
- We sign locally with the user's keypair and submit via Solana RPC
- Price endpoint returns Pyth oracle prices (raw integer + exponent + human-readable float)

## Coding Conventions
- `tracing` for all logging (info/warn/error/debug), never `println`
- `anyhow::Result` for error handling in application code
- All dollar amounts are f64 in USD
- All prices are f64 in human-readable format
- Config is loaded once at startup from TOML, never mutated at runtime
- Position state is held in `engine.rs` (Option<Position>)
- Trade journal is appended to `perps-trades.json` in cwd

## Risk Management
- Circuit breaker halts trading on: daily loss limit or max drawdown
- Cooldown after any loss (configurable seconds)
- Native TP/SL via Flash Trade trigger orders (on-chain enforcement)
- Software-side SL/TP/trailing as faster soft exits
- Time stop closes positions that exceed max hold duration

## Config Format
TOML with 4 sections: `[agent]`, `[flash]`, `[strategy]`, `[risk]`
See `config/perps.toml` for the full schema with defaults.

## Testing
No unit tests yet. Use `--dry-run` to test against live API without signing.
Dry run fetches price, previews an open position, and shows pool data.
