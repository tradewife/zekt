# Portfolio Construction & Backtest Analysis

> Generated: 2026-05-31 07:38 UTC
> Backtest period: 2026-03-01 to 2026-05-30 (90 days)
> Initial balance: $1000.00
> Candidates from M2 leverage-sizing: 9
> Cost modes: flash-only, imperial-route-oracle

## Methodology

### Candidate Selection

From the M2 leverage-sizing frontier, the best configuration per
(strategy, market, cost_mode) candidate is selected. Selection criteria:
1. Highest Sharpe ratio across all (leverage, sizing_mode) combinations
2. Ties broken by net PnL (higher is better)
3. Ties broken by max drawdown (lower is better)

Each candidate uses its individually optimal leverage and sizing mode
from the frontier analysis — no one-size-fits-all approach.

### Correlation Computation

Cross-candidate correlation is computed from **daily PnL returns**:
- Each candidate's trades are aggregated into daily PnL sums
- Pearson correlation is computed pairwise across all trading days
- Missing days (no trades) are filled with zero PnL
- Pairs with correlation > 0.7 are flagged as highly correlated

### Allocation Strategies

Three allocation strategies are tested:

1. **Equal Weight**: Each candidate receives 1/N of the portfolio.
   Simple baseline that avoids estimation error from noisy metrics.

2. **Risk Parity**: Weight inversely proportional to daily PnL volatility.
   Candidates with more volatile returns get less allocation.
   Formula: `w_i = (1/σ_i) / Σ(1/σ_j)` where σ is daily PnL std dev.

3. **Sharpe-Weighted**: Weight proportional to Sharpe ratio.
   Candidates with higher risk-adjusted returns get more allocation.
   Negative Sharpe candidates receive zero weight; excess redistributed.
   Formula: `w_i = max(Sharpe_i, 0) / Σmax(Sharpe_j, 0)`.

### Risk Constraints

| Constraint | Value | Rationale |
|-----------|-------|-----------|
| Max allocation per candidate | 40% | Prevents concentration in single strategy |
| Max correlated exposure | 60% | Limits same-market risk (BTC/SOL/ETH groups) |
| Max simultaneous positions | 3 | Controls operational complexity |
| Daily drawdown breaker | 5% | Halts trading after large daily loss |
| Weekly drawdown breaker | 10% | Halts trading after large weekly loss |

### Correlated Market Groups

| Group | Strategies | Markets |
|-------|-----------|---------|
| BTC | cluster-007, cluster-008 | BTC |
| ETH | cluster-005 | ETH |
| SOL | cluster-002, cluster-005, cluster-009 | SOL |

### Single-Best Comparison

The single best candidate (by Sharpe) is compared against each portfolio
allocation strategy to determine whether diversification adds value.

### Top-Signal-Only Mode

An alternative approach: when multiple candidates signal simultaneously,
only the highest-Sharpe candidate's trade is taken. This tests whether
selectivity beats diversification.

## Candidate Summary

| # | Candidate | Cost Mode | Best Lev | Best Sizing | Sharpe | PnL | Trades | Max DD |
|---|-----------|-----------|----------|-------------|--------|-----|--------|--------|
| 1 | cluster-002:SOL | imperial-route-oracle | 4.0x | fixed-notional | -0.0482 | $-3.66 | 125 | $2.12 |
| 2 | cluster-005:ETH | flash-only | 2.0x | volatility-adjusted | -0.1067 | $-8.82 | 90 | $2.40 |
| 3 | cluster-005:ETH | imperial-route-oracle | 3.0x | volatility-adjusted | 0.0831 | $2.15 | 90 | $1.15 |
| 4 | cluster-005:SOL | flash-only | 3.0x | volatility-adjusted | -0.1015 | $-3.24 | 102 | $1.09 |
| 5 | cluster-005:SOL | imperial-route-oracle | 10.0x | volatility-adjusted | -0.0081 | $-0.29 | 102 | $0.72 |
| 6 | cluster-007:BTC | flash-only | 7.5x | volatility-adjusted | -0.3146 | $-10.32 | 42 | $3.73 |
| 7 | cluster-007:BTC | imperial-route-oracle | 5.0x | fixed-notional | -0.1536 | $-12.94 | 42 | $7.14 |
| 8 | cluster-008:BTC | flash-only | 10.0x | volatility-adjusted | -0.2524 | $-8.81 | 41 | $2.10 |
| 9 | cluster-008:BTC | imperial-route-oracle | 1.0x | volatility-adjusted | 0.0196 | $-0.12 | 40 | $1.02 |

