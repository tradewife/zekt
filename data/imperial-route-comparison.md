# Imperial Route Oracle — Before/After Comparison

Comparison of all 10 blueprint strategies under `flash-only` vs `imperial-route-oracle` cost modes.

## Ranked Strategy Table (sorted by imperial_net_pnl)

| Rank | Strategy | Market | Flash Net$ | Imperial Net$ | Δ PnL | Flash Fees | Imperial Fees | Δ Fees | Flash Sharpe | Imperial Sharpe | Veto | Improved | Venue Dist | Near BE? | Turned +? | Fee BPS (F) | Fee BPS (I) | Promotable |
|------|----------|--------|------------|---------------|-------|------------|---------------|--------|--------------|-----------------|------|----------|------------|----------|-----------|-------------|-------------|------------|
| 1 | blueprint-scalper | BTC | 67.50 | 71.28 | 3.78 | 15.12 | 11.34 | 3.78 | 6.69 | 7.23 | 0 | 7 | gmtrade:4, flash_trade:3, phoenix:3 |  |  | 1831 | 1373 | ✓ |
| 2 | blueprint-scalper | ETH | 54.00 | 57.27 | 3.27 | 13.10 | 9.82 | 3.28 | 6.18 | 6.68 | 0 | 6 | flash_trade:4, gmtrade:3, phoenix:3 |  |  | 1952 | 1464 | ✓ |
| 3 | blueprint-cluster-007 | BTC | 52.50 | 54.82 | 2.32 | 12.88 | 10.56 | 2.32 | 6.12 | 6.61 | 0 | 7 | flash_trade:3, phoenix:3, gmtrade:4 |  |  | 1969 | 1615 | ✓ |
| 4 | blueprint-scalper | SOL | 45.00 | 47.94 | 2.94 | 11.75 | 8.81 | 2.94 | 5.74 | 6.20 | 0 | 5 | phoenix:2, flash_trade:5, gmtrade:3 | ✓ |  | 2070 | 1553 | ✓ |
| 5 | blueprint-cluster-007 | ETH | 42.00 | 44.03 | 2.03 | 11.30 | 9.27 | 2.03 | 5.58 | 6.02 | 0 | 6 | phoenix:3, gmtrade:3, flash_trade:4 | ✓ |  | 2120 | 1738 | ✓ |
| 6 | blueprint-cluster-002 | BTC | 37.50 | 39.41 | 1.91 | 10.62 | 8.71 | 1.91 | 5.29 | 5.72 | 0 | 7 | phoenix:3, flash_trade:3, gmtrade:4 | ✓ |  | 2208 | 1810 | ✓ |
| 7 | blueprint-cluster-007 | SOL | 35.00 | 36.84 | 1.84 | 10.25 | 8.41 | 1.84 | 5.12 | 5.53 | 0 | 5 | flash_trade:5, phoenix:2, gmtrade:3 | ✓ |  | 2265 | 1857 | ✓ |
| 8 | blueprint-cluster-002 | ETH | 30.00 | 31.71 | 1.71 | 9.50 | 7.79 | 1.71 | 4.74 | 5.12 | 0 | 6 | phoenix:3, gmtrade:3, flash_trade:4 | ✓ |  | 2405 | 1972 | ✓ |
| 9 | blueprint-cluster-002 | SOL | 25.00 | 26.57 | 1.57 | 8.75 | 7.18 | 1.57 | 4.29 | 4.63 | 0 | 5 | gmtrade:3, phoenix:2, flash_trade:5 | ✓ |  | 2593 | 2126 | ✓ |
| 10 | blueprint-cluster-009 | BTC | 22.50 | 24.01 | 1.51 | 8.38 | 6.87 | 1.51 | 4.03 | 4.35 | 0 | 7 | phoenix:3, flash_trade:3, gmtrade:4 | ✓ |  | 2713 | 2224 | ✓ |
| 11 | blueprint-cluster-009 | ETH | 18.00 | 19.39 | 1.39 | 7.70 | 6.31 | 1.39 | 3.51 | 3.79 | 0 | 6 | flash_trade:4, gmtrade:3, phoenix:3 | ✓ |  | 2996 | 2457 | ✓ |
| 12 | blueprint-cluster-005 | BTC | 15.00 | 16.30 | 1.30 | 7.25 | 5.95 | 1.30 | 3.10 | 3.35 | 0 | 7 | gmtrade:4, phoenix:3, flash_trade:3 | ✓ |  | 3258 | 2672 | ✓ |
| 13 | blueprint-cluster-009 | SOL | 15.00 | 16.30 | 1.30 | 7.25 | 5.95 | 1.30 | 3.10 | 3.35 | 0 | 5 | phoenix:2, flash_trade:5, gmtrade:3 | ✓ |  | 3258 | 2672 | ✓ |
| 14 | blueprint-cluster-005 | ETH | 12.00 | 13.22 | 1.22 | 6.80 | 5.58 | 1.22 | 2.65 | 2.86 | 0 | 6 | flash_trade:4, phoenix:3, gmtrade:3 | ✓ |  | 3617 | 2966 | ✓ |
| 15 | blueprint-cluster-005 | SOL | 10.00 | 11.17 | 1.17 | 6.50 | 5.33 | 1.17 | 2.31 | 2.49 | 0 | 5 | phoenix:2, flash_trade:5, gmtrade:3 | ✓ |  | 3939 | 3230 | ✓ |
| 16 | blueprint-cluster-006 | SOL | -5.00 | -3.97 | 1.03 | 5.75 | 4.72 | 1.03 | -1.30 | -1.34 | 1 | 5 | flash_trade:5, phoenix:2, gmtrade:3 | ✓ |  | 76667 | 62867 | ✗ |
| 17 | blueprint-cluster-006 | ETH | -6.00 | -4.94 | 1.06 | 5.90 | 4.84 | 1.06 | -1.53 | -1.57 | 1 | 6 | phoenix:3, flash_trade:4, gmtrade:3 | ✓ |  | 590000 | 483800 | ✗ |
| 18 | blueprint-cluster-006 | BTC | -7.50 | -6.40 | 1.10 | 6.12 | 5.02 | 1.10 | -1.84 | -1.89 | 1 | 7 | gmtrade:4, flash_trade:3, phoenix:3 | ✓ |  | 44545 | 36527 | ✗ |
| 19 | blueprint-mean-revert | SOL | -15.00 | -13.70 | 1.30 | 7.25 | 5.95 | 1.30 | -3.10 | -3.20 | 1 | 5 | gmtrade:3, phoenix:2, flash_trade:5 | ✓ |  | 9355 | 7671 | ✗ |
| 20 | blueprint-mean-revert | ETH | -18.00 | -16.61 | 1.39 | 7.70 | 6.31 | 1.39 | -3.51 | -3.61 | 1 | 6 | flash_trade:4, phoenix:3, gmtrade:3 | ✓ |  | 7476 | 6130 | ✗ |
| 21 | blueprint-cluster-008 | SOL | -20.00 | -18.56 | 1.44 | 8.00 | 6.56 | 1.44 | -3.75 | -3.86 | 1 | 5 | gmtrade:3, phoenix:2, flash_trade:5 | ✓ |  | 6667 | 5467 | ✗ |
| 22 | blueprint-mean-revert | BTC | -22.50 | -20.99 | 1.51 | 8.38 | 6.87 | 1.51 | -4.03 | -4.15 | 2 | 7 | gmtrade:4, phoenix:3, flash_trade:3 | ✓ |  | 5929 | 4862 | ✗ |
| 23 | blueprint-cluster-008 | ETH | -24.00 | -22.45 | 1.55 | 8.60 | 7.05 | 1.55 | -4.19 | -4.31 | 2 | 6 | flash_trade:4, phoenix:3, gmtrade:3 | ✓ |  | 5584 | 4579 | ✗ |
| 24 | blueprint-cluster-003 | SOL | -30.00 | -28.29 | 1.71 | 9.50 | 7.79 | 1.71 | -4.74 | -4.88 | 2 | 5 | gmtrade:3, flash_trade:5, phoenix:2 | ✓ |  | 4634 | 3800 | ✗ |
| 25 | blueprint-cluster-008 | BTC | -30.00 | -28.29 | 1.71 | 9.50 | 7.79 | 1.71 | -4.74 | -4.88 | 2 | 7 | flash_trade:3, gmtrade:4, phoenix:3 | ✓ |  | 4634 | 3800 | ✗ |
| 26 | blueprint-cluster-003 | ETH | -36.00 | -34.13 | 1.87 | 10.40 | 8.53 | 1.87 | -5.19 | -5.35 | 2 | 6 | phoenix:3, flash_trade:4, gmtrade:3 | ✓ |  | 4062 | 3331 | ✗ |
| 27 | blueprint-hft-market-maker | SOL | -40.00 | -36.70 | 3.30 | 11.00 | 7.70 | 3.30 | -5.45 | -5.62 | 2 | 5 | gmtrade:3, flash_trade:5, phoenix:2 | ✓ |  | 3793 | 2655 | ✗ |
| 28 | blueprint-cluster-003 | BTC | -45.00 | -42.89 | 2.11 | 11.75 | 9.64 | 2.11 | -5.74 | -5.92 | 2 | 7 | flash_trade:3, phoenix:3, gmtrade:4 | ✓ |  | 3534 | 2898 | ✗ |
| 29 | blueprint-hft-market-maker | ETH | -48.00 | -44.34 | 3.66 | 12.20 | 8.54 | 3.66 | -5.90 | -6.08 | 2 | 6 | phoenix:3, flash_trade:4, gmtrade:3 | ✓ |  | 3408 | 2385 | ✗ |
| 30 | blueprint-hft-market-maker | BTC | -60.00 | -55.80 | 4.20 | 14.00 | 9.80 | 4.20 | -6.43 | -6.62 | 2 | 7 | flash_trade:3, phoenix:3, gmtrade:4 |  |  | 3043 | 2130 | ✗ |

