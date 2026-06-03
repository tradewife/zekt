# liquidity-memory-fisher — Replay Promotion Report

## Summary

- **Strategy:** liquidity-memory-fisher
- **Verdict:** Denied
- **Data Points Replayed:** 4848
- **Starting Balance:** $1000.00
- **Final Balance:** $855.56
- **Net PnL:** $-144.18
- **Baseline PnL:** $0.00 (no-trade)
- **PnL vs Baseline:** $-144.18

## Performance Metrics

| Metric | Value |
|---|---|
| Trades | 4 |
| Wins / Losses | 2 / 2 |
| Win Rate | 50.0% |
| Gross PnL | $-143.72 |
| Total Fees | $0.46 |
| Net PnL | $-144.18 |
| Sharpe Ratio | -0.0404 |
| Sortino Ratio | -0.0570 |
| Calmar Ratio | -0.0008 |
| Max Drawdown | $44911.31 (4491.13%) |
| Net Expectancy | $-36.0589 |
| Avg MAE | $0.0000 |
| Avg MFE | $0.0000 |
| Avg Stop Efficiency | 0.0000 |
| Fishing Fill Rate | 100.00% |
| Zone-Touch Win Rate | 50.0% (2 / 4) |
| Avg Post-Liq Drift | $0.0000 |
| Avg Time-to-Reversal | 0.0s |
| Avg Time-to-Next-Zone | 0.0s |
| Single-Trade Dependency | ✅ OK |
| Avg Hold Time | 0.0s |
| Stale Trades | 0 |
| Duplicate Pendings | 0 |
| Signal Events | 4 |

## Promotion Criteria

| Criterion | Status | Actual | Threshold |
|---|---|---|---|
| Positive net expectancy after route costs | ❌ FAIL | -36.0589 USD | > 0 USD |
| Max drawdown within policy limit | ❌ FAIL | 4491.13 pct | ≤ 10.0 pct |
| Zero stale-data trades | ✅ PASS | 0 count | = 0 count |
| Zero duplicate pending trades | ✅ PASS | 0 count | = 0 count |
| Minimum 30 signal events for statistical validity | ❌ FAIL | 4 count | ≥ 30 count |
| Sharpe ratio ≥ 1.0 threshold | ❌ FAIL | -0.0404 ratio | ≥ 1.0 ratio |
| Fee/gross ratio < 35% | ✅ PASS | 0.32 pct | < 35.0 pct |
| No single event contributes > 25% of total profit | ✅ PASS | ok pct | ≤ 25% pct |
| Fishing orders improve expectancy or reduce drawdown | ✅ PASS | 0.0500 (delta) delta | positive delta delta |
| Pyramiding improves risk-adjusted return | ❌ FAIL | -239.8920 USD unrealized USD | positive unrealized PnL USD |
| Route cost does not consume edge | ❌ FAIL | 0.00 pct | < 50.0 pct |
| Liquidation distance safe at proposed leverage | ❌ FAIL | 0.0 bps | > 3333 bps (leverage 3.0x) bps |

## Trade Log (first 20)

| # | Symbol | Side | Entry | Exit | Net PnL | Hold(s) | Exit Reason | Stale |
|---|---|---|---|---|---|---|---|---|
| 1 | BTC | long | 80.74 | 72292.00 | $44719.11 | 0 | TakeProfit |  |
| 2 | BTC | short | 79.74 | 71628.50 | $-44862.49 | 0 | StopLoss |  |
| 3 | ETH | long | 69462.50 | 1978.15 | $-48.69 | 0 | StopLoss |  |
| 4 | SOL | short | 1834.95 | 73.01 | $47.90 | 0 | TakeProfit |  |
