# Leverage & Position Sizing Frontier Analysis

> Extended backtest period: 2026-03-01 to 2026-05-30 (90 days, 5m candles)
> Walk-forward: expanding, 5 windows
> Candidates tested: 9
> Total grid cells: 315 (315 successful)

## Methodology

### Leverage Grid

Seven leverage levels are tested to map the risk-return frontier:

- **1.0x**: liquidation at 100.0% from entry
- **2.0x**: liquidation at 50.0% from entry
- **3.0x**: liquidation at 33.3% from entry
- **4.0x**: liquidation at 25.0% from entry
- **5.0x**: liquidation at 20.0% from entry
- **7.5x**: liquidation at 13.3% from entry
- **10.0x**: liquidation at 10.0% from entry

### Position Sizing Modes

Five sizing methodologies from the `SizingMode` enum in `backtest.rs`:

1. **fixed-notional** — Constant notional size from strategy params (baseline).
   Simplest approach; position size does not adapt to market conditions.
   Justified as baseline comparison — all improvements must beat this.

2. **fixed-fractional** — Risk a fixed fraction of current equity per trade.
   `size = equity × risk_fraction` (default 2%). Scales with account growth,
   compounds gains, and reduces naturally after losses. Based on the
   **Kelly Criterion** concept of proportional betting: for a strategy with
   win rate `p` and win/loss ratio `b`, the Kelly-optimal fraction is
   `f* = p - (1-p)/b`. We use a conservative 2% (fractional Kelly)
   to avoid overbetting on uncertain edge estimates.

3. **volatility-adjusted** — Scale position inversely with ATR (Average True Range).
   `size = min(equity × base_fraction × (ATR_baseline / ATR_current), max_size_usd)`.
   Reduces exposure in high-vol regimes, increases in calm markets.
   Justified by the principle of **risk parity**: equalizing the dollar
   volatility per trade regardless of market regime.

4. **drawdown-throttled** — Reduce position size as drawdown deepens.
   Linear throttle from `throttle_start_pct` (5%) to `max_drawdown_pct` (20%),
   where trading is paused entirely. A practical risk management approach
   that prevents catastrophic compounding of losses.
   Justified by behavioral finance research showing that drawdowns impair
   decision quality — reducing exposure during drawdowns is rational.

5. **route-cost-adjusted** — Penalize position size for expensive execution routes.
   `size = equity × base_fraction × (1 - penalty)`, where penalty scales
   with route cost (spread + fees) relative to a threshold. Routes costing
   more than `max_penalty_pct` (80%) of expected edge are skipped entirely.
   Justified by the net-edge principle: a signal's value must exceed
   execution cost to be worth trading.

### Risk Metrics

| Metric | Description | Computation |
|--------|-------------|-------------|
| Net PnL | Total profit after fees, borrow, slippage | Sum of trade net PnLs |
| Sharpe Ratio | Risk-adjusted return | mean(returns) / std(returns), annualized |
| Sortino Ratio | Downside-adjusted return | mean(returns) / downside_deviation |
| Calmar Ratio | Return vs max drawdown | annualized_return / max_drawdown |
| Max Drawdown | Largest peak-to-trough decline | Equity curve tracking |
| Liquidation Proximity | Avg % distance from worst price to liq | Per-trade, leverage-dependent |
| Risk of Ruin | Monte Carlo probability of >90% loss | 1000 shuffle simulations |
| Fee-to-Gross | Fees as fraction of gross profit | total_fees / |gross_pnl| |
| Max Consecutive Losses | Longest losing streak | Sequential counting |
| Recovery Time | Avg time to recover from >5% drawdown | Equity curve analysis |

### Walk-Forward Validation

All backtests use expanding walk-forward validation with 5 windows.
The initial training window uses 60% of the data. Each successive window
expands by including more data. Only out-of-sample (test) results are
used for the frontier analysis, avoiding in-sample overfitting.

## Summary

| Candidate | Cost Mode | Best Sharpe | Best PnL | Trades (90d) | Optimal Lev |
|-----------|-----------|-------------|----------|--------------|-------------|
| cluster-005:ETH | imperial-route-oracle | 0.08 | $4.84 | 90 | 3.0x (volatility-adjusted) |
| cluster-008:BTC | imperial-route-oracle | 0.02 | $-0.12 | 40 | 1.0x (volatility-adjusted) |
| cluster-005:ETH | flash-only | -0.11 | $-3.59 | 90 | 2.0x (volatility-adjusted) |
| cluster-005:SOL | imperial-route-oracle | -0.01 | $-0.29 | 102 | 10.0x (volatility-adjusted) |
| cluster-005:SOL | flash-only | -0.10 | $-2.74 | 102 | 3.0x (volatility-adjusted) |
| cluster-008:BTC | flash-only | -0.25 | $-5.12 | 41 | 10.0x (volatility-adjusted) |
| cluster-007:BTC | imperial-route-oracle | -0.15 | $-3.31 | 42 | 5.0x (fixed-notional) |
| cluster-007:BTC | flash-only | -0.31 | $-7.21 | 42 | 7.5x (volatility-adjusted) |
| cluster-002:SOL | imperial-route-oracle | -0.05 | $-1.91 | 125 | 4.0x (fixed-notional) |

## Detailed Results Per Candidate

### cluster-005:ETH (imperial-route-oracle)

