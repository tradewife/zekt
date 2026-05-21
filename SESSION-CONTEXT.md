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
cargo test    # 140 unit tests

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
- `risk.max_total_notional_usd` = 10000
- `risk.max_daily_loss_usd` = 200
- `risk.max_drawdown_pct` = 15%

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
- `src/bin/scrape-leaderboards.rs` — Wallet discovery via QuickNode HyperCore API (8 tests)
- `src/bin/analyze-wallet.rs` — Wallet strategy classifier + blueprint generator (24 tests)
- `config/perps.toml` — All tunable parameters
- `analysis/` — Python analysis pipeline (position clustering, metrics, classification, blueprints)
- `analysis/tests/` — Python unit tests (pytest)

## Milestones

**M1 (Completed): P0 Bug Fixes + Root Documentation** — Fixed borrow accrual (60x understated), added cross-cell position limits, populated pool data for LP Consumption strategy. Updated all root .md files.

**M2: Real HL Wallet Discovery via QuickNode** — Replace fake Hyperliquid seed addresses with QuickNode HyperCore API. Batch scan 100+ wallets. Filter: >50 fills, positive PnL, active <30 days.

**M3: Fill-Level Deep Analysis (Python)** — Implement Bulk.Trade methodology: position clustering, wallet metrics, strategy classification, entry reconstruction, cluster analysis, blueprint generation.

**M4: Flash Trade Market Intelligence + Strategy Implementation** — Market scanner, data-driven strategies from blueprints, every parameter traceable to wallet cluster.

**M5: Validation + Monitoring Foundation** — Backtest (Sharpe ≥ 1.0), paper trade 24h+ (positive net PnL), monitoring loop skeleton.

## Testing
**Rust:** 140 unit tests total. Run with `cargo test`.
- strategy.rs: 63 tests
- paper.rs: 14 tests
- backtest.rs: 15 tests
- analyze-wallet.rs: 24 tests
- scrape-leaderboards.rs: 8 tests
- Other modules: 16 tests

**Python:** Run with `python -m pytest analysis/tests/ -v`.

## TODO / Next Steps
- [ ] **QuickNode wallet scanner** — Batch scan HL wallets via HyperCore API
- [ ] **Python analysis pipeline** — Fill-level Bulk.Trade methodology
- [ ] **Market scanner** — Rank Flash Trade markets by liquidity concentration
- [ ] **Data-driven strategies** — Parameters from blueprints, not invented defaults
- [ ] WebSocket streaming for real-time price updates (instead of polling)
- [ ] Monitoring loop with periodic re-scanning for new strategies
- [x] ~~Backtesting engine against historical prices~~ (done: Hyperliquid candleSnapshot)
- [x] ~~LP detection~~ (done: lp-consumption strategy)
- [x] ~~LP consumption rate signal~~ (done: lp-consumption strategy, pool data populated)
- [x] ~~Unit tests~~ (done: 140 tests)
- [x] ~~Fee-awareness~~ (done: per-trade fee tracking in paper + backtest engines)
- [x] ~~P0 bug fixes~~ (done: borrow accrual, cross-cell limits, pool data)
