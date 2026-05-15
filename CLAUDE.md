# Zekt -- Coding Guidelines

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
main.rs         CLI (clap) + graceful shutdown (ctrlc) -- routes to live, paper, or dry-run mode
config.rs       TOML config (agent, flash, strategy, risk sections)
flash_api.rs    REST client for Flash Trade API (prices, positions, tx builder)
executor.rs     Solana keypair loading + tx sign/submit via Arc<RpcClient> + spawn_blocking
signal.rs       Momentum detector -- price velocity, consecutive moves, exit signals
risk.rs         Risk manager -- SL/TP/trailing, circuit breaker, daily reset, fee tracking, trade journal
engine.rs       Live trading loop -- poll price -> detect -> preview -> build tx -> sign -> monitor
paper.rs        Paper trading loop -- same signal/exit logic, simulated PnL against live prices, no signing
```

## Key Dependencies
- `solana-sdk` / `solana-client` -- Keypair, Transaction, RPC
- `spl-associated-token-account` / `spl-token` -- USDC balance queries
- `reqwest` -- HTTP client for Flash Trade API
- `tokio` -- Async runtime
- `clap` -- CLI argument parsing
- `ctrlc` -- Graceful shutdown on SIGINT/SIGTERM
- `chrono` -- Timestamps
- `serde` / `serde_json` / `toml` -- Serialization
- `base64` / `bs58` / `bincode` -- Encoding

## Flash Trade API
Base URL: `https://flashapi.trade`
- Public, no auth required
- Transaction builder returns base64-encoded unsigned Solana transactions
- We sign locally with a fresh blockhash + user's keypair and submit via Solana RPC
- Price endpoint returns Pyth oracle prices (raw integer + exponent + human-readable float)
- Wallet balances NOT available via Flash Trade API -- use Solana RPC `getTokenAccountsByOwner`
- API errors are classified via `classify_api_error()` (insufficient balance, rate limited, etc.)
- MCP server available: `npx flash-trade-mcp` for AI agent integration

## Coding Conventions
- `tracing` for all logging (info/warn/error/debug), never `println`
- `anyhow::Result` for error handling in application code
- `parse_f64_safe(s, field_name)` for string-to-f64 parsing (returns Result with context, never silent 0.0)
- All dollar amounts are f64 in USD
- All prices are f64 in human-readable format
- Config is loaded once at startup from TOML, never mutated at runtime
- Position state is held in `engine.rs` (Option<Position>)
- Trade journal uses atomic writes (write to .tmp then rename) to `perps-trades.json`

## Runtime Safety
- `Arc<RpcClient>` wraps Solana RPC client for sharing across spawn_blocking closures
- All synchronous RPC calls go through `tokio::task::spawn_blocking` to avoid blocking the async runtime
- Fresh blockhash fetched before every transaction signing to avoid expiry (~60s window)
- USDC balance checked via SPL token account before opening positions
- SIGINT/SIGTERM handled via `ctrlc` -- engine finishes current tick then exits cleanly

## Risk Management
- Circuit breaker halts trading on: daily loss limit or max drawdown
- Daily PnL resets automatically at midnight UTC (tracked via `trade_date`)
- Peak balance initialized from real USDC balance, updated after each trade close
- Cooldown after any loss (configurable seconds)
- Native TP/SL via Flash Trade trigger orders (on-chain enforcement)
- Software-side SL/TP/trailing as faster soft exits
- Trailing stop for shorts correctly tracks lowest price (peak_price = best price for direction)
- Time stop closes positions that exceed max hold duration
- Position size validated against `max_position_notional_usd` config

## Config Format
TOML with 4 sections: `[agent]`, `[flash]`, `[strategy]`, `[risk]`
See `config/perps.toml` for the full schema with defaults.

## Testing
No unit tests yet. Use `--dry-run` to test against live API without signing.
Dry run fetches price, previews an open position, and shows pool data.
