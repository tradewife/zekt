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
cargo test                               # Run 828 Rust unit tests
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

## Milestones

| ID | Milestone | Status | Outcome |
|----|-----------|--------|---------|
| M10 | Walk-Forward Edge Hardening + Leverage-Aware Position Sizing | ✅ Complete | **REJECT ALL** — no candidate passes promotion gate |

### M10: Walk-Forward Edge Hardening (2026-05-31)

Tested 9 strategy-market candidates (blueprint-cluster-002/005/007/008/009 × BTC/ETH/SOL) through a three-phase validation pipeline:

1. **M1 — Walk-Forward Parameter Search:** 294,081 parameter combinations across 6 dimensions × 5 values, 5 expanding windows. Best OOS Sharpe 20.57 (cluster-009:ETH) but with only 5 trades — pure noise. 100% of top-3 parameter sets flagged as overfit.

2. **M2 — Leverage & Position Sizing Frontier:** Extended to 90-day backtest with 315 grid cells (7 leverage × 5 sizing modes). All candidates collapsed: M1 best Sharpe 4.05 → M2 best Sharpe 0.08. Fee-to-gross ratios 51–8,657%. More trades confirmed the signal was noise.

3. **M3 — Portfolio Construction:** 3 allocation strategies (Equal Weight, Risk Parity, Sharpe Weighted) — all net-negative. Cross-candidate diversification cannot rescue negative-edge strategies.

4. **Liquidation Zone Capture:** Infrastructure validated (9 zones from OI imbalance in 37s), needs dedicated 24-72h capture run.

**Verdict:** Honest negative result. Reject all candidates for paper promotion. The promising short-window signals were overfit artifacts; fee dominance overwhelms any remaining edge.

**Commits:** `63b62d0` → `28bd4fd` → `41ba5ae` → `b732ed3` → `429a721` → `231cc2e`

## Next Steps

Ranked by expected impact (from M10 root cause analysis):

1. **New strategy architectures** — Explore mean-reversion, funding-capture, or regime-adaptive approaches. The current momentum-threshold blueprints appear inherently noisy for this timeframe/market.
2. **Expand wallet pool** — Current 9 candidates come from limited HL clusters. Broader discovery (100+ wallets) could find genuinely different edges.
3. **Reduce execution costs** — Limit-order execution, maker rebates, or cross-venue routing. Need 80%+ fee reduction (current 50% Imperial savings is insufficient).
4. **Liquidation cascade mission** — Dedicated 24-72h capture with expanded watchlist (50-100 traders) for event-driven alpha.
5. **Longer backtest windows** — 180-365 day validation for low-frequency strategies (90 days may still be insufficient).
6. **Higher trade count threshold** — Require ≥50 OOS trades (not 30) before promotion to reduce small-sample noise.

## Testing

**Rust:** 828 unit tests. `cargo test`
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
