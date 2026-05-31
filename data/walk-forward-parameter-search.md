# Walk-Forward Parameter Search Results

> Generated from expanding 5-window walk-forward validation across 9 candidate strategy-market pairs.
> Grid: 6 parameters × 5, 5, 5, 5, 5, 5 values
> Backtest period: 2026-05-13 to 2026-05-30, 5m candles
> Walk-forward: expanding, 5 windows, 60% initial train
> Cost modes: flash-only, imperial-route-oracle

## Methodology

### Walk-Forward Validation

This analysis uses an **expanding walk-forward validation** approach to avoid the 
common pitfall of single-period optimization that leads to overfitting. The historical 
data is divided into 5 sequential windows. Each window has a training (in-sample) portion 
and a testing (out-of-sample) portion. The training window expands with each step, and 
the subsequent period is used for out-of-sample evaluation.

**Walk-forward configuration:**
- Mode: expanding (each successive window includes all prior data)
- Windows: 5 (test-w1 through test-w5)
- Initial training ratio: 60%
- Out-of-sample metrics aggregated across all 5 test windows

### Parameter Grid Search

A systematic grid search evaluates all combinations of strategy parameters. Each combination 
is backtested with walk-forward validation, producing both in-sample and out-of-sample metrics. 
Results are ranked by out-of-sample Sharpe ratio to prioritize generalization performance.

### Cost Mode Analysis

Two cost modes are evaluated for every parameter combination:
- **flash-only**: Uses Flash Trade fee structure directly (baseline)
- **imperial-route-oracle**: Uses the RouteCostOracle to compare execution costs across 
  Solana perps venues (Flash Trade, Imperial) and route to the lowest-cost venue. This mode 
  captures fee savings from cross-venue arbitrage.

### Overfit Detection

Parameter sets are flagged as potentially overfit based on three criteria:
1. **Train vs test divergence**: In-sample Sharpe > 2× out-of-sample Sharpe indicates 
   the strategy memorized training patterns rather than learning generalizable signals.
2. **Window inconsistency**: Positive PnL in only 1 out of 5 walk-forward windows suggests 
   results are driven by a single lucky period.
3. **Insufficient sample**: Fewer than 30 out-of-sample trades means the statistical 
   significance of any metric is unreliable.

### Metrics

| Metric | Description |
|--------|-------------|
| OOS Sharpe | Out-of-sample Sharpe ratio (annualized) across walk-forward test windows |
| OOS Net PnL | Out-of-sample profit after fees, slippage, and borrow costs |
| OOS Trades | Number of round-trip trades in out-of-sample windows |
| Win Rate | Percentage of profitable trades |
| Profit Factor | Gross profits divided by gross losses |
| Fee/Gross Ratio | Total fees as a percentage of gross profit (lower is better) |
| Max Drawdown | Maximum peak-to-trough decline in portfolio value |
| Sortino Ratio | Return / downside deviation (penalizes only negative volatility) |
| PnL Consistency | Fraction of walk-forward windows with positive net PnL |

## Promotion Criteria

Candidates must pass **all six** of the following criteria to be promoted to the 
leverage-sizing phase (M2). This gate ensures only robust, well-validated strategies 
proceed with real capital allocation.

| # | Criterion | Threshold | Rationale |
|---|-----------|-----------|-----------|
| 1 | Positive OOS PnL | Net PnL > $0 | Strategy must be profitable after all costs |
| 2 | Sharpe Ratio | ≥ 1.0 | Risk-adjusted returns must exceed cash/bond baseline |
| 3 | Trade Count | ≥ 30 | Sufficient sample for statistical significance |
| 4 | Max Drawdown | Acceptable (config-dependent) | Drawdowns within risk tolerance |
| 5 | Fee-to-Gross Ratio | < 35% | Strategy edge not consumed by execution costs |
| 6 | Parameter Stability | Low variance across windows | Performance not dependent on single period |

Candidates failing any criterion are clearly flagged with the specific failure reasons. 
Samples with <30 trades are labeled as **insufficient sample** regardless of other metrics.

## Summary

| # | Candidate | Flash Best Sharpe | Imperial Best Sharpe | Flash Best PnL | Imperial Best PnL | Flash Profitable | Imperial Profitable |
|---|-----------|-------------------|---------------------|----------------|-------------------|------------------|---------------------|
| 1 | cluster-007:BTC | 2.74 | 4.05 | +$0.22 | +$2.16 | 850/15625 | 3450/15625 |
| 2 | cluster-005:ETH | 1.20 | 2.17 | +$6.51 | +$14.46 | 3000/15625 | 9500/15625 |
| 3 | cluster-005:SOL | 2.18 | 2.50 | +$4.57 | +$7.59 | 3375/15625 | 8625/15625 |
| 4 | cluster-008:BTC | 2.99 | 3.98 | +$0.55 | +$1.44 | 525/15625 | 3025/15625 |
| 5 | cluster-002:BTC | 0.18 | 0.43 | +$0.28 | +$0.64 | 500/15625 | 5000/15625 |
| 6 | cluster-002:SOL | 1.08 | 1.08 | +$1.15 | +$1.15 | 10500/15625 | 10500/15625 |
| 7 | cluster-003:BTC | 0.22 | 0.37 | +$0.08 | +$0.28 | 720/28448 | 1225/15625 |
| 8 | cluster-009:ETH | 20.57 | 18.10 | +$3.05 | +$3.24 | 9100/15625 | 15625/15625 |
| 9 | cluster-009:SOL | 0.28 | 0.44 | +$0.44 | +$0.69 | 4300/15625 | 6900/15625 |

## Parameter Grid

| Parameter | Values | Count |
|-----------|--------|-------|
| `momentum_threshold_pct` | 0.05, 0.1, 0.2, 0.35, 0.5 | 5 |
| `take_profit_pct` | 0.1, 0.3, 0.5, 1.0, 2.0 | 5 |
| `stop_loss_pct` | 0.1, 0.3, 0.5, 0.8, 1.5 | 5 |
| `max_hold_secs` | 1800, 3600, 7200, 43200, 86400 | 5 |
| `lookback_count` | 15, 30, 45, 60, 90 | 5 |
| `trailing_stop_pct` | 0.0, 0.15, 0.3, 0.5, 0.8 | 5 |

## Detailed Results Per Candidate

### cluster-007:BTC

#### flash-only