## Near Break-Even Strategies (|flash net PnL| < $50)

- **blueprint-scalper / SOL**: flash=$45.00, imperial=$47.94, Δ=$2.94
- **blueprint-cluster-007 / ETH**: flash=$42.00, imperial=$44.03, Δ=$2.03
- **blueprint-cluster-002 / BTC**: flash=$37.50, imperial=$39.41, Δ=$1.91
- **blueprint-cluster-007 / SOL**: flash=$35.00, imperial=$36.84, Δ=$1.84
- **blueprint-cluster-002 / ETH**: flash=$30.00, imperial=$31.71, Δ=$1.71
- **blueprint-cluster-002 / SOL**: flash=$25.00, imperial=$26.57, Δ=$1.57
- **blueprint-cluster-009 / BTC**: flash=$22.50, imperial=$24.01, Δ=$1.51
- **blueprint-cluster-009 / ETH**: flash=$18.00, imperial=$19.39, Δ=$1.39
- **blueprint-cluster-005 / BTC**: flash=$15.00, imperial=$16.30, Δ=$1.30
- **blueprint-cluster-009 / SOL**: flash=$15.00, imperial=$16.30, Δ=$1.30
- **blueprint-cluster-005 / ETH**: flash=$12.00, imperial=$13.22, Δ=$1.22
- **blueprint-cluster-005 / SOL**: flash=$10.00, imperial=$11.17, Δ=$1.17
- **blueprint-cluster-006 / SOL**: flash=$-5.00, imperial=$-3.97, Δ=$1.03
- **blueprint-cluster-006 / ETH**: flash=$-6.00, imperial=$-4.94, Δ=$1.06
- **blueprint-cluster-006 / BTC**: flash=$-7.50, imperial=$-6.40, Δ=$1.10
- **blueprint-mean-revert / SOL**: flash=$-15.00, imperial=$-13.70, Δ=$1.30
- **blueprint-mean-revert / ETH**: flash=$-18.00, imperial=$-16.61, Δ=$1.39
- **blueprint-cluster-008 / SOL**: flash=$-20.00, imperial=$-18.56, Δ=$1.44
- **blueprint-mean-revert / BTC**: flash=$-22.50, imperial=$-20.99, Δ=$1.51
- **blueprint-cluster-008 / ETH**: flash=$-24.00, imperial=$-22.45, Δ=$1.55
- **blueprint-cluster-003 / SOL**: flash=$-30.00, imperial=$-28.29, Δ=$1.71
- **blueprint-cluster-008 / BTC**: flash=$-30.00, imperial=$-28.29, Δ=$1.71
- **blueprint-cluster-003 / ETH**: flash=$-36.00, imperial=$-34.13, Δ=$1.87
- **blueprint-hft-market-maker / SOL**: flash=$-40.00, imperial=$-36.70, Δ=$3.30
- **blueprint-cluster-003 / BTC**: flash=$-45.00, imperial=$-42.89, Δ=$2.11
- **blueprint-hft-market-maker / ETH**: flash=$-48.00, imperial=$-44.34, Δ=$3.66

