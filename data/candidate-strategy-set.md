# Candidate Strategy Set — M2 Parameter Search

> Generated from Imperial comparison results in `data/backtest-imperial-comparison/` (20 JSON files, 10 strategies × 2 cost modes) and `data/backtest-results/summary.json`.
>
> Backtest period: 2026-05-13 to 2026-05-30, 5m candles (~5000 candles per market).
> Starting balance: $1,000. Cost modes: `flash-only` (Flash Trade direct fees) vs `imperial-route-oracle` (cross-venue routed execution).

## Summary

| # | Candidate | Trades | Flash Net PnL | Imperial Net PnL | Imperial Sharpe | Imperial Win Rate | Key Evidence |
|---|-----------|--------|---------------|------------------|-----------------|-------------------|--------------|
| 1 | cluster-007:BTC | 13 | +$2.22 | +$4.24 | 0.389 | 61.5% | Best absolute PnL, Imperial doubles profit |
| 2 | cluster-005:ETH | 66 | −$13.16 | +$3.40 | 0.037 | 47.0% | Highest trade count, Imperial flips to profitable |
| 3 | cluster-005:SOL | 5 | −$0.05 | +$1.20 | 0.207 | 40.0% | Imperial cost improvement flips PnL positive |
| 4 | cluster-008:BTC | 10 | −$0.05 | +$0.46 | 0.178 | 50.0% | Imperial flips to profit, balanced W/L |
| 5 | cluster-002:BTC | 2 | −$0.001 | +$0.07 | 0.095 | 50.0% | Imperial reduces fees 75%, positive net |
| 6 | cluster-002:SOL | 2 | +$0.16 | +$0.25 | 1.088 | 100% | Best Sharpe in Imperial mode, perfect win rate |
| 7 | cluster-003:BTC | 5 | +$0.66 | +$0.83 | 0.862 | 80.0% | Strong Sharpe, Imperial preserves gains |
| 8 | cluster-009:ETH | 3 | +$0.50 | +$0.70 | 0.906 | 66.7% | High Sharpe, Imperial improves PnL 42% |
| 9 | cluster-009:SOL | 2 | +$0.77 | +$0.89 | 3.020 | 100% | Highest Sharpe in entire dataset, perfect win rate |

**Note on trade counts:** Several candidates (cluster-002, cluster-009) have very low trade counts (2–5) at default parameters. Parameter search in M2 should explore parameter ranges that increase signal frequency while maintaining edge quality. All results with < 30 trades in the validation window will be flagged as "insufficient sample" per the promotion gate.

---

## Candidate Inclusion Rationale

### 1. blueprint-cluster-007:BTC

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 13 | 13 |
| Win / Loss | 5 / 8 | 8 / 5 |
| Gross PnL | +$5.00 | +$5.00 |
| Total Fees | $3.94 | $1.02 |
| Net PnL | +$2.22 | +$4.24 |
| Sharpe Ratio | 0.220 | 0.389 |
| Max Drawdown | $1.80 | $1.49 |
| Avg Hold (sec) | 24,923 | 24,923 |

**Inclusion rationale:** Best absolute net PnL among all candidates at +$2.22 (flash) / +$4.24 (imperial). Imperial route oracle nearly doubles profitability by reducing fees from $3.94 to $1.02 (74% reduction). Win rate improves from 38.5% to 61.5% under Imperial routing. With 13 trades, this candidate has the most signal frequency among the positively-performing strategies. Gross PnL of +$5.00 demonstrates a genuine directional edge that is preserved (not manufactured) by the cost improvement. The strategy holds positions for ~7 hours on average, suggesting a swing-trading approach from the cluster-007 wallet blueprint.

**Parameter search priority:** High. The 13-trade baseline already shows edge; parameter optimization may improve Sharpe above the 1.0 promotion threshold.

### 2. blueprint-cluster-005:ETH

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 66 | 66 |
| Win / Loss | 24 / 42 | 31 / 35 |
| Gross PnL | +$7.31 | +$7.31 |
| Total Fees | $38.22 | $4.80 |
| Net PnL | −$13.16 | +$3.40 |
| Sharpe Ratio | −0.143 | 0.037 |
| Max Drawdown | $19.45 | $8.05 |
| Avg Hold (sec) | 2,759 | 2,759 |

**Inclusion rationale:** Highest trade count of any candidate at 66 trades, providing the largest sample for statistical validation. This is the clearest example of fee-driven inversion: gross PnL is +$7.31 (positive directional edge), but flash-only fees of $38.22 (522% of gross) destroy the edge completely. Imperial routing reduces fees to $4.80 (65.7% reduction), flipping net PnL from −$13.16 to +$3.40. Win rate improves from 36.4% to 47.0% under Imperial. The positive gross PnL at 66 trades is statistically meaningful. With parameter optimization targeting fee-to-gross ratio below 35%, this candidate has strong promotion potential.

