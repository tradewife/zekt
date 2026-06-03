# Zekt — Liquidation Zone Exploitation Engine for Solana Perps

A strategy research and validation platform for Solana perpetual futures (Flash Trade). Discovers profitable trading patterns from Hyperliquid wallets via fill-level analysis, then validates them rigorously before execution. Rust binary for trading infrastructure + Python analysis pipeline for wallet intelligence.

**Pipeline:** Research on Hyperliquid (rich data via QuickNode HyperCore API) → Validate rigorously → Execute on Flash Trade (Solana perps).

**Current status:** 19 strategies implemented and tested. Liquidation zone strategies validated against 72h captured data with confidence sweep completed — no strategy promoted to live paper trading yet. Data resolution and multi-source corroboration are the remaining gaps. No live trading has ever occurred. A durable capture service runs under systemd, capturing BTC/ETH/SOL zone data from Hyperliquid + Imperial with a 75-wallet watchlist.

## Pipeline

```
1. Discover   → Scrape Hyperliquid leaderboards via QuickNode for profitable wallets
2. Analyze    → Extract fill-level data, classify strategies, generate blueprints (Python)
3. Implement  → Build data-driven strategies in Rust from blueprint parameters
4. Backtest   → Walk-forward validation against Hyperliquid historical candles (Sharpe ≥ 1.0)
5. Validate   → Leverage/sizing frontier analysis (90-day out-of-sample)
6. Paper Trade → Confirm against live Flash Trade prices with real fee estimates (24h+)
7. Live       → Execute with real capital (requires human approval)
```

Steps 1–4 run semi-automatically. Steps 5–7 require increasing human oversight. No strategy has ever reached step 7.

## Quick Start

```bash
cargo build --release                    # Build
cargo test                               # Run 1,254 Rust tests
python -m pytest analysis/tests/ -v      # Run 132 Python analysis tests
python -m pytest scripts/tests/ -v       # Run 210 Python scripts tests

# Backtest against Hyperliquid historical data
./target/release/zekt --backtest --strategies momentum-scalper --markets BTC,SOL \
  --backtest-start 2026-05-01 --backtest-interval 5m --paper-balance 1000

# Backtest with Imperial route oracle (multi-venue cost estimation)
./target/release/zekt --backtest --strategies momentum-scalper --markets SOL \
  --backtest-start 2026-05-15 --backtest-interval 5m \
  --cost-mode imperial-route-oracle

# Multi-strategy paper trading
./target/release/zekt --paper --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Liquidation zone replay against captured data
./target/release/zekt --liquidation-replay --data-dir data/liquidation-snapshots

# Dry run (single API preview, then exit)
./target/release/zekt --dry-run

# Wallet discovery
QUICKNODE_HL_URL=https://your-endpoint.quiknode.pro/... \
  cargo run --bin scrape-leaderboards -- --quicknode-url $QUICKNODE_HL_URL --output data/wallets-hl.json

# Wallet analysis
cargo run --bin analyze-wallet -- --wallets data/wallets-hl.json --output data/reports/

# Pipeline orchestration
cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48
```

## Architecture

```
src/
  main.rs              CLI (clap) + graceful shutdown
  config.rs            TOML config parser
  strategy.rs          Strategy trait + 19 implementations + factory (194+ tests)
  signal.rs            MomentumDetector, Signal/ExitReason types
  backtest.rs          BacktestEngine (walk-forward, slippage, regime filter) (42 tests)
  flash_api.rs         Flash Trade REST client
  hl_info.rs           Hyperliquid Info API client
  imperial.rs          Imperial Solana perps aggregator client (53 tests)
  route_cost.rs        Multi-venue route cost oracle (27 tests)
  liquidation.rs       Liquidation zone data model, 4-source fusion, persistence (101 tests)
  replay.rs            Replay validation pipeline with 12-criterion promotion gate (45+ tests)
  liquidity_memory.rs  Durable zone memory — historical zone classifications, touch tracking, aging
  fishing.rs           Fishing order simulator — fill rate, adverse selection, expectancy
  pyramiding.rs        Bounded pyramiding engine — multi-entry position scaling with risk limits
  regime.rs            Market regime detector (LowVol/Trending/HighVol/Choppy) (20 tests)
  funding_capture.rs   Funding rate capture strategy (40 tests)
  pnl_tracker.rs       Combined PnL tracking
  executor.rs          Solana keypair loading + tx sign/submit
  risk.rs              Risk manager (SL/TP/trailing, circuit breaker, ATR sizing) (28 tests)
  engine.rs            Live trading loop
  paper.rs             Paper trading engine (24 tests)
  src/bin/             CLI binaries (pipeline, alpha-scanner, copy-trader, whale-watcher, etc.)
analysis/              Python analysis pipeline (clustering, metrics, classification, blueprints)
scripts/               Python utility scripts (param-search, leverage-sizing, portfolio, pyramiding, capture)
```

## Strategies (19 total)

### Original (5)
| Strategy | Edge |
|----------|------|
| **momentum-scalper** | Price velocity in thin markets |
| **lp-consumption** | Dominant LP being consumed in one direction |
| **mean-reversion** | Fading momentum spikes after deviation from SMA |
| **trend-follower** | Confirmed momentum breakouts with wider stops |
| **funding-capture** | Delta-neutral yield from positive funding rates |

