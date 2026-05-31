# Liquidation Event Replay Report

**Generated:** 2026-05-31T15:08:44.424205+00:00
**Assertion:** VAL-REPORTS-004

## Replay Parameters

| Parameter | Value |
|-----------|-------|
| Strategy | liquidation-zone-arbiter |
| Starting Balance | $1,000.00 |
| Fee Rate | 0.1% per side |
| Route Cost | 3.0 bps |
| Proposed Leverage | 3.0x |
| Pyramid Variant | reclaim |
| SL | 1.5% | TP | 3.0% |

## Trade Summary

| Metric | Value |
|--------|-------|
| Total Trades | 15 |
| Winning Trades | 7 |
| Losing Trades | 8 |
| Win Rate | 46.67% |
| Gross PnL | $32.25 |
| Total Fees | $25.88 |
| Net PnL | $6.37 |
| Final Balance | $1006.37 |
| Fee/Gross Ratio | 80.24% |
| Avg Hold Time | 270s |

## Extended Metrics

| Metric | Value | Description |
|--------|-------|-------------|
| Sharpe Ratio | 0.0290 | Mean return / std deviation |
| Sortino Ratio | 0.1234 | Mean return / downside deviation |
| Calmar Ratio | 170.0199 | Annualized return / max drawdown |
| Max Drawdown | $43.79 (4.38%) | Worst peak-to-trough |
| Avg MAE | $8.4435 | Maximum adverse excursion per trade |
| Avg MFE | $10.1354 | Maximum favorable excursion per trade |
| Fishing Fill Rate | 0.0000 | Filled fishing orders / total orders |
| Zone-Touch Win Rate | 46.67% (7/15) | Win rate at zone-touch events |
| Avg Stop Efficiency | 0.3632 | Actual PnL / MFE |
| Single-Trade Dependency | ⚠️ Flagged | >25% of profit from one trade |
| Net Expectancy | $0.4248 | Expected value per trade |

## Promotion Gate Verdict

### **Verdict: Denied** (6/12 criteria passed)

| # | Criterion | Threshold | Actual | Passed |
|---|-----------|-----------|--------|--------|
| 1 | Positive net expectancy after fees | > $0.00 | $0.4248 | ✅ |
| 2 | Max drawdown ≤ 10.0% | ≤ 10.0% | 4.38% | ✅ |
| 3 | Zero stale-data trades | = 0 | 0 | ✅ |
| 4 | Zero duplicate pending trades | = 0 | 0 | ✅ |
| 5 | ≥ 30 qualified replay events | ≥ 30 | 15 | ❌ |
| 6 | Sharpe ratio ≥ 1.0 | ≥ 1.0 | 0.0290 | ❌ |
| 7 | Fee-to-gross ratio < 35.0% | < 35.0% | 80.24% | ❌ |
| 8 | No single event contributes > 25.0% of profit | < 25.0% | 442.70% | ❌ |
| 9 | Fishing orders improve expectancy or reduce drawdown | fishing > market | fishing=0.0000, market=0.0000 | ❌ |
| 10 | Pyramiding improves risk-adjusted return (not just gross PnL) | Positive delta | Reclaim variant Δ expectancy +$0.52 | ✅ |
| 11 | Route cost < 50.0% of expectancy | < 50.0% | 70.62% | ❌ |
| 12 | Zone distance ≥ 200.0 bps at 3.0x leverage | ≥ 200.0 bps | 5000 bps avg | ✅ |

### Failed Criteria Analysis

**min_signal_events**: ≥ 30 qualified replay events
- Required: ≥ 30
- Actual: 15

**min_sharpe**: Sharpe ratio ≥ 1.0
- Required: ≥ 1.0
- Actual: 0.0290

**fee_to_gross**: Fee-to-gross ratio < 35.0%
- Required: < 35.0%
- Actual: 80.24%

**no_single_trade_dominance**: No single event contributes > 25.0% of profit
- Required: < 25.0%
- Actual: 442.70%

**fishing_improves_expectancy**: Fishing orders improve expectancy or reduce drawdown
- Required: fishing > market
- Actual: fishing=0.0000, market=0.0000

**route_cost_within_budget**: Route cost < 50.0% of expectancy
- Required: < 50.0%
- Actual: 70.62%

## Per-Symbol Breakdown

| Symbol | Trades | Wins | Win Rate | Net PnL |
|--------|--------|------|----------|---------|
| BTC | 5 | 2 | 40.0% | $2.18 |
| ETH | 5 | 2 | 40.0% | $-18.15 |
| SOL | 5 | 3 | 60.0% | $22.34 |

## Top 5 Trades

| # | Symbol | Zone | Size | Net PnL | Hold Time |
|---|--------|------|------|---------|-----------|
| 1 | SOL | $41.28 | $750.00 | $28.21 | 78s |
| 2 | BTC | $110,654.25 | $750.00 | $19.62 | 103s |
| 3 | ETH | $3,024.68 | $750.00 | $16.76 | 438s |
| 4 | SOL | $41.28 | $750.00 | $13.94 | 299s |
| 5 | BTC | $110,654.25 | $750.00 | $11.05 | 500s |

## Bottom 5 Trades

| # | Symbol | Zone | Size | Net PnL | Hold Time |
|---|--------|------|------|---------|-----------|
| 1 | BTC | $110,654.25 | $750.00 | $-11.18 | 196s |
| 2 | ETH | $3,024.68 | $750.00 | $-11.41 | 301s |
| 3 | SOL | $41.28 | $750.00 | $-12.50 | 41s |
| 4 | SOL | $41.28 | $750.00 | $-13.53 | 144s |
| 5 | ETH | $3,024.68 | $750.00 | $-18.12 | 62s |

## Conclusion

The liquidation zone strategy **does not pass the promotion gate** — 6/12 criteria met. 
Key deficiencies:

- **min_signal_events**: 15 (need ≥ 30)
- **min_sharpe**: 0.0290 (need ≥ 1.0)
- **fee_to_gross**: 80.24% (need < 35.0%)
- **no_single_trade_dominance**: 442.70% (need < 25.0%)
- **fishing_improves_expectancy**: fishing=0.0000, market=0.0000 (need fishing > market)
- **route_cost_within_budget**: 70.62% (need < 50.0%)

### Primary Blockers

**Insufficient signal events:** The capture duration (~2.6 hours with 8 cycles) produced too 
few replay events to meet the ≥30 threshold. A longer capture period (24-72 hours) with 
expanded wallet watchlist is needed to accumulate sufficient zone-touch events.

**Fee dominance:** Trading fees consume too much of gross profits. This is consistent with 
the M10 findings where fee-to-gross ratios consistently exceeded 100% for blueprint strategies. 
The liquidation zone approach needs lower fee execution (maker rebates, limit orders) to be viable.

**Insufficient risk-adjusted return:** The Sharpe ratio is below the 1.0 threshold, 
indicating the strategy does not generate enough return per unit of risk. More data points 
from a longer capture may change this, but the current evidence does not support promotion.

---
*Report generated by `scripts/generate_validation_reports.py`*
*Replay module: `src/replay.rs`*
*Fishing module: `src/fishing.rs`*
*Pyramiding module: `src/pyramiding.rs`*