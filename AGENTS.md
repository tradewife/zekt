# Zekt — Coding Guidelines for AI Agents

## What This Is

A **strategy research and validation platform** for Solana perpetual futures (Flash Trade). The system discovers profitable trading patterns from Hyperliquid wallets, reverse-engineers them from fill-level data (the Bulk.Trade methodology), and validates whether those patterns hold under rigorous testing before any execution. Rust binary for trading infrastructure + Python analysis pipeline for wallet intelligence. Single Rust crate, standalone workspace. Targets Solana mainnet via Flash Trade's public REST API.

**Pipeline:** Research on Hyperliquid (rich data via QuickNode HyperCore API) → Validate rigorously → Execute on Flash Trade (Solana perps). Semi-automated: discovery, analysis, and backtesting run without human intervention; paper trading and live execution require human approval.

**16 strategies:** momentum-scalper, lp-consumption, mean-reversion, trend-follower, funding-capture, blueprint-scalper, blueprint-mean-revert, blueprint-cluster-002 through 009, blueprint-hft-market-maker, liquidation-cascade-hunter. All implement the `Strategy` trait in `strategy.rs`. Strategy blueprints are generated from fill-level analysis of profitable HL wallets (see `analysis/` directory).

**Validation status (M10):** After exhaustive 90-day walk-forward validation — 294K parameter combinations, 315 leverage/sizing grid cells, 3 portfolio allocation strategies — **no current blueprint strategy passes the six-criterion promotion gate**. All promising short-window signals were overfit artifacts. Fee dominance (fee-to-gross >100%) overwhelms any remaining edge. The correct outcome is "reject all candidates." No strategy has been promoted to paper or live trading.

## Recommended Pipeline
```
1. Discover  → Scrape HL leaderboards via QuickNode HyperCore API for profitable wallets
2. Analyze   → Python pipeline: position clustering → metrics → classification → blueprints
3. Implement → Rust Strategy trait with parameters from blueprint JSON
4. Backtest  → Validate on HL historical candles (must pass Sharpe ≥ 1.0 threshold)
5. Validate  → Walk-forward parameter search → leverage/sizing frontier (90-day OOS)
6. Paper Trade → Confirm against live Flash Trade prices with real fee estimates (24h+)
7. Live      → Execute with real capital (requires human approval)
```

## Build & Run
```bash
cargo build --release                          # Build Rust binary
cargo test                                     # Run 828 Rust unit tests

# Python analysis tests
python -m pytest analysis/tests/ -v            # Run 132 Python analysis module tests

# Full pipeline orchestration (alpha-scanner + copy-trader + whale-watcher + paper)
cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48
cargo run --bin pipeline -- --once              # Single scan + report, then exit
cargo run --bin pipeline -- --skip-scanner      # Use existing watchlist
cargo run --bin pipeline -- --help              # All options

# Backtest against Hyperliquid historical data
./target/release/zekt --backtest --strategies momentum-scalper --markets BTC,SOL \
  --backtest-start 2026-05-15 --backtest-interval 5m --paper-balance 1000

# Backtest with Imperial route oracle cost mode
./target/release/zekt --backtest --strategies momentum-scalper --markets SOL \
  --backtest-start 2026-05-15 --backtest-interval 5m \
  --cost-mode imperial-route-oracle

# Multi-strategy multi-market paper trading
./target/release/zekt --paper --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Dry run (single API preview, then exit)
./target/release/zekt --dry-run

# Live (requires funded Solana wallet with USDC)
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL

# Wallet discovery (requires QuickNode endpoint)
QUICKNODE_HL_URL=https://your-endpoint.quiknode.pro/... \
  cargo run --bin scrape-leaderboards -- --quicknode-url $QUICKNODE_HL_URL --output data/wallets-hl.json

# Wallet analysis
cargo run --bin analyze-wallet -- --wallets data/wallets-hl.json --output data/reports/
```

## Architecture