*M1 baseline: imperial, Sharpe 1.36, 33 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.49475, max_hold_secs=86400*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 90 | $-17.18 | 0.01 | 0.01 | $16.45 | 1.075 | 49.87% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 90 | $-0.76 | -0.01 | -0.01 | $0.53 | 1.528 | 49.87% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 90 | $1.03 | 0.06 | 0.10 | $1.29 | 0.630 | 49.87% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 33.16% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 33.16% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 90 | $-17.18 | 0.01 | 0.01 | $16.45 | 1.075 | 33.16% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 90 | $-0.76 | -0.01 | -0.01 | $0.53 | 1.528 | 33.16% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 90 | $1.03 | 0.06 | 0.10 | $1.29 | 0.630 | 33.16% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 90 | $-1.36 | 0.00 | 0.01 | $1.20 | 1.355 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 90 | $-1.36 | 0.00 | 0.01 | $1.20 | 1.355 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 90 | $4.84 | 0.04 | 0.07 | $13.97 | 0.464 | 24.80% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 90 | $-2.12 | -0.08 | -0.12 | $0.70 | 3.263 | 24.80% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 90 | $2.15 | 0.08 | 0.13 | $1.15 | 0.517 | 24.80% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 19.79% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 19.79% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 90 | $-17.18 | 0.01 | 0.01 | $16.45 | 1.075 | 19.79% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 90 | $-0.76 | -0.01 | -0.01 | $0.53 | 1.528 | 19.79% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 90 | $1.03 | 0.06 | 0.10 | $1.29 | 0.630 | 19.79% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 90 | $-17.18 | 0.01 | 0.01 | $16.45 | 1.075 | 16.44% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 90 | $-0.76 | -0.01 | -0.01 | $0.53 | 1.528 | 16.44% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 90 | $1.03 | 0.06 | 0.10 | $1.29 | 0.630 | 16.44% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 11.53% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 90 | $-1.28 | 0.01 | 0.01 | $1.22 | 1.075 | 11.53% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 90 | $-17.17 | 0.01 | 0.01 | $16.45 | 1.074 | 11.53% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 90 | $-0.53 | 0.01 | 0.01 | $0.51 | 1.075 | 11.53% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 90 | $1.03 | 0.06 | 0.10 | $1.29 | 0.630 | 11.53% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 90 | $-1.27 | 0.01 | 0.01 | $1.22 | 1.073 | 8.85% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 90 | $-1.27 | 0.01 | 0.01 | $1.22 | 1.073 | 8.85% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 90 | $-17.09 | 0.01 | 0.01 | $16.44 | 1.072 | 8.85% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 90 | $-0.75 | -0.00 | -0.01 | $0.53 | 1.522 | 8.85% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 90 | $1.04 | 0.06 | 0.10 | $1.29 | 0.628 | 8.85% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 90 | $-17.18 | 0.01 | $16.45 | 1.075 |
| 2.0x | 50.0% | 90 | $-17.18 | 0.01 | $16.45 | 1.075 |
| 3.0x | 33.3% | 90 | $4.84 | 0.04 | $13.97 | 0.464 |
| 4.0x | 25.0% | 90 | $-17.18 | 0.01 | $16.45 | 1.075 |
| 5.0x | 20.0% | 90 | $-17.18 | 0.01 | $16.45 | 1.075 |
| 7.5x | 13.3% | 90 | $-17.17 | 0.01 | $16.45 | 1.074 |
| 10.0x | 10.0% | 90 | $-17.09 | 0.01 | $16.44 | 1.072 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 90 | $-1.36 | 0.00 | $1.20 | 1.355 | 0.00% |
| fixed-fractional | 90 | $-1.36 | 0.00 | $1.20 | 1.355 | 0.00% |
| fixed-notional | 90 | $4.84 | 0.04 | $13.97 | 0.464 | 0.00% |
| route-cost-adjusted | 90 | $-2.12 | -0.08 | $0.70 | 3.263 | 0.00% |
| volatility-adjusted | 90 | $2.15 | 0.08 | $1.15 | 0.517 | 0.00% |

### cluster-008:BTC (imperial-route-oracle)

*M1 baseline: imperial, Sharpe 2.50, 31 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.111975, max_hold_secs=86400*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 49.81% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 49.81% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.191 | 49.81% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 40 | $-2.80 | -0.14 | -0.17 | $0.71 | 4.689 | 49.81% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 40 | $-0.12 | 0.02 | 0.03 | $1.02 | 0.968 | 49.81% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 40 | $-2.75 | -0.07 | -0.09 | $0.93 | 2.997 | 33.08% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 33.08% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.191 | 33.08% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 40 | $-2.80 | -0.14 | -0.17 | $0.71 | 4.689 | 33.08% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 40 | $-0.75 | -0.00 | -0.00 | $1.07 | 1.500 | 33.08% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 40 | $-3.83 | -0.12 | -0.15 | $1.05 | 4.321 | 24.71% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 40 | $-3.83 | -0.12 | -0.15 | $1.05 | 4.321 | 24.71% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 40 | $-3.62 | -0.01 | -0.01 | $2.04 | 1.717 | 24.71% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 40 | $-3.88 | -0.22 | -0.26 | $0.86 | 6.761 | 24.71% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 40 | $-2.94 | -0.08 | -0.11 | $1.28 | 3.509 | 24.71% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 19.69% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 19.69% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.191 | 19.69% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 40 | $-1.06 | -0.02 | -0.03 | $0.53 | 1.866 | 19.69% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 40 | $-0.75 | -0.00 | -0.00 | $1.07 | 1.500 | 19.69% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 16.34% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 16.34% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.191 | 16.34% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 40 | $-1.69 | -0.06 | -0.08 | $0.58 | 2.891 | 16.34% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 40 | $-0.75 | -0.00 | -0.00 | $1.07 | 1.500 | 16.34% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 11.42% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.193 | 11.42% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.191 | 11.42% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 40 | $-2.80 | -0.14 | -0.17 | $0.71 | 4.689 | 11.42% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 40 | $-0.75 | -0.00 | -0.00 | $1.07 | 1.500 | 11.42% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.192 | 8.74% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 40 | $-1.01 | 0.01 | 0.02 | $0.78 | 1.192 | 8.74% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 40 | $-2.53 | 0.01 | 0.02 | $1.96 | 1.190 | 8.74% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 40 | $-2.79 | -0.14 | -0.17 | $0.71 | 4.685 | 8.74% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 40 | $-0.75 | -0.00 | -0.00 | $1.07 | 1.499 | 8.74% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 40 | $-2.53 | 0.01 | $1.96 | 1.191 |
| 2.0x | 50.0% | 40 | $-2.53 | 0.01 | $1.96 | 1.191 |
| 3.0x | 33.3% | 40 | $-3.62 | -0.01 | $2.04 | 1.717 |
| 4.0x | 25.0% | 40 | $-2.53 | 0.01 | $1.96 | 1.191 |
| 5.0x | 20.0% | 40 | $-2.53 | 0.01 | $1.96 | 1.191 |
| 7.5x | 13.3% | 40 | $-2.53 | 0.01 | $1.96 | 1.191 |
| 10.0x | 10.0% | 40 | $-2.53 | 0.01 | $1.96 | 1.190 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 40 | $-3.83 | -0.12 | $1.05 | 4.321 | 0.00% |
| fixed-fractional | 40 | $-3.83 | -0.12 | $1.05 | 4.321 | 0.00% |
| fixed-notional | 40 | $-3.62 | -0.01 | $2.04 | 1.717 | 0.00% |
| route-cost-adjusted | 40 | $-3.88 | -0.22 | $0.86 | 6.761 | 0.00% |
| volatility-adjusted | 40 | $-2.94 | -0.08 | $1.28 | 3.509 | 0.00% |