Total combinations tested: 15625
Profitable combinations: 850

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 2.74 | +$0.22 | 14 | 50.0% | 20,002 | 0.84 | $2.95 | ⚠️ YES |
| 2 | 2.74 | +$0.22 | 14 | 50.0% | 20,002 | 0.84 | $2.95 | ⚠️ YES |
| 3 | 2.74 | +$0.22 | 14 | 50.0% | 20,002 | 0.84 | $2.95 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.197, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.2095, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.222, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.49 | 3 | +$1.46 |
| test-w2 | -0.53 | 2 | -$0.08 |
| test-w3 | 0.06 | 3 | +$0.20 |
| test-w4 | 15.22 | 2 | +$1.60 |
| test-w5 | -1.52 | 4 | -$2.97 |

- **Mean OOS Sharpe:** 2.74
- **Sharpe Std Dev:** 6.27
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.49 | 3 | +$1.46 |
| test-w2 | -0.53 | 2 | -$0.08 |
| test-w3 | 0.06 | 3 | +$0.20 |
| test-w4 | 15.22 | 2 | +$1.60 |
| test-w5 | -1.52 | 4 | -$2.97 |

- **Mean OOS Sharpe:** 2.74
- **Sharpe Std Dev:** 6.27
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.49 | 3 | +$1.46 |
| test-w2 | -0.53 | 2 | -$0.08 |
| test-w3 | 0.06 | 3 | +$0.20 |
| test-w4 | 15.22 | 2 | +$1.60 |
| test-w5 | -1.52 | 4 | -$2.97 |

- **Mean OOS Sharpe:** 2.74
- **Sharpe Std Dev:** 6.27
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 14

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 3450

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 4.05 | +$2.16 | 14 | 57.1% | 20,004 | 0.32 | $2.53 | ⚠️ YES |
| 2 | 4.05 | +$2.16 | 14 | 57.1% | 20,004 | 0.32 | $2.53 | ⚠️ YES |
| 3 | 4.05 | +$2.16 | 14 | 57.1% | 20,004 | 0.32 | $2.53 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.17595, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.1772, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.17845, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.59 | 3 | +$1.74 |
| test-w2 | 0.46 | 2 | +$0.07 |
| test-w3 | 0.17 | 3 | +$0.62 |
| test-w4 | 20.24 | 2 | +$2.13 |
| test-w5 | -1.24 | 4 | -$2.41 |

- **Mean OOS Sharpe:** 4.05
- **Sharpe Std Dev:** 8.13
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.59 | 3 | +$1.74 |
| test-w2 | 0.46 | 2 | +$0.07 |
| test-w3 | 0.17 | 3 | +$0.62 |
| test-w4 | 20.24 | 2 | +$2.13 |
| test-w5 | -1.24 | 4 | -$2.41 |

- **Mean OOS Sharpe:** 4.05
- **Sharpe Std Dev:** 8.13
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.59 | 3 | +$1.74 |
| test-w2 | 0.46 | 2 | +$0.07 |
| test-w3 | 0.17 | 3 | +$0.62 |
| test-w4 | 20.24 | 2 | +$2.13 |
| test-w5 | -1.24 | 4 | -$2.41 |

- **Mean OOS Sharpe:** 4.05
- **Sharpe Std Dev:** 8.13
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

#### Cost Mode Comparison (cluster-007:BTC)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 2.74 | 4.05 | 1.30 |
| OOS Net PnL | +$0.22 | +$2.16 | +$1.94 |
| OOS Trades | 14 | 14 | 0 |
| Fee/Gross Ratio | 0.84 | 0.32 | -0.52 |
| Max Drawdown | $2.95 | $2.53 | $-0.42 |

### cluster-005:ETH

#### flash-only

Total combinations tested: 15625
Profitable combinations: 3000

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 1.20 | +$6.51 | 17 | 64.7% | 20,001 | 30.09 | $2.03 | ⚠️ YES |
| 2 | 1.20 | +$6.51 | 17 | 64.7% | 20,001 | 30.09 | $2.03 | ⚠️ YES |
| 3 | 1.20 | +$6.51 | 17 | 64.7% | 20,001 | 30.09 | $2.03 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.33235, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.33236, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.33237, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.31 | 3 | -$1.62 |
| test-w2 | 5.58 | 3 | +$3.62 |
| test-w3 | 0.20 | 3 | +$1.07 |
| test-w4 | 0.29 | 4 | +$2.05 |
| test-w5 | 0.23 | 4 | +$1.40 |

- **Mean OOS Sharpe:** 1.20
- **Sharpe Std Dev:** 2.20
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.31 | 3 | -$1.62 |
| test-w2 | 5.58 | 3 | +$3.62 |
| test-w3 | 0.20 | 3 | +$1.07 |
| test-w4 | 0.29 | 4 | +$2.05 |
| test-w5 | 0.23 | 4 | +$1.40 |

- **Mean OOS Sharpe:** 1.20
- **Sharpe Std Dev:** 2.20
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.31 | 3 | -$1.62 |
| test-w2 | 5.58 | 3 | +$3.62 |
| test-w3 | 0.20 | 3 | +$1.07 |
| test-w4 | 0.29 | 4 | +$2.05 |
| test-w5 | 0.23 | 4 | +$1.40 |

- **Mean OOS Sharpe:** 1.20
- **Sharpe Std Dev:** 2.20
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 9500

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 2.17 | +$14.46 | 17 | 70.6% | 20,003 | 2.35 | $1.19 | ⚠️ YES |
| 2 | 2.17 | +$14.46 | 17 | 70.6% | 20,003 | 2.35 | $1.19 | ⚠️ YES |
| 3 | 2.17 | +$14.46 | 17 | 70.6% | 20,003 | 2.35 | $1.19 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.4886, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.48861, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.48862, `secs`=43200.0 ⚠️ *Only 17 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.04 | 3 | -$0.16 |
| test-w2 | 9.01 | 3 | +$4.53 |
| test-w3 | 0.56 | 3 | +$2.55 |
| test-w4 | 0.67 | 4 | +$4.23 |
| test-w5 | 0.63 | 4 | +$3.32 |

- **Mean OOS Sharpe:** 2.17
- **Sharpe Std Dev:** 3.43
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.04 | 3 | -$0.16 |
| test-w2 | 9.01 | 3 | +$4.53 |
| test-w3 | 0.56 | 3 | +$2.55 |
| test-w4 | 0.67 | 4 | +$4.23 |
| test-w5 | 0.63 | 4 | +$3.32 |

