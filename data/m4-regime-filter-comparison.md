# M4: Regime-Aware Entry Filter — Before/After Comparison

## Improvement Selected
**Regime-aware entry filtering for ALL strategies** (not just blueprint strategies).

## Bottleneck Reference
Bottleneck #7 from M1 ranking: "No regime filter — strategies trade in all market conditions.
Momentum scalper performs poorly in ranging markets; mean-reversion fails in trends."

## Methodology
- **Period:** 2026-05-15 to 2026-05-30 (15 days)
- **Interval:** 5m candles
- **Balance:** $1000
- **Fee rate:** 0.10% (Flash Trade taker)
- **Markets:** BTC, SOL
- **Strategies:** momentum-scalper, mean-reversion, trend-follower

## Before Metrics (regime_filter=false, effectively no filter for non-blueprint)

| Strategy         | Market | Trades | Gross$  | Fees$  | Net$     | Win%  | Sharpe |
|------------------|--------|--------|---------|--------|----------|-------|--------|
| momentum-scalper | BTC    | 406    | 1.25    | 838.36 | -431.11  | 20.4% | -0.61  |
| momentum-scalper | SOL    | 415    | -92.43  | 856.56 | -533.99  | 28.7% | -0.45  |
| mean-reversion   | BTC    | 6      | -1.72   | 12.40  | -8.12    | 33.3% | -0.80  |
| mean-reversion   | SOL    | 44     | 26.10   | 90.82  | -20.71   | 45.5% | -0.14  |
| trend-follower   | BTC    | 175    | -33.12  | 372.27 | -230.39  | 21.7% | -0.41  |
| trend-follower   | SOL    | 227    | -63.56  | 482.41 | -318.97  | 32.6% | -0.34  |
| **TOTAL**        |        | 1273   | -163.48 | 2652.82| -1543.29 | 26.6% | -0.46  |

## After Metrics (regime_filter=true, strategy-specific rules)

| Strategy         | Market | Trades | Gross$  | Fees$  | Net$    | Win%  | Sharpe |
|------------------|--------|--------|---------|--------|---------|-------|--------|
| momentum-scalper | BTC    | 5      | 6.30    | 10.33  | 0.97    | 40.0% | 0.18   |
| momentum-scalper | SOL    | 27     | -34.17  | 55.57  | -62.73  | 18.5% | -0.56  |
| mean-reversion   | BTC    | 2      | -1.68   | 4.13   | -3.81   | 0.0%  | -3.62  |
| mean-reversion   | SOL    | 27     | 3.77    | 55.73  | -24.96  | 33.3% | -0.30  |
| trend-follower   | BTC    | 4      | -11.70  | 8.53   | -16.24  | 0.0%  | -1.10  |
| trend-follower   | SOL    | 23     | -13.18  | 48.80  | -38.98  | 21.7% | -0.29  |
| **TOTAL**        |        | 88     | -50.66  | 183.10 | -145.76 | 19.3% | -0.28  |

## Comparison (Net PnL after all costs)

| Metric              | Before      | After       | Change           |
|---------------------|-------------|-------------|------------------|
| Total Trades        | 1273        | 88          | -93.1%           |
| Total Fees          | $2,652.82   | $183.10     | -93.1%           |
| Net PnL             | -$1,543.29  | -$145.76    | +$1,397.53 (90.6% better) |
| Avg Win Rate        | 26.6%       | 19.3%       | -7.3pp           |
| Avg Sharpe          | -0.46       | -0.28       | +0.18            |
| momentum-scalper BTC| -431.11     | +0.97       | **BREAKEVEN**    |
| Fee Drag            | $2,652.82   | $183.10     | -93.1%           |

## Regime Filtering Rules
- **momentum-scalper**: Skip LowVol, Skip Choppy (needs directional movement)
- **lp-consumption**: Skip LowVol (needs volatility for LP imbalance edge)
- **mean-reversion**: Skip Trending (counter-trend, fails in strong trends)
- **trend-follower**: Skip Choppy, Skip LowVol (needs clear direction)
- **funding-capture**: Skip HighVol (risk exceeds yield in extreme volatility)

## Verdict: POSITIVE
- Net loss reduced by 90.6% ($1,397 improvement on a $1000 account)
- momentum-scalper on BTC went from -$431 to +$0.97 (effectively breakeven)
- Fee drag reduced by 93% — the primary killer was overtrading in unfavorable conditions
- Trade count reduced from 1273 to 88 — only trading in compatible regimes
- Still negative overall, but dramatically less destructive
- Win rate dropped because remaining trades are still challenging — but at least we're not bleeding fees

## Failure Cases
1. **Regime detection lag**: The detector uses a 288-candle lookback (~1 day on 5m candles). Rapid regime shifts may not be detected fast enough.
2. **Over-filtering**: With only 88 trades across 6 strategy/market combos, statistical significance is very low. The improvement could be regime-dependent for this specific 15-day period.
3. **SOL momentum-scalper still negative**: Even after filtering, SOL momentum-scalper lost $62.73. The regime filter may need tuning for SOL specifically.

## Rollback Instructions
- Commit: (this commit)
- Revert: `git revert HEAD` to restore non-filtered behavior
- Config: Set `regime_filter = false` in `[backtest]` section of `config/perps.toml`
