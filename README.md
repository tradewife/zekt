# Zekt — Multi-Strategy Perps Trading System

Autonomous multi-strategy trading system for Flash Trade (Solana perps DEX) with backtesting via Hyperliquid historical data. Originally a momentum scalper reverse-engineered from Bulk.Trade's $229K devnet competition winners, now evolved into a pluggable strategy platform via the Alpha Hunter mission.

## Strategies

Zekt implements 4 strategies as pluggable Rust modules via the `Strategy` trait. Each strategy was reverse-engineered from profitable wallets scraped from perp DEX leaderboards, then validated through backtesting and paper trading.

| Strategy | Edge | Entry Signal | Typical Hold |
|----------|------|-------------|-------------|
| **momentum-scalper** | Price velocity in thin markets | Momentum exceeds threshold over lookback window | Minutes |
| **lp-consumption** | Dominant LP being consumed in one direction | Utilization velocity + directional concentration | Minutes-hours |
| **mean-reversion** | Fading momentum spikes after deviation from SMA | Price deviates from SMA then reverses | Minutes |
| **trend-follower** | Confirmed momentum breakouts with wider stops | Velocity above breakout threshold for N consecutive ticks | Hours |

Strategies are selectable via `--strategy <name>` or `--strategies <a,b,c>` for multi-strategy mode.

**Core loop per strategy:**
```
Push price → Detect entry signal → Open position → Monitor exit conditions → Close → Record trade
```

**Exit conditions:** take profit, stop loss, trailing stop, time stop, momentum loss, reversal detection.

## Recommended Pipeline

```
1. Backtest    → Validate strategy parameters against months of Hyperliquid historical data
2. Paper Trade → Confirm against live Flash Trade prices with real fee estimates
3. Live        → Execute with real capital
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
```

## Backtesting

Replay Hyperliquid historical OHLCV candles through any strategy. No Solana wallet or RPC needed -- only the public Hyperliquid API.

**Data source:** Hyperliquid `candleSnapshot` API (`api.hyperliquid.xyz/info`)
- Intervals: 1m, 5m, 15m, 1h, 4h, 1d, 1w
- Up to 5000 candles per request, auto-paginated
- Markets: BTC, SOL, ETH, and all Hyperliquid perps

**Output:** `data/backtest-results/summary.json` (per-strategy x market stats) and `data/backtest-trades.json` (every simulated trade with entry/exit fees, PnL, hold time, exit reason).

**Metrics tracked:** net PnL, gross PnL, total fees (entry + exit + borrow), fee ratio, win rate, Sharpe ratio, max drawdown, avg hold time, best/worst trade.

```bash
# Backtest momentum scalper on BTC over 2 weeks at 5m candles
./target/release/zekt --backtest \
  --strategies momentum-scalper \
  --markets BTC \
  --backtest-start 2026-05-01 --backtest-end 2026-05-15 \
  --backtest-interval 5m --backtest-fee-rate 0.001 --paper-balance 1000
```

**CLI flags:** `--backtest`, `--backtest-start`, `--backtest-end`, `--backtest-interval` (default 5m), `--backtest-fee-rate` (default 0.001)

## Config

Edit `config/perps.toml`:

```toml
[agent]
poll_interval_secs = 5
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
direction_bias = "neutral"
momentum_threshold_pct = 0.15
take_profit_pct = 2.5
stop_loss_pct = 1.0
trailing_stop_pct = 0.8
trailing_activation_pct = 1.5
max_hold_secs = 1800
cooldown_after_loss_secs = 300

# Per-strategy overrides (see config/perps.toml for full schema)
[strategy.lp-consumption]
consumption_velocity_threshold = 0.5
lp_concentration_min = 0.7
take_profit_pct = 2.0
stop_loss_pct = 1.0

[strategy.mean-reversion]
mean_lookback = 120
deviation_threshold_pct = 1.5
take_profit_pct = 1.0
stop_loss_pct = 1.5

[strategy.trend-follower]
breakout_threshold_pct = 0.25
confirmation_ticks = 4
take_profit_pct = 5.0
stop_loss_pct = 2.0
max_hold_secs = 7200

[risk]
max_position_notional_usd = 1000.0
max_daily_loss_usd = 200.0
max_drawdown_pct = 15.0
```

## Supported Markets

Flash Trade Crypto.1 pool: SOL, BTC, ETH, ZEC, BNB, XAU, XAG, EUR, JPY, JUP, BONK, WIF, PENGU, FARTCOIN, and more.

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
    scrape-leaderboards.rs   Scrape profitable wallets from perp DEX leaderboards
    analyze-wallet.rs        Classify wallet strategies, generate strategy blueprints
```

## CLI Binaries

```bash
# Scrape wallets from Flash Trade (fstats.io), Jupiter, Hyperliquid leaderboards
cargo run --bin scrape-leaderboards -- --source all --output data/wallets.json

# Analyze wallets and generate strategy blueprints
cargo run --bin analyze-wallet -- --wallets data/wallets.json --output data/reports/
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
- Cooldown after losses, position sizing caps, leverage limits
- Take profit, stop loss, trailing stop, time stop (per strategy)
- Native on-chain TP/SL via Flash Trade trigger orders
- Real USDC balance checks (SPL token account) before every trade
- Graceful shutdown on SIGINT/SIGTERM

## Testing

140 unit tests covering all strategies, backtesting, paper trading, wallet analysis, and leaderboard scraping. Run with `cargo test`.

## Origin

See `docs/bulktrade-analysis.md` for the original Bulk.Trade devnet analysis. See `docs/MISSION-ALPHA-HUNTER.md` for the Alpha Hunter mission that evolved Zekt from a single-strategy scalper into a multi-strategy platform with backtesting.

## Flash Trade API

Base URL: `https://flashapi.trade`

Key endpoints:
- `GET /prices/{symbol}` -- Real-time oracle prices
- `GET /positions/owner/{owner}` -- Enriched positions with PnL
- `POST /transaction-builder/open-position` -- Build unsigned Solana tx
- `POST /transaction-builder/close-position` -- Build close tx
- `POST /transaction-builder/place-trigger-order` -- Place TP/SL
- `WS /owner/{owner}/ws` -- Real-time WebSocket streaming

No auth required. Transactions signed locally with your Solana keypair.

## License

MIT
