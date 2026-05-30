# Zekt — Session Context

## What This Is
Zekt is an **autonomous strategy poaching system** that discovers profitable Hyperliquid wallets, reverse-engineers their strategies from fill-level data, and replicates them on Flash Trade (Solana perps). Originally a momentum scalper reverse-engineered from Bulk.Trade's $229K devnet competition winners, evolved via the Alpha Hunter mission into a multi-strategy platform, now fully rearchitected as a poaching pipeline: **Research on Hyperliquid → Execute on Flash Trade**.

**Autonomy level:** Semi-autonomous. Wallet discovery, analysis, backtesting, and paper trading run automatically. Live trading requires human approval.

## Architecture: Intelligence Layer ↔ Execution Layer

```
┌──────────────────────────────────────────────────────────────────┐
│                  INTELLIGENCE LAYER (Hyperliquid)                 │
│                                                                   │
│  QuickNode HyperCore API                                         │
│       │                                                           │
│       ├── hl_batchClearinghouseStates → batch wallet scanning    │
│       ├── hl_batchPortfolioStates → portfolio snapshots           │
│       ├── userFills / userFillsByTime → fill-level trade records │
│       └── candleSnapshot → historical OHLCV for backtesting      │
│               │                                                   │
│       ┌───────▼────────┐                                          │
│       │  Wallet Scanner │  scrape-leaderboards.rs                 │
│       │  (Rust binary)  │  → data/wallets-hl.json                │
│       └───────┬────────┘                                          │
│               │                                                   │
│       ┌───────▼──────────────────────────────────┐               │
│       │  Python Analysis Pipeline (analysis/)     │               │
│       │                                            │               │
│       │  position_clustering.py                    │               │
│       │    → Cluster fills into position cycles    │               │
│       │                                            │               │
│       │  wallet_metrics.py                         │               │
│       │    → Clip consistency, hold time, win rate │               │
│       │    → Fee-adjusted PnL, fill intervals      │               │
│       │                                            │               │
│       │  strategy_classifier.py                    │               │
│       │    → Classify into strategy types          │               │
│       │                                            │               │
│       │  entry_reconstruction.py                   │               │
│       │    → Fetch HL candles → find triggers      │               │
│       │                                            │               │
│       │  cluster_analysis.py                       │               │
│       │    → Group similar wallets                 │               │
│       │                                            │               │
│       │  blueprint_generator.py                    │               │
│       │    → Data-derived strategy blueprints      │               │
│       │    → data/blueprints/*.json                │               │
│       └────────────────────────────────────────────┘               │
│                                                                   │
│  Data flow: HL leaderboard → wallet addresses → userFills →      │
│  fill records → position clusters → metrics → classification →   │
│  strategy blueprints → backtest on HL candles                     │
└───────────────────────────────────┬──────────────────────────────┘
                                    │
                                    ▼
┌──────────────────────────────────────────────────────────────────┐
│                  EXECUTION LAYER (Flash Trade)                    │
│                                                                   │
│  Market Scanner (scan-markets.rs)                                │
│    → Rank Flash Trade markets by LP concentration, leverage,     │
│      volume, utilization                                         │
│    → data/market-rankings.json                                   │
│               │                                                   │
│  Strategy Implementation (strategy.rs)                           │
│    → Load blueprint JSON parameters                              │
│    → Implement via Strategy trait (not hardcoded values)         │
│               │                                                   │
│  ┌────────────┤                                                   │
│  │            │                                                   │
│  ▼            ▼                                                   │
│  Backtest    Paper Trade                                          │
│  (HL candles) (live Flash Trade prices)                          │
│  Sharpe ≥1.0?  24h+ net positive PnL?                           │
│  │            │                                                   │
│  └──────┬─────┘                                                   │
│         │                                                         │
│    ┌────▼────────────┐    ┌──────────────────┐                   │
│    │  Live Engine    │◄───│  HUMAN APPROVAL  │                   │
│    │  (keypair)      │    │  (explicit sign) │                   │
│    └─────────────────┘    └──────────────────┘                   │
└──────────────────────────────────────────────────────────────────┘
```

