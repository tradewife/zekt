# Zekt — Session Context

## What This Is
Zekt is a **multi-strategy trading system** for Flash Trade (Solana perps DEX). Originally a single-strategy momentum scalper, evolved via the Alpha Hunter mission into a pluggable strategy platform with 4 strategies, backtesting (Hyperliquid), and multi-strategy paper trading.

**4 strategies:** momentum-scalper, lp-consumption, mean-reversion, trend-follower. All implement the `Strategy` trait (`strategy.rs`). Strategy blueprints were extracted from profitable wallets scraped from perp DEX leaderboards.

## Recommended Pipeline
```
Backtest (Hyperliquid historical data) → Paper Trade (live Flash Trade prices) → Live (real money)
```
Always validate strategy parameters with backtesting first.

## Architecture
```
Flash Trade API (prices, positions, tx builder)
  → Strategy trait (push_price → detect_entry → detect_exit)
    → Risk Manager (SL/TP/trailing, circuit breaker, daily reset, cooldown)
      → Executor (Arc<RpcClient>, spawn_blocking, fresh blockhash, USDC balance)
        → Trade Journal (atomic JSON log of all closed trades)

Backtest mode: Hyperliquid candleSnapshot API → Strategy trait → simulated fills + fee accounting
Paper mode: same Strategy trait, live Flash Trade prices, simulated PnL with real fee estimates
Live mode: same Strategy trait, real transactions
```

## Commands
```bash
# Build and test
cargo build --release
cargo test    # 140 unit tests

# Backtest against Hyperliquid historical data (no wallet needed)
./target/release/zekt --backtest \
  --strategies momentum-scalper,mean-reversion \
  --markets BTC,SOL,ETH \
  --backtest-start 2026-05-01 --backtest-end 2026-05-15 \
  --backtest-interval 5m --paper-balance 1000

# Multi-strategy multi-market paper trading
./target/release/zekt --paper \
  --strategies momentum-scalper,lp-consumption,mean-reversion \
  --markets SOL,BTC,ETH --paper-balance 1000

# Single-strategy paper trading
./target/release/zekt --paper --market SOL --strategy mean-reversion

# Dry run (single preview, then exit)
./target/release/zekt --dry-run --market BTC

# Live trading
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL

# CLI tools
cargo run --bin scrape-leaderboards -- --source all --output data/wallets.json
cargo run --bin analyze-wallet -- --wallets data/wallets.json --output data/reports/
```

## CLI Flags Reference

| Flag | Description |
|------|-------------|
| `--backtest` | Backtest mode: replay Hyperliquid historical candles |
| `--backtest-start` | Start time (ISO 8601 or date-only, e.g., "2026-05-01") |
| `--backtest-end` | End time (default: now) |
| `--backtest-interval` | Candle interval: 1m, 5m, 15m, 1h, 4h, 1d (default: 5m) |
| `--backtest-fee-rate` | Fee rate as decimal (default: 0.001 = 0.1%) |
| `--paper` | Paper trading mode (live prices, simulated PnL) |
| `--paper-balance` | Starting balance in USD (default: 1000) |
| `--paper-output` | Output directory for paper results (default: data/paper-results) |
| `--strategies` | Comma-separated strategy list for multi-strategy mode |
| `--markets` | Comma-separated market list for multi-market mode |
| `--strategy` | Single strategy override |
| `--market` | Single market override |
| `--dry-run` | Single API preview, then exit |
| `--config` | Config file path (default: config/perps.toml) |
| `--keypair` | Solana keypair path |

## Config (`config/perps.toml`)

### Agent
- `agent.poll_interval_secs` = 5
- `agent.log_level` = "info"

### Flash Trade Connection
- `flash.api_url` = `https://flashapi.trade`
- `flash.rpc_url` = `https://api.mainnet-beta.solana.com`
- `flash.keypair_path` = Solana keypair file
- `flash.market` = Trading pair (SOL, BTC, ETH, etc.)
- `flash.input_token` = USDC
- `flash.pool` = "Crypto.1"
- `flash.leverage` = 10.0
- `flash.slippage_pct` = "0.5"

### Strategy (shared defaults)
- `strategy.active` = "momentum-scalper"
- `strategy.clip_size_usd` = 100.0
- `strategy.momentum_threshold_pct` = 0.15
- `strategy.lookback_count` = 60
- `strategy.direction_bias` = "neutral"
- `strategy.take_profit_pct` = 2.5
- `strategy.stop_loss_pct` = 1.0
- `strategy.trailing_stop_pct` = 0.8
- `strategy.trailing_activation_pct` = 1.5
- `strategy.max_hold_secs` = 1800
- `strategy.cooldown_after_loss_secs` = 300

### Strategy Sub-Tables (per-strategy overrides)
- `[strategy.lp-consumption]` — consumption_velocity_threshold=0.5, lp_concentration_min=0.7, take_profit=2.0, stop_loss=1.0
- `[strategy.mean-reversion]` — mean_lookback=120, deviation_threshold_pct=1.5, take_profit=1.0, stop_loss=1.5
- `[strategy.trend-follower]` — breakout_threshold_pct=0.25, confirmation_ticks=4, take_profit=5.0, stop_loss=2.0, max_hold_secs=7200

### Risk Limits
- `risk.max_position_notional_usd` = 1000
- `risk.max_daily_loss_usd` = 200
- `risk.max_drawdown_pct` = 15%

## Supported Markets (Flash Trade Crypto.1)
SOL, BTC, ETH, ZEC, BNB, XAU, XAG, EUR, JPY, JUP, BONK, WIF, PENGU, FARTCOIN, and more.

Backtesting also supports any Hyperliquid perps market (BTC, SOL, ETH, etc.).

## Hyperliquid API (Backtesting)
- Endpoint: `POST https://api.hyperliquid.xyz/info`
- Request: `{"type": "candleSnapshot", "req": {"coin": "BTC", "interval": "5m", "startTime": ms, "endTime": ms}}`
- Response: OHLCV candles with fields t, T, s, i, o, c, h, l, v, n
- Max 5000 candles per request (auto-paginated)
- Intervals: 1m, 5m, 15m, 1h, 4h, 1d, 1w
- No auth. Rate limit: 1200 weight/min/IP.

## Flash Trade API Key Endpoints
- `GET /prices/{symbol}` → Oracle price (Pyth)
- `GET /positions/owner/{owner}` → Enriched positions with PnL
- `POST /transaction-builder/open-position` → Unsigned Solana tx
- `POST /transaction-builder/close-position` → Unsigned close tx
- `POST /transaction-builder/place-trigger-order` → TP/SL trigger orders
- `WS /owner/{owner}/ws` → Real-time WebSocket streaming
- No auth required. Transactions signed locally.

## Critical Flash Trade Rules
- **One position per market per side per wallet**
- **Blockhash expiry ~60s** -- fresh blockhash before every sign
- **Max 5 trigger orders** (TP/SL) per market position
- **Min collateral >$10** after fees for trigger orders
- **SOL positions use JitoSOL** as underlying collateral on-chain
- **All amounts are UI format** (human-readable) in API requests
- **Wallet balances NOT available via Flash Trade API** -- use Solana RPC

## Shutdown
```bash
Ctrl+C          # Graceful: finishes current tick
kill -INT <pid> # Graceful
kill -9 <pid>   # Emergency
```

## Key Files
- `src/strategy.rs` — Strategy trait + 4 implementations + factory (3369 lines, 63 tests)
- `src/backtest.rs` — Hyperliquid candle fetcher + BacktestEngine (1170 lines, 15 tests)
- `src/paper.rs` — Paper trading: single engine + MultiPaperEngine (1684 lines, 14 tests)
- `src/engine.rs` — Live trading engine
- `src/signal.rs` — MomentumDetector, MomentumSnapshot, Signal/ExitReason types
- `src/risk.rs` — Risk manager (daily reset, fee tracking, position size validation)
- `src/flash_api.rs` — Flash Trade REST client
- `src/executor.rs` — Solana tx signing
- `src/config.rs` — TOML config parser
- `src/main.rs` — CLI entrypoint, routes to backtest/paper/dry-run/live
- `src/bin/scrape-leaderboards.rs` — Wallet scraper from perp DEX leaderboards (8 tests)
- `src/bin/analyze-wallet.rs` — Wallet strategy classifier + blueprint generator (24 tests)
- `config/perps.toml` — All tunable parameters

## Alpha Hunter Mission (Completed)
The mission transformed Zekt from a single-strategy momentum scalper into a multi-strategy system. See `docs/MISSION-ALPHA-HUNTER.md` for full details.

**M1: Leaderboard Scraping** — Scraped 112 wallets from fstats.io (Flash Trade), Jupiter, Hyperliquid. Built `scrape-leaderboards` binary.

**M2: Wallet Analysis** — Classified wallets into 4 strategy types. Generated 2 strategy blueprints (swing-trader, lp-consumer). Built `analyze-wallet` binary.

**M3: Strategy Implementation** — Created `Strategy` trait, refactored momentum scalper, implemented lp-consumption, mean-reversion, trend-follower strategies. 63 unit tests.

**M4: Paper Trading** — Built `MultiPaperEngine` with strategy x market position matrix, per-cell fee accounting, summary JSON output. 14 unit tests.

## Backtesting (Post-Mission)
Added `src/backtest.rs` with `HlCandleFetcher` (Hyperliquid `candleSnapshot` API) and `BacktestEngine` (replays candles through `Strategy` trait). Outputs `data/backtest-results/summary.json` and `data/backtest-trades.json`. 15 unit tests. CLI flags: `--backtest`, `--backtest-start`, `--backtest-end`, `--backtest-interval`, `--backtest-fee-rate`.

## Testing
140 unit tests total. Run with `cargo test`.
- strategy.rs: 63 tests
- paper.rs: 14 tests
- backtest.rs: 15 tests
- analyze-wallet.rs: 24 tests
- scrape-leaderboards.rs: 8 tests
- Other modules: 16 tests

## TODO / Next Steps
- [ ] **Market scanner** -- Rank Flash Trade markets by liquidity concentration
- [ ] WebSocket streaming for real-time price updates (instead of polling)
- [ ] Adaptive momentum threshold (adjust based on volatility regime)
- [ ] Scale-in logic (add to winning positions when momentum accelerates)
- [ ] Use `reverse-position` endpoint for momentum reversals
- [ ] RPC retry with backoff on 429 rate limits
- [x] ~~Backtesting engine against historical prices~~ (done: Hyperliquid candleSnapshot)
- [x] ~~LP detection~~ (done: lp-consumption strategy)
- [x] ~~LP consumption rate signal~~ (done: lp-consumption strategy)
- [x] ~~Unit tests~~ (done: 140 tests)
- [x] ~~Fee-awareness~~ (done: per-trade fee tracking in paper + backtest engines)