- **Mean OOS Sharpe:** 2.17
- **Sharpe Std Dev:** 3.43
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.04 | 3 | -$0.16 |
| test-w2 | 9.01 | 3 | +$4.53 |
| test-w3 | 0.56 | 3 | +$2.55 |
| test-w4 | 0.67 | 4 | +$4.23 |
| test-w5 | 0.63 | 4 | +$3.32 |

- **Mean OOS Sharpe:** 2.17
- **Sharpe Std Dev:** 3.43
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 17

#### Cost Mode Comparison (cluster-005:ETH)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 1.20 | 2.17 | 0.97 |
| OOS Net PnL | +$6.51 | +$14.46 | +$7.95 |
| OOS Trades | 17 | 17 | 0 |
| Fee/Gross Ratio | 30.09 | 2.35 | -27.74 |
| Max Drawdown | $2.03 | $1.19 | $-0.84 |

### cluster-005:SOL

#### flash-only

Total combinations tested: 15625
Profitable combinations: 3375

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 2.18 | +$4.57 | 19 | 78.9% | 40,000 | 2.48 | $4.44 | ⚠️ YES |
| 2 | 2.18 | +$4.57 | 19 | 78.9% | 40,000 | 2.48 | $4.44 | ⚠️ YES |
| 3 | 2.18 | +$4.57 | 19 | 78.9% | 40,000 | 2.48 | $4.44 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.6511, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.65111, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.65112, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.07 | 2 | +$0.25 |
| test-w2 | 7.72 | 4 | +$4.63 |
| test-w3 | 3.49 | 3 | +$5.07 |
| test-w4 | -0.24 | 6 | -$3.96 |
| test-w5 | -0.13 | 4 | -$1.41 |

- **Mean OOS Sharpe:** 2.18
- **Sharpe Std Dev:** 3.10
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.07 | 2 | +$0.25 |
| test-w2 | 7.72 | 4 | +$4.63 |
| test-w3 | 3.49 | 3 | +$5.07 |
| test-w4 | -0.24 | 6 | -$3.96 |
| test-w5 | -0.13 | 4 | -$1.41 |

- **Mean OOS Sharpe:** 2.18
- **Sharpe Std Dev:** 3.10
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.07 | 2 | +$0.25 |
| test-w2 | 7.72 | 4 | +$4.63 |
| test-w3 | 3.49 | 3 | +$5.07 |
| test-w4 | -0.24 | 6 | -$3.96 |
| test-w5 | -0.13 | 4 | -$1.41 |

- **Mean OOS Sharpe:** 2.18
- **Sharpe Std Dev:** 3.10
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 8625

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 2.50 | +$7.59 | 19 | 78.9% | 40,001 | 1.43 | $4.28 | ⚠️ YES |
| 2 | 2.50 | +$7.59 | 19 | 78.9% | 40,001 | 1.43 | $4.28 | ⚠️ YES |
| 3 | 2.50 | +$7.59 | 19 | 78.9% | 40,001 | 1.43 | $4.28 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.80735, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.80736, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.80737, `secs`=86400.0 ⚠️ *Only 19 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.15 | 2 | +$0.56 |
| test-w2 | 8.78 | 4 | +$5.26 |
| test-w3 | 3.82 | 3 | +$5.55 |
| test-w4 | -0.18 | 6 | -$3.01 |
| test-w5 | -0.07 | 4 | -$0.78 |

- **Mean OOS Sharpe:** 2.50
- **Sharpe Std Dev:** 3.48
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.15 | 2 | +$0.56 |
| test-w2 | 8.78 | 4 | +$5.26 |
| test-w3 | 3.82 | 3 | +$5.55 |
| test-w4 | -0.18 | 6 | -$3.01 |
| test-w5 | -0.07 | 4 | -$0.78 |

- **Mean OOS Sharpe:** 2.50
- **Sharpe Std Dev:** 3.48
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.15 | 2 | +$0.56 |
| test-w2 | 8.78 | 4 | +$5.26 |
| test-w3 | 3.82 | 3 | +$5.55 |
| test-w4 | -0.18 | 6 | -$3.01 |
| test-w5 | -0.07 | 4 | -$0.78 |

- **Mean OOS Sharpe:** 2.50
- **Sharpe Std Dev:** 3.48
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 19

#### Cost Mode Comparison (cluster-005:SOL)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 2.18 | 2.50 | 0.32 |
| OOS Net PnL | +$4.57 | +$7.59 | +$3.01 |
| OOS Trades | 19 | 19 | 0 |
| Fee/Gross Ratio | 2.48 | 1.43 | -1.05 |
| Max Drawdown | $4.44 | $4.28 | $-0.16 |

### cluster-008:BTC

#### flash-only

Total combinations tested: 15625
Profitable combinations: 525

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 2.99 | +$0.55 | 9 | 55.6% | 20,001 | 0.43 | $0.62 | ⚠️ YES |
| 2 | 2.99 | +$0.55 | 9 | 55.6% | 20,001 | 0.43 | $0.62 | ⚠️ YES |
| 3 | 2.99 | +$0.55 | 9 | 55.6% | 20,001 | 0.43 | $0.62 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.95745, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=1.9587, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=1.95995, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.66 | 2 | -$0.60 |
| test-w2 | 0.00 | 1 | -$0.05 |
| test-w3 | 0.12 | 2 | +$0.14 |
| test-w4 | 15.22 | 2 | +$0.90 |
| test-w5 | 0.25 | 2 | +$0.16 |

- **Mean OOS Sharpe:** 2.99
- **Sharpe Std Dev:** 6.12
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.66 | 2 | -$0.60 |
| test-w2 | 0.00 | 1 | -$0.05 |
| test-w3 | 0.12 | 2 | +$0.14 |
| test-w4 | 15.22 | 2 | +$0.90 |
| test-w5 | 0.25 | 2 | +$0.16 |

- **Mean OOS Sharpe:** 2.99
- **Sharpe Std Dev:** 6.12
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.66 | 2 | -$0.60 |
| test-w2 | 0.00 | 1 | -$0.05 |
| test-w3 | 0.12 | 2 | +$0.14 |
| test-w4 | 15.22 | 2 | +$0.90 |
| test-w5 | 0.25 | 2 | +$0.16 |