## Cross-Candidate Correlation Matrix

Correlation computed from daily PnL returns across the 90-day backtest period.

| | cluster-002:SOL:imperial-route-oracle | cluster-005:ETH:flash-only | cluster-005:ETH:imperial-route-oracle | cluster-005:SOL:flash-only | cluster-005:SOL:imperial-route-oracle | cluster-007:BTC:flash-only | cluster-007:BTC:imperial-route-oracle | cluster-008:BTC:flash-only | cluster-008:BTC:imperial-route-oracle |
|---|---|---|---|---|---|---|---|---|---|
| **cluster-002:SOL:imperial-route-oracle** | 1.000 | -0.431 | -0.423 | -0.103 | -0.107 | -0.196 | -0.249 | -0.352 | -0.298 |
| **cluster-005:ETH:flash-only** | -0.431 | 1.000 | 0.960 ⚠️ | 0.017 | -0.019 | -0.048 | 0.042 | -0.045 | -0.267 |
| **cluster-005:ETH:imperial-route-oracle** | -0.423 | 0.960 ⚠️ | 1.000 | -0.053 | -0.056 | -0.110 | 0.001 | -0.065 | -0.249 |
| **cluster-005:SOL:flash-only** | -0.103 | 0.017 | -0.053 | 1.000 | 0.990 ⚠️ | 0.449 | 0.515 | 0.600 | 0.537 |
| **cluster-005:SOL:imperial-route-oracle** | -0.107 | -0.019 | -0.056 | 0.990 ⚠️ | 1.000 | 0.432 | 0.504 | 0.608 | 0.565 |
| **cluster-007:BTC:flash-only** | -0.196 | -0.048 | -0.110 | 0.449 | 0.432 | 1.000 | 0.953 ⚠️ | 0.573 | 0.580 |
| **cluster-007:BTC:imperial-route-oracle** | -0.249 | 0.042 | 0.001 | 0.515 | 0.504 | 0.953 ⚠️ | 1.000 | 0.523 | 0.526 |
| **cluster-008:BTC:flash-only** | -0.352 | -0.045 | -0.065 | 0.600 | 0.608 | 0.573 | 0.523 | 1.000 | 0.943 ⚠️ |
| **cluster-008:BTC:imperial-route-oracle** | -0.298 | -0.267 | -0.249 | 0.537 | 0.565 | 0.580 | 0.526 | 0.943 ⚠️ | 1.000 |

⚠️ = correlation > 0.7 (highly correlated)

### Highly Correlated Pairs (>0.7)

- **cluster-005:ETH:flash-only** ↔ **cluster-005:ETH:imperial-route-oracle**: 0.960
- **cluster-005:SOL:flash-only** ↔ **cluster-005:SOL:imperial-route-oracle**: 0.990
- **cluster-007:BTC:flash-only** ↔ **cluster-007:BTC:imperial-route-oracle**: 0.953
- **cluster-008:BTC:flash-only** ↔ **cluster-008:BTC:imperial-route-oracle**: 0.943

## Allocation Weights

### Equal Weight

| Candidate | Weight | Market | Strategy |
|-----------|--------|--------|----------|
| cluster-002:SOL:imperial-route-oracle | 11.1% | SOL | blueprint-cluster-002 |
| cluster-005:ETH:flash-only | 11.1% | ETH | blueprint-cluster-005 |
| cluster-005:ETH:imperial-route-oracle | 11.1% | ETH | blueprint-cluster-005 |
| cluster-005:SOL:flash-only | 11.1% | SOL | blueprint-cluster-005 |
| cluster-005:SOL:imperial-route-oracle | 11.1% | SOL | blueprint-cluster-005 |
| cluster-007:BTC:flash-only | 11.1% | BTC | blueprint-cluster-007 |
| cluster-007:BTC:imperial-route-oracle | 11.1% | BTC | blueprint-cluster-007 |
| cluster-008:BTC:flash-only | 11.1% | BTC | blueprint-cluster-008 |
| cluster-008:BTC:imperial-route-oracle | 11.1% | BTC | blueprint-cluster-008 |
| **Total** | **100.0%** | | |

