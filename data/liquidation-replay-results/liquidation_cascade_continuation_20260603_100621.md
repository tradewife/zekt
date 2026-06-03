# liquidation-cascade-continuation — Replay Promotion Report

## Summary

- **Strategy:** liquidation-cascade-continuation
- **Verdict:** Denied
- **Data Points Replayed:** 4848
- **Starting Balance:** $1000.00
- **Final Balance:** $-2521.73
- **Net PnL:** $-3521.08
- **Baseline PnL:** $0.00 (no-trade)
- **PnL vs Baseline:** $-3521.08

## Performance Metrics

| Metric | Value |
|---|---|
| Trades | 5 |
| Wins / Losses | 2 / 3 |
| Win Rate | 40.0% |
| Gross PnL | $-3519.93 |
| Total Fees | $1.15 |
| Net PnL | $-3521.08 |
| Sharpe Ratio | -0.4412 |
| Sortino Ratio | -0.6238 |
| Calmar Ratio | -0.0098 |
| Max Drawdown | $93206.13 (9320.61%) |
| Net Expectancy | $-704.2462 |
| Avg MAE | $0.0000 |
| Avg MFE | $0.0000 |
| Avg Stop Efficiency | 0.0000 |
| Fishing Fill Rate | 100.00% |
| Zone-Touch Win Rate | 40.0% (2 / 5) |
| Avg Post-Liq Drift | $0.0000 |
| Avg Time-to-Reversal | 0.0s |
| Avg Time-to-Next-Zone | 0.0s |
| Single-Trade Dependency | ✅ OK |
| Avg Hold Time | 37.0s |
| Stale Trades | 0 |
| Duplicate Pendings | 0 |
| Signal Events | 5 |

## Promotion Criteria

| Criterion | Status | Actual | Threshold |
|---|---|---|---|
| Positive net expectancy after route costs | ❌ FAIL | -704.2462 USD | > 0 USD |
| Max drawdown within policy limit | ❌ FAIL | 9320.61 pct | ≤ 10.0 pct |
| Zero stale-data trades | ✅ PASS | 0 count | = 0 count |
| Zero duplicate pending trades | ✅ PASS | 0 count | = 0 count |
| Minimum 30 signal events for statistical validity | ❌ FAIL | 5 count | ≥ 30 count |
| Sharpe ratio ≥ 1.0 threshold | ❌ FAIL | -0.4412 ratio | ≥ 1.0 ratio |
| Fee/gross ratio < 35% | ✅ PASS | 0.03 pct | < 35.0 pct |
| No single event contributes > 25% of total profit | ✅ PASS | ok pct | ≤ 25% pct |
| Fishing orders improve expectancy or reduce drawdown | ✅ PASS | 0.0500 (delta) delta | positive delta delta |
| Pyramiding improves risk-adjusted return | ❌ FAIL | -239.8920 USD unrealized USD | positive unrealized PnL USD |
| Route cost does not consume edge | ❌ FAIL | 0.00 pct | < 50.0 pct |
| Liquidation distance safe at proposed leverage | ❌ FAIL | 0.0 bps | > 3333 bps (leverage 3.0x) bps |

## Trade Log (first 20)

| # | Symbol | Side | Entry | Exit | Net PnL | Hold(s) | Exit Reason | Stale |
|---|---|---|---|---|---|---|---|---|
| 1 | BTC | long | 81.96 | 73509.50 | $89587.61 | 89 | TakeProfit |  |
| 2 | ETH | short | 73509.50 | 2000.05 | $97.05 | 0 | TakeProfit |  |
| 3 | SOL | long | 2000.05 | 81.99 | $-96.13 | 0 | ReversalDetected |  |
| 4 | BTC | short | 1999.85 | 73487.50 | $-3574.88 | 0 | ReversalDetected |  |
| 5 | BTC | short | 81.82 | 73338.50 | $-89534.73 | 96 | ReversalDetected |  |
