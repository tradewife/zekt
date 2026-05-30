# Zekt — Autonomous Strategy Poaching System

Semi-autonomous trading system that discovers profitable strategies from Hyperliquid wallets, reverse-engineers them from fill-level data, and replicates them on Solana perps. Multi-venue route comparison across Flash Trade, Phoenix, GMTrade, and Jupiter via the Imperial aggregator. Liquidation zone intelligence with 4-source fusion. Research on Hyperliquid (rich data) → Execute on Flash Trade (Solana perps).

## Pipeline

```
1. Discover   → Scrape Hyperliquid leaderboards via QuickNode for profitable wallets
2. Analyze    → Extract fill-level data, classify strategies, generate blueprints (Python)
3. Implement  → Build data-driven strategies in Rust from blueprint parameters
4. Backtest   → Validate against Hyperliquid historical candles (Sharpe ≥ 1.0)
5. Paper Trade → Confirm against live Flash Trade prices with real fee estimates (24h+)
6. Live       → Execute with real capital (requires human approval)
```

## Quick Start

```bash
cargo build --release                    # Build
cargo test                               # Run 736 Rust unit tests
python -m pytest analysis/tests/ -v      # Run 132 Python analysis tests

# Backtest against Hyperliquid historical data
./target/release/zekt --backtest --strategies momentum-scalper --markets BTC,SOL \
  --backtest-start 2026-05-01 --backtest-interval 5m --paper-balance 1000

# Multi-strategy paper trading
./target/release/zekt --paper --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Dry run (single API preview, then exit)
./target/release/zekt --dry-run

# Live (requires funded Solana wallet with USDC)
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL

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
  strategy.rs          Strategy trait + 16 implementations + factory
  signal.rs            MomentumDetector, Signal/ExitReason types
  backtest.rs          BacktestEngine (walk-forward, slippage, regime filter)
  flash_api.rs         Flash Trade REST client
  hl_info.rs           Hyperliquid Info API client
  imperial.rs          Imperial Solana perps aggregator client (read-only)
  route_cost.rs        Multi-venue route cost oracle (Flash vs Imperial)
  liquidation.rs       Liquidation zone data model, 4-source fusion, persistence
  replay.rs            Replay validation pipeline with promotion gate
  regime.rs            Market regime detector (LowVol/Trending/HighVol/Choppy)
  funding_capture.rs   Funding rate capture strategy
  pnl_tracker.rs       Combined PnL tracking
  executor.rs          Solana keypair loading + tx sign/submit
  risk.rs              Risk manager (SL/TP/trailing, circuit breaker, ATR sizing)
  engine.rs            Live trading loop
  paper.rs             Paper trading engine
  src/bin/             CLI binaries (pipeline, alpha-scanner, copy-trader, whale-watcher, etc.)
analysis/              Python analysis pipeline (clustering, metrics, classification, blueprints)
```

## Strategies (16 total)

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

### Liquidation Intelligence (1)
| Strategy | Edge |
|----------|------|
| **liquidation-cascade-hunter** | Liquidation cascade continuation + exhaustion reversal (paper-only, gated) |

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

## Testing

**Rust:** 736 unit tests. `cargo test`
**Python:** 132 analysis tests. `python -m pytest analysis/tests/ -v`

## Configuration

Config is in `config/perps.toml`. Key sections:

```toml
[agent]          # Poll interval, log level
[flash]          # Market, leverage, slippage
[strategy]       # Active strategy, clip size, per-strategy overrides
[risk]           # Loss limits, circuit breaker, correlated exposure, ATR sizing
[backtest]       # Walk-forward, slippage model, regime filter
[imperial]       # Imperial API base URL, timeout, enabled flag
[route-oracle]   # Cost mode (flash/imperial), slippage BPS
[liquidation]    # Enabled, snapshot dir, retention, confidence threshold
[alpha-scanner]  # Wallet discovery settings
[copy-trader]    # Position mirroring settings
[whale-watcher]  # WebSocket alert settings
```

## License

MIT
