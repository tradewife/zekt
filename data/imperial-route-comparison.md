# Imperial Route Oracle vs Flash-Only: Backtest Comparison

Comparison of all 10 blueprint strategies under `flash-only` vs `imperial-route-oracle` cost modes.

**Backtest parameters:**
- Period: 2026-04-01 → 2026-05-30 (~60 days)
- Markets: BTC, SOL, ETH
- Interval: 5m
- Starting balance: $1,000
- Fee rate: 0.1% per side (Flash base taker)
- Regime filter: enabled

Data source: Real Hyperliquid candle data via `candleSnapshot` API.

## Summary

| Metric | Flash-Only | Imperial-Route-Oracle | Delta |
|--------|-----------|----------------------|-------|
| **Total Net PnL** | $-342.24 | $-120.35 | $221.89 |
| **Total Fees** | $651.67 | $154.24 | $497.43 |
| **Total Trades** | 3493 | 3493 | — |
| **Profitable Pairs** | 5/30 | 10/30 | +5 |
| **Turned Positive** | — | 5 | Strategies Imperial routing flipped from loss to profit |
| **Near Break-Even (|net| < $50)** | 27 | — | Candidates for promotion with better routing |

## Ranked Results (sorted by Imperial Net PnL)

| # | Strategy | Mkt | Flash Net$ | Imp Net$ | PnL Δ | Flash Fees | Imp Fees | Fee Δ | Flash Sharpe | Imp Sharpe | Trades | Win% (F/I) | Max DD (F/I) | Promo |
|---|----------|-----|-----------|---------|-------|-----------|---------|-------|-------------|-----------|--------|-----------|-------------|-------|
| 1 | blueprint-cluster-007 ⚡ | BTC | $2.22 | $4.24 | $2.02 | $3.94 | $1.02 | $2.93 | 0.22 | 0.39 | 13 | 38.5%/61.5% | $1.80/$1.49 | ✅ |
| 2 | blueprint-cluster-005 ⚡ 🔄 | ETH | $-13.16 | $3.40 | $16.56 | $38.22 | $4.80 | $33.42 | -0.14 | 0.04 | 66 | 36.4%/47.0% | $19.45/$8.05 | ✅ |
| 3 | blueprint-cluster-005 ⚡ 🔄 | SOL | $-0.05 | $1.20 | $1.24 | $2.89 | $0.41 | $2.48 | -0.01 | 0.21 | 5 | 40.0%/40.0% | $2.38/$1.64 | ✅ |
| 4 | blueprint-cluster-009 ⚡ | SOL | $0.77 | $0.89 | $0.12 | $0.28 | $0.05 | $0.23 | 2.63 | 3.02 | 2 | 100.0%/100.0% | $0.00/$0.00 | ✅ |
| 5 | blueprint-cluster-003 ⚡ | BTC | $0.66 | $0.83 | $0.18 | $0.54 | $0.21 | $0.33 | 0.68 | 0.86 | 5 | 80.0%/80.0% | $0.17/$0.13 | ✅ |
| 6 | blueprint-cluster-009 ⚡ | ETH | $0.50 | $0.70 | $0.21 | $0.43 | $0.05 | $0.38 | 0.64 | 0.91 | 3 | 66.7%/66.7% | $0.12/$0.05 | ✅ |
| 7 | blueprint-cluster-008 ⚡ 🔄 | BTC | $-0.05 | $0.46 | $0.51 | $1.20 | $0.28 | $0.92 | -0.02 | 0.18 | 10 | 50.0%/50.0% | $0.54/$0.37 | ✅ |
| 8 | blueprint-cluster-002 ⚡ | SOL | $0.16 | $0.25 | $0.10 | $0.20 | $0.02 | $0.18 | 0.61 | 1.09 | 2 | 50.0%/100.0% | $0.01/$0.00 | ✅ |
| 9 | blueprint-cluster-007 ⚡ 🔄 | SOL | $-0.06 | $0.23 | $0.29 | $0.83 | $0.29 | $0.53 | -0.01 | 0.04 | 3 | 33.3%/33.3% | $1.87/$1.67 | ✅ |
| 10 | blueprint-cluster-002 ⚡ 🔄 | BTC | $-0.00 | $0.07 | $0.07 | $0.18 | $0.05 | $0.14 | -0.00 | 0.09 | 2 | 50.0%/50.0% | $0.26/$0.22 | ✅ |
| 11 | blueprint-cluster-006 ⚡ | SOL | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | 0.00 | 0.00 | 0 | 0.0%/0.0% | $0.00/$0.00 | ❌ |
| 12 | blueprint-cluster-006 ⚡ | ETH | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | 0.00 | 0.00 | 0 | 0.0%/0.0% | $0.00/$0.00 | ❌ |
| 13 | blueprint-cluster-009 ⚡ | BTC | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | 0.00 | 0.00 | 0 | 0.0%/0.0% | $0.00/$0.00 | ❌ |
| 14 | blueprint-cluster-006 ⚡ | BTC | $-0.07 | $-0.05 | $0.02 | $0.12 | $0.09 | $0.03 | 0.00 | 0.00 | 1 | 0.0%/0.0% | $0.07/$0.05 | ❌ |
| 15 | blueprint-cluster-002 ⚡ | ETH | $-0.21 | $-0.09 | $0.12 | $0.28 | $0.04 | $0.24 | -0.26 | -0.11 | 3 | 66.7%/66.7% | $0.38/$0.34 | ❌ |
| 16 | blueprint-mean-revert ⚡ | SOL | $-3.98 | $-0.27 | $3.71 | $8.01 | $0.75 | $7.27 | -0.27 | -0.02 | 33 | 51.5%/51.5% | $5.30/$3.03 | ❌ |
| 17 | blueprint-cluster-003 ⚡ | SOL | $-0.39 | $-0.34 | $0.05 | $0.10 | $0.01 | $0.09 | 0.00 | 0.00 | 1 | 0.0%/0.0% | $0.39/$0.34 | ❌ |
| 18 | blueprint-cluster-008 ⚡ | SOL | $-0.45 | $-0.40 | $0.05 | $0.13 | $0.03 | $0.09 | 0.00 | 0.00 | 1 | 0.0%/0.0% | $0.45/$0.40 | ❌ |
| 19 | blueprint-cluster-008 ⚡ | ETH | $-1.35 | $-0.57 | $0.78 | $1.58 | $0.18 | $1.40 | -0.38 | -0.16 | 13 | 30.8%/30.8% | $1.38/$0.82 | ❌ |
| 20 | blueprint-mean-revert ⚡ | BTC | $-1.40 | $-0.76 | $0.63 | $1.70 | $0.44 | $1.26 | -0.43 | -0.23 | 7 | 42.9%/42.9% | $1.40/$0.82 | ❌ |
| 21 | blueprint-cluster-007 ⚡ | ETH | $-3.78 | $-1.22 | $2.56 | $5.32 | $1.31 | $4.01 | -0.22 | -0.07 | 18 | 38.9%/50.0% | $5.02/$4.22 | ❌ |
| 22 | blueprint-cluster-003 ⚡ | ETH | $-1.67 | $-1.30 | $0.37 | $0.85 | $0.16 | $0.69 | -0.95 | -0.74 | 9 | 11.1%/11.1% | $1.67/$1.30 | ❌ |
| 23 | blueprint-scalper ⚡ | ETH | $-29.57 | $-4.15 | $25.43 | $57.53 | $6.62 | $50.91 | -0.67 | -0.09 | 448 | 15.6%/41.3% | $29.57/$4.69 | ❌ |
| 24 | blueprint-mean-revert ⚡ | ETH | $-7.67 | $-4.86 | $2.81 | $6.34 | $0.88 | $5.46 | -0.66 | -0.42 | 26 | 34.6%/38.5% | $7.67/$4.86 | ❌ |
| 25 | blueprint-scalper ⚡ | SOL | $-38.76 | $-7.83 | $30.92 | $67.16 | $5.49 | $61.67 | -0.81 | -0.16 | 523 | 15.5%/43.0% | $39.08/$8.52 | ❌ |
| 26 | blueprint-scalper ⚡ | BTC | $-26.23 | $-7.88 | $18.35 | $48.93 | $12.06 | $36.87 | -1.03 | -0.31 | 381 | 12.1%/34.4% | $26.29/$8.15 | ❌ |
| 27 | blueprint-cluster-005 ⚡ | BTC | $-22.63 | $-12.57 | $10.07 | $27.58 | $7.14 | $20.44 | -0.51 | -0.28 | 47 | 27.7%/40.4% | $24.58/$14.94 | ❌ |
| 28 | blueprint-hft-market-maker | ETH | $-68.03 | $-23.33 | $44.70 | $127.86 | $26.81 | $101.05 | -0.89 | -0.30 | 634 | 11.0%/35.6% | $68.03/$23.41 | ❌ |
| 29 | blueprint-hft-market-maker | BTC | $-58.86 | $-29.13 | $29.73 | $117.98 | $44.07 | $73.90 | -0.95 | -0.46 | 585 | 10.4%/26.0% | $58.86/$29.46 | ❌ |
| 30 | blueprint-hft-market-maker | SOL | $-68.16 | $-37.85 | $30.31 | $131.49 | $40.98 | $90.51 | -0.69 | -0.37 | 652 | 18.9%/31.3% | $68.16/$37.85 | ❌ |

