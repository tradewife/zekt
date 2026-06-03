# liquidation-cascade-continuation — Replay Promotion Report

## Summary

- **Strategy:** liquidation-cascade-continuation
- **Verdict:** Denied
- **Data Points Replayed:** 4833
- **Starting Balance:** $1000.00
- **Final Balance:** $1099.53
- **Net PnL:** $99.66
- **Baseline PnL:** $0.00 (no-trade)
- **PnL vs Baseline:** $99.66

## Performance Metrics

| Metric | Value |
|---|---|
| Trades | 1 |
| Wins / Losses | 1 / 0 |
| Win Rate | 100.0% |
| Gross PnL | $99.89 |
| Total Fees | $0.23 |
| Net PnL | $99.66 |
| Sharpe Ratio | 0.0000 |
| Sortino Ratio | 0.0000 |
| Calmar Ratio | 0.0000 |
| Max Drawdown | $0.00 (0.00%) |
| Net Expectancy | $99.6289 |
| Avg MAE | $-0.0000 |
| Avg MFE | $0.0000 |
| Avg Stop Efficiency | 0.0000 |
| Fishing Fill Rate | 100.00% |
| Zone-Touch Win Rate | 100.0% (1 / 1) |
| Avg Post-Liq Drift | $0.0000 |
| Avg Time-to-Reversal | 0.0s |
| Avg Time-to-Next-Zone | 0.0s |
| Single-Trade Dependency | ⚠️ FLAGGED (>25%) |
| Avg Hold Time | 0.0s |
| Stale Trades | 0 |
| Duplicate Pendings | 0 |
| Signal Events | 1 |

## Promotion Criteria

| Criterion | Status | Actual | Threshold |
|---|---|---|---|
| Positive net expectancy after route costs | ✅ PASS | 99.6289 USD | > 0 USD |
| Max drawdown within policy limit | ✅ PASS | 0.00 pct | ≤ 10.0 pct |
| Zero stale-data trades | ✅ PASS | 0 count | = 0 count |
| Zero duplicate pending trades | ✅ PASS | 0 count | = 0 count |
| Minimum 30 signal events for statistical validity | ❌ FAIL | 1 count | ≥ 30 count |
| Sharpe ratio ≥ 1.0 threshold | ❌ FAIL | 0.0000 ratio | ≥ 1.0 ratio |
| Fee/gross ratio < 35% | ✅ PASS | 0.23 pct | < 35.0 pct |
| No single event contributes > 25% of total profit | ❌ FAIL | flagged pct | ≤ 25% pct |
| Fishing orders improve expectancy or reduce drawdown | ✅ PASS | 0.0500 (delta) delta | positive delta delta |
| Pyramiding improves risk-adjusted return | ❌ FAIL | -239.8920 USD unrealized USD | positive unrealized PnL USD |
| Route cost does not consume edge | ✅ PASS | 0.03 pct | < 50.0 pct |
| Liquidation distance safe at proposed leverage | ❌ FAIL | 0.0 bps | > 3333 bps (leverage 3.0x) bps |

## Trade Log (first 20)

| # | Symbol | Side | Entry | Exit | Net PnL | Hold(s) | Exit Reason | Stale |
|---|---|---|---|---|---|---|---|---|
| 1 | SOL | short | 71436.50 | 79.39 | $99.66 | 0 | TakeProfit |  |
