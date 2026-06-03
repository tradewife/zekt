# Liquidity Memory Map Report

**Generated:** 2026-06-03T07:03:43.006604+00:00
**Assertion:** VAL-REPORTS-002

## Overview

This report presents the liquidity memory map built from captured liquidation zone data. 
Zones are classified by lifecycle behavior (Magnet, Reversal, Inactive) and ranked by quality score.

## Zone Count by Classification

| Classification | Count | Percentage |
|---------------|-------|------------|
| Magnet | 0 | 0.0% |
| Reversal | 0 | 0.0% |
| Untested | 59 | 98.3% |
| Inactive | 1 | 1.7% |
| **Total** | **60** | **100%** |

## BTC Zones (Mark Price: $66,841.50)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $33,227.48 – $33,561.42 | long | Untested | 0.45 | 0.1575 | 0 | 0 | 0.00 | 0.00 | 5004 |
| 2 | $54,699.64 – $55,249.38 | long | Untested | 0.42 | 0.1456 | 0 | 0 | 0.00 | 0.00 | 1775 |
| 3 | $26,279.03 – $26,543.14 | long | Untested | 0.41 | 0.1442 | 0 | 0 | 0.00 | 0.00 | 6049 |
| 4 | $86,668.35 – $87,539.38 | short | Untested | 0.40 | 0.1404 | 0 | 0 | 0.00 | 0.00 | 3031 |
| 5 | $77,086.56 – $77,861.29 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.30 | 1591 |
| 6 | $147,694.56 – $149,178.92 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 12207 |
| 7 | $184,576.89 – $186,431.93 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 17753 |
| 8 | $247,124.24 – $249,607.90 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 27158 |
| 9 | $414,081.42 – $418,243.05 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 52261 |
| 10 | $701,898.17 – $708,952.42 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 95537 |

### BTC Zone Lifecycle Evidence

#### Zone 1: $33,227.48 – $33,561.42 (Untested)

- **Side at Risk:** long
- **Confidence:** 0.4500
- **Sources:** hyperliquid_positions, oi_imbalance
- **Estimated Notional:** $495,842.27
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1575
- **Distance from Price:** 5004 bps

#### Zone 2: $54,699.64 – $55,249.38 (Untested)

- **Side at Risk:** long
- **Confidence:** 0.4160
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $9,864,557.17
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1456
- **Distance from Price:** 1775 bps

#### Zone 3: $26,279.03 – $26,543.14 (Untested)

- **Side at Risk:** long
- **Confidence:** 0.4119
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $15,608,674.46
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1442
- **Distance from Price:** 6049 bps

#### Zone 4: $86,668.35 – $87,539.38 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4012
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $1,314,824.98
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1404
- **Distance from Price:** 3031 bps

#### Zone 5: $77,086.56 – $77,861.29 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4000
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $109,904.96
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.3000
- **Quality Score:** 0.1400
- **Distance from Price:** 1591 bps

## ETH Zones (Mark Price: $1,867.85)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $2,833.10 – $2,861.58 | short | Untested | 0.42 | 0.1457 | 0 | 0 | 0.00 | 0.00 | 5244 |
| 2 | $4,270.98 – $4,313.91 | short | Untested | 0.41 | 0.1426 | 0 | 0 | 0.00 | 0.00 | 12981 |
| 3 | $2,596.92 – $2,623.02 | short | Untested | 0.41 | 0.1423 | 0 | 0 | 0.00 | 0.00 | 3973 |
| 4 | $2,459.92 – $2,484.64 | short | Untested | 0.40 | 0.1412 | 0 | 0 | 0.00 | 0.00 | 3236 |
| 5 | $5,048.48 – $5,099.22 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 17164 |
| 6 | $5,936.57 – $5,996.23 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 21943 |
| 7 | $8,621.57 – $8,708.22 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 36390 |
| 8 | $9,557.26 – $9,653.31 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 41424 |
| 9 | $24,761.90 – $25,010.77 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 123235 |
| 10 | $32,304.60 – $32,629.27 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 163820 |

### ETH Zone Lifecycle Evidence

#### Zone 1: $2,833.10 – $2,861.58 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4163
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $42,879,652.05
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1457
- **Distance from Price:** 5244 bps

#### Zone 2: $4,270.98 – $4,313.91 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4075
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $1,403,730.43
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1426
- **Distance from Price:** 12981 bps

#### Zone 3: $2,596.92 – $2,623.02 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4066
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $4,563,712.20
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1423
- **Distance from Price:** 3973 bps

#### Zone 4: $2,459.92 – $2,484.64 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4035
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $2,219,082.64
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1412
- **Distance from Price:** 3236 bps