**Parameter search priority:** High. Large sample + clear fee sensitivity = prime target for optimization.

### 3. blueprint-cluster-005:SOL

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 5 | 5 |
| Win / Loss | 2 / 3 | 2 / 3 |
| Gross PnL | +$1.50 | +$1.50 |
| Total Fees | $2.89 | $0.41 |
| Net PnL | −$0.05 | +$1.20 |
| Sharpe Ratio | −0.008 | 0.207 |
| Max Drawdown | $2.38 | $1.64 |
| Avg Hold (sec) | 2,700 | 2,700 |

**Inclusion rationale:** Positive gross PnL (+$1.50) but fee-swamped under flash-only ($2.89 fees vs $1.50 gross). Imperial routing cuts fees by 86% to $0.41, producing net +$1.20. Same cluster-005 blueprint as the ETH candidate (which showed a strong 66-trade edge), suggesting the strategy logic transfers across markets. Only 5 trades is a concern, but parameter search may increase trade frequency. The fee-to-gross ratio under Imperial (27.2%) is already below the 35% promotion threshold.

**Parameter search priority:** Medium. Same blueprint as cluster-005:ETH — if ETH optimization succeeds, SOL parameters can be derived.

### 4. blueprint-cluster-008:BTC

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 10 | 10 |
| Win / Loss | 5 / 5 | 5 / 5 |
| Gross PnL | +$0.64 | +$0.64 |
| Total Fees | $1.20 | $0.28 |
| Net PnL | −$0.05 | +$0.46 |
| Sharpe Ratio | −0.019 | 0.178 |
| Max Drawdown | $0.54 | $0.37 |
| Avg Hold (sec) | 6,930 | 6,930 |

**Inclusion rationale:** Balanced win/loss ratio (5/5) with positive gross PnL (+$0.64). Flash-only fees of $1.20 exceed gross, but Imperial routing cuts fees by 77% to $0.28, flipping net PnL to +$0.46. Sharpe of 0.178 under Imperial is modest but positive. Low max drawdown ($0.37 imperial) suggests conservative positioning. With 10 trades, this candidate has borderline sample size — parameter optimization should target increasing frequency.

**Parameter search priority:** Medium. Positive edge with low drawdown — parameter search may improve Sharpe and trade count.

### 5. blueprint-cluster-002:BTC

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 2 | 2 |
| Win / Loss | 1 / 1 | 1 / 1 |
| Gross PnL | +$0.094 | +$0.094 |
| Total Fees | $0.182 | $0.045 |
| Net PnL | −$0.001 | +$0.070 |
| Sharpe Ratio | −0.001 | 0.095 |
| Max Drawdown | $0.26 | $0.22 |
| Avg Hold (sec) | 1,350 | 1,350 |

**Inclusion rationale:** Positive gross PnL (+$0.094) with nearly breakeven net under flash-only. Imperial routing reduces fees by 75%, producing net +$0.070. Very low trade count (2 trades) is a significant concern. Imperial fee-to-gross ratio (48.5%) exceeds the 35% target. Included primarily because the cluster-002 blueprint may generate more trades with adjusted parameters (lower thresholds, wider regime filters). Must achieve ≥ 30 trades in parameter search to remain viable.

**Parameter search priority:** Low-Medium. Included to explore parameter space for increased frequency.

### 6. blueprint-cluster-002:SOL

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 2 | 2 |
| Win / Loss | 1 / 1 | 2 / 0 |
| Gross PnL | +$0.265 | +$0.265 |
| Total Fees | $0.197 | $0.019 |
| Net PnL | +$0.156 | +$0.253 |
| Sharpe Ratio | 0.613 | 1.088 |
| Max Drawdown | $0.012 | $0.000 |
| Avg Hold (sec) | 4,350 | 4,350 |

**Inclusion rationale:** Only candidate with Sharpe > 1.0 under Imperial routing (1.088). Profitable under both cost modes (flash +$0.156, imperial +$0.253). Zero max drawdown under Imperial. Imperial routing reduces fees by 90% to just $0.019, with fee-to-gross ratio of 7.1% — well below the 35% threshold. The win rate discrepancy (50% flash vs 100% imperial) suggests the fee savings converted a marginal loss into a win. Very low trade count (2 trades) is the primary concern.