**Weight sum: 100.00%** (normalized to 100%)

### Risk Parity

| Candidate | Weight | Market | Strategy |
|-----------|--------|--------|----------|
| cluster-005:SOL:imperial-route-oracle | 20.6% | SOL | blueprint-cluster-005 |
| cluster-005:SOL:flash-only | 20.1% | SOL | blueprint-cluster-005 |
| cluster-005:ETH:imperial-route-oracle | 10.9% | ETH | blueprint-cluster-005 |
| cluster-008:BTC:imperial-route-oracle | 10.5% | BTC | blueprint-cluster-008 |
| cluster-005:ETH:flash-only | 10.0% | ETH | blueprint-cluster-005 |
| cluster-008:BTC:flash-only | 9.8% | BTC | blueprint-cluster-008 |
| cluster-007:BTC:flash-only | 8.3% | BTC | blueprint-cluster-007 |
| cluster-002:SOL:imperial-route-oracle | 7.2% | SOL | blueprint-cluster-002 |
| cluster-007:BTC:imperial-route-oracle | 2.5% | BTC | blueprint-cluster-007 |
| **Total** | **100.0%** | | |

**Weight sum: 100.00%** (normalized to 100%)

### Sharpe Weighted

| Candidate | Weight | Market | Strategy |
|-----------|--------|--------|----------|
| cluster-005:ETH:imperial-route-oracle | 40.0% | ETH | blueprint-cluster-005 |
| cluster-008:BTC:imperial-route-oracle | 24.2% | BTC | blueprint-cluster-008 |
| cluster-002:SOL:imperial-route-oracle | 5.1% | SOL | blueprint-cluster-002 |
| cluster-005:ETH:flash-only | 5.1% | ETH | blueprint-cluster-005 |
| cluster-005:SOL:flash-only | 5.1% | SOL | blueprint-cluster-005 |
| cluster-005:SOL:imperial-route-oracle | 5.1% | SOL | blueprint-cluster-005 |
| cluster-007:BTC:flash-only | 5.1% | BTC | blueprint-cluster-007 |
| cluster-007:BTC:imperial-route-oracle | 5.1% | BTC | blueprint-cluster-007 |
| cluster-008:BTC:flash-only | 5.1% | BTC | blueprint-cluster-008 |
| **Total** | **100.0%** | | |

**Weight sum: 100.00%** (normalized to 100%)

## Portfolio Simulation Results

### Combined Portfolio Metrics

#### Equal Weight

| Metric | Value |
|--------|-------|
| Net PnL | $-2.35 |
| Final Balance | $997.65 |
| Sharpe Ratio | -1.9208 |
| Max Drawdown | $6.83 |
| Trade Count | 629 |
| Win Rate | 59.9% |
| Gross Profit | $44.68 |
| Gross Loss | $47.02 |
| Fee/Gross Ratio | 1.052 |
| Drawdown Breakers | 0 |

#### Risk Parity

| Metric | Value |
|--------|-------|
| Net PnL | $-1.12 |
| Final Balance | $998.88 |
| Sharpe Ratio | -1.3240 |
| Max Drawdown | $4.27 |
| Trade Count | 629 |
| Win Rate | 59.9% |
| Gross Profit | $37.62 |
| Gross Loss | $38.74 |
| Fee/Gross Ratio | 1.030 |
| Drawdown Breakers | 0 |

#### Sharpe Weighted

| Metric | Value |
|--------|-------|
| Net PnL | $-1.26 |
| Final Balance | $998.74 |
| Sharpe Ratio | -2.0882 |
| Max Drawdown | $4.06 |
| Trade Count | 629 |
| Win Rate | 59.9% |
| Gross Profit | $24.37 |
| Gross Loss | $25.63 |
| Fee/Gross Ratio | 1.052 |
| Drawdown Breakers | 0 |

## Single-Best vs Portfolio Comparison

### Single Best Candidate

