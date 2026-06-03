# Pyramiding Analysis Report

## Overview

This report compares 5 pyramiding variants through a simulated replay pipeline:

1. **None** — Single tranche only (baseline)
2. **Reclaim** — Add tranches after price reclaims a level + higher low
3. **Retest** — Add tranches after successful retest of support/resistance
4. **Profit-funded** — Tranche size limited to unrealized profit
5. **ATR Trail** — Add when ATR trail confirms continuation

- **Scenarios:** 600
- **Symbols:** BTC, ETH, SOL
- **Starting Balance:** $1000.00
- **Target Position Size:** $1000.00
- **Default Sizing:** 25% / 25% / 25% / 25%
- **Max Tranches:** 4
- **Fee Rate:** 0.1% per side
- **Route Cost:** 3.0 bps

## Variant Comparison Table

| Metric | None (baseline) | Reclaim | Retest | Profit-Funded | ATR Trail |
|--------|----------------|---------|--------|---------------|-----------|
| Total Trades | 600 | 600 | 600 | 600 | 600 |
| Win Count | 345 | 318 | 333 | 345 | 330 |
| Loss Count | 255 | 282 | 267 | 255 | 270 |
| Win Rate | 57.5% | 53.0% | 55.5% | 57.5% | 55.0% |
| Gross PnL | $938.91 | $2159.67 | $1518.73 | $944.85 | $1636.04 |
| Total Fees | $345.00 | $1147.12 | $938.40 | $349.13 | $817.65 |
| Net PnL | $593.91 | $1012.54 | $580.33 | $595.72 | $818.39 |
| Sharpe Ratio | 8.9580 | 5.3140 | 3.3671 | 8.8959 | 4.7727 |
| Sortino Ratio | 15.8299 | 8.8222 | 5.1228 | 15.7448 | 7.6244 |
| Calmar Ratio | 1.1114 | 0.6762 | 0.2493 | 1.1096 | 0.4285 |
| Max Drawdown | $23.63 (2.4%) | $66.20 (6.6%) | $102.93 (10.3%) | $23.74 (2.4%) | $84.44 (8.4%) |
| Net Expectancy | $0.9149 | $1.4382 | $0.7632 | $0.9170 | $1.1862 |
| Avg Hold (s) | 155 | 155 | 155 | 155 | 155 |
| Stopped Out | 18 | 15 | 18 | 21 | 15 |
| Single-Trade Dep. | ✅ OK | ✅ OK | ✅ OK | ✅ OK | ✅ OK |

## Per-Variant Metrics

### None

- **Trades:** 600 (345W / 255L)
- **Win Rate:** 57.5%
- **Sharpe Ratio:** 8.9580
- **Sortino Ratio:** 15.8299
- **Calmar Ratio:** 1.1114
- **Max Drawdown:** $23.63 (2.4%)
- **Net Expectancy:** $0.9149
- **Net PnL:** $593.91
- **Gross PnL:** $938.91
- **Total Fees:** $345.00
- **Fee/Gross:** 36.7%
- **Avg Tranche Count:** 1.00
- **Tranche Range:** 1 - 1
- **Avg Hold Time:** 155s
- **Stopped Out:** 18 trades
- **Single-Trade Dependency:** ✅ OK

### Reclaim

- **Trades:** 600 (318W / 282L)
- **Win Rate:** 53.0%
- **Sharpe Ratio:** 5.3140
- **Sortino Ratio:** 8.8222
- **Calmar Ratio:** 0.6762
- **Max Drawdown:** $66.20 (6.6%)
- **Net Expectancy:** $1.4382
- **Net PnL:** $1012.54
- **Gross PnL:** $2159.67
- **Total Fees:** $1147.12
- **Fee/Gross:** 53.1%
- **Avg Tranche Count:** 3.33
- **Tranche Range:** 1 - 4
- **Avg Hold Time:** 155s
- **Stopped Out:** 15 trades
- **Single-Trade Dependency:** ✅ OK

### Retest

- **Trades:** 600 (333W / 267L)
- **Win Rate:** 55.5%
- **Sharpe Ratio:** 3.3671
- **Sortino Ratio:** 5.1228
- **Calmar Ratio:** 0.2493
- **Max Drawdown:** $102.93 (10.3%)
- **Net Expectancy:** $0.7632
- **Net PnL:** $580.33
- **Gross PnL:** $1518.73
- **Total Fees:** $938.40
- **Fee/Gross:** 61.8%
- **Avg Tranche Count:** 2.72
- **Tranche Range:** 1 - 4
- **Avg Hold Time:** 155s
- **Stopped Out:** 18 trades
- **Single-Trade Dependency:** ✅ OK

### Profit_funded

- **Trades:** 600 (345W / 255L)
- **Win Rate:** 57.5%
- **Sharpe Ratio:** 8.8959
- **Sortino Ratio:** 15.7448
- **Calmar Ratio:** 1.1096
- **Max Drawdown:** $23.74 (2.4%)
- **Net Expectancy:** $0.9170
- **Net PnL:** $595.72
- **Gross PnL:** $944.85
- **Total Fees:** $349.13
- **Fee/Gross:** 37.0%
- **Avg Tranche Count:** 3.77
- **Tranche Range:** 1 - 4
- **Avg Hold Time:** 155s
- **Stopped Out:** 21 trades
- **Single-Trade Dependency:** ✅ OK