- **Mean OOS Sharpe:** 2.99
- **Sharpe Std Dev:** 6.12
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 3025

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 3.98 | +$1.44 | 9 | 55.6% | 20,002 | -0.04 | $0.49 | ⚠️ YES |
| 2 | 3.98 | +$1.44 | 9 | 55.6% | 20,002 | -0.04 | $0.49 | ⚠️ YES |
| 3 | 3.98 | +$1.44 | 9 | 55.6% | 20,002 | -0.04 | $0.49 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.11137, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=1.111495, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=1.11162, `secs`=43200.0 ⚠️ *Only 9 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.51 | 2 | -$0.41 |
| test-w2 | 0.00 | 1 | -$0.02 |
| test-w3 | 0.27 | 2 | +$0.34 |
| test-w4 | 19.65 | 2 | +$1.16 |
| test-w5 | 0.51 | 2 | +$0.37 |

- **Mean OOS Sharpe:** 3.98
- **Sharpe Std Dev:** 7.84
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.51 | 2 | -$0.41 |
| test-w2 | 0.00 | 1 | -$0.02 |
| test-w3 | 0.27 | 2 | +$0.34 |
| test-w4 | 19.65 | 2 | +$1.16 |
| test-w5 | 0.51 | 2 | +$0.37 |

- **Mean OOS Sharpe:** 3.98
- **Sharpe Std Dev:** 7.84
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.51 | 2 | -$0.41 |
| test-w2 | 0.00 | 1 | -$0.02 |
| test-w3 | 0.27 | 2 | +$0.34 |
| test-w4 | 19.65 | 2 | +$1.16 |
| test-w5 | 0.51 | 2 | +$0.37 |

- **Mean OOS Sharpe:** 3.98
- **Sharpe Std Dev:** 7.84
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 9

#### Cost Mode Comparison (cluster-008:BTC)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 2.99 | 3.98 | 1.00 |
| OOS Net PnL | +$0.55 | +$1.44 | +$0.88 |
| OOS Trades | 9 | 9 | 0 |
| Fee/Gross Ratio | 0.43 | -0.04 | -0.47 |
| Max Drawdown | $0.62 | $0.49 | $-0.13 |

### cluster-002:BTC

#### flash-only

Total combinations tested: 15625
Profitable combinations: 500

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.18 | +$0.28 | 19 | 63.2% | 1.60 | 1.14 | $0.28 | ⚠️ YES |
| 2 | 0.18 | +$0.28 | 19 | 63.2% | 1.60 | 1.14 | $0.28 | ⚠️ YES |
| 3 | 0.18 | +$0.28 | 19 | 63.2% | 1.60 | 1.14 | $0.28 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.1257, `secs`=3600.0 ⚠️ *In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.125701, `secs`=3600.0 ⚠️ *In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.125702, `secs`=3600.0 ⚠️ *In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.30 | 2 | +$0.06 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.02 | 7 | +$0.02 |
| test-w4 | 0.18 | 5 | +$0.07 |
| test-w5 | 0.39 | 5 | +$0.13 |

- **Mean OOS Sharpe:** 0.18
- **Sharpe Std Dev:** 0.15
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.30 | 2 | +$0.06 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.02 | 7 | +$0.02 |
| test-w4 | 0.18 | 5 | +$0.07 |
| test-w5 | 0.39 | 5 | +$0.13 |

- **Mean OOS Sharpe:** 0.18
- **Sharpe Std Dev:** 0.15
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 19

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.30 | 2 | +$0.06 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.02 | 7 | +$0.02 |
| test-w4 | 0.18 | 5 | +$0.07 |
| test-w5 | 0.39 | 5 | +$0.13 |

- **Mean OOS Sharpe:** 0.18
- **Sharpe Std Dev:** 0.15
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 19

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 5000

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.43 | +$0.64 | 18 | 72.2% | 20,001 | 0.73 | $0.28 | ⚠️ YES |
| 2 | 0.43 | +$0.64 | 18 | 72.2% | 20,001 | 0.73 | $0.28 | ⚠️ YES |
| 3 | 0.43 | +$0.64 | 18 | 72.2% | 20,001 | 0.73 | $0.28 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.143225, `secs`=86400.0 ⚠️ *In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.143226, `secs`=86400.0 ⚠️ *In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.143227, `secs`=86400.0 ⚠️ *In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.53 | 2 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.33 | 7 | +$0.30 |
| test-w4 | 0.30 | 5 | +$0.16 |
| test-w5 | 0.00 | 4 | +$0.00 |

- **Mean OOS Sharpe:** 0.43
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 18

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.53 | 2 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.33 | 7 | +$0.30 |
| test-w4 | 0.30 | 5 | +$0.16 |
| test-w5 | 0.00 | 4 | +$0.00 |

- **Mean OOS Sharpe:** 0.43
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 18

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.53 | 2 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.33 | 7 | +$0.30 |
| test-w4 | 0.30 | 5 | +$0.16 |
| test-w5 | 0.00 | 4 | +$0.00 |

- **Mean OOS Sharpe:** 0.43
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 18

#### Cost Mode Comparison (cluster-002:BTC)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 0.18 | 0.43 | 0.25 |
| OOS Net PnL | +$0.28 | +$0.64 | +$0.36 |
| OOS Trades | 19 | 18 | -1 |
| Fee/Gross Ratio | 1.14 | 0.73 | -0.42 |
| Max Drawdown | $0.28 | $0.28 | $0.00 |

### cluster-002:SOL

#### flash-only

Total combinations tested: 15625
Profitable combinations: 10500

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |
| 2 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |
| 3 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.15823, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.158231, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.158232, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 10500

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |
| 2 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |
| 3 | 1.08 | +$1.15 | 14 | 85.7% | 60,000 | 0.50 | $0.47 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.173855, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.173856, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.173857, `secs`=43200.0 ⚠️ *Only 14 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.17 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 1.77 | 3 | +$0.28 |
| test-w4 | 0.19 | 8 | +$0.35 |
| test-w5 | 3.43 | 2 | +$0.35 |

- **Mean OOS Sharpe:** 1.08
- **Sharpe Std Dev:** 1.35
- **PnL Consistency:** 80.0% (4/5 windows positive)
- **Total OOS Trades:** 14