### Data-Driven Blueprint (10)
| Strategy | Source |
|----------|--------|
| **blueprint-scalper** | Cluster-001 (scalping wallets) |
| **blueprint-mean-revert** | Cluster-004 (mean-reversion wallets) |
| **blueprint-cluster-002** | Cluster-002 parameters |
| **blueprint-cluster-003** | Cluster-003 parameters |
| **blueprint-cluster-005** | Cluster-005 parameters |
| **blueprint-cluster-006** | Cluster-006 parameters |
| **blueprint-cluster-007** | Cluster-007 parameters |
| **blueprint-cluster-008** | Cluster-008 parameters |
| **blueprint-cluster-009** | Cluster-009 parameters |
| **blueprint-hft-market-maker** | HFT market-making cluster |

### Liquidation Intelligence (4)
| Strategy | Edge |
|----------|------|
| **liquidation-cascade-hunter** | Liquidation cascade continuation + exhaustion reversal |
| **sweep-reclaim** | Price reclaim after liquidity sweep at liquidation zones |
| **liquidity-memory-fisher** | Fishing orders at historically validated liquidation zones |
| **liquidation-zone-arbiter** | Multi-strategy zone coordinator — fuses signals from zone strategies, manages entry priority |

All strategies implement the `Strategy` trait (`detect_entry`, `detect_exit`, `parameters`, `push_price`, `snapshot`). Exit conditions: take profit, stop loss, trailing stop, time stop, momentum loss, reversal detection.

## Key APIs

| API | Purpose | Auth |
|-----|---------|------|
| **Flash Trade** | Execution target: prices, positions, tx builder | None (public) |
| **Hyperliquid Info** | Intelligence layer: fills, candles, wallet state, funding rates | None (1200 weight/min) |
| **Imperial** | Solana perps aggregator: route cost comparison, liquidation OI, orderbook depth | None (public, read-only) |
| **QuickNode HyperCore** | Batch wallet scanning, fills, portfolio snapshots | Endpoint URL + Token |
| **Solana RPC** | Transaction submission | None (public) |

86 tradeable symbols across 4 Solana perps venues (Flash Trade, Phoenix, GMTrade, Jupiter).

## Validation Results (M1–M4)

### M1–M3: Blueprint Strategy Validation

The most recent validation mission tested 9 strategy-market candidates through a three-phase pipeline:

1. **Walk-Forward Parameter Search:** 294,081 parameter combinations across 6 dimensions × 5 values, 5 expanding windows. Best out-of-sample Sharpe 20.57 (cluster-009:ETH) — but with only 5 trades, pure noise. 100% of top-3 parameter sets flagged as overfit.

2. **Leverage & Position Sizing Frontier:** Extended to 90-day backtest with 315 grid cells (7 leverage × 5 sizing modes). All candidates collapsed: best Sharpe went from 4.05 (17-day) to 0.08 (90-day). Fee-to-gross ratios 51–8,657%.

3. **Portfolio Construction:** 3 allocation strategies (Equal Weight, Risk Parity, Sharpe Weighted) — all net-negative. Diversification cannot rescue negative-edge strategies.

**Verdict:** Reject all candidates. The promising short-window signals were overfit artifacts. The platform's validation pipeline worked correctly by catching this before any capital was deployed.

### M4: Liquidation Zone Exploitation Engine

Built 3 new liquidation strategies (sweep-reclaim, liquidity-memory-fisher, liquidation-zone-arbiter), a zone arbiter coordinator, fishing order simulator, bounded pyramiding engine, and a 12-criterion promotion gate. All infrastructure is functional and tested.

A 72-hour capture with 75 wallets produced zone data across BTC/ETH/SOL from Hyperliquid + Imperial sources. Confidence sweep completed. The promotion gate returned **Denied** — data resolution and multi-source corroboration remain the gaps. No strategy promoted to live paper trading yet.

**Key modules:** `liquidity_memory.rs` (durable zone memory), `fishing.rs` (fishing order simulator), `pyramiding.rs` (bounded pyramiding), extended `replay.rs` (12-criterion promotion gate).

**Capture service:** Runs under systemd, continuously capturing BTC/ETH/SOL zone data from HL + Imperial with a 75-wallet watchlist. Snapshots stored in `data/liquidation-snapshots/`.

## Testing

**Rust:** 1,254 tests. `cargo test`
**Python analysis:** 132 tests. `python -m pytest analysis/tests/ -v`
**Python scripts:** 210 tests. `python -m pytest scripts/tests/ -v`

## Configuration

Config is in `config/perps.toml`. Key sections:

```toml
[agent]          # Poll interval, log level
[flash]          # Market, leverage, slippage
[strategy]       # Active strategy, clip size, per-strategy overrides
[risk]           # Loss limits, circuit breaker, correlated exposure, ATR sizing
[backtest]       # Walk-forward, slippage model, regime filter, cost mode
[imperial]       # Imperial API base URL, timeout, enabled flag
[route-oracle]   # Cost mode (flash/imperial), min improvement BPS
[liquidation]    # Sources, snapshot dir, retention, confidence threshold
[alpha-scanner]  # Wallet discovery settings
[copy-trader]    # Position mirroring settings
[whale-watcher]  # WebSocket alert settings
```

## License

MIT