### cluster-005:ETH (flash-only)

*M1 baseline: flash, Sharpe 1.20, 17 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.33235, max_hold_secs=43200*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 49.87% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 49.87% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 49.87% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 33.16% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 33.16% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 33.16% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 33.16% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 33.16% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 24.80% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 24.80% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 24.80% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 19.79% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 19.79% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 19.79% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 19.79% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 19.79% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 16.44% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 16.44% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 16.44% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 11.53% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 11.53% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 11.53% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 11.53% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 11.53% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 8.85% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 90 | $-8.65 | -0.16 | -0.21 | $2.23 | 6.198 | 8.85% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 90 | $-116.32 | -0.16 | -0.21 | $30.05 | 6.191 | 8.85% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 90 | $-3.59 | -0.16 | -0.21 | $0.93 | 6.194 | 8.85% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 90 | $-8.82 | -0.11 | -0.14 | $2.40 | 3.257 | 8.85% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 2.0x | 50.0% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 3.0x | 33.3% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 4.0x | 25.0% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 5.0x | 20.0% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 7.5x | 13.3% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |
| 10.0x | 10.0% | 90 | $-116.32 | -0.16 | $30.05 | 6.191 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 90 | $-8.65 | -0.16 | $2.23 | 6.198 | 0.00% |
| fixed-fractional | 90 | $-8.65 | -0.16 | $2.23 | 6.198 | 0.00% |
| fixed-notional | 90 | $-116.32 | -0.16 | $30.05 | 6.191 | 0.00% |
| route-cost-adjusted | 90 | $-3.59 | -0.16 | $0.93 | 6.194 | 0.00% |
| volatility-adjusted | 90 | $-8.82 | -0.11 | $2.40 | 3.257 | 0.00% |

### cluster-005:SOL (imperial-route-oracle)

*M1 baseline: imperial, Sharpe 2.50, 19 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.80735, max_hold_secs=86400*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 49.86% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 49.86% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 102 | $-18.85 | -0.04 | -0.05 | $17.00 | 2.468 | 49.86% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 102 | $-0.58 | -0.04 | -0.06 | $0.52 | 2.469 | 49.86% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 102 | $-1.29 | -0.04 | -0.06 | $0.81 | 2.323 | 49.86% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 33.15% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 33.15% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 102 | $-18.85 | -0.04 | -0.05 | $17.00 | 2.468 | 33.15% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 102 | $-2.69 | -0.15 | -0.19 | $0.84 | 5.227 | 33.15% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 102 | $-0.57 | -0.02 | -0.02 | $0.75 | 1.928 | 33.15% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 102 | $-9.05 | -0.21 | -0.25 | $2.45 | 6.622 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 102 | $-9.05 | -0.21 | -0.25 | $2.45 | 6.622 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 102 | $-18.82 | -0.04 | -0.05 | $17.00 | 2.467 | 24.80% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 102 | $-10.35 | -0.56 | -0.52 | $2.45 | 15.226 | 24.80% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 102 | $-8.94 | -0.29 | -0.34 | $2.10 | 6.546 | 24.80% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 19.78% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 102 | $-1.40 | -0.04 | -0.06 | $1.26 | 2.470 | 19.78% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 102 | $-18.85 | -0.04 | -0.05 | $17.00 | 2.468 | 19.78% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 102 | $-0.58 | -0.04 | -0.06 | $0.52 | 2.469 | 19.78% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 102 | $-0.57 | -0.02 | -0.02 | $0.75 | 1.928 | 19.78% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 102 | $-1.40 | -0.04 | -0.05 | $1.26 | 2.469 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 102 | $-1.40 | -0.04 | -0.05 | $1.26 | 2.469 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 102 | $-18.81 | -0.04 | -0.05 | $16.99 | 2.467 | 16.44% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 102 | $-2.69 | -0.15 | -0.19 | $0.84 | 5.225 | 16.44% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 102 | $-0.57 | -0.02 | -0.02 | $0.75 | 1.928 | 16.44% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 102 | $-0.87 | -0.03 | -0.04 | $1.19 | 2.577 | 11.52% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 102 | $-0.87 | -0.03 | -0.04 | $1.19 | 2.577 | 11.52% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 102 | $-11.70 | -0.03 | -0.04 | $15.96 | 2.575 | 11.52% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 102 | $-0.36 | -0.03 | -0.04 | $0.49 | 2.576 | 11.52% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 102 | $-0.44 | -0.01 | -0.02 | $0.73 | 2.100 | 11.52% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 102 | $-0.69 | -0.03 | -0.04 | $1.16 | 2.526 | 8.84% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 102 | $-0.69 | -0.03 | -0.04 | $1.16 | 2.526 | 8.84% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 102 | $-9.20 | -0.03 | -0.04 | $15.59 | 2.525 | 8.84% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 102 | $-3.12 | -0.18 | -0.22 | $0.91 | 6.084 | 8.84% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 102 | $-0.29 | -0.01 | -0.01 | $0.72 | 2.059 | 8.84% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 102 | $-18.85 | -0.04 | $17.00 | 2.468 |
| 2.0x | 50.0% | 102 | $-18.85 | -0.04 | $17.00 | 2.468 |
| 3.0x | 33.3% | 102 | $-18.82 | -0.04 | $17.00 | 2.467 |
| 4.0x | 25.0% | 102 | $-18.85 | -0.04 | $17.00 | 2.468 |
| 5.0x | 20.0% | 102 | $-18.81 | -0.04 | $16.99 | 2.467 |
| 7.5x | 13.3% | 102 | $-11.70 | -0.03 | $15.96 | 2.575 |
| 10.0x | 10.0% | 102 | $-9.20 | -0.03 | $15.59 | 2.525 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 102 | $-9.05 | -0.21 | $2.45 | 6.622 | 0.00% |
| fixed-fractional | 102 | $-9.05 | -0.21 | $2.45 | 6.622 | 0.00% |
| fixed-notional | 102 | $-18.82 | -0.04 | $17.00 | 2.467 | 0.00% |
| route-cost-adjusted | 102 | $-10.35 | -0.56 | $2.45 | 15.226 | 0.00% |
| volatility-adjusted | 102 | $-8.94 | -0.29 | $2.10 | 6.546 | 0.00% |