## Near Break-Even Analysis (|Flash Net PnL| < $50)

These strategies are close to profitability and may become profitable with better execution routing.

| Strategy | Mkt | Flash Net$ | Imp Net$ | PnL Δ | Flash Fees | Imp Fees | Fee Savings | Status |
|----------|-----|-----------|---------|-------|-----------|---------|-------------|--------|
| blueprint-cluster-007 | BTC | $2.22 | $4.24 | $2.02 | $3.94 | $1.02 | $2.93 | Promoted ✅ |
| blueprint-cluster-005 | ETH | $-13.16 | $3.40 | $16.56 | $38.22 | $4.80 | $33.42 | Promoted ✅ |
| blueprint-cluster-005 | SOL | $-0.05 | $1.20 | $1.24 | $2.89 | $0.41 | $2.48 | Promoted ✅ |
| blueprint-cluster-009 | SOL | $0.77 | $0.89 | $0.12 | $0.28 | $0.05 | $0.23 | Promoted ✅ |
| blueprint-cluster-003 | BTC | $0.66 | $0.83 | $0.18 | $0.54 | $0.21 | $0.33 | Promoted ✅ |
| blueprint-cluster-009 | ETH | $0.50 | $0.70 | $0.21 | $0.43 | $0.05 | $0.38 | Promoted ✅ |
| blueprint-cluster-008 | BTC | $-0.05 | $0.46 | $0.51 | $1.20 | $0.28 | $0.92 | Promoted ✅ |
| blueprint-cluster-002 | SOL | $0.16 | $0.25 | $0.10 | $0.20 | $0.02 | $0.18 | Promoted ✅ |
| blueprint-cluster-007 | SOL | $-0.06 | $0.23 | $0.29 | $0.83 | $0.29 | $0.53 | Promoted ✅ |
| blueprint-cluster-002 | BTC | $-0.00 | $0.07 | $0.07 | $0.18 | $0.05 | $0.14 | Promoted ✅ |
| blueprint-cluster-006 | SOL | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | Still negative ❌ |
| blueprint-cluster-006 | ETH | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | Still negative ❌ |
| blueprint-cluster-009 | BTC | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | $0.00 | Still negative ❌ |
| blueprint-cluster-006 | BTC | $-0.07 | $-0.05 | $0.02 | $0.12 | $0.09 | $0.03 | Still negative ❌ |
| blueprint-cluster-002 | ETH | $-0.21 | $-0.09 | $0.12 | $0.28 | $0.04 | $0.24 | Still negative ❌ |
| blueprint-mean-revert | SOL | $-3.98 | $-0.27 | $3.71 | $8.01 | $0.75 | $7.27 | Still negative ❌ |
| blueprint-cluster-003 | SOL | $-0.39 | $-0.34 | $0.05 | $0.10 | $0.01 | $0.09 | Still negative ❌ |
| blueprint-cluster-008 | SOL | $-0.45 | $-0.40 | $0.05 | $0.13 | $0.03 | $0.09 | Still negative ❌ |
| blueprint-cluster-008 | ETH | $-1.35 | $-0.57 | $0.78 | $1.58 | $0.18 | $1.40 | Still negative ❌ |
| blueprint-mean-revert | BTC | $-1.40 | $-0.76 | $0.63 | $1.70 | $0.44 | $1.26 | Still negative ❌ |
| blueprint-cluster-007 | ETH | $-3.78 | $-1.22 | $2.56 | $5.32 | $1.31 | $4.01 | Still negative ❌ |
| blueprint-cluster-003 | ETH | $-1.67 | $-1.30 | $0.37 | $0.85 | $0.16 | $0.69 | Still negative ❌ |
| blueprint-scalper | ETH | $-29.57 | $-4.15 | $25.43 | $57.53 | $6.62 | $50.91 | Still negative ❌ |
| blueprint-mean-revert | ETH | $-7.67 | $-4.86 | $2.81 | $6.34 | $0.88 | $5.46 | Still negative ❌ |
| blueprint-scalper | SOL | $-38.76 | $-7.83 | $30.92 | $67.16 | $5.49 | $61.67 | Still negative ❌ |
| blueprint-scalper | BTC | $-26.23 | $-7.88 | $18.35 | $48.93 | $12.06 | $36.87 | Still negative ❌ |
| blueprint-cluster-005 | BTC | $-22.63 | $-12.57 | $10.07 | $27.58 | $7.14 | $20.44 | Still negative ❌ |