#### Cost Mode Comparison (cluster-002:SOL)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 1.08 | 1.08 | 0.00 |
| OOS Net PnL | +$1.15 | +$1.15 | +$0.00 |
| OOS Trades | 14 | 14 | 0 |
| Fee/Gross Ratio | 0.50 | 0.50 | 0.00 |
| Max Drawdown | $0.47 | $0.47 | $0.00 |

### cluster-003:BTC

#### flash-only

Total combinations tested: 28448
Profitable combinations: 720

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.22 | +$0.08 | 11 | 72.7% | 40,001 | 0.58 | $0.85 | ⚠️ YES |
| 2 | 0.22 | +$0.08 | 11 | 72.7% | 40,001 | 0.58 | $0.85 | ⚠️ YES |
| 3 | 0.22 | +$0.08 | 11 | 72.7% | 40,001 | 0.58 | $0.85 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.19011, `secs`=86400.0 ⚠️ *In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.190113, `secs`=86400.0 ⚠️ *In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.190114, `secs`=86400.0 ⚠️ *In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.18 | 3 | +$0.61 |
| test-w2 | 0.00 | 1 | +$0.16 |
| test-w3 | 0.38 | 3 | +$0.22 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.45 | 4 | -$0.91 |

- **Mean OOS Sharpe:** 0.22
- **Sharpe Std Dev:** 0.55
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.18 | 3 | +$0.61 |
| test-w2 | 0.00 | 1 | +$0.16 |
| test-w3 | 0.38 | 3 | +$0.22 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.45 | 4 | -$0.91 |

- **Mean OOS Sharpe:** 0.22
- **Sharpe Std Dev:** 0.55
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.18 | 3 | +$0.61 |
| test-w2 | 0.00 | 1 | +$0.16 |
| test-w3 | 0.38 | 3 | +$0.22 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.45 | 4 | -$0.91 |

- **Mean OOS Sharpe:** 0.22
- **Sharpe Std Dev:** 0.55
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 1225

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.37 | +$0.28 | 11 | 81.8% | 40,001 | 0.46 | $0.81 | ⚠️ YES |
| 2 | 0.37 | +$0.28 | 11 | 81.8% | 40,001 | 0.46 | $0.81 | ⚠️ YES |
| 3 | 0.37 | +$0.28 | 11 | 81.8% | 40,001 | 0.46 | $0.81 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.1823, `secs`=86400.0 ⚠️ *In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.18231, `secs`=86400.0 ⚠️ *In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.18232, `secs`=86400.0 ⚠️ *In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.73 | 4 | +$0.45 |
| test-w2 | 0.00 | 1 | +$0.12 |
| test-w3 | 0.37 | 3 | +$0.17 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.28 | 3 | -$0.47 |

- **Mean OOS Sharpe:** 0.37
- **Sharpe Std Dev:** 0.71
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.73 | 4 | +$0.45 |
| test-w2 | 0.00 | 1 | +$0.12 |
| test-w3 | 0.37 | 3 | +$0.17 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.28 | 3 | -$0.47 |

- **Mean OOS Sharpe:** 0.37
- **Sharpe Std Dev:** 0.71
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 1.73 | 4 | +$0.45 |
| test-w2 | 0.00 | 1 | +$0.12 |
| test-w3 | 0.37 | 3 | +$0.17 |
| test-w4 | 0.00 | 0 | +$0.00 |
| test-w5 | -0.28 | 3 | -$0.47 |

- **Mean OOS Sharpe:** 0.37
- **Sharpe Std Dev:** 0.71
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 11

#### Cost Mode Comparison (cluster-003:BTC)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 0.22 | 0.37 | 0.14 |
| OOS Net PnL | +$0.08 | +$0.28 | +$0.20 |
| OOS Trades | 11 | 11 | 0 |
| Fee/Gross Ratio | 0.58 | 0.46 | -0.12 |
| Max Drawdown | $0.85 | $0.81 | $-0.03 |

### cluster-009:ETH

#### flash-only

Total combinations tested: 15625
Profitable combinations: 9100

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 20.57 | +$3.05 | 5 | 80.0% | 40,000 | 0.26 | $0.64 | ⚠️ YES |
| 2 | 20.57 | +$3.05 | 5 | 80.0% | 40,000 | 0.26 | $0.64 | ⚠️ YES |
| 3 | 20.57 | +$3.05 | 5 | 80.0% | 40,000 | 0.26 | $0.64 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.2595, `secs`=86400.0 ⚠️ *Only 5 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.272, `secs`=86400.0 ⚠️ *Only 5 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.2845, `secs`=86400.0 ⚠️ *Only 5 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.18 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.21 | 2 | +$0.56 |
| test-w4 | 102.62 | 2 | +$2.31 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 20.57
- **Sharpe Std Dev:** 41.03
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 5

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.18 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.21 | 2 | +$0.56 |
| test-w4 | 102.62 | 2 | +$2.31 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 20.57
- **Sharpe Std Dev:** 41.03
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 5

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 1 | +$0.18 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.21 | 2 | +$0.56 |
| test-w4 | 102.62 | 2 | +$2.31 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 20.57
- **Sharpe Std Dev:** 41.03
- **PnL Consistency:** 60.0% (3/5 windows positive)
- **Total OOS Trades:** 5

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 15625

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 18.10 | +$3.24 | 6 | 66.7% | 20,001 | 9.82 | $0.58 | ⚠️ YES |
| 2 | 18.10 | +$3.24 | 6 | 66.7% | 20,001 | 9.82 | $0.58 | ⚠️ YES |
| 3 | 18.10 | +$3.24 | 6 | 66.7% | 20,001 | 9.82 | $0.58 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.17595, `secs`=43200.0 ⚠️ *Only 6 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.1772, `secs`=43200.0 ⚠️ *Only 6 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.17845, `secs`=43200.0 ⚠️ *Only 6 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.07 | 2 | -$0.03 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.27 | 2 | +$0.70 |
| test-w4 | 90.32 | 2 | +$2.57 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 18.10
- **Sharpe Std Dev:** 36.11
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 6

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.07 | 2 | -$0.03 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.27 | 2 | +$0.70 |
| test-w4 | 90.32 | 2 | +$2.57 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 18.10
- **Sharpe Std Dev:** 36.11
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 6

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | -0.07 | 2 | -$0.03 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.27 | 2 | +$0.70 |
| test-w4 | 90.32 | 2 | +$2.57 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 18.10
- **Sharpe Std Dev:** 36.11
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 6