### cluster-005:SOL (flash-only)

*M1 baseline: flash, Sharpe 2.18, 19 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.6511, max_hold_secs=86400*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 49.86% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 49.86% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 49.86% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 49.86% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 49.86% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 33.15% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 33.15% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 33.15% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 33.15% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 33.15% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 24.80% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 24.80% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 24.80% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 24.80% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 19.78% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 19.78% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 19.78% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 19.78% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 19.78% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 16.44% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 16.44% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 16.44% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 16.44% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 11.52% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 11.52% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 11.52% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 11.52% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 11.52% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 8.84% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 102 | $-6.59 | -0.16 | -0.19 | $2.04 | 5.282 | 8.84% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 102 | $-88.63 | -0.16 | -0.19 | $27.50 | 5.281 | 8.84% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 102 | $-2.74 | -0.16 | -0.19 | $0.85 | 5.281 | 8.84% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 102 | $-3.24 | -0.10 | -0.13 | $1.09 | 3.337 | 8.84% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 2.0x | 50.0% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 3.0x | 33.3% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 4.0x | 25.0% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 5.0x | 20.0% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 7.5x | 13.3% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |
| 10.0x | 10.0% | 102 | $-88.63 | -0.16 | $27.50 | 5.281 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 102 | $-6.59 | -0.16 | $2.04 | 5.282 | 0.00% |
| fixed-fractional | 102 | $-6.59 | -0.16 | $2.04 | 5.282 | 0.00% |
| fixed-notional | 102 | $-88.63 | -0.16 | $27.50 | 5.281 | 0.00% |
| route-cost-adjusted | 102 | $-2.74 | -0.16 | $0.85 | 5.281 | 0.00% |
| volatility-adjusted | 102 | $-3.24 | -0.10 | $1.09 | 3.337 | 0.00% |

### cluster-008:BTC (flash-only)

*M1 baseline: flash, Sharpe 2.99, 9 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.95745, max_hold_secs=43200*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 49.80% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 49.80% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 49.80% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 49.80% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 49.80% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 33.07% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 33.07% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 33.07% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 33.07% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 33.07% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 24.70% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 24.70% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 24.70% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 24.70% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 24.70% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 19.68% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 19.68% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 19.68% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 19.68% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 19.68% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 16.34% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 16.34% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 16.34% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 16.34% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 16.34% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 11.41% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 11.41% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 11.41% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 11.41% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 11.41% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 8.73% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 41 | $-8.01 | -0.28 | -0.32 | $1.75 | 303.740 | 8.73% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 41 | $-20.12 | -0.28 | -0.32 | $4.40 | 340.206 | 8.73% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 41 | $-5.12 | -0.28 | -0.32 | $1.12 | 315.970 | 8.73% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 41 | $-8.81 | -0.25 | -0.29 | $2.10 | 86.577 | 8.73% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 2.0x | 50.0% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 3.0x | 33.3% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 4.0x | 25.0% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 5.0x | 20.0% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 7.5x | 13.3% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |
| 10.0x | 10.0% | 41 | $-20.12 | -0.28 | $4.40 | 340.206 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 41 | $-8.01 | -0.28 | $1.75 | 303.740 | 0.00% |
| fixed-fractional | 41 | $-8.01 | -0.28 | $1.75 | 303.740 | 0.00% |
| fixed-notional | 41 | $-20.12 | -0.28 | $4.40 | 340.206 | 0.00% |
| route-cost-adjusted | 41 | $-5.12 | -0.28 | $1.12 | 315.970 | 0.00% |
| volatility-adjusted | 41 | $-8.81 | -0.25 | $2.10 | 86.577 | 0.00% |

### cluster-007:BTC (imperial-route-oracle)