## Promotion Status

Strategies with positive Imperial net PnL are candidates for paper/live promotion.

| Strategy | Market | Imp Net$ | Imp Sharpe | Trades | Status |
|----------|--------|---------|-----------|--------|--------|
| blueprint-cluster-007 | BTC | $4.24 | 0.39 | 13 | 🟢 Promotable |
| blueprint-cluster-005 | ETH | $3.40 | 0.04 | 66 | 🟢 Promotable |
| blueprint-cluster-005 | SOL | $1.20 | 0.21 | 5 | 🟢 Promotable |
| blueprint-cluster-009 | SOL | $0.89 | 3.02 | 2 | 🟢 Promotable |
| blueprint-cluster-003 | BTC | $0.83 | 0.86 | 5 | 🟢 Promotable |
| blueprint-cluster-009 | ETH | $0.70 | 0.91 | 3 | 🟢 Promotable |
| blueprint-cluster-008 | BTC | $0.46 | 0.18 | 10 | 🟢 Promotable |
| blueprint-cluster-002 | SOL | $0.25 | 1.09 | 2 | 🟢 Promotable |
| blueprint-cluster-007 | SOL | $0.23 | 0.04 | 3 | 🟢 Promotable |
| blueprint-cluster-002 | BTC | $0.07 | 0.09 | 2 | 🟢 Promotable |
| blueprint-cluster-006 | SOL | $0.00 | 0.00 | 0 | 🔴 Not promotable |
| blueprint-cluster-006 | ETH | $0.00 | 0.00 | 0 | 🔴 Not promotable |
| blueprint-cluster-009 | BTC | $0.00 | 0.00 | 0 | 🔴 Not promotable |
| blueprint-cluster-006 | BTC | $-0.05 | 0.00 | 1 | 🔴 Not promotable |
| blueprint-cluster-002 | ETH | $-0.09 | -0.11 | 3 | 🔴 Not promotable |
| blueprint-mean-revert | SOL | $-0.27 | -0.02 | 33 | 🔴 Not promotable |
| blueprint-cluster-003 | SOL | $-0.34 | 0.00 | 1 | 🔴 Not promotable |
| blueprint-cluster-008 | SOL | $-0.40 | 0.00 | 1 | 🔴 Not promotable |
| blueprint-cluster-008 | ETH | $-0.57 | -0.16 | 13 | 🔴 Not promotable |
| blueprint-mean-revert | BTC | $-0.76 | -0.23 | 7 | 🔴 Not promotable |
| blueprint-cluster-007 | ETH | $-1.22 | -0.07 | 18 | 🔴 Not promotable |
| blueprint-cluster-003 | ETH | $-1.30 | -0.74 | 9 | 🔴 Not promotable |
| blueprint-scalper | ETH | $-4.15 | -0.09 | 448 | 🔴 Not promotable |
| blueprint-mean-revert | ETH | $-4.86 | -0.42 | 26 | 🔴 Not promotable |
| blueprint-scalper | SOL | $-7.83 | -0.16 | 523 | 🔴 Not promotable |
| blueprint-scalper | BTC | $-7.88 | -0.31 | 381 | 🔴 Not promotable |
| blueprint-cluster-005 | BTC | $-12.57 | -0.28 | 47 | 🔴 Not promotable |
| blueprint-hft-market-maker | ETH | $-23.33 | -0.30 | 634 | 🔴 Not promotable |
| blueprint-hft-market-maker | BTC | $-29.13 | -0.46 | 585 | 🔴 Not promotable |
| blueprint-hft-market-maker | SOL | $-37.85 | -0.37 | 652 | 🔴 Not promotable |