## Recommended Pipeline
```
1. Discover  → Scrape HL leaderboards via QuickNode HyperCore API
2. Analyze   → Python fill-level analysis → strategy blueprints
3. Implement → Rust Strategy trait with data-driven parameters
4. Backtest  → Validate on HL historical candles (Sharpe ≥ 1.0)
5. Paper     → Confirm against live Flash Trade prices (24h+ run)
6. Live      → Execute with real capital (human approval required)
```

## Commands
```bash
# Build and test
cargo build --release
cargo test    # 560 unit tests

# Python analysis tests
python -m pytest analysis/tests/ -v

# Wallet discovery via QuickNode HyperCore API
QUICKNODE_HL_URL=https://your-endpoint.quiknode.pro/... \
  cargo run --bin scrape-leaderboards -- \
    --quicknode-url $QUICKNODE_HL_URL --output data/wallets-hl.json

# Backtest against Hyperliquid historical data (no wallet needed)
./target/release/zekt --backtest \
  --strategies momentum-scalper,mean-reversion \
  --markets BTC,SOL,ETH \
  --backtest-start 2026-05-01 --backtest-end 2026-05-15 \
  --backtest-interval 5m --paper-balance 1000

# Multi-strategy multi-market paper trading
./target/release/zekt --paper \
  --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Dry run (single preview, then exit)
./target/release/zekt --dry-run --market BTC

# Live trading (requires human approval)
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL

# CLI tools
cargo run --bin analyze-wallet -- --wallets data/wallets-hl.json --output data/reports/

# Pipeline orchestration (runs all alpha engine components together)
cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48
cargo run --bin pipeline -- --once  # single scan + report
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
| `--quicknode-url` | QuickNode HyperCore endpoint URL (scrape-leaderboards) |

## Config (`config/perps.toml`)

### Agent
- `agent.poll_interval_secs` = 300
- `agent.log_level` = "info"

### QuickNode
- `QUICKNODE_HL_URL` env var or `--quicknode-url` CLI flag
- Used for batch wallet scanning via HyperCore API

### Flash Trade Connection
- `flash.api_url` = `https://flashapi.trade`
- `flash.rpc_url` = `https://api.mainnet-beta.solana.com`
- `flash.keypair_path` = Solana keypair file
- `flash.market` = Trading pair (SOL, BTC, ETH, etc.)
- `flash.input_token` = USDC
- `flash.pool` = "Crypto.1"
- `flash.leverage` = 3.0
- `flash.slippage_pct` = "0.5"

### Strategy (shared defaults)
- `strategy.active` = "momentum-scalper"
- `strategy.clip_size_usd` = 1000.0
- `strategy.momentum_threshold_pct` = 0.15
- `strategy.lookback_count` = 60
- `strategy.direction_bias` = "neutral"
- `strategy.take_profit_pct` = 1.0
- `strategy.stop_loss_pct` = 0.5
- `strategy.trailing_stop_pct` = 0.3
- `strategy.trailing_activation_pct` = 0.5
- `strategy.max_hold_secs` = 1200
- `strategy.cooldown_after_loss_secs` = 300

### Strategy Sub-Tables (per-strategy overrides)
- `[strategy.lp-consumption]` — consumption_velocity_threshold=0.5, lp_concentration_min=0.7, take_profit=1.0, stop_loss=0.5, leverage=2.5
- `[strategy.mean-reversion]` — mean_lookback=120, deviation_threshold_pct=1.5, take_profit=1.0, stop_loss=0.5, leverage=2.0
- `[strategy.trend-follower]` — breakout_threshold_pct=0.25, confirmation_ticks=4, take_profit=2.0, stop_loss=0.8, max_hold_secs=2400, leverage=3.0
- `[strategy.funding-capture]` — min_annualized_rate_pct=20.0, exit_annualized_rate_pct=5.0, max_position_hours=72, leverage=1.0, clip_size_usd=200.0