*M1 baseline: imperial, Sharpe 4.05, 14 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.17595, max_hold_secs=43200*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 49.71% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 49.71% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 42 | $-12.94 | -0.15 | -0.19 | $7.14 | 2.569 | 49.71% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 42 | $-3.42 | -0.19 | -0.23 | $1.42 | 3.500 | 49.71% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 42 | $-4.10 | -0.19 | -0.22 | $2.77 | 2.566 | 49.71% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 62.70% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 62.70% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 42 | $-12.94 | -0.15 | -0.19 | $7.14 | 2.569 | 62.70% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 42 | $-3.31 | -0.18 | -0.23 | $1.40 | 3.339 | 62.70% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 42 | $-4.10 | -0.19 | -0.22 | $2.77 | 2.566 | 62.70% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 42 | $-6.16 | -0.26 | -0.30 | $2.09 | 5.725 | 35.64% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 42 | $-6.16 | -0.26 | -0.30 | $2.09 | 5.725 | 35.64% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 42 | $-12.90 | -0.15 | -0.19 | $7.00 | 2.436 | 35.64% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 42 | $-5.86 | -0.28 | -0.32 | $1.91 | 6.485 | 35.64% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 42 | $-5.92 | -0.22 | -0.26 | $2.82 | 3.493 | 35.64% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 42 | $-6.19 | -0.25 | -0.30 | $2.14 | 6.324 | 25.40% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 25.40% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 42 | $-12.94 | -0.15 | -0.19 | $7.14 | 2.569 | 25.40% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 42 | $-5.90 | -0.27 | -0.32 | $1.96 | 7.192 | 25.40% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 42 | $-4.10 | -0.19 | -0.22 | $2.77 | 2.566 | 25.40% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 19.81% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 42 | $-3.71 | -0.18 | -0.22 | $1.63 | 3.194 | 19.81% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 42 | $-12.94 | -0.15 | -0.19 | $7.14 | 2.569 | 19.81% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 42 | $-3.42 | -0.19 | -0.23 | $1.42 | 3.500 | 19.81% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 42 | $-4.10 | -0.19 | -0.22 | $2.77 | 2.566 | 19.81% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 42 | $-4.30 | -0.20 | -0.24 | $1.71 | 3.637 | 12.79% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 42 | $-4.30 | -0.20 | -0.24 | $1.71 | 3.637 | 12.79% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 42 | $-13.52 | -0.16 | -0.20 | $7.16 | 2.668 | 12.79% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 42 | $-6.48 | -0.29 | -0.34 | $2.08 | 7.714 | 12.79% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 42 | $-4.68 | -0.20 | -0.24 | $2.79 | 2.801 | 12.79% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 42 | $-4.29 | -0.20 | -0.24 | $1.70 | 3.630 | 9.40% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 42 | $-4.29 | -0.20 | -0.24 | $1.70 | 3.630 | 9.40% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 42 | $-13.48 | -0.16 | -0.20 | $7.16 | 2.662 | 9.40% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 42 | $-6.44 | -0.29 | -0.33 | $2.07 | 7.685 | 9.40% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 42 | $-4.66 | -0.20 | -0.24 | $2.79 | 2.795 | 9.40% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 42 | $-12.94 | -0.15 | $7.14 | 2.569 |
| 2.0x | 50.0% | 42 | $-12.94 | -0.15 | $7.14 | 2.569 |
| 3.0x | 33.3% | 42 | $-12.90 | -0.15 | $7.00 | 2.436 |
| 4.0x | 25.0% | 42 | $-12.94 | -0.15 | $7.14 | 2.569 |
| 5.0x | 20.0% | 42 | $-12.94 | -0.15 | $7.14 | 2.569 |
| 7.5x | 13.3% | 42 | $-13.52 | -0.16 | $7.16 | 2.668 |
| 10.0x | 10.0% | 42 | $-13.48 | -0.16 | $7.16 | 2.662 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 42 | $-6.16 | -0.26 | $2.09 | 5.725 | 0.00% |
| fixed-fractional | 42 | $-6.16 | -0.26 | $2.09 | 5.725 | 0.00% |
| fixed-notional | 42 | $-12.90 | -0.15 | $7.00 | 2.436 | 0.00% |
| route-cost-adjusted | 42 | $-5.86 | -0.28 | $1.91 | 6.485 | 0.00% |
| volatility-adjusted | 42 | $-5.92 | -0.22 | $2.82 | 3.493 | 0.00% |

### cluster-007:BTC (flash-only)

*M1 baseline: flash, Sharpe 2.74, 14 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.197, max_hold_secs=43200*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 49.71% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 49.71% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 49.71% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 49.71% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 49.71% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 62.70% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 62.70% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 62.70% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 62.70% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 62.70% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 35.64% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 35.64% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 35.64% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 35.64% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 35.64% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 25.40% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 25.40% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 25.40% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 25.40% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 25.40% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 19.81% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 19.81% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 19.81% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 19.81% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 19.81% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 12.79% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 12.79% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 12.79% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 12.79% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 12.79% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 9.40% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 42 | $-8.51 | -0.33 | -0.37 | $2.60 | 7.638 | 9.40% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 42 | $-38.16 | -0.33 | -0.37 | $11.67 | 7.641 | 9.40% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 42 | $-7.21 | -0.33 | -0.37 | $2.21 | 7.638 | 9.40% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 42 | $-10.32 | -0.31 | -0.34 | $3.73 | 5.594 | 9.40% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 2.0x | 50.0% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 3.0x | 33.3% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 4.0x | 25.0% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 5.0x | 20.0% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 7.5x | 13.3% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |
| 10.0x | 10.0% | 42 | $-38.16 | -0.33 | $11.67 | 7.641 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 42 | $-8.51 | -0.33 | $2.60 | 7.638 | 0.00% |
| fixed-fractional | 42 | $-8.51 | -0.33 | $2.60 | 7.638 | 0.00% |
| fixed-notional | 42 | $-38.16 | -0.33 | $11.67 | 7.641 | 0.00% |
| route-cost-adjusted | 42 | $-7.21 | -0.33 | $2.21 | 7.638 | 0.00% |
| volatility-adjusted | 42 | $-10.32 | -0.31 | $3.73 | 5.594 | 0.00% |

### cluster-002:SOL (imperial-route-oracle)

*M1 baseline: imperial, Sharpe 1.08, 14 trades in 17d*
*Parameters: lookback_count=15, momentum_threshold_pct=0.173855, max_hold_secs=43200*

#### Full Grid (All Metrics)

| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |
|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|
| 1.0x | drawdown-throttled | 125 | $-3.20 | -0.08 | -0.10 | $1.17 | 6.472 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-fractional | 125 | $-3.20 | -0.08 | -0.10 | $1.17 | 6.472 | 49.87% | 0.00% | 0.00h |
| 1.0x | fixed-notional | 125 | $-4.77 | -0.06 | -0.07 | $2.22 | 5.093 | 49.87% | 0.00% | 0.00h |
| 1.0x | route-cost-adjusted | 125 | $-2.64 | -0.11 | -0.13 | $0.83 | 8.347 | 49.87% | 0.00% | 0.00h |
| 1.0x | volatility-adjusted | 125 | $-3.07 | -0.12 | -0.14 | $0.89 | 9.581 | 49.87% | 0.00% | 0.00h |
| 2.0x | drawdown-throttled | 125 | $-2.35 | -0.06 | -0.08 | $1.02 | 5.335 | 70.29% | 0.00% | 0.00h |
| 2.0x | fixed-fractional | 125 | $-2.83 | -0.07 | -0.09 | $1.09 | 5.972 | 70.29% | 0.00% | 0.00h |
| 2.0x | fixed-notional | 125 | $-4.39 | -0.05 | -0.07 | $2.19 | 4.865 | 70.29% | 0.00% | 0.00h |
| 2.0x | route-cost-adjusted | 125 | $-2.81 | -0.11 | -0.14 | $0.87 | 8.755 | 70.29% | 0.00% | 0.00h |
| 2.0x | volatility-adjusted | 125 | $-2.69 | -0.11 | -0.13 | $0.80 | 8.715 | 70.29% | 0.00% | 0.00h |
| 3.0x | drawdown-throttled | 125 | $-7.20 | -0.16 | -0.19 | $1.98 | 11.814 | 38.69% | 0.00% | 0.00h |
| 3.0x | fixed-fractional | 125 | $-7.20 | -0.16 | -0.19 | $1.98 | 11.814 | 38.69% | 0.00% | 0.00h |
| 3.0x | fixed-notional | 125 | $-8.77 | -0.10 | -0.12 | $2.91 | 7.525 | 38.69% | 0.00% | 0.00h |
| 3.0x | route-cost-adjusted | 125 | $-7.19 | -0.27 | -0.29 | $1.82 | 18.948 | 38.69% | 0.00% | 0.00h |
| 3.0x | volatility-adjusted | 125 | $-7.07 | -0.25 | -0.28 | $1.82 | 18.824 | 38.69% | 0.00% | 0.00h |
| 4.0x | drawdown-throttled | 125 | $-2.10 | -0.06 | -0.07 | $1.01 | 4.998 | 27.17% | 0.00% | 0.00h |
| 4.0x | fixed-fractional | 125 | $-2.10 | -0.06 | -0.07 | $1.01 | 4.998 | 27.17% | 0.00% | 0.00h |
| 4.0x | fixed-notional | 125 | $-3.66 | -0.05 | -0.06 | $2.12 | 4.422 | 27.17% | 0.00% | 0.00h |
| 4.0x | route-cost-adjusted | 125 | $-2.56 | -0.11 | -0.13 | $0.82 | 8.166 | 27.17% | 0.00% | 0.00h |
| 4.0x | volatility-adjusted | 125 | $-1.96 | -0.08 | -0.10 | $0.67 | 7.030 | 27.17% | 0.00% | 0.00h |
| 5.0x | drawdown-throttled | 125 | $-2.82 | -0.07 | -0.09 | $1.09 | 5.970 | 21.05% | 0.00% | 0.00h |
| 5.0x | fixed-fractional | 125 | $-2.82 | -0.07 | -0.09 | $1.09 | 5.970 | 21.05% | 0.00% | 0.00h |
| 5.0x | fixed-notional | 125 | $-4.39 | -0.05 | -0.07 | $2.19 | 4.864 | 21.05% | 0.00% | 0.00h |
| 5.0x | route-cost-adjusted | 125 | $-2.26 | -0.09 | -0.12 | $0.76 | 7.471 | 21.05% | 0.00% | 0.00h |
| 5.0x | volatility-adjusted | 125 | $-2.69 | -0.11 | -0.13 | $0.80 | 8.712 | 21.05% | 0.00% | 0.00h |
| 7.5x | drawdown-throttled | 125 | $-2.52 | -0.07 | -0.08 | $1.05 | 5.844 | 13.52% | 0.00% | 0.00h |
| 7.5x | fixed-fractional | 125 | $-2.52 | -0.07 | -0.08 | $1.05 | 5.844 | 13.52% | 0.00% | 0.00h |
| 7.5x | fixed-notional | 125 | $-4.23 | -0.05 | -0.07 | $2.20 | 5.089 | 13.52% | 0.00% | 0.00h |
| 7.5x | route-cost-adjusted | 125 | $-1.91 | -0.08 | -0.10 | $0.68 | 6.864 | 13.52% | 0.00% | 0.00h |
| 7.5x | volatility-adjusted | 125 | $-2.43 | -0.10 | -0.12 | $0.74 | 8.395 | 13.52% | 0.00% | 0.00h |
| 10.0x | drawdown-throttled | 125 | $-3.81 | -0.09 | -0.11 | $1.29 | 7.618 | 9.95% | 0.00% | 0.00h |
| 10.0x | fixed-fractional | 125 | $-3.81 | -0.09 | -0.11 | $1.29 | 7.618 | 9.95% | 0.00% | 0.00h |
| 10.0x | fixed-notional | 125 | $-5.35 | -0.06 | -0.08 | $2.29 | 5.845 | 9.95% | 0.00% | 0.00h |
| 10.0x | route-cost-adjusted | 125 | $-4.48 | -0.17 | -0.20 | $1.22 | 12.974 | 9.95% | 0.00% | 0.00h |
| 10.0x | volatility-adjusted | 125 | $-4.31 | -0.16 | -0.19 | $1.18 | 12.853 | 9.95% | 0.00% | 0.00h |

#### Sharpe by Leverage Level (fixed-notional baseline)

| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |
|----------|-------------|--------|---------|--------|--------|-----------|
| 1.0x | 100.0% | 125 | $-4.77 | -0.06 | $2.22 | 5.093 |
| 2.0x | 50.0% | 125 | $-4.39 | -0.05 | $2.19 | 4.865 |
| 3.0x | 33.3% | 125 | $-8.77 | -0.10 | $2.91 | 7.525 |
| 4.0x | 25.0% | 125 | $-3.66 | -0.05 | $2.12 | 4.422 |
| 5.0x | 20.0% | 125 | $-4.39 | -0.05 | $2.19 | 4.864 |
| 7.5x | 13.3% | 125 | $-4.23 | -0.05 | $2.20 | 5.089 |
| 10.0x | 10.0% | 125 | $-5.35 | -0.06 | $2.29 | 5.845 |

#### Sizing Mode Comparison (3x leverage)

| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |
|-------------|--------|---------|--------|--------|-----------|-----|
| drawdown-throttled | 125 | $-7.20 | -0.16 | $1.98 | 11.814 | 0.00% |
| fixed-fractional | 125 | $-7.20 | -0.16 | $1.98 | 11.814 | 0.00% |
| fixed-notional | 125 | $-8.77 | -0.10 | $2.91 | 7.525 | 0.00% |
| route-cost-adjusted | 125 | $-7.19 | -0.27 | $1.82 | 18.948 | 0.00% |
| volatility-adjusted | 125 | $-7.07 | -0.25 | $1.82 | 18.824 | 0.00% |

## Liquidation Price Estimates

For leveraged positions, estimated liquidation prices depend on entry price,
leverage, and direction. The table below shows theoretical liquidation
distances for reference entry prices.

### Long Positions