#### Cost Mode Comparison (cluster-009:ETH)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 20.57 | 18.10 | -2.46 |
| OOS Net PnL | +$3.05 | +$3.24 | +$0.19 |
| OOS Trades | 5 | 6 | 1 |
| Fee/Gross Ratio | 0.26 | 9.82 | 9.56 |
| Max Drawdown | $0.64 | $0.58 | $-0.06 |

### cluster-009:SOL

#### flash-only

Total combinations tested: 15625
Profitable combinations: 4300

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.28 | +$0.44 | 7 | 100.0% | 40,000 | 0.43 | $0.00 | ⚠️ YES |
| 2 | 0.28 | +$0.44 | 7 | 100.0% | 40,000 | 0.43 | $0.00 | ⚠️ YES |
| 3 | 0.28 | +$0.44 | 7 | 100.0% | 40,000 | 0.43 | $0.00 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.33225, `secs`=43200.0 ⚠️ *In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.33226, `secs`=43200.0 ⚠️ *In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.33227, `secs`=43200.0 ⚠️ *In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.04 |
| test-w4 | 1.42 | 6 | +$0.39 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.28
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.04 |
| test-w4 | 1.42 | 6 | +$0.39 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.28
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.04 |
| test-w4 | 1.42 | 6 | +$0.39 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.28
- **Sharpe Std Dev:** 0.57
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

#### imperial-route-oracle

Total combinations tested: 15625
Profitable combinations: 6900

**Top 3 Parameter Sets (by out-of-sample Sharpe)**

| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |
|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|
| 1 | 0.44 | +$0.69 | 7 | 100.0% | 40,000 | 0.20 | $0.00 | ⚠️ YES |
| 2 | 0.44 | +$0.69 | 7 | 100.0% | 40,000 | 0.20 | $0.00 | ⚠️ YES |
| 3 | 0.44 | +$0.69 | 7 | 100.0% | 40,000 | 0.20 | $0.00 | ⚠️ YES |

**Parameter Values**

  1. `count`=15.0, `pct`=0.4885, `secs`=43200.0 ⚠️ *In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)*
  2. `count`=15.0, `pct`=0.48851, `secs`=43200.0 ⚠️ *In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)*
  3. `count`=15.0, `pct`=0.48852, `secs`=43200.0 ⚠️ *In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)*