## NOT PROMOTED (negative imperial net PnL)

- **blueprint-cluster-006 / SOL**: imperial_net=$-3.97, flash_net=$-5.00
- **blueprint-cluster-006 / ETH**: imperial_net=$-4.94, flash_net=$-6.00
- **blueprint-cluster-006 / BTC**: imperial_net=$-6.40, flash_net=$-7.50
- **blueprint-mean-revert / SOL**: imperial_net=$-13.70, flash_net=$-15.00
- **blueprint-mean-revert / ETH**: imperial_net=$-16.61, flash_net=$-18.00
- **blueprint-cluster-008 / SOL**: imperial_net=$-18.56, flash_net=$-20.00
- **blueprint-mean-revert / BTC**: imperial_net=$-20.99, flash_net=$-22.50
- **blueprint-cluster-008 / ETH**: imperial_net=$-22.45, flash_net=$-24.00
- **blueprint-cluster-003 / SOL**: imperial_net=$-28.29, flash_net=$-30.00
- **blueprint-cluster-008 / BTC**: imperial_net=$-28.29, flash_net=$-30.00
- **blueprint-cluster-003 / ETH**: imperial_net=$-34.13, flash_net=$-36.00
- **blueprint-hft-market-maker / SOL**: imperial_net=$-36.70, flash_net=$-40.00
- **blueprint-cluster-003 / BTC**: imperial_net=$-42.89, flash_net=$-45.00
- **blueprint-hft-market-maker / ETH**: imperial_net=$-44.34, flash_net=$-48.00
- **blueprint-hft-market-maker / BTC**: imperial_net=$-55.80, flash_net=$-60.00

## Summary

- Total strategy-market combinations: 30
- Promotable (positive imperial net PnL): 15/30
- Imperial routing turned positive: 0
- Near break-even strategies: 26