| Metric | Value |
|--------|-------|
| Net PnL | $15.73 |
| Final Balance | $1015.73 |
| Sharpe Ratio | 2.3631 |
| Max Drawdown | $33.47 |
| Trade Count | 262 |
| Win Rate | 48.5% |
| Gross Profit | $170.63 |
| Gross Loss | $154.90 |
| Fee/Gross Ratio | 0.908 |
| Drawdown Breakers | 0 |

### Top-Signal-Only Mode

| Metric | Value |
|--------|-------|
| Net PnL | $21.73 |
| Final Balance | $1021.73 |
| Sharpe Ratio | 5.7187 |
| Max Drawdown | $15.87 |
| Trade Count | 193 |
| Win Rate | 54.9% |
| Drawdown Breakers | 0 |

### Head-to-Head Comparison

| Strategy | Net PnL | Sharpe | Max DD | Trades | Win Rate |
|----------|---------|--------|--------|--------|----------|
| Single Best | $15.73 | 2.3631 | $33.47 | 262 | 48.5% |
| Top-Signal-Only | $21.73 | 5.7187 | $15.87 | 193 | 54.9% |
| Portfolio (Equal Weight) | $-2.35 | -1.9208 | $6.83 | 629 | 59.9% |
| Portfolio (Risk Parity) | $-1.12 | -1.3240 | $4.27 | 629 | 59.9% |
| Portfolio (Sharpe Weighted) | $-1.26 | -2.0882 | $4.06 | 629 | 59.9% |

## Drawdown Breaker Analysis

### Equal Weight: 0 breaker events

No drawdown breaker events triggered during the period.

### Risk Parity: 0 breaker events

No drawdown breaker events triggered during the period.

### Sharpe Weighted: 0 breaker events

No drawdown breaker events triggered during the period.

## Promotion Decision

### Evaluation Framework

The promotion decision evaluates the portfolio (and individual candidates)
against the six promotion gate criteria from the validation contract:

1. **Positive out-of-sample PnL** — Net PnL > $0 after all costs
2. **Sharpe ratio ≥ 1.0** — Risk-adjusted returns exceed baseline
3. **Trade count ≥ 30** — Sufficient sample for statistical significance
4. **Acceptable max drawdown** — Drawdowns within risk tolerance
5. **Fee-to-gross ratio < 35%** — Edge not consumed by execution costs
6. **Parameter stability** — Performance not dependent on single period

### Individual Candidate Assessment

**cluster-002:SOL:imperial-route-oracle**: ❌ FAIL
- PnL: ❌ $-3.66
- Sharpe: ❌ -0.0482 (need ≥ 1.0)
- Trades: ✅ 125 (need ≥ 30)
- Fee/Gross: ❌ 4.422 (need < 0.35)

**cluster-005:ETH:flash-only**: ❌ FAIL
- PnL: ❌ $-8.82
- Sharpe: ❌ -0.1067 (need ≥ 1.0)
- Trades: ✅ 90 (need ≥ 30)
- Fee/Gross: ❌ 3.257 (need < 0.35)

**cluster-005:ETH:imperial-route-oracle**: ❌ FAIL
- PnL: ✅ $2.15
- Sharpe: ❌ 0.0831 (need ≥ 1.0)
- Trades: ✅ 90 (need ≥ 30)
- Fee/Gross: ❌ 0.517 (need < 0.35)

**cluster-005:SOL:flash-only**: ❌ FAIL
- PnL: ❌ $-3.24
- Sharpe: ❌ -0.1015 (need ≥ 1.0)
- Trades: ✅ 102 (need ≥ 30)
- Fee/Gross: ❌ 3.337 (need < 0.35)

**cluster-005:SOL:imperial-route-oracle**: ❌ FAIL
- PnL: ❌ $-0.29
- Sharpe: ❌ -0.0081 (need ≥ 1.0)
- Trades: ✅ 102 (need ≥ 30)
- Fee/Gross: ❌ 2.059 (need < 0.35)

**cluster-007:BTC:flash-only**: ❌ FAIL
- PnL: ❌ $-10.32
- Sharpe: ❌ -0.3146 (need ≥ 1.0)
- Trades: ✅ 42 (need ≥ 30)
- Fee/Gross: ❌ 5.594 (need < 0.35)

**cluster-007:BTC:imperial-route-oracle**: ❌ FAIL
- PnL: ❌ $-12.94
- Sharpe: ❌ -0.1536 (need ≥ 1.0)
- Trades: ✅ 42 (need ≥ 30)
- Fee/Gross: ❌ 2.569 (need < 0.35)