#### Zone 5: $5,048.48 – $5,099.22 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4000
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $737.89
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1400
- **Distance from Price:** 17164 bps

## SOL Zones (Mark Price: $74.79)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $174.12 – $175.87 | short | Untested | 0.40 | 0.1405 | 0 | 0 | 0.00 | 0.00 | 13397 |
| 2 | $541.27 – $546.71 | short | Untested | 0.40 | 0.1405 | 0 | 0 | 0.00 | 0.00 | 62733 |
| 3 | $101.91 – $102.94 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 3694 |
| 4 | $280.04 – $282.86 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 27631 |
| 5 | $399.97 – $403.99 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 43746 |
| 6 | $437.55 – $441.95 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 48796 |
| 7 | $980.14 – $990.00 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 121707 |
| 8 | $1,069.20 – $1,079.95 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 133674 |
| 9 | $1,931.69 – $1,951.10 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 249571 |
| 10 | $11,654.44 – $11,771.57 | short | Untested | 0.40 | 0.1400 | 0 | 0 | 0.00 | 0.00 | 1556067 |

### SOL Zone Lifecycle Evidence

#### Zone 1: $174.12 – $175.87 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4013
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $1,344,796.39
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1405
- **Distance from Price:** 13397 bps

#### Zone 2: $541.27 – $546.71 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4015
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $1,400,468.95
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1405
- **Distance from Price:** 62733 bps

#### Zone 3: $101.91 – $102.94 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4000
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $22,566.52
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1400
- **Distance from Price:** 3694 bps

#### Zone 4: $280.04 – $282.86 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4000
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $47,249.06
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1400
- **Distance from Price:** 27631 bps

#### Zone 5: $399.97 – $403.99 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.4000
- **Sources:** hyperliquid_positions
- **Estimated Notional:** $55,728.98
- **Age:** 1550 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1400
- **Distance from Price:** 43746 bps

## Time-Based Evolution Summary

Zone evolution across the capture period:

### BTC
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-06-03 07:02:16 UTC
- **Duration:** 71.3 hours
- **Snapshots:** 1550
- **First Zones:** 1 | **Last Zones:** 22
- **Confidence Range:** 0.40 – 0.45

### ETH
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-06-03 07:02:16 UTC
- **Duration:** 71.3 hours
- **Snapshots:** 1550
- **First Zones:** 1 | **Last Zones:** 20
- **Confidence Range:** 0.30 – 0.42

### SOL
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-06-03 07:02:16 UTC
- **Duration:** 71.3 hours
- **Snapshots:** 1550
- **First Zones:** 1 | **Last Zones:** 18
- **Confidence Range:** 0.30 – 0.40

## Decay Curves

Zone quality decay over time based on touch frequency and age:

| Symbol | Zone Price | Initial Quality | Current Quality | Decay Score | Status |
|--------|-----------|----------------|-----------------|-------------|--------|
| BTC | $33,394.45 | 0.4500 | 0.1575 | 0.0000 | Active |
| BTC | $54,974.51 | 0.4160 | 0.1456 | 0.0000 | Active |
| BTC | $26,411.09 | 0.4119 | 0.1442 | 0.0000 | Active |
| ETH | $2,847.34 | 0.4163 | 0.1457 | 0.0000 | Active |
| ETH | $4,292.44 | 0.4075 | 0.1426 | 0.0000 | Active |
| ETH | $2,609.97 | 0.4066 | 0.1423 | 0.0000 | Active |
| SOL | $174.99 | 0.4013 | 0.1405 | 0.0000 | Active |
| SOL | $543.99 | 0.4015 | 0.1405 | 0.0000 | Active |
| SOL | $102.42 | 0.4000 | 0.1400 | 0.0000 | Active |

## Data Source Coverage

All zones in this report are sourced from the following captured data:

- **Snapshots:** `data/liquidation-zones/`
- **Total Snapshots Processed:** 4650
- **Capture Duration:** 71.3 hours
- **Symbols:** BTC, ETH, SOL
- **Data Sources:** hyperliquid_fills, hyperliquid_positions, oi_imbalance

### Data Limitations

1. **Capture duration:** 71.3 hours of continuous data captured
2. **Multi-source zones:** 1/60 (1.7%) are multi-source
3. **Few near-price zones:** Most zones are far from current price (deep liquidation levels)
4. **Synthetic lifecycle data:** Zone touch/sweep/reversal rates are simulated from mark price proximity
5. **Confidence range:** 0.30–0.48 (moderate)

---
*Report generated by `scripts/generate_validation_reports.py`*
*Data source: `data/liquidation-zones/`*