### Rust (Trading Infrastructure)
```
main.rs         CLI (clap) + graceful shutdown (ctrlc) -- routes to backtest, paper, dry-run, or live mode
config.rs       TOML config (agent, flash, strategy, risk, backtest sections + strategy sub-tables)
strategy.rs     Strategy trait + 16 implementations + factory function (194 tests)
signal.rs       MomentumDetector, MomentumSnapshot, PoolSnapshot, Signal/ExitReason types
backtest.rs     Hyperliquid candle fetcher + BacktestEngine (walk-forward, slippage, regime filter, 42 tests)
flash_api.rs    REST client for Flash Trade API (prices, positions, tx builder)
hl_info.rs      REST client for Hyperliquid Info API (positions, funding rates, fills, market contexts)
regime.rs       Market regime detector (LowVol/Trending/HighVol/Choppy) + strategy-specific compatibility rules (20 tests)
funding_capture.rs  Funding rate capture strategy (delta-neutral short perp for yield) (40 tests)
pnl_tracker.rs  Combined PnL tracking across all strategies (copy-trader + whale-watcher + paper)
executor.rs     Solana keypair loading + tx sign/submit via Arc<RpcClient> + spawn_blocking
risk.rs         Risk manager -- SL/TP/trailing, circuit breaker, daily/weekly reset, correlated exposure, ATR sizing, API degradation, divergence tracking (28 tests)
engine.rs       Live trading loop -- poll price -> detect -> preview -> build tx -> sign -> monitor
paper.rs        Paper trading -- single + MultiPaperEngine (strategy x market matrix, 24 tests)
imperial.rs     ImperialClient -- REST client for Imperial API (Solana perps venue comparison) (53 tests)
route_cost.rs   RouteCostOracle -- cross-venue fee/spread comparison for optimal execution routing (27 tests)
liquidation.rs  LiquidationZoneCapture -- multi-source liquidation zone data model with confidence scoring + snapshot persistence (101 tests)
replay.rs       ReplayPipeline -- replay validation for strategies with promotion gate (45 tests)
src/bin/
  pipeline.rs             Orchestrator: launches alpha-scanner + copy-trader + whale-watcher + paper (14 tests)
  scrape-leaderboards.rs  CLI: discover profitable wallets via QuickNode HyperCore API + leaderboards (22 tests)
  analyze-wallet.rs       CLI: classify wallet strategies and generate blueprints (24 tests)
  alpha-scanner.rs        Daemon: wallet discovery via Dextrabot + Hypurrscan enrichment + composite scoring (64 tests)
  copy-trader.rs          Daemon: real-time position mirroring with paper trading + risk management (106 tests)
  whale-watcher.rs        Daemon: WebSocket fill monitoring, notional alerts, accuracy tracking (41 tests)
  scan-markets.rs         CLI: rank Flash Trade markets by LP concentration, leverage, volume (18 tests)
  scrape-dextrabot.rs     CLI: Dextrabot discover-wallets API integration (8 tests)
```

### Python (Analysis Pipeline)
```
analysis/
  position_clustering.py     Cluster individual fills into open→close position cycles
  wallet_metrics.py          Per-wallet metrics: clip consistency, hold time, win rate, PnL distribution,
                             fill interval stats, scale-in count, active hours, fee-adjusted PnL
  strategy_classifier.py     Classify wallets into strategy types with confidence and evidence
  entry_reconstruction.py    Fetch HL candles before entries; find common trigger patterns
  cluster_analysis.py        Find groups of wallets running identical strategies
  blueprint_generator.py     Generate strategy blueprints with data-derived parameters (cluster medians)
  tests/
    test_position_clustering.py
    test_wallet_metrics.py
    test_strategy_classifier.py
    test_entry_reconstruction.py
    test_cluster_analysis.py
    test_blueprint_generator.py
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
Strategies are created via `create_strategy_from_config(name, sub_table, fallback_params)`. Available names: `["momentum-scalper", "lp-consumption", "mean-reversion", "trend-follower", "funding-capture", "blueprint-scalper", "blueprint-mean-revert", "blueprint-cluster-002", "blueprint-cluster-003", "blueprint-cluster-004", "blueprint-cluster-005", "blueprint-cluster-006", "blueprint-cluster-007", "blueprint-cluster-008", "blueprint-cluster-009", "blueprint-hft-market-maker", "liquidation-cascade-hunter"]` (see `available_strategies()`).

## Key Dependencies

### Rust
- `solana-sdk` / `solana-client` -- Keypair, Transaction, RPC
- `spl-associated-token-account` / `spl-token` -- USDC balance queries
- `reqwest` -- HTTP client for Flash Trade API + Hyperliquid API + QuickNode + Imperial API
- `tokio` -- Async runtime
- `clap` -- CLI argument parsing
- `ctrlc` -- Graceful shutdown on SIGINT/SIGTERM
- `chrono` -- Timestamps
- `serde` / `serde_json` / `toml` -- Serialization
- `base64` / `bs58` / `bincode` -- Encoding

### Python
- `requests` -- HTTP client for Hyperliquid API
- `numpy` -- Numerical computation
- `pandas` -- Data manipulation (fill records, position clusters)
- `scikit-learn` -- Clustering algorithms for wallet group analysis
- `pytest` -- Testing framework for analysis modules

## QuickNode Integration (HyperCore API)

QuickNode provides the primary data source for Hyperliquid wallet discovery and fill-level analysis.

**Configuration:**
```bash
# Environment variable (recommended, gitignored)
export QUICKNODE_HL_URL="https://your-endpoint.quiknode.pro/your-token/"

