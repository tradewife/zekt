# Zekt — Autonomous Strategy Poaching System

Semi-autonomous trading system that **discovers profitable strategies from Hyperliquid wallets, reverse-engineers them from fill-level data, and replicates them on Flash Trade** (Solana perps). Research on Hyperliquid (rich data) → Execute on Flash Trade (Solana perps). Poach, analyze, and paper trade automatically; require human approval before live execution.

## Pipeline: Discovery → Analysis → Implementation → Validation

```
┌──────────────────────────────────────────────────────────────────┐
│  1. DISCOVERY (Intelligence Layer — Hyperliquid)                 │
│  QuickNode HyperCore API → HL leaderboards → 100+ wallet addrs  │
│  → userFills/userFillsByTime → fill-level trade records          │
├──────────────────────────────────────────────────────────────────┤
│  2. ANALYSIS (Bulk.Trade Methodology — Python)                   │
│  Position clustering → wallet metrics → strategy classification  │
│  → entry trigger reconstruction → cluster analysis → blueprints  │
├──────────────────────────────────────────────────────────────────┤
│  3. IMPLEMENTATION (Execution Layer — Flash Trade)               │
│  Data-driven strategies from blueprints → Flash Trade market     │
│  intelligence → implement in Rust via Strategy trait              │
├──────────────────────────────────────────────────────────────────┤
│  4. VALIDATION (Truth Teller)                                    │
│  Backtest on HL candles (Sharpe ≥ 1.0) → Paper trade 24h+       │
│  (positive net PnL after fees) → Human approval → Live           │
└──────────────────────────────────────────────────────────────────┘
```

## Intelligence Layer ↔ Execution Layer

| Layer | Platform | Purpose |
|-------|----------|---------|
| **Intelligence** | Hyperliquid | Wallet discovery, fill-level analysis, strategy extraction, backtesting |
| **Execution** | Flash Trade | Market scanning, strategy implementation, paper trading, live execution |

**Why this split?** Hyperliquid has rich per-wallet fill data (`userFills` returns coin, side, px, sz, fee, closedPnl, time, dir) that Flash Trade lacks. We research where the data is richest, execute where the opportunity is.

## Current Strategies

Zekt implements 4 strategies as pluggable Rust modules via the `Strategy` trait. Originally reverse-engineered from profitable wallets scraped from perp DEX leaderboards, these are being replaced with data-driven implementations as blueprints are generated from the analysis pipeline.

| Strategy | Edge | Entry Signal | Typical Hold |
|----------|------|-------------|-------------|
| **momentum-scalper** | Price velocity in thin markets | Momentum exceeds threshold over lookback window | Minutes |
| **lp-consumption** | Dominant LP being consumed in one direction | Utilization velocity + directional concentration | Minutes-hours |
| **mean-reversion** | Fading momentum spikes after deviation from SMA | Price deviates from SMA then reverses | Minutes |
| **trend-follower** | Confirmed momentum breakouts with wider stops | Velocity above breakout threshold for N consecutive ticks | Hours |

**Core loop per strategy:**
```
Push price → Detect entry signal → Open position → Monitor exit conditions → Close → Record trade
```

**Exit conditions:** take profit, stop loss, trailing stop, time stop, momentum loss, reversal detection.

## Recommended Pipeline

```
1. Discover   → Scrape Hyperliquid leaderboards via QuickNode for profitable wallets
2. Analyze    → Extract fill-level data, classify strategies, generate blueprints (Python)
3. Implement  → Build data-driven strategies in Rust from blueprint parameters
4. Backtest   → Validate strategy parameters against months of Hyperliquid historical data
5. Paper Trade → Confirm against live Flash Trade prices with real fee estimates
6. Live       → Execute with real capital (requires human approval)
```

## Quick Start