**cluster-008:BTC:flash-only**: ❌ FAIL
- PnL: ❌ $-8.81
- Sharpe: ❌ -0.2524 (need ≥ 1.0)
- Trades: ✅ 41 (need ≥ 30)
- Fee/Gross: ❌ 86.577 (need < 0.35)

**cluster-008:BTC:imperial-route-oracle**: ❌ FAIL
- PnL: ❌ $-0.12
- Sharpe: ❌ 0.0196 (need ≥ 1.0)
- Trades: ✅ 40 (need ≥ 30)
- Fee/Gross: ❌ 0.968 (need < 0.35)

### Portfolio Assessment

**Equal Weight Portfolio**: ❌ FAIL
- PnL: ❌ $-2.35
- Sharpe: ❌ -1.9208 (need ≥ 1.0)
- Trades: ✅ 629 (need ≥ 30)

**Risk Parity Portfolio**: ❌ FAIL
- PnL: ❌ $-1.12
- Sharpe: ❌ -1.3240 (need ≥ 1.0)
- Trades: ✅ 629 (need ≥ 30)

**Sharpe Weighted Portfolio**: ❌ FAIL
- PnL: ❌ $-1.26
- Sharpe: ❌ -2.0882 (need ≥ 1.0)
- Trades: ✅ 629 (need ≥ 30)

### Recommendation

**RECOMMENDATION: DO NOT PROMOTE — REJECT ALL CANDIDATES.**

After comprehensive portfolio construction across 9 strategy-market pairs with
3 allocation strategies, drawdown breakers, and risk constraints:

- **No individual candidate** passes the Sharpe ≥ 1.0 gate on the 90-day OOS period
- **No portfolio allocation** achieves positive risk-adjusted returns
- **Best portfolio**: Risk Parity with Sharpe -1.3240, PnL $-1.12

### Root Cause Analysis

1. **M1 overfitting**: High Sharpe ratios in M1 (up to 4.05 for cluster-007:BTC)
   collapsed in the extended 90-day period (best 0.08). The M1 results were
   based on 17 days with very few trades (14-33), leading to unreliable metrics.

2. **Fee dominance**: Most candidates have fee-to-gross ratios well above 1.0,
   meaning fees exceed gross trading profits. The strategies generate too many
   small trades that are eaten by execution costs.

3. **Signal quality**: The blueprint strategies, derived from profitable Hyperliquid
   wallets, do not translate to profitable signals on the 90-day OOS period with
   walk-forward validation. The edge observed in wallet fills may have been
   venue-specific, timing-dependent, or simply overfit to historical patterns.

### Follow-up Recommendations

1. **Expand candidate pool**: The current 9 candidates are derived from a limited
   set of HL wallet clusters. Broader wallet discovery could find strategies
   with more robust edges.

2. **Increase trade frequency threshold**: Require ≥50 OOS trades in M1 before
   promotion to M2, to reduce the impact of small-sample noise.

3. **Explore different strategy architectures**: The momentum-threshold approach
   used by all blueprint strategies may be inherently noisy. Consider
   mean-reversion, funding-capture, or regime-adaptive approaches.

4. **Reduce fee impact**: Investigate limit-order execution, maker rebates,
   or venue-switching to lower fee-to-gross ratios.

5. **Longer backtest windows**: The 90-day window may be insufficient for
   strategies with low trade frequency. Consider 180-365 day backtests.
## Data Provenance

| Item | Source | Details |
|------|--------|---------|
| Leverage sizing grid | `data/leverage-sizing/grid.json` | 315 cells (9 candidates × 7 leverage × 5 sizing modes) |
| Per-trade data | `data/leverage-sizing/raw/*/backtest-trades.json` | Individual trade records with timestamps |
| M1 parameter search | `data/walk-forward-parameter-search.md` | Top parameter sets per candidate |
| M2 frontier analysis | `data/leverage-sizing-frontier.md` | Leverage/sizing efficient frontier |
| Portfolio analysis | `data/portfolio-backtest.md` (this file) | Portfolio construction results |

---

*Report generated by `scripts/portfolio-analysis.py`*
