# liquidation-cascade-continuation — Replay Promotion Report

## Summary

- **Strategy:** liquidation-cascade-continuation
- **Verdict:** Denied
- **Data Points Replayed:** 4844
- **Starting Balance:** $1000.00
- **Final Balance:** $-2478.09
- **Net PnL:** $-3477.83
- **Baseline PnL:** $0.00 (no-trade)
- **PnL vs Baseline:** $-3477.83

## Performance Metrics

| Metric | Value |
|---|---|
| Trades | 2 |
| Wins / Losses | 1 / 1 |
| Win Rate | 50.0% |
| Gross PnL | $-3477.37 |
| Total Fees | $0.46 |
| Net PnL | $-3477.83 |
| Sharpe Ratio | -33.6201 |
| Sortino Ratio | -24.4184 |
| Calmar Ratio | -0.2530 |
| Max Drawdown | $3575.01 (357.50%) |
| Net Expectancy | $-1738.9457 |
| Avg MAE | $-0.0000 |
| Avg MFE | $0.0000 |
| Avg Stop Efficiency | 0.0000 |
| Fishing Fill Rate | 100.00% |
| Zone-Touch Win Rate | 50.0% (1 / 2) |
| Avg Post-Liq Drift | $0.0000 |
| Avg Time-to-Reversal | 0.0s |
| Avg Time-to-Next-Zone | 0.0s |
| Single-Trade Dependency | ✅ OK |
| Avg Hold Time | 0.0s |
| Stale Trades | 0 |
| Duplicate Pendings | 0 |
| Signal Events | 6 |

## Promotion Criteria

| Criterion | Status | Actual | Threshold |
|---|---|---|---|
| Positive net expectancy after route costs | ❌ FAIL | -1738.9457 USD | > 0 USD |
| Max drawdown within policy limit | ❌ FAIL | 357.50 pct | ≤ 10.0 pct |
| Zero stale-data trades | ✅ PASS | 0 count | = 0 count |
| Zero duplicate pending trades | ✅ PASS | 0 count | = 0 count |
| Minimum 30 signal events for statistical validity | ❌ FAIL | 6 count | ≥ 30 count |
| Sharpe ratio ≥ 1.0 threshold | ❌ FAIL | -33.6201 ratio | ≥ 1.0 ratio |
| Fee/gross ratio < 35% | ✅ PASS | 0.01 pct | < 35.0 pct |
| No single event contributes > 25% of total profit | ✅ PASS | ok pct | ≤ 25% pct |
| Fishing orders improve expectancy or reduce drawdown | ✅ PASS | 0.0500 (delta) delta | positive delta delta |
| Pyramiding improves risk-adjusted return | ❌ FAIL | -239.8920 USD unrealized USD | positive unrealized PnL USD |
| Route cost does not consume edge | ❌ FAIL | 0.00 pct | < 50.0 pct |
| Liquidation distance safe at proposed leverage | ❌ FAIL | 0.0 bps | > 3333 bps (leverage 3.0x) bps |

## Trade Log (first 20)

| # | Symbol | Side | Entry | Exit | Net PnL | Hold(s) | Exit Reason | Stale |
|---|---|---|---|---|---|---|---|---|
| 1 | ETH | short | 73509.50 | 2000.05 | $97.05 | 0 | TakeProfit |  |
| 2 | BTC | short | 1999.85 | 73487.50 | $-3574.88 | 0 | ReversalDetected |  |