| Leverage | BTC Entry $100K Liq | Distance | ETH Entry $2.5K Liq | Distance | SOL Entry $170 Liq | Distance |
|----------|---------------------|----------|---------------------|----------|---------------------|----------|
| 1.0x | $0 | 100.0% | $0 | 100.0% | $0.0 | 100.0% |
| 2.0x | $50000 | 50.0% | $1250 | 50.0% | $85.0 | 50.0% |
| 3.0x | $66667 | 33.3% | $1667 | 33.3% | $113.3 | 33.3% |
| 4.0x | $75000 | 25.0% | $1875 | 25.0% | $127.5 | 25.0% |
| 5.0x | $80000 | 20.0% | $2000 | 20.0% | $136.0 | 20.0% |
| 7.5x | $86667 | 13.3% | $2167 | 13.3% | $147.3 | 13.3% |
| 10.0x | $90000 | 10.0% | $2250 | 10.0% | $153.0 | 10.0% |

### Short Positions

| Leverage | BTC Entry $100K Liq | Distance | ETH Entry $2.5K Liq | Distance | SOL Entry $170 Liq | Distance |
|----------|---------------------|----------|---------------------|----------|---------------------|----------|
| 1.0x | $∞ | 100.0% | $5000 | 100.0% | $340.0 | 100.0% |
| 2.0x | $∞ | 50.0% | $3750 | 50.0% | $255.0 | 50.0% |
| 3.0x | $∞ | 33.3% | $3333 | 33.3% | $226.7 | 33.3% |
| 4.0x | $∞ | 25.0% | $3125 | 25.0% | $212.5 | 25.0% |
| 5.0x | $∞ | 20.0% | $3000 | 20.0% | $204.0 | 20.0% |
| 7.5x | $∞ | 13.3% | $2833 | 13.3% | $192.7 | 13.3% |
| 10.0x | $∞ | 10.0% | $2750 | 10.0% | $187.0 | 10.0% |

### Observed Average Liquidation Proximity

Average % distance from worst intra-trade price to estimated liquidation price,
computed per-trade and averaged across all trades.

| Candidate | 1x | 2x | 3x | 5x | 7.5x | 10x |
|-----------|----|----|----|----|----|------|
| cluster-005:ETH | 49.87% | 33.16% | 24.80% | 16.44% | 11.53% | 8.85% |
| cluster-008:BTC | 49.81% | 33.08% | 24.71% | 16.34% | 11.42% | 8.74% |
| cluster-005:ETH | 49.87% | 33.16% | 24.80% | 16.44% | 11.53% | 8.85% |
| cluster-005:SOL | 49.86% | 33.15% | 24.80% | 16.44% | 11.52% | 8.84% |
| cluster-005:SOL | 49.86% | 33.15% | 24.80% | 16.44% | 11.52% | 8.84% |
| cluster-008:BTC | 49.80% | 33.07% | 24.70% | 16.34% | 11.41% | 8.73% |
| cluster-007:BTC | 49.71% | 62.70% | 35.64% | 19.81% | 12.79% | 9.40% |
| cluster-007:BTC | 49.71% | 62.70% | 35.64% | 19.81% | 12.79% | 9.40% |
| cluster-002:SOL | 49.87% | 70.29% | 38.69% | 21.05% | 13.52% | 9.95% |

## Efficient Frontier Analysis

The efficient frontier maps leverage against risk-adjusted return (Sharpe).
For each candidate, we identify the 'knee' of the curve — the point where
increasing leverage no longer improves Sharpe proportionally (diminishing returns)
while drawdowns accelerate.

### cluster-005:ETH (imperial-route-oracle)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | 0.01 | $-17.18 | $16.45 | 0.00% |  |
| 2.0x | 0.01 | $-17.18 | $16.45 | 0.00% | +0.00 |
| 3.0x | 0.04 | $4.84 | $13.97 | 0.00% | +0.04 |
| 4.0x | 0.01 | $-17.18 | $16.45 | 0.00% | -0.04 |
| 5.0x | 0.01 | $-17.18 | $16.45 | 0.00% | +0.00 |
| 7.5x | 0.01 | $-17.17 | $16.45 | 0.00% | +0.00 |
| 10.0x | 0.01 | $-17.09 | $16.44 | 0.00% | +0.00 |

### cluster-008:BTC (imperial-route-oracle)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | 0.01 | $-2.53 | $1.96 | 0.00% |  |
| 2.0x | 0.01 | $-2.53 | $1.96 | 0.00% | +0.00 |
| 3.0x | -0.01 | $-3.62 | $2.04 | 0.00% | -0.02 |
| 4.0x | 0.01 | $-2.53 | $1.96 | 0.00% | +0.02 |
| 5.0x | 0.01 | $-2.53 | $1.96 | 0.00% | +0.00 |
| 7.5x | 0.01 | $-2.53 | $1.96 | 0.00% | +0.00 |
| 10.0x | 0.01 | $-2.53 | $1.96 | 0.00% | +0.00 |

### cluster-005:ETH (flash-only)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.16 | $-116.32 | $30.05 | 0.00% |  |
| 2.0x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |
| 3.0x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |
| 4.0x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |
| 5.0x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |
| 7.5x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |
| 10.0x | -0.16 | $-116.32 | $30.05 | 0.00% | +0.00 |

### cluster-005:SOL (imperial-route-oracle)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.04 | $-18.85 | $17.00 | 0.00% |  |
| 2.0x | -0.04 | $-18.85 | $17.00 | 0.00% | +0.00 |
| 3.0x | -0.04 | $-18.82 | $17.00 | 0.00% | +0.00 |
| 4.0x | -0.04 | $-18.85 | $17.00 | 0.00% | -0.00 |
| 5.0x | -0.04 | $-18.81 | $16.99 | 0.00% | +0.00 |
| 7.5x | -0.03 | $-11.70 | $15.96 | 0.00% | +0.01 |
| 10.0x | -0.03 | $-9.20 | $15.59 | 0.00% | +0.00 |

### cluster-005:SOL (flash-only)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.16 | $-88.63 | $27.50 | 0.00% |  |
| 2.0x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |
| 3.0x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |
| 4.0x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |
| 5.0x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |
| 7.5x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |
| 10.0x | -0.16 | $-88.63 | $27.50 | 0.00% | +0.00 |

### cluster-008:BTC (flash-only)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.28 | $-20.12 | $4.40 | 0.00% |  |
| 2.0x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |
| 3.0x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |
| 4.0x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |
| 5.0x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |
| 7.5x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |
| 10.0x | -0.28 | $-20.12 | $4.40 | 0.00% | +0.00 |