# Or CLI flag
--quicknode-url "https://your-endpoint.quiknode.pro/..."
```

**QuickNode HyperCore Methods Used:**
- `hl_batchClearinghouseStates` — Batch wallet position scanning (efficient for 100+ wallets)
- `hl_batchPortfolioStates` — Portfolio snapshots for multiple wallets
- `userFills` / `userFillsByTime` — Per-wallet fill records (coin, side, px, sz, fee, closedPnl, time, dir, hash, startPosition)
- `candleSnapshot` — Historical OHLCV candles for backtesting and entry reconstruction

**Fallback:** Direct Hyperliquid Info API (`api.hyperliquid.xyz/info`) for userFills, userFillsByTime, clearinghouseState, candleSnapshot when QuickNode is not configured.

## Flash Trade API
Base URL: `https://flashapi.trade`
- Public, no auth required
- Transaction builder returns base64-encoded unsigned Solana transactions
- We sign locally with a fresh blockhash + user's keypair and submit via Solana RPC
- Price endpoint returns Pyth oracle prices (raw integer + exponent + human-readable float)
- Wallet balances NOT available via Flash Trade API -- use Solana RPC `getTokenAccountsByOwner`
- API errors are classified via `classify_api_error()` (insufficient balance, rate limited, etc.)
- MCP server available: `npx flash-trade-mcp` for AI agent integration

## Imperial API (Solana Perps Venue Comparison)
Base URL: `https://api.imperial.space`
- Public, no auth required, read-only
- 10 GET endpoints for Solana perps venue comparison (fees, spreads, liquidity, execution quality)
- Used by `RouteCostOracle` to compare execution costs across Solana perps venues
- Rate limit: standard REST (no special handling required)
- Client implementation: `src/imperial.rs` (`ImperialClient`)

## Hyperliquid API (Backtesting + Fill Analysis)
Base URL: `https://api.hyperliquid.xyz/info`
- `POST` with `{"type": "candleSnapshot", "req": {"coin": "BTC", "interval": "5m", "startTime": ..., "endTime": ...}}`
- Returns OHLCV candles: `t`, `T`, `s`, `i`, `o`, `c`, `h`, `l`, `v`, `n`
- Max 5000 candles per request, auto-paginated in `HlCandleFetcher`
- Intervals: 1m, 5m, 15m, 1h, 4h, 1d, 1w
- No auth required. Rate limit: 1200 weight/min/IP.
- Fill endpoints: `userFills` (2000 fills), `userFillsByTime` (10K recent, time-filtered)
- Wallet state: `clearinghouseState` (positions, leverage, uPnL)

## Coding Conventions

### Rust
- `tracing` for all logging (info/warn/error/debug), never `println`
- `anyhow::Result` for error handling in application code
- `parse_f64_safe(s, field_name)` for string-to-f64 parsing (returns Result with context, never silent 0.0)
- All dollar amounts are f64 in USD
- All prices are f64 in human-readable format
- Config is loaded once at startup from TOML, never mutated at runtime
- Position state is held in `engine.rs` (Option<Position>) or `paper.rs` (HashMap<CellKey, CellPosition>)
- Trade journal uses atomic writes (write to .tmp then rename) to `perps-trades.json`
- Backtest results use atomic writes to `data/backtest-results/summary.json` and `data/backtest-trades.json`