```bash
# Build
cargo build --release

# Run tests (140 unit tests)
cargo test

# 1. BACKTEST -- replay Hyperliquid candles through strategies (no wallet needed)
./target/release/zekt --backtest \
  --strategies momentum-scalper,mean-reversion \
  --markets BTC,SOL,ETH \
  --backtest-start 2026-05-01 --backtest-end 2026-05-15 \
  --backtest-interval 5m --paper-balance 1000

# 2. PAPER TRADING -- multi-strategy multi-market against live prices
./target/release/zekt --paper \
  --strategies momentum-scalper,lp-consumption \
  --markets SOL,BTC,ETH --paper-balance 1000

# Dry run (single API preview, then exit)
./target/release/zekt --dry-run

# 3. LIVE (requires funded Solana wallet with USDC)
./target/release/zekt --keypair ~/.config/solana/id.json --market SOL

# Wallet discovery (requires QuickNode endpoint)
QUICKNODE_HL_URL=https://your-endpoint.quiknode.pro/... \
  cargo run --bin scrape-leaderboards -- --quicknode-url $QUICKNODE_HL_URL --output data/wallets-hl.json

# Wallet analysis
cargo run --bin analyze-wallet -- --wallets data/wallets-hl.json --output data/reports/

# Python analysis pipeline
python -m pytest analysis/tests/ -v
```

## Backtesting

Replay Hyperliquid historical OHLCV candles through any strategy. No Solana wallet or RPC needed -- only the public Hyperliquid API.

**Data source:** Hyperliquid `candleSnapshot` API (`api.hyperliquid.xyz/info`)
- Intervals: 1m, 5m, 15m, 1h, 4h, 1d, 1w
- Up to 5000 candles per request, auto-paginated
- Markets: BTC, SOL, ETH, and all Hyperliquid perps

**Output:** `data/backtest-results/summary.json` (per-strategy × market stats) and `data/backtest-trades.json` (every simulated trade with entry/exit fees, PnL, hold time, exit reason).

**Metrics tracked:** net PnL, gross PnL, total fees (entry + exit + borrow), fee ratio, win rate, Sharpe ratio, max drawdown, avg hold time, best/worst trade.

## Python Analysis Pipeline

The analysis pipeline implements the Bulk.Trade fill-level methodology in Python:

| Module | Purpose |
|--------|---------|
| `analysis/position_clustering.py` | Cluster individual fills into open→close position cycles |
| `analysis/wallet_metrics.py` | Per-wallet metrics: clip consistency, hold time, win rate, PnL distribution |
| `analysis/strategy_classifier.py` | Classify wallets into strategy types with evidence |
| `analysis/entry_reconstruction.py` | Reconstruct entry triggers from HL candle data |
| `analysis/cluster_analysis.py` | Find groups of wallets running identical strategies |
| `analysis/blueprint_generator.py` | Generate strategy blueprints with data-derived parameters |

```bash
# Run analysis tests
python -m pytest analysis/tests/ -v
```

## Config

Edit `config/perps.toml`:

```toml
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
market = "SOL"
leverage = 10.0
input_token = "USDC"
pool = "Crypto.1"
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"
clip_size_usd = 100.0
# ... (see config/perps.toml for full schema)

# Per-strategy overrides
[strategy.lp-consumption]
consumption_velocity_threshold = 0.5
lp_concentration_min = 0.7

[strategy.mean-reversion]
mean_lookback = 120
deviation_threshold_pct = 1.5

[strategy.trend-follower]
breakout_threshold_pct = 0.25
confirmation_ticks = 4

[risk]
max_position_notional_usd = 1000.0
max_total_notional_usd = 10000.0
max_daily_loss_usd = 200.0
max_drawdown_pct = 15.0
```

### QuickNode Configuration

Set the QuickNode HyperCore endpoint URL for Hyperliquid wallet discovery:

```bash
# Environment variable (recommended)
export QUICKNODE_HL_URL="https://your-endpoint.quiknode.pro/your-token/"

# Or CLI flag
cargo run --bin scrape-leaderboards -- --quicknode-url "https://your-endpoint.quiknode.pro/..."
```

## Supported Markets

Flash Trade Crypto.1 pool: SOL, BTC, ETH, ZEC, BNB, XAU, XAG, EUR, JPY, JUP, BONK, WIF, PENGU, FARTCOIN, and more.

Backtesting supports any Hyperliquid perps market (BTC, SOL, ETH, etc.).

## Architecture