## Key Findings

1. **Imperial routing flipped 5 strategy-market pair(s) from loss to profit.**
2. **Total fee savings with Imperial routing: $497.43** (76.3% reduction)
3. **10/30 strategy-market pairs are profitable under Imperial routing** vs 5/30 under flash-only.
4. **None of the 30 strategy-market pairs meet the Sharpe ≥ 1.0 threshold** under either cost mode. All strategies require parameter tuning or are fundamentally not suited for this backtest period.
5. **Best Imperial performer:** blueprint-cluster-007:BTC with $4.24 net PnL
6. **Worst Imperial performer:** blueprint-hft-market-maker:SOL with $-37.85 net PnL

## Methodology

- **Flash-only:** Uses Flash Trade base taker fee (0.1% per side) for all trades.
- **Imperial-route-oracle:** Uses `RouteCostOracle` to compare execution costs across Solana perps venues (Flash Trade, Drift, Zeta, others via Imperial API). When a cheaper route is found, the lower fee is used. When no route data is available, falls back to Flash fees.
- **Veto:** When the oracle determines routing costs exceed the strategy's edge budget, the trade is blocked.
- **Fallback:** When oracle data is stale or missing, Flash-only fees are used as fallback.

---
*Generated on 2026-05-31 from real Hyperliquid candle data. All backtests use $1,000 starting balance, 5m interval, regime filter enabled.*