### Atr_trail

- **Trades:** 600 (330W / 270L)
- **Win Rate:** 55.0%
- **Sharpe Ratio:** 4.7727
- **Sortino Ratio:** 7.6244
- **Calmar Ratio:** 0.4285
- **Max Drawdown:** $84.44 (8.4%)
- **Net Expectancy:** $1.1862
- **Net PnL:** $818.39
- **Gross PnL:** $1636.04
- **Total Fees:** $817.65
- **Fee/Gross:** 50.0%
- **Avg Tranche Count:** 2.37
- **Tranche Range:** 1 - 4
- **Avg Hold Time:** 155s
- **Stopped Out:** 15 trades
- **Single-Trade Dependency:** ✅ OK

## Tranche Distribution Analysis

Distribution of final tranche counts across all trades per variant:

| Tranches | None | Reclaim | Retest | Profit_funded | Atr_trail |
|----------|--------|--------|--------|--------|--------|
| 1 | 600 (100%) | 90 (15%) | 162 (27%) | 27 (4%) | 306 (51%) |
| 2 | 0 (0%) | 48 (8%) | 99 (16%) | 24 (4%) | 24 (4%) |
| 3 | 0 (0%) | 39 (6%) | 84 (14%) | 12 (2%) | 12 (2%) |
| 4 | 0 (0%) | 423 (70%) | 255 (42%) | 537 (90%) | 258 (43%) |

### Tranche Size Allocation

Default allocation per tranche:

- **Tranche 0** (Probe): 25% = $250.00
- **Tranche 1** (Confirm): 25% = $250.00
- **Tranche 2** (Retest): 25% = $250.00
- **Tranche 3** (Final): 25% = $250.00

Note: Profit-funded variant caps tranche size to unrealized PnL, so actual sizes vary.

## Does Pyramiding Improve Expectancy?

### Expectancy vs Baseline (None)

| Variant | Δ Expectancy | Δ Net PnL | Δ Sharpe | Δ Sortino | Δ Drawdown | Improves? |
|---------|-------------|-----------|----------|-----------|------------|-----------|
| Reclaim | $+0.5233 | $+418.63 | -3.6440 | -7.0077 | $+42.58 | ✅ Yes |
| Retest | $-0.1516 | $-13.58 | -5.5910 | -10.7071 | $+79.31 | ❌ No |
| Profit_funded | $+0.0021 | $+1.81 | -0.0621 | -0.0851 | $+0.11 | ✅ Yes |
| Atr_trail | $+0.2714 | $+224.47 | -4.1853 | -8.2055 | $+60.81 | ✅ Yes |

## Variance Impact Analysis

- **None:** mean=$0.9899, std=$3.9223, CV=3.96
- **Reclaim:** mean=$1.6876, std=$11.2726, CV=6.68
- **Retest:** mean=$0.9672, std=$10.1967, CV=10.54
- **Profit_funded:** mean=$0.9929, std=$3.9617, CV=3.99
- **Atr_trail:** mean=$1.3640, std=$10.1444, CV=7.44

## Recommendation

**Verdict: Pyramiding shows mixed results — some variants improve expectancy.**

- Best variant by expectancy: **Reclaim** (Δ $+0.5233)
- Best variant by Sharpe: **Profit_funded** (Δ -0.0621)
- Best variant by Sortino: **Profit_funded** (Δ -0.0851)

**Recommendation:** Use **Reclaim** pyramiding variant for paper trading validation. Monitor closely for variance amplification in live conditions.

### Key Findings

- **None:** Average 1.0 tranches per trade, 18 stopped out, $345.00 total fees
- **Reclaim:** Average 3.3 tranches per trade, 15 stopped out, $1147.12 total fees
- **Retest:** Average 2.7 tranches per trade, 18 stopped out, $938.40 total fees
- **Profit_funded:** Average 3.8 tranches per trade, 21 stopped out, $349.13 total fees
- **Atr_trail:** Average 2.4 tranches per trade, 15 stopped out, $817.65 total fees

### Caveats

1. Scenarios are synthetic — real liquidation zone dynamics may differ
2. Fee rates are fixed at 0.1% per side — actual rates vary by venue
3. Slippage is not modeled — multi-tranche exits may face worse execution
4. The replay pipeline uses the Rust `pyramiding.rs` logic for production validation
5. This report serves as a pre-validation analysis before the full Rust replay pipeline
6. **72h capture context:** After 71.1h of live capture (4,650 snapshots), the sweep-reclaim strategy with Reclaim pyramiding passes 11/12 promotion criteria (only Sharpe fails at 0.48 vs 1.0 required)

## Metadata

- **Generated:** 2026-05-31T14:42:35.547458+00:00
- **Updated:** 2026-06-03 (72h capture evaluation complete)
- **Script:** scripts/pyramiding-analysis.py
- **Pyramiding Module:** src/pyramiding.rs
- **Replay Module:** src/replay.rs
- **Scenarios:** 600
- **Variants:** none, reclaim, retest, profit_funded, atr_trail