```
src/
  main.rs              CLI entrypoint (clap) + graceful shutdown (ctrlc)
  config.rs            TOML config parser (agent, flash, strategy, risk)
  strategy.rs          Strategy trait + 4 implementations + factory (3369 lines)
  signal.rs            MomentumDetector, MomentumSnapshot, Signal/ExitReason types
  backtest.rs          Hyperliquid candle fetcher + BacktestEngine (1170 lines)
  flash_api.rs         Flash Trade REST client (prices, positions, tx builder)
  executor.rs          Solana keypair loading + tx sign/submit
  risk.rs              Risk manager (SL/TP/trailing, circuit breaker, daily reset, trade journal)
  engine.rs            Live trading loop (poll -> detect -> preview -> build tx -> sign -> monitor)
  paper.rs             Paper trading engine (single + MultiPaperEngine with position matrix)
  src/bin/
    scrape-leaderboards.rs   Discover profitable wallets via QuickNode HyperCore API + leaderboards
    analyze-wallet.rs        Classify wallet strategies, generate strategy blueprints
analysis/
  position_clustering.py     Cluster fills into open→close position cycles
  wallet_metrics.py          Per-wallet metrics (clip consistency, hold time, win rate, PnL)
  strategy_classifier.py     Classify wallets into strategy types with evidence
  entry_reconstruction.py    Reconstruct entry triggers from candle data
  cluster_analysis.py        Find groups of wallets running identical strategies
  blueprint_generator.py     Generate strategy blueprints from cluster statistics
  tests/                     Python unit tests (pytest)
```

## CLI Binaries

```bash
# Discover wallets from Hyperliquid via QuickNode
cargo run --bin scrape-leaderboards -- --quicknode-url $QUICKNODE_HL_URL --output data/wallets-hl.json

# Analyze wallets and generate strategy blueprints (Rust)
cargo run --bin analyze-wallet -- --wallets data/wallets-hl.json --output data/reports/

# Run Python analysis pipeline
python -m pytest analysis/tests/ -v
```

## Running Modes

| Mode | Flag | What it does | Risk | Data Source |
|------|------|-------------|------|-------------|
| Backtest | `--backtest` | Replay historical candles through strategies. Simulated PnL with configurable fees. | Zero | Hyperliquid API |
| Paper | `--paper` | Full open/monitor/close loop against live prices. Simulated PnL with live fee estimates. | Zero | Flash Trade API |
| Dry run | `--dry-run` | Single API preview, then exits. Shows price, fee estimate, pool data. | Zero | Flash Trade API |
| Live | (default) | Real trading with real funds. Signs and submits transactions to mainnet. | Full | Flash Trade API |

**Start with `--backtest`** to validate strategy parameters, then `--paper` to confirm with live prices, then go live.

## Risk Controls

- Daily loss limit with automatic day-boundary reset
- Max drawdown circuit breaker
- Cross-cell position limit (`max_total_notional_usd`) caps total exposure across all strategy×market cells
- Cooldown after losses, position sizing caps, leverage limits
- Take profit, stop loss, trailing stop, time stop (per strategy)
- Native on-chain TP/SL via Flash Trade trigger orders
- Real USDC balance checks (SPL token account) before every trade
- Graceful shutdown on SIGINT/SIGTERM

## Testing

**Rust:** 140 unit tests covering all strategies, backtesting, paper trading, wallet analysis, and leaderboard scraping. Run with `cargo test`.

**Python:** Analysis pipeline unit tests. Run with `python -m pytest analysis/tests/ -v`.

## External APIs

| API | Purpose | Auth |
|-----|---------|------|
| QuickNode HyperCore | HL wallet data, fills, positions (batch methods) | Endpoint URL + Token |
| Hyperliquid Info | userFills, userFillsByTime, candleSnapshot, clearinghouseState | None (1200 weight/min) |
| Flash Trade | Prices, positions, tx builder | None |
| Dune MCP | Market-level analytics | API Key |
| Solana RPC | Transaction submission | None (public) |

## Origin

See `docs/bulktrade-analysis.md` for the original Bulk.Trade devnet analysis. See `docs/MISSION-ALPHA-HUNTER.md` for the Alpha Hunter mission that evolved Zekt from a single-strategy scalper into a multi-strategy platform with backtesting.

## License

MIT