### Risk Limits
- `risk.max_position_notional_usd` = 5000
- `risk.max_total_notional_usd` = 100000
- `risk.max_daily_loss_usd` = 500
- `risk.max_drawdown_pct` = 15%
- `risk.max_weekly_loss_usd` = 100000 (effectively disabled)
- `risk.max_correlated_exposure_pct` = 100 (no limit)
- `risk.consecutive_loss_circuit_breaker` = 0 (disabled)
- `risk.volatility_sizing_enabled` = false
- `risk.volatility_sizing_atr_threshold_pct` = 75
- `risk.volatility_sizing_min_fraction` = 0.25
- `risk.api_degradation_threshold` = 0 (disabled)

### Backtest Engine
- `backtest.walk_forward_enabled` = false
- `backtest.walk_forward_train_ratio` = 0.7
- `backtest.slippage_bps` = 0.0
- `backtest.regime_filter` = true

### Pipeline (orchestrator)
- `pipeline.paper_balance` = 1000
- `pipeline.report_interval_secs` = 300
- `pipeline.scanner_refresh_secs` = 21600
- `pipeline.strategies` = "funding-capture"
- `pipeline.markets` = "BTC"
- `pipeline.combined_output` = "data/combined-pnl.json"

## Supported Markets (Flash Trade Crypto.1)
SOL, BTC, ETH, ZEC, BNB, XAU, XAG, EUR, JPY, JUP, BONK, WIF, PENGU, FARTCOIN, and more.

Backtesting also supports any Hyperliquid perps market (BTC, SOL, ETH, etc.).

## External APIs

### QuickNode HyperCore API (Primary HL Data Source)
- Batch methods: `hl_batchClearinghouseStates`, `hl_batchPortfolioStates`
- Per-wallet: `userFills`, `userFillsByTime`
- Configuration: `QUICKNODE_HL_URL` env var or `--quicknode-url` CLI flag

### Hyperliquid API (Backtesting + Fallback)
- Endpoint: `POST https://api.hyperliquid.xyz/info`
- `userFills` — per-wallet fill records (coin, side, px, sz, fee, closedPnl, time, dir, hash, startPosition)
- `userFillsByTime` — time-filtered fills
- `clearinghouseState` — wallet positions with leverage and uPnL
- `candleSnapshot` — OHLCV candles for backtesting
- No auth. Rate limit: 1200 weight/min/IP.

### Flash Trade API
- Base URL: `https://flashapi.trade`
- `GET /prices/{symbol}` → Oracle price (Pyth)
- `GET /positions/owner/{owner}` → Enriched positions with PnL
- `POST /transaction-builder/open-position` → Unsigned Solana tx
- `POST /transaction-builder/close-position` → Unsigned close tx
- `POST /transaction-builder/place-trigger-order` → TP/SL trigger orders
- `WS /owner/{owner}/ws` → Real-time WebSocket streaming
- No auth required. Transactions signed locally.

### Dune MCP
- Endpoint: `https://api.dune.com/mcp/v1`
- API key via `DUNE_API_KEY` env var
- Used for market-level analytics and bridge deposit tracking

## Critical Flash Trade Rules
- **One position per market per side per wallet**
- **Blockhash expiry ~60s** -- fresh blockhash before every sign
- **Max 5 trigger orders** (TP/SL) per market position
- **Min collateral >$10** after fees for trigger orders
- **SOL positions use JitoSOL** as underlying collateral on-chain
- **All amounts are UI format** (human-readable) in API requests
- **Wallet balances NOT available via Flash Trade API** -- use Solana RPC

## Data Flow (End-to-End)

```
QuickNode HL API → userFills → data/fills/{address}.json
                                     │
                     ┌───────────────┼───────────────┐
                     │               │               │
              position_clustering  wallet_metrics  entry_reconstruction
                     │               │               │
                     └───────┬───────┘               │
                             │                       │
                    strategy_classifier               │
                             │                       │
                    cluster_analysis                  │
                             │           │           │
                    blueprint_generator ◄─┘           │
                             │
                    data/blueprints/{cluster_id}.json
                             │
                    Rust: strategy.rs ← load blueprint params
                             │
                    Backtest → Paper → (Human Approval) → Live
```

