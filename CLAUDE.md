# Zekt -- Coding Guidelines

## What This Is
A **multi-strategy trading system** for Flash Trade (Solana perps DEX). Originally a liquidity-aware momentum scalper, evolved via the Alpha Hunter mission into a pluggable multi-strategy platform with backtesting (Hyperliquid data) and paper trading. Rust binary, single crate, standalone workspace. Targets Solana mainnet via Flash Trade's public REST API.

**4 strategies:** momentum-scalper, lp-consumption, mean-reversion, trend-follower. All implement the `Strategy` trait in `strategy.rs`. Strategy blueprints were reverse-engineered from profitable wallets scraped from perp DEX leaderboards (see `docs/MISSION-ALPHA-HUNTER.md`).

## Recommended Pipeline
```
Backtest (Hyperliquid historical candles) → Paper Trade (live prices) → Live (real money)
```
Always backtest first to validate strategy parameters, then paper trade to confirm against live prices with real fee estimates, then go live.

## Build & Run
```bash
cargo build --release                          # Build
cargo test                                     # Run 140 unit tests

# Backtest against Hyperliquid historical data
./target/release/zekt --backtest --strategies momentum-scalper --markets BTC,SOL \
  --backtest-start 2026-05-15 --backtest-interval 5m --paper-balance 1000

# Multi-strategy multi-market paper trading
./target/release/zekt --paper --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Dry run (single API preview, then exit)
./target/release/zekt --dry-run

# Live (requires funded Solana wallet with USDC)
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL
```

## Architecture
```
main.rs         CLI (clap) + graceful shutdown (ctrlc) -- routes to backtest, paper, dry-run, or live mode
config.rs       TOML config (agent, flash, strategy, risk sections + strategy sub-tables)
strategy.rs     Strategy trait + 4 implementations + factory function (3369 lines, 63 tests)
signal.rs       MomentumDetector, MomentumSnapshot, PoolSnapshot, Signal/ExitReason types
backtest.rs     Hyperliquid candle fetcher + BacktestEngine (replay through Strategy trait, 15 tests)
flash_api.rs    REST client for Flash Trade API (prices, positions, tx builder)
executor.rs     Solana keypair loading + tx sign/submit via Arc<RpcClient> + spawn_blocking
risk.rs         Risk manager -- SL/TP/trailing, circuit breaker, daily reset, fee tracking, trade journal
engine.rs       Live trading loop -- poll price -> detect -> preview -> build tx -> sign -> monitor
paper.rs        Paper trading -- single + MultiPaperEngine (strategy x market matrix, 14 tests)
src/bin/
  scrape-leaderboards.rs   CLI: scrape profitable wallets from perp DEX leaderboards (8 tests)
  analyze-wallet.rs        CLI: classify wallet strategies and generate blueprints (24 tests)
```

## Strategy Trait
All strategies implement `Strategy: Send + Sync` (object-safe):
```rust
fn name(&self) -> &str;
fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal;
fn detect_exit(&self, snapshot: &MomentumSnapshot, context: &PositionContext) -> Option<Signal>;
fn parameters(&self) -> &StrategyParams;
fn push_price(&mut self, price: f64, timestamp_ms: i64);
fn snapshot(&self) -> MomentumSnapshot;
```
Strategies are created via `create_strategy_from_config(name, sub_table, fallback_params)`. Available names: `["momentum-scalper", "lp-consumption", "mean-reversion", "trend-follower"]` (see `available_strategies()`).

## Key Dependencies
- `solana-sdk` / `solana-client` -- Keypair, Transaction, RPC
- `spl-associated-token-account` / `spl-token` -- USDC balance queries
- `reqwest` -- HTTP client for Flash Trade API + Hyperliquid API
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

## Hyperliquid API (Backtesting)
Base URL: `https://api.hyperliquid.xyz/info`
- `POST` with `{"type": "candleSnapshot", "req": {"coin": "BTC", "interval": "5m", "startTime": ..., "endTime": ...}}`
- Returns OHLCV candles: `t`, `T`, `s`, `i`, `o`, `c`, `h`, `l`, `v`, `n`
- Max 5000 candles per request, auto-paginated in `HlCandleFetcher`
- Intervals: 1m, 5m, 15m, 1h, 4h, 1d, 1w
- No auth required. Rate limit: 1200 weight/min/IP.

## Coding Conventions
- `tracing` for all logging (info/warn/error/debug), never `println`
- `anyhow::Result` for error handling in application code
- `parse_f64_safe(s, field_name)` for string-to-f64 parsing (returns Result with context, never silent 0.0)
- All dollar amounts are f64 in USD
- All prices are f64 in human-readable format
- Config is loaded once at startup from TOML, never mutated at runtime
- Position state is held in `engine.rs` (Option<Position>) or `paper.rs` (HashMap<CellKey, CellPosition>)
- Trade journal uses atomic writes (write to .tmp then rename) to `perps-trades.json`
- Backtest results use atomic writes to `data/backtest-results/summary.json` and `data/backtest-trades.json`

## Runtime Safety
- `Arc<RpcClient>` wraps Solana RPC client for sharing across spawn_blocking closures
- All synchronous RPC calls go through `tokio::task::spawn_blocking` to avoid blocking the async runtime
- Fresh blockhash fetched before every transaction signing to avoid expiry (~60s window)
- USDC balance checked via SPL token account before opening positions
- SIGINT/SIGTERM handled via `ctrlc` -- engine finishes current tick then exits cleanly
- Backtest mode requires no Solana RPC, no keypair -- only Hyperliquid public API

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
TOML with 4 main sections: `[agent]`, `[flash]`, `[strategy]`, `[risk]`
Strategy sub-tables: `[strategy.lp-consumption]`, `[strategy.mean-reversion]`, `[strategy.trend-follower]`
See `config/perps.toml` for the full schema with defaults.

## Testing
140 unit tests across all modules. Run with `cargo test`.
- `strategy.rs`: 63 tests (entry/exit for each strategy, parameter validation, factory)
- `paper.rs`: 14 tests (MultiPaperEngine, position matrix, fee accounting)
- `backtest.rs`: 15 tests (candle parsing, position PnL, fee accrual, synthetic replay)
- `src/bin/analyze-wallet.rs`: 24 tests (wallet classification, blueprint generation)
- `src/bin/scrape-leaderboards.rs`: 8 tests (API parsing, deduplication)
- Use `--dry-run` for integration testing against live API without signing.