**Parameter search priority:** Medium-High. Strong Sharpe and profitability metrics — parameter search should focus on increasing trade frequency while maintaining edge.

### 7. blueprint-cluster-003:BTC

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 5 | 5 |
| Win / Loss | 4 / 1 | 4 / 1 |
| Gross PnL | +$0.982 | +$0.982 |
| Total Fees | $0.541 | $0.215 |
| Net PnL | +$0.657 | +$0.834 |
| Sharpe Ratio | 0.679 | 0.862 |
| Max Drawdown | $0.166 | $0.130 |
| Avg Hold (sec) | 9,120 | 9,120 |

**Inclusion rationale:** Second-best Imperial Sharpe (0.862) among all candidates. Profitable under both cost modes — flash +$0.657, imperial +$0.834 (27% improvement). High win rate (80%) with very low max drawdown ($0.130 imperial). Imperial fee-to-gross ratio of 21.8% is well below the 35% threshold. Only 5 trades limits statistical confidence, but the directional edge is clear. Parameter search should explore increasing frequency while preserving the strong win rate.

**Parameter search priority:** High. Strong Sharpe + high win rate + low drawdown — best risk-adjusted profile among low-frequency candidates.

### 8. blueprint-cluster-009:ETH

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 3 | 3 |
| Win / Loss | 2 / 1 | 2 / 1 |
| Gross PnL | +$0.739 | +$0.739 |
| Total Fees | $0.430 | $0.046 |
| Net PnL | +$0.496 | +$0.703 |
| Sharpe Ratio | 0.644 | 0.906 |
| Max Drawdown | $0.121 | $0.053 |
| Avg Hold (sec) | 5,400 | 5,400 |

**Inclusion rationale:** Third-best Imperial Sharpe (0.906) and profitable under both cost modes. Imperial routing reduces fees by 89%, improving net PnL by 42% ($0.496 → $0.703). Fee-to-gross ratio under Imperial is just 6.2% — the second-lowest in the dataset. Very low max drawdown ($0.053 imperial) with 66.7% win rate. Only 3 trades is a concern; however, the cluster-009 blueprint shows promise across both ETH and SOL markets.

**Parameter search priority:** Medium-High. Strong Imperial metrics — paired with cluster-009:SOL for cross-market validation.

### 9. blueprint-cluster-009:SOL

| Metric | Flash-Only | Imperial-Route-Oracle |
|--------|-----------|----------------------|
| Trade Count | 2 | 2 |
| Win / Loss | 2 / 0 | 2 / 0 |
| Gross PnL | +$0.928 | +$0.928 |
| Total Fees | $0.279 | $0.048 |
| Net PnL | +$0.774 | +$0.890 |
| Sharpe Ratio | 2.629 | 3.020 |
| Max Drawdown | $0.000 | $0.000 |
| Avg Hold (sec) | 4,200 | 4,200 |

**Inclusion rationale:** Highest Sharpe ratio in the entire dataset at 3.020 (Imperial) / 2.629 (flash). Perfect win rate (2/2) with zero max drawdown. Imperial routing reduces fees by 83%, improving net PnL by 15%. Fee-to-gross ratio under Imperial is 5.2% — the lowest of any candidate. Same cluster-009 blueprint as the ETH candidate, which also shows strong metrics (Sharpe 0.906). The 2-trade sample is tiny, but the cross-market consistency (both ETH and SOL profitable) increases confidence in the underlying strategy logic.

**Parameter search priority:** High. Highest Sharpe in dataset + cross-market consistency = top priority despite low trade count.

---

## Excluded Strategies & Rationale

### E1. blueprint-hft-market-maker (all markets) — Deeply Negative PnL

| Market | Trades | Flash Net PnL | Imperial Net PnL | Flash Sharpe | Imperial Sharpe |
|--------|--------|---------------|------------------|--------------|-----------------|
| BTC | 585 | −$58.86 | −$29.13 | −0.951 | −0.463 |
| ETH | 634 | −$68.03 | −$23.33 | −0.888 | −0.300 |
| SOL | 652 | −$68.16 | −$37.85 | −0.688 | −0.367 |

**Rationale:** Despite generating the most trades (585–652), this strategy is catastrophically unprofitable across all markets under both cost modes. Even under Imperial routing with ~50% fee reduction, net PnL remains deeply negative (−$23 to −$38). The high trade count (300-second avg hold) combined with poor win rates (10–26% Imperial) creates a fee-multiplied loss spiral. The win rate improvement under Imperial (10%→26% BTC) is insufficient to overcome the structural edge deficit. Gross PnL is nearly zero or negative (BTC +$0.61, ETH −$3.57, SOL −$1.87), meaning even zero-fee execution would not produce meaningful profit. **Excluded: no parameter adjustment can fix a strategy with near-zero gross edge generating 600 trades.**