## Shutdown
```bash
Ctrl+C          # Graceful: finishes current tick
kill -INT <pid> # Graceful
kill -9 <pid>   # Emergency
```

## Key Files
- `src/strategy.rs` — Strategy trait + 5 implementations + factory (6000+ lines, 63 tests)
- `src/funding_capture.rs` — Funding rate capture strategy (40 tests)
- `src/pnl_tracker.rs` — Combined PnL tracking across all strategies (10 tests)
- `src/hl_info.rs` — Hyperliquid Info API client (positions, funding rates, fills)
- `src/backtest.rs` — Hyperliquid candle fetcher + BacktestEngine (walk-forward, slippage, regime filter)
- `src/regime.rs` — Market regime detector (LowVol/Trending/HighVol/Choppy) + strategy-specific compatibility rules
- `src/paper.rs` — Paper trading: single engine + MultiPaperEngine (1684 lines, 14 tests)
- `src/engine.rs` — Live trading engine
- `src/signal.rs` — MomentumDetector, MomentumSnapshot, Signal/ExitReason types
- `src/risk.rs` — Risk manager (daily/weekly reset, circuit breaker, consecutive loss, correlated exposure, ATR sizing, API degradation, divergence tracking)
- `src/flash_api.rs` — Flash Trade REST client
- `src/executor.rs` — Solana tx signing
- `src/config.rs` — TOML config parser
- `src/main.rs` — CLI entrypoint, routes to backtest/paper/dry-run/live
- `src/bin/pipeline.rs` — Pipeline orchestrator: alpha-scanner + copy-trader + whale-watcher + paper (14 tests)
- `src/bin/alpha-scanner.rs` — Wallet discovery daemon (64 tests)
- `src/bin/copy-trader.rs` — Position mirroring engine (85 tests)
- `src/bin/whale-watcher.rs` — WebSocket fill monitoring (41 tests)
- `src/bin/scrape-leaderboards.rs` — Wallet discovery via QuickNode (22 tests)
- `src/bin/analyze-wallet.rs` — Strategy classifier + blueprints (24 tests)
- `src/bin/scan-markets.rs` — Flash Trade market ranking (18 tests)
- `src/bin/scrape-dextrabot.rs` — Dextrabot API integration (8 tests)
- `config/perps.toml` — All tunable parameters
- `analysis/` — Python analysis pipeline (position clustering, metrics, classification, blueprints)
- `analysis/tests/` — Python unit tests (pytest)

## Milestones

**M1 (Completed): P0 Bug Fixes + Root Documentation** — Fixed borrow accrual (60x understated), added cross-cell position limits, populated pool data for LP Consumption strategy. Updated all root .md files.

**M2 (Completed): Alpha Discovery Engine — M1 Foundation** — HL Info client, alpha-scanner binary, config registration. 98 new tests.

**M3 (Completed): Alpha Discovery Engine — M2 Traders** — Copy-trader binary (85 tests), whale-watcher binary (41 tests). WebSocket integration.

**M4 (Completed): Funding Rate Capture Strategy** — FundingRateCaptureStrategy in funding_capture.rs, wired into strategy factory. 40 tests.

**M5 (Completed): Integration + Validation** — Pipeline orchestrator (`pipeline.rs`), combined PnL tracker (`pnl_tracker.rs`), wired alpha-scanner → watchlist → copy-trader + whale-watcher. 560 tests pass. `[pipeline]` config section.

**M6 (Completed): QuickNode + Python Pipeline** — QuickNode HyperCore API integration for batch wallet scanning. Python Bulk.Trade analysis pipeline (position clustering, wallet metrics, strategy classification, entry reconstruction, cluster analysis, blueprint generation). 132 Python tests.

**M7 (Completed): Flash Trade Market Intelligence + Data-Driven Strategies** — Market scanner, data-driven strategies from blueprints (blueprint-scalper from cluster-001, blueprint-mean-revert from cluster-004). Every parameter traceable to wallet cluster.