### cluster-007:BTC (imperial-route-oracle)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.15 | $-12.94 | $7.14 | 0.00% |  |
| 2.0x | -0.15 | $-12.94 | $7.14 | 0.00% | +0.00 |
| 3.0x | -0.15 | $-12.90 | $7.00 | 0.00% | -0.00 |
| 4.0x | -0.15 | $-12.94 | $7.14 | 0.00% | +0.00 |
| 5.0x | -0.15 | $-12.94 | $7.14 | 0.00% | +0.00 |
| 7.5x | -0.16 | $-13.52 | $7.16 | 0.00% | -0.00 |
| 10.0x | -0.16 | $-13.48 | $7.16 | 0.00% | +0.00 |

### cluster-007:BTC (flash-only)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.33 | $-38.16 | $11.67 | 0.00% |  |
| 2.0x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |
| 3.0x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |
| 4.0x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |
| 5.0x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |
| 7.5x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |
| 10.0x | -0.33 | $-38.16 | $11.67 | 0.00% | +0.00 |

### cluster-002:SOL (imperial-route-oracle)

| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |
|----------|--------|---------|--------|-----|-------------------|
| 1.0x | -0.06 | $-4.77 | $2.22 | 0.00% |  |
| 2.0x | -0.05 | $-4.39 | $2.19 | 0.00% | +0.00 |
| 3.0x | -0.10 | $-8.77 | $2.91 | 0.00% | -0.04 |
| 4.0x | -0.05 | $-3.66 | $2.12 | 0.00% | +0.05 |
| 5.0x | -0.05 | $-4.39 | $2.19 | 0.00% | -0.01 |
| 7.5x | -0.05 | $-4.23 | $2.20 | 0.00% | +0.00 |
| 10.0x | -0.06 | $-5.35 | $2.29 | 0.00% | -0.01 |

## Recommended Maximum Leverage

Based on the efficient frontier analysis, the following maximum leverage
levels are recommended per strategy-market pair. The recommendation balances
three criteria:
1. **Sharpe improvement**: Leverage should improve risk-adjusted return
2. **Drawdown tolerance**: Max drawdown should not exceed 20% of starting balance
3. **Risk of ruin**: RoR should remain below 10%

| Candidate | Cost Mode | Recommended Max Lev | Sharpe at Max | DD at Max | RoR at Max | Rationale |
|-----------|-----------|--------------------|----------------|-----------|-----------|-----------|
| cluster-005:ETH | imperial-route-oracle | 10.0x | 0.01 | $16.44 | 0.00% | Sharpe 0.01, DD $16.44, RoR 0.00% |
| cluster-008:BTC | imperial-route-oracle | 10.0x | 0.01 | $1.96 | 0.00% | Sharpe 0.01, DD $1.96, RoR 0.00% |
| cluster-005:ETH | flash-only | 1x | — | — | — | No profitable leverage found |
| cluster-005:SOL | imperial-route-oracle | 1x | — | — | — | No profitable leverage found |
| cluster-005:SOL | flash-only | 1x | — | — | — | No profitable leverage found |
| cluster-008:BTC | flash-only | 1x | — | — | — | No profitable leverage found |
| cluster-007:BTC | imperial-route-oracle | 1x | — | — | — | No profitable leverage found |
| cluster-007:BTC | flash-only | 1x | — | — | — | No profitable leverage found |
| cluster-002:SOL | imperial-route-oracle | 1x | — | — | — | No profitable leverage found |

## Sizing Mode Recommendations

For each candidate at its recommended leverage, which sizing mode performs best?

| Candidate | Cost Mode | Lev | Best Sizing | Sharpe | PnL | DD | Rationale |
|-----------|-----------|-----|-------------|--------|-----|----|-----------|
| cluster-005:ETH | imperial-route-oracle | 3.0x | volatility-adjusted | 0.08 | $2.15 | $1.15 | Best risk-adjusted return |
| cluster-008:BTC | imperial-route-oracle | 1.0x | volatility-adjusted | 0.02 | $-0.12 | $1.02 | Best risk-adjusted return |
| cluster-005:ETH | flash-only | 2.0x | volatility-adjusted | -0.11 | $-8.82 | $2.40 | Best risk-adjusted return |
| cluster-005:SOL | imperial-route-oracle | 10.0x | volatility-adjusted | -0.01 | $-0.29 | $0.72 | Best risk-adjusted return |
| cluster-005:SOL | flash-only | 3.0x | volatility-adjusted | -0.10 | $-3.24 | $1.09 | Best risk-adjusted return |
| cluster-008:BTC | flash-only | 10.0x | volatility-adjusted | -0.25 | $-8.81 | $2.10 | Best risk-adjusted return |
| cluster-007:BTC | imperial-route-oracle | 5.0x | fixed-notional | -0.15 | $-12.94 | $7.14 | Best risk-adjusted return |
| cluster-007:BTC | flash-only | 7.5x | volatility-adjusted | -0.31 | $-10.32 | $3.73 | Best risk-adjusted return |
| cluster-002:SOL | imperial-route-oracle | 4.0x | fixed-notional | -0.05 | $-3.66 | $2.12 | Best risk-adjusted return |

## Conclusions

1. **Extended period effectiveness**: 9/9 candidates achieved ≥30 trades in the 90-day window.
2. **Profitability**: 1/9 candidates showed positive net PnL at 1x leverage.

3. **cluster-005:ETH** (primary): Best Sharpe 0.08 at 3.0x with volatility-adjusted, PnL $2.15, 90 trades.
4. **cluster-008:BTC** (primary): Best Sharpe 0.02 at 1.0x with volatility-adjusted, PnL $-0.12, 40 trades.

### Promotion Decision

Based on the leverage-sizing frontier analysis, candidates are evaluated
for promotion to M3 portfolio construction using the same 6 promotion gate
criteria, now with the extended 90-day sample size.

- **cluster-005:ETH** (imperial-route-oracle): ✅ positive PnL, ❌ Sharpe 0.06 < 1.0, ✅ 90 trades, ❌ fee/gross 0.63
- **cluster-008:BTC** (imperial-route-oracle): ❌ negative PnL, ❌ Sharpe 0.02 < 1.0, ✅ 40 trades, ❌ fee/gross 0.97