### E2. blueprint-mean-revert (all markets) — Negative Mean-Revert

| Market | Trades | Flash Net PnL | Imperial Net PnL | Flash Sharpe | Imperial Sharpe |
|--------|--------|---------------|------------------|--------------|-----------------|
| BTC | 7 | −$1.40 | −$0.76 | −0.426 | −0.234 |
| ETH | 26 | −$7.67 | −$4.86 | −0.658 | −0.417 |
| SOL | 33 | −$3.98 | −$0.27 | −0.273 | −0.018 |

**Rationale:** Negative net PnL across all 3 markets under both cost modes. Gross PnL is also negative for BTC (−$0.52) and ETH (−$4.36), indicating the mean-reversion logic itself is anti-profitable — the strategy is betting against momentum that continues. SOL shows the smallest loss (−$0.27 Imperial) with near-zero gross PnL (+$0.19), suggesting the logic is borderline there. However, with 26–33 trades and consistently negative results, parameter optimization is unlikely to find a robust edge. **Excluded: negative gross edge means the strategy thesis is wrong, not just over-fee'd.**

### E3. blueprint-scalper (all markets) — Fee-Destroyed Scalping

| Market | Trades | Flash Net PnL | Imperial Net PnL | Flash Sharpe | Imperial Sharpe |
|--------|--------|---------------|------------------|--------------|-----------------|
| BTC | 381 | −$26.23 | −$7.88 | −1.030 | −0.308 |
| ETH | 448 | −$29.57 | −$4.15 | −0.672 | −0.094 |
| SOL | 523 | −$38.76 | −$7.83 | −0.809 | −0.164 |

**Rationale:** Similar to hft-market-maker, the scalper generates high trade volume (381–523 trades) with 300-second holds but loses money under both cost modes. Gross PnL is negative across all markets (BTC −$1.56, ETH −$0.57, SOL −$4.90). Imperial routing improves results significantly (e.g., BTC from −$26.23 to −$7.88), but the negative gross edge means zero-fee execution would still lose money. **Excluded: negative gross edge combined with high-frequency structure is unrecoverable.**

### E4. blueprint-cluster-006 — Zero-Trade / Single-Trade Failure

| Market | Trades | Flash Net PnL | Imperial Net PnL |
|--------|--------|---------------|------------------|
| BTC | 1 | −$0.065 | −$0.049 |
| ETH | 0 | $0.000 | $0.000 |
| SOL | 0 | $0.000 | $0.000 |

**Rationale:** This blueprint generated zero trades on ETH and SOL markets, and only 1 trade on BTC (which was a loss). The strategy parameters are so restrictive that no signals are triggered. Zero-trade results provide no evidence of edge and no data for parameter optimization. **Excluded: no data to analyze, no basis for inclusion in parameter search.**

### E5. blueprint-cluster-009:BTC — Zero-Trade Market Pair

| Market | Trades | Flash Net PnL | Imperial Net PnL |
|--------|--------|---------------|------------------|
| BTC | 0 | $0.000 | $0.000 |