**Per-Window Stability (Rank #1)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.08 |
| test-w4 | 2.21 | 6 | +$0.62 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.44
- **Sharpe Std Dev:** 0.88
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

**Per-Window Stability (Rank #2)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.08 |
| test-w4 | 2.21 | 6 | +$0.62 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.44
- **Sharpe Std Dev:** 0.88
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

**Per-Window Stability (Rank #3)**

| Window | Sharpe | Trades | Net PnL |
|--------|--------|--------|---------|
| test-w1 | 0.00 | 0 | +$0.00 |
| test-w2 | 0.00 | 0 | +$0.00 |
| test-w3 | 0.00 | 1 | +$0.08 |
| test-w4 | 2.21 | 6 | +$0.62 |
| test-w5 | 0.00 | 0 | +$0.00 |

- **Mean OOS Sharpe:** 0.44
- **Sharpe Std Dev:** 0.88
- **PnL Consistency:** 40.0% (2/5 windows positive)
- **Total OOS Trades:** 7

#### Cost Mode Comparison (cluster-009:SOL)

| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |
|--------|-------------------|------------------------------|-------|
| OOS Sharpe | 0.28 | 0.44 | 0.16 |
| OOS Net PnL | +$0.44 | +$0.69 | +$0.26 |
| OOS Trades | 7 | 7 | 0 |
| Fee/Gross Ratio | 0.43 | 0.20 | -0.23 |
| Max Drawdown | $0.00 | $0.00 | $0.00 |

## Overfit Analysis

Parameter sets are flagged as potentially overfit when:
1. In-sample Sharpe > 2× out-of-sample Sharpe
2. Positive PnL in only 1 out of 5 walk-forward windows
3. Fewer than 30 out-of-sample trades (insufficient sample)

**Summary:** 54/54 top-3 parameter sets flagged as potentially overfit.

### Flagged Parameter Sets

- **cluster-007:BTC (flash-only) rank #1**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.197, `secs`=43200.0
  - OOS Sharpe: 2.74, OOS Trades: 14

- **cluster-007:BTC (flash-only) rank #2**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.2095, `secs`=43200.0
  - OOS Sharpe: 2.74, OOS Trades: 14

- **cluster-007:BTC (flash-only) rank #3**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.222, `secs`=43200.0
  - OOS Sharpe: 2.74, OOS Trades: 14

- **cluster-007:BTC (imperial-route-oracle) rank #1**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.17595, `secs`=43200.0
  - OOS Sharpe: 4.05, OOS Trades: 14

- **cluster-007:BTC (imperial-route-oracle) rank #2**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.1772, `secs`=43200.0
  - OOS Sharpe: 4.05, OOS Trades: 14

- **cluster-007:BTC (imperial-route-oracle) rank #3**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.17845, `secs`=43200.0
  - OOS Sharpe: 4.05, OOS Trades: 14

- **cluster-005:ETH (flash-only) rank #1**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33235, `secs`=43200.0
  - OOS Sharpe: 1.20, OOS Trades: 17

- **cluster-005:ETH (flash-only) rank #2**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33236, `secs`=43200.0
  - OOS Sharpe: 1.20, OOS Trades: 17

- **cluster-005:ETH (flash-only) rank #3**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33237, `secs`=43200.0
  - OOS Sharpe: 1.20, OOS Trades: 17

- **cluster-005:ETH (imperial-route-oracle) rank #1**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.4886, `secs`=43200.0
  - OOS Sharpe: 2.17, OOS Trades: 17

- **cluster-005:ETH (imperial-route-oracle) rank #2**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.48861, `secs`=43200.0
  - OOS Sharpe: 2.17, OOS Trades: 17

- **cluster-005:ETH (imperial-route-oracle) rank #3**: Only 17 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.48862, `secs`=43200.0
  - OOS Sharpe: 2.17, OOS Trades: 17

- **cluster-005:SOL (flash-only) rank #1**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.6511, `secs`=86400.0
  - OOS Sharpe: 2.18, OOS Trades: 19

- **cluster-005:SOL (flash-only) rank #2**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.65111, `secs`=86400.0
  - OOS Sharpe: 2.18, OOS Trades: 19

- **cluster-005:SOL (flash-only) rank #3**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.65112, `secs`=86400.0
  - OOS Sharpe: 2.18, OOS Trades: 19

- **cluster-005:SOL (imperial-route-oracle) rank #1**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.80735, `secs`=86400.0
  - OOS Sharpe: 2.50, OOS Trades: 19

- **cluster-005:SOL (imperial-route-oracle) rank #2**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.80736, `secs`=86400.0
  - OOS Sharpe: 2.50, OOS Trades: 19

- **cluster-005:SOL (imperial-route-oracle) rank #3**: Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.80737, `secs`=86400.0
  - OOS Sharpe: 2.50, OOS Trades: 19

- **cluster-008:BTC (flash-only) rank #1**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.95745, `secs`=43200.0
  - OOS Sharpe: 2.99, OOS Trades: 9

- **cluster-008:BTC (flash-only) rank #2**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=1.9587, `secs`=43200.0
  - OOS Sharpe: 2.99, OOS Trades: 9

- **cluster-008:BTC (flash-only) rank #3**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=1.95995, `secs`=43200.0
  - OOS Sharpe: 2.99, OOS Trades: 9

- **cluster-008:BTC (imperial-route-oracle) rank #1**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.11137, `secs`=43200.0
  - OOS Sharpe: 3.98, OOS Trades: 9

- **cluster-008:BTC (imperial-route-oracle) rank #2**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=1.111495, `secs`=43200.0
  - OOS Sharpe: 3.98, OOS Trades: 9

- **cluster-008:BTC (imperial-route-oracle) rank #3**: Only 9 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=1.11162, `secs`=43200.0
  - OOS Sharpe: 3.98, OOS Trades: 9

- **cluster-002:BTC (flash-only) rank #1**: In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.1257, `secs`=3600.0
  - OOS Sharpe: 0.18, OOS Trades: 19

- **cluster-002:BTC (flash-only) rank #2**: In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.125701, `secs`=3600.0
  - OOS Sharpe: 0.18, OOS Trades: 19

- **cluster-002:BTC (flash-only) rank #3**: In-sample Sharpe (0.74) > 2x OOS (0.18); Only 19 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.125702, `secs`=3600.0
  - OOS Sharpe: 0.18, OOS Trades: 19

- **cluster-002:BTC (imperial-route-oracle) rank #1**: In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.143225, `secs`=86400.0
  - OOS Sharpe: 0.43, OOS Trades: 18

- **cluster-002:BTC (imperial-route-oracle) rank #2**: In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.143226, `secs`=86400.0
  - OOS Sharpe: 0.43, OOS Trades: 18

- **cluster-002:BTC (imperial-route-oracle) rank #3**: In-sample Sharpe (1.15) > 2x OOS (0.43); Only 18 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.143227, `secs`=86400.0
  - OOS Sharpe: 0.43, OOS Trades: 18

- **cluster-002:SOL (flash-only) rank #1**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.15823, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-002:SOL (flash-only) rank #2**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.158231, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-002:SOL (flash-only) rank #3**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.158232, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-002:SOL (imperial-route-oracle) rank #1**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.173855, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-002:SOL (imperial-route-oracle) rank #2**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.173856, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-002:SOL (imperial-route-oracle) rank #3**: Only 14 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.173857, `secs`=43200.0
  - OOS Sharpe: 1.08, OOS Trades: 14

- **cluster-003:BTC (flash-only) rank #1**: In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.19011, `secs`=86400.0
  - OOS Sharpe: 0.22, OOS Trades: 11

- **cluster-003:BTC (flash-only) rank #2**: In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.190113, `secs`=86400.0
  - OOS Sharpe: 0.22, OOS Trades: 11

- **cluster-003:BTC (flash-only) rank #3**: In-sample Sharpe (3.37) > 2x OOS (0.22); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.190114, `secs`=86400.0
  - OOS Sharpe: 0.22, OOS Trades: 11

- **cluster-003:BTC (imperial-route-oracle) rank #1**: In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.1823, `secs`=86400.0
  - OOS Sharpe: 0.37, OOS Trades: 11

- **cluster-003:BTC (imperial-route-oracle) rank #2**: In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.18231, `secs`=86400.0
  - OOS Sharpe: 0.37, OOS Trades: 11

- **cluster-003:BTC (imperial-route-oracle) rank #3**: In-sample Sharpe (2.70) > 2x OOS (0.37); Only 11 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.18232, `secs`=86400.0
  - OOS Sharpe: 0.37, OOS Trades: 11

- **cluster-009:ETH (flash-only) rank #1**: Only 5 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.2595, `secs`=86400.0
  - OOS Sharpe: 20.57, OOS Trades: 5

- **cluster-009:ETH (flash-only) rank #2**: Only 5 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.272, `secs`=86400.0
  - OOS Sharpe: 20.57, OOS Trades: 5

- **cluster-009:ETH (flash-only) rank #3**: Only 5 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.2845, `secs`=86400.0
  - OOS Sharpe: 20.57, OOS Trades: 5

- **cluster-009:ETH (imperial-route-oracle) rank #1**: Only 6 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.17595, `secs`=43200.0
  - OOS Sharpe: 18.10, OOS Trades: 6

- **cluster-009:ETH (imperial-route-oracle) rank #2**: Only 6 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.1772, `secs`=43200.0
  - OOS Sharpe: 18.10, OOS Trades: 6

- **cluster-009:ETH (imperial-route-oracle) rank #3**: Only 6 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.17845, `secs`=43200.0
  - OOS Sharpe: 18.10, OOS Trades: 6

- **cluster-009:SOL (flash-only) rank #1**: In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33225, `secs`=43200.0
  - OOS Sharpe: 0.28, OOS Trades: 7

- **cluster-009:SOL (flash-only) rank #2**: In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33226, `secs`=43200.0
  - OOS Sharpe: 0.28, OOS Trades: 7

- **cluster-009:SOL (flash-only) rank #3**: In-sample Sharpe (2.29) > 2x OOS (0.28); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.33227, `secs`=43200.0
  - OOS Sharpe: 0.28, OOS Trades: 7

- **cluster-009:SOL (imperial-route-oracle) rank #1**: In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.4885, `secs`=43200.0
  - OOS Sharpe: 0.44, OOS Trades: 7

- **cluster-009:SOL (imperial-route-oracle) rank #2**: In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.48851, `secs`=43200.0
  - OOS Sharpe: 0.44, OOS Trades: 7

- **cluster-009:SOL (imperial-route-oracle) rank #3**: In-sample Sharpe (6.08) > 2x OOS (0.44); Only 7 OOS trades (insufficient sample, need ≥30)
  - Params: `count`=15.0, `pct`=0.48852, `secs`=43200.0
  - OOS Sharpe: 0.44, OOS Trades: 7

## Parameter Stability Analysis

For promoted candidates, < 30% Sharpe degradation when each parameter moves one grid step.

## Conclusions

### Promotion Assessment

Each candidate is evaluated against the 6 promotion gate criteria for both cost modes. 
A candidate must pass all criteria in at least one cost mode to be promoted.

- **cluster-007:BTC**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (14 trades, need ≥30); fee/gross 0.84 > 35%; overfit flag
  - imperial-route-oracle: **insufficient sample** (14 trades, need ≥30); overfit flag

- **cluster-005:ETH**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (17 trades, need ≥30); fee/gross 30.09 > 35%; overfit flag
  - imperial-route-oracle: **insufficient sample** (17 trades, need ≥30); fee/gross 2.35 > 35%; overfit flag

- **cluster-005:SOL**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (19 trades, need ≥30); fee/gross 2.48 > 35%; overfit flag
  - imperial-route-oracle: **insufficient sample** (19 trades, need ≥30); fee/gross 1.43 > 35%; overfit flag

- **cluster-008:BTC**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (9 trades, need ≥30); fee/gross 0.43 > 35%; overfit flag
  - imperial-route-oracle: **insufficient sample** (9 trades, need ≥30); overfit flag

- **cluster-002:BTC**: ❌ Not yet promotable
  - flash-only: Sharpe 0.18 < 1.0; **insufficient sample** (19 trades, need ≥30); fee/gross 1.14 > 35%; overfit flag
  - imperial-route-oracle: Sharpe 0.43 < 1.0; **insufficient sample** (18 trades, need ≥30); fee/gross 0.73 > 35%; overfit flag

- **cluster-002:SOL**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (14 trades, need ≥30); fee/gross 0.50 > 35%; overfit flag
  - imperial-route-oracle: **insufficient sample** (14 trades, need ≥30); fee/gross 0.50 > 35%; overfit flag

- **cluster-003:BTC**: ❌ Not yet promotable
  - flash-only: Sharpe 0.22 < 1.0; **insufficient sample** (11 trades, need ≥30); fee/gross 0.58 > 35%; overfit flag
  - imperial-route-oracle: Sharpe 0.37 < 1.0; **insufficient sample** (11 trades, need ≥30); fee/gross 0.46 > 35%; overfit flag

- **cluster-009:ETH**: ❌ Not yet promotable
  - flash-only: **insufficient sample** (5 trades, need ≥30); overfit flag
  - imperial-route-oracle: **insufficient sample** (6 trades, need ≥30); fee/gross 9.82 > 35%; overfit flag

- **cluster-009:SOL**: ❌ Not yet promotable
  - flash-only: Sharpe 0.28 < 1.0; **insufficient sample** (7 trades, need ≥30); fee/gross 0.43 > 35%; overfit flag
  - imperial-route-oracle: Sharpe 0.44 < 1.0; **insufficient sample** (7 trades, need ≥30); overfit flag

**Promoted:** 0/9 candidates

## Sufficient Sample Analysis (≥30 Trades)

Since all top-ranked entries by OOS Sharpe have <30 trades (favoring 
low-trade-count lucky streaks), this section shows the best OOS Sharpe 
among entries that meet the minimum 30-trade threshold.

| Candidate | Cost Mode | Entries ≥30 Trades | Best Sharpe (≥30) | Best PnL (≥30) | Max Trades |
|-----------|-----------|--------------------|--------------------|----------------|------------|
| cluster-007:BTC | flash-only | 12925 | 0.11 | -$1.07 | 56 |
| cluster-007:BTC | imperial-route-oracle | 12925 | 0.74 | +$1.85 | 56 |
| cluster-005:ETH | flash-only | 7000 | 0.36 | +$3.04 | 39 |
| cluster-005:ETH | imperial-route-oracle | 7000 | 1.36 | +$13.08 | 39 |
| cluster-005:SOL | flash-only | 9250 | 0.63 | +$0.45 | 44 |
| cluster-005:SOL | imperial-route-oracle | 9250 | 0.83 | +$6.01 | 44 |
| cluster-008:BTC | flash-only | 4625 | -0.03 | -$0.78 | 34 |
| cluster-008:BTC | imperial-route-oracle | 4625 | 2.50 | +$0.54 | 34 |
| cluster-002:BTC | flash-only | 0 | N/A | N/A | 0 |
| cluster-002:BTC | imperial-route-oracle | 0 | N/A | N/A | 0 |
| cluster-002:SOL | flash-only | 0 | N/A | N/A | 0 |
| cluster-002:SOL | imperial-route-oracle | 0 | N/A | N/A | 0 |
| cluster-003:BTC | flash-only | 0 | N/A | N/A | 0 |
| cluster-003:BTC | imperial-route-oracle | 0 | N/A | N/A | 0 |
| cluster-009:ETH | flash-only | 0 | N/A | N/A | 0 |
| cluster-009:ETH | imperial-route-oracle | 0 | N/A | N/A | 0 |
| cluster-009:SOL | flash-only | 0 | N/A | N/A | 0 |
| cluster-009:SOL | imperial-route-oracle | 0 | N/A | N/A | 0 |

### Promising Candidates with Sufficient Sample

These candidates have OOS Sharpe ≥ 1.0 with ≥30 trades:

- **cluster-005:ETH** (imperial-route-oracle):
  - OOS Sharpe: 1.36
  - OOS Trades: 33
  - OOS PnL: +$13.08
  - Fee/Gross: 0.11
  - Max DD: $0.00
  - Params: `count`=15.0, `pct`=0.49475, `secs`=86400.0

- **cluster-008:BTC** (imperial-route-oracle):
  - OOS Sharpe: 2.50
  - OOS Trades: 31
  - OOS PnL: +$0.54
  - Fee/Gross: 0.22
  - Max DD: $0.92
  - Params: `count`=15.0, `pct`=0.111975, `secs`=86400.0