### Python
- `logging` module for all output (never `print()`)
- Functions accept dicts/DataFrames, return structured results
- Handle network errors gracefully (retry with backoff)
- Each module has a corresponding `tests/test_<module>.py`
- Tests use synthetic fill data (don't require API calls)
- Tests run with `pytest`

## Runtime Safety
- `Arc<RpcClient>` wraps Solana RPC client for sharing across spawn_blocking closures
- All synchronous RPC calls go through `tokio::task::spawn_blocking` to avoid blocking the async runtime
- Fresh blockhash fetched before every transaction signing to avoid expiry (~60s window)
- USDC balance checked via SPL token account before opening positions
- SIGINT/SIGTERM handled via `ctrlc` -- engine finishes current tick then exits cleanly
- Backtest mode requires no Solana RPC, no keypair -- only Hyperliquid public API

## Risk Management
- Circuit breaker halts trading on: daily loss limit, weekly loss limit, max drawdown, consecutive losses, API degradation
- Daily PnL resets automatically at midnight UTC (tracked via `trade_date`)
- Weekly PnL resets automatically at Monday 00:00 UTC
- Peak balance initialized from real USDC balance, updated after each trade close
- Cooldown after any loss (configurable seconds)
- Native TP/SL via Flash Trade trigger orders (on-chain enforcement)
- Software-side SL/TP/trailing as faster soft exits
- Trailing stop for shorts correctly tracks lowest price (peak_price = best price for direction)
- Time stop closes positions that exceed max hold duration
- Position size validated against `max_position_notional_usd` config
- Cross-cell position limit enforced via `max_total_notional_usd` config (sums all CellPosition.size_usd)
- Correlated exposure limit caps exposure to correlated market groups (configurable in `correlated_groups`)
- ATR-based volatility position sizing reduces clip size in high-vol regimes
- API degradation circuit breaker halts after N consecutive API failures
- Consecutive loss circuit breaker halts after N consecutive losses, resets on win
- Paper/live divergence detection framework tracks fill-level comparison

## Config Format
TOML with 4 main sections: `[agent]`, `[flash]`, `[strategy]`, `[risk]`
Additional sections: `[alpha-scanner]`, `[copy-trader]`, `[whale-watcher]`, `[hypurrscan]`, `[pipeline]`, `[backtest]`, `[imperial]`, `[route-oracle]`, `[liquidation]`
Strategy sub-tables: `[strategy.lp-consumption]`, `[strategy.mean-reversion]`, `[strategy.trend-follower]`, `[strategy.funding-capture]`
Backtest config includes: `walk_forward_enabled`, `slippage_bps`, `regime_filter`, `cost_mode`
QuickNode configuration: `QUICKNODE_HL_URL` env var or `--quicknode-url` CLI flag (not in TOML, gitignored)
See `config/perps.toml` for the full schema with defaults.

## Testing

### Rust Tests
828 unit tests across all modules. Run with `cargo test`.
- `strategy.rs`: 194 tests (entry/exit for each strategy, parameter validation, factory)
- `liquidation.rs`: 101 tests (zone capture, multi-source fusion, confidence scoring, snapshot persistence, retention)
- `imperial.rs`: 53 tests (ImperialClient, venue comparison, fee parsing, endpoint coverage)
- `replay.rs`: 45 tests (replay validation, promotion gate, strategy replay, threshold checks)
- `backtest.rs`: 42 tests (candle parsing, position PnL, fee accrual, synthetic replay, walk-forward, slippage, regime)
- `funding_capture.rs`: 40 tests (entry/exit, parameter validation, funding tracking, pipeline)
- `route_cost.rs`: 27 tests (cross-venue fee comparison, spread analysis, optimal routing)
- `risk.rs`: 28 tests (daily/weekly reset, circuit breaker, consecutive loss, volatility sizing, API degradation, correlated exposure, divergence tracking)
- `config.rs`: 25 tests (TOML parsing, section validation, defaults)
- `paper.rs`: 24 tests (MultiPaperEngine, position matrix, fee accounting)
- `regime.rs`: 20 tests (regime labels, fingerprints, strategy compatibility, snapshots)
- `src/bin/copy-trader.rs`: 106 tests (position mirroring, risk management, trade log)
- `src/bin/alpha-scanner.rs`: 64 tests (wallet discovery, scoring, decay detection)
- `src/bin/whale-watcher.rs`: 41 tests (WebSocket parsing, alerts, accuracy tracking)
- `src/bin/scrape-leaderboards.rs`: 22 tests (API parsing, deduplication)
- `src/bin/analyze-wallet.rs`: 24 tests (wallet classification, blueprint generation)
- `src/bin/scan-markets.rs`: 18 tests (market ranking, pool data)
- `src/bin/pipeline.rs`: 14 tests (CLI args, report generation, managed child process)
- `src/bin/scrape-dextrabot.rs`: 8 tests (Dextrabot API integration)
- Other modules: ~24 tests (pnl_tracker, hl_info, signal, executor, engine, flash_api)

### Python Tests
Run with `python -m pytest analysis/tests/ -v`. 132 tests total.
- Each module has its own test file: `test_position_clustering.py`, `test_wallet_metrics.py`, etc.
- Use synthetic fill data for unit tests (no API calls required)
- Edge cases: <10 trades, empty fills, single-market wallets
- At least 3 tests per module covering happy path + edge cases