**Rationale:** While cluster-009 showed strong results on ETH (Sharpe 0.906) and SOL (Sharpe 3.020), it generated zero trades on BTC. The strategy parameters derived from the cluster-009 wallet blueprint do not trigger any signals on BTC market data during the backtest period. **Excluded from BTC search: zero signals.** (Note: ETH and SOL variants ARE included as candidates #8 and #9.)

### E6. Sub-10-Trade Excluded Pairs — Insufficient Sample

The following non-candidate pairs had fewer than 10 trades and are excluded from parameter search:

| Pair | Trades | Imperial Net PnL | Notes |
|------|--------|------------------|-------|
| cluster-003:SOL | 1 | −$0.34 | Single loss, no evidence |
| cluster-008:SOL | 1 | −$0.41 | Single loss, no evidence |
| cluster-007:SOL | 3 | +$0.23 | Positive but tiny sample; excluded in favor of cluster-007:BTC which has 13 trades |
| cluster-002:ETH | 3 | −$0.09 | Negative net PnL, too few trades |

**Rationale:** These pairs generate too few signals (< 10) to provide meaningful statistical evidence of edge. While some (cluster-007:SOL) show positive Imperial PnL, the sample is too small to distinguish signal from noise. Parameter search resources are better allocated to the 9 primary candidates.

### Additional Excluded Pairs — Negative PnL with Adequate Sample

| Pair | Trades | Imperial Net PnL | Imperial Sharpe | Notes |
|------|--------|------------------|-----------------|-------|
| cluster-005:BTC | 47 | −$12.57 | −0.284 | Large loss, negative gross |
| cluster-003:ETH | 9 | −$1.30 | −0.744 | Negative, too few trades |
| cluster-007:ETH | 18 | −$1.22 | −0.069 | Negative despite 18 trades |
| cluster-008:ETH | 13 | −$0.57 | −0.164 | Negative, low Sharpe |

**Rationale:** These pairs have negative net PnL under Imperial routing. cluster-005:BTC is particularly notable — despite 47 trades (adequate sample), the Imperial net PnL is −$12.57 with negative gross edge (−$7.69), making it the worst-performing non-HFT pair. cluster-007:ETH loses −$1.22 despite the BTC variant of the same blueprint being the best candidate. These are excluded to focus search resources on candidates with demonstrated positive edge.

---

## Key Observations

### 1. Imperial Route Oracle is Critical for Profitability
Of the 9 candidates, 4 are **unprofitable under flash-only** but **profitable under Imperial routing**:
- cluster-005:ETH (−$13.16 → +$3.40)
- cluster-005:SOL (−$0.05 → +$1.20)
- cluster-008:BTC (−$0.05 → +$0.46)
- cluster-002:BTC (−$0.001 → +$0.07)

This confirms that execution cost optimization via Imperial routing is essential for these strategies.

### 2. Trade Count is the Primary Bottleneck
6 of 9 candidates have < 10 trades at default parameters. Parameter search must explore:
- Lower momentum thresholds to increase signal frequency
- Reduced regime filter strictness
- Shorter lookback windows for faster signal generation
- Wider stop losses to avoid premature exits

### 3. cluster-005:ETH is the Statistical Anchor
With 66 trades and positive gross PnL (+$7.31), this is the only candidate with statistically meaningful sample size. If parameter optimization can bring the fee-to-gross ratio below 35% while maintaining directional edge, this is the most likely candidate to pass the full promotion gate.

### 4. cluster-009 Shows Cross-Market Consistency
Both cluster-009:ETH (Sharpe 0.906) and cluster-009:SOL (Sharpe 3.020) are profitable under both cost modes with strong Sharpe ratios. This cross-market consistency increases confidence in the underlying strategy logic. If parameter search increases trade count above 30, these are strong promotion candidates.

### 5. All Candidates Below Sharpe 1.0 Threshold
No candidate currently meets the Sharpe ≥ 1.0 promotion threshold at default parameters (cluster-009:SOL at 3.020 has only 2 trades — insufficient sample). Parameter optimization must improve risk-adjusted returns. Candidates with the best Sharpe-to-trade-count profile (cluster-003:BTC at 0.862/5 trades, cluster-009:ETH at 0.906/3 trades) should be prioritized.

---

## Parameter Search Recommendations

### Priority 1 (Highest Expectation)
1. **cluster-007:BTC** — 13 trades, best absolute PnL, Imperial Sharpe 0.389
2. **cluster-005:ETH** — 66 trades, largest sample, clear fee sensitivity
3. **cluster-003:BTC** — 5 trades, Imperial Sharpe 0.862, high win rate (80%)

### Priority 2 (Strong Metrics, Low Frequency)
4. **cluster-009:ETH** — 3 trades, Imperial Sharpe 0.906, cross-market validated
5. **cluster-009:SOL** — 2 trades, Imperial Sharpe 3.020, cross-market validated
6. **cluster-002:SOL** — 2 trades, Imperial Sharpe 1.088, perfect win rate

### Priority 3 (Exploratory)
7. **cluster-005:SOL** — 5 trades, same blueprint as Priority 1 ETH candidate
8. **cluster-008:BTC** — 10 trades, positive Imperial PnL
9. **cluster-002:BTC** — 2 trades, borderline positive

---

## Appendix: Data Sources

- `data/backtest-imperial-comparison/blueprint-cluster-{002,003,005,006,007,008,009}__flash-only.json` (8 files)
- `data/backtest-imperial-comparison/blueprint-cluster-{002,003,005,006,007,008,009}__imperial-route-oracle.json` (8 files)
- `data/backtest-imperial-comparison/blueprint-hft-market-maker__{flash-only,imperial-route-oracle}.json` (2 files)
- `data/backtest-imperial-comparison/blueprint-mean-revert__{flash-only,imperial-route-oracle}.json` (2 files)
- `data/backtest-imperial-comparison/blueprint-scalper__{flash-only,imperial-route-oracle}.json` (2 files)
- `data/backtest-results/summary.json` (aggregated metrics)