**M8 (Completed): Bootstrap Alpha Compounding System** — 7-milestone mission covering:
- Cost model + backtest integrity (walk-forward validation, slippage model, regime segmentation, fee audit)
- Risk + kill-switch upgrade (weekly loss, correlated exposure, consecutive loss breaker, ATR sizing, API degradation, divergence framework)
- Regime-aware entry filter for all strategies (90.6% net loss reduction, 93% fee drag reduction)
- Tooling integration review (fintool REJECT, senpi-skills DEFER, atlas-gic REJECT)
- Paper-trading promotion gate (runbook, thresholds, monitoring checklist, human approval)
- Final mission report with before/after metrics, next missions ranked by impact

## Testing
**Rust:** 711 unit tests total (414 main binary + 297 binary tests). Run with `cargo test`.
- strategy.rs: 63 tests
- funding_capture.rs: 40 tests
- pnl_tracker.rs: 10 tests
- paper.rs: 14 tests
- backtest.rs: 17 tests (walk-forward, slippage, regime filter, fee decomposition)
- regime.rs: 21 tests (regime labels, fingerprints, strategy compatibility)
- risk.rs: 30 tests (daily/weekly reset, consecutive loss, correlated exposure, volatility sizing, API degradation, divergence)
- pipeline.rs: 14 tests
- analyze-wallet.rs: 24 tests
- scrape-leaderboards.rs: 22 tests
- alpha-scanner.rs: 64 tests
- copy-trader.rs: 106 tests
- whale-watcher.rs: 41 tests
- scan-markets.rs: 18 tests
- scrape-dextrabot.rs: 8 tests
- config + hl_info + other: ~33 tests

**Python:** 132 tests. Run with `python -m pytest analysis/tests/ -v`.

## TODO / Next Steps
- [ ] **Strategy parameter optimization** — Walk-forward parameter sweep to find profitable configs (Mission A)
- [ ] **Blueprint strategy validation** — Test data-driven strategies with regime filter and fee model (Mission B)
- [ ] **Self-tuning parameters** — Adaptive thresholds based on recent trade performance (Mission C, inspired by Senpi Lynx)
- [ ] WebSocket streaming for real-time price updates (instead of polling)
- [ ] Monitoring loop with periodic re-scanning for new strategies
- [x] ~~**QuickNode wallet scanner** — Batch scan HL wallets via HyperCore API~~ (done: M6)
- [x] ~~**Python analysis pipeline** — Fill-level Bulk.Trade methodology~~ (done: M6)
- [x] ~~**Data-driven strategies** — Parameters from blueprints, not invented defaults~~ (done: M7)
- [x] ~~**Integration validation** — Pipeline orchestrator + combined PnL tracker~~ (done: M5)
- [x] ~~Backtesting engine against historical prices~~ (done: Hyperliquid candleSnapshot)
- [x] ~~LP detection~~ (done: lp-consumption strategy)
- [x] ~~LP consumption rate signal~~ (done: lp-consumption strategy, pool data populated)
- [x] ~~Unit tests~~ (done: 711 tests)
- [x] ~~Fee-awareness~~ (done: per-trade fee tracking in paper + backtest engines)
- [x] ~~P0 bug fixes~~ (done: borrow accrual, cross-cell limits, pool data)
- [x] ~~Alpha scanner binary~~ (done: Dextrabot + Hypurrscan + HL enrichment, 64 tests)
- [x] ~~Copy trader binary~~ (done: position mirroring + paper trading, 106 tests)
- [x] ~~Whale watcher binary~~ (done: WebSocket alerts + accuracy tracking, 41 tests)
- [x] ~~Funding rate capture strategy~~ (done: delta-neutral yield strategy, 40 tests)
- [x] ~~HL Info client~~ (done: positions, funding rates, fills, market contexts)
- [x] ~~Risk engine upgrade~~ (done: weekly loss, correlated exposure, consecutive loss, ATR sizing, API degradation)
- [x] ~~Regime-aware entry filtering~~ (done: strategy-specific rules, 90.6% loss reduction)
- [x] ~~Walk-forward validation~~ (done: train/test split with separate metrics)
- [x] ~~Slippage model~~ (done: configurable basis points in backtest)
- [x] ~~Paper-trading promotion gate~~ (done: runbook, thresholds, human approval checklist)
