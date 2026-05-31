# Liquidity Memory Map Report

**Generated:** 2026-05-31T15:08:44.424205+00:00
**Assertion:** VAL-REPORTS-002

## Overview

This report presents the liquidity memory map built from captured liquidation zone data. 
Zones are classified by lifecycle behavior (Magnet, Reversal, Inactive) and ranked by quality score.

## Zone Count by Classification

| Classification | Count | Percentage |
|---------------|-------|------------|
| Magnet | 0 | 0.0% |
| Reversal | 0 | 0.0% |
| Untested | 3 | 100.0% |
| Inactive | 0 | 0.0% |
| **Total** | **3** | **100%** |

## BTC Zones (Mark Price: $73,769.50)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $110,100.98 – $111,207.52 | short | Untested | 0.30 | 0.1050 | 0 | 0 | 0.00 | 0.00 | 5000 |

### BTC Zone Lifecycle Evidence

#### Zone 1: $110,100.98 – $111,207.52 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.3000
- **Sources:** oi_imbalance
- **Estimated Notional:** $130,553.86
- **Age:** 8 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1050
- **Distance from Price:** 5000 bps

## ETH Zones (Mark Price: $2,016.45)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $3,009.55 – $3,039.80 | short | Untested | 0.30 | 0.1050 | 0 | 0 | 0.00 | 0.00 | 5000 |

### ETH Zone Lifecycle Evidence

#### Zone 1: $3,009.55 – $3,039.80 (Untested)

- **Side at Risk:** short
- **Confidence:** 0.3000
- **Sources:** oi_imbalance
- **Estimated Notional:** $4,381.82
- **Age:** 8 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1050
- **Distance from Price:** 5000 bps

## SOL Zones (Mark Price: $82.56)

### Top Zones by Quality Score

| Rank | Price Range | Side at Risk | Type | Confidence | Quality | Touches | Sweeps | Rev Rate | Decay | Distance (bps) |
|------|-------------|-------------|------|------------|---------|---------|--------|----------|-------|----------------|
| 1 | $41.07 – $41.48 | long | Untested | 0.30 | 0.1050 | 0 | 0 | 0.00 | 0.00 | 5000 |

### SOL Zone Lifecycle Evidence

#### Zone 1: $41.07 – $41.48 (Untested)

- **Side at Risk:** long
- **Confidence:** 0.3000
- **Sources:** oi_imbalance
- **Estimated Notional:** $45,452.03
- **Age:** 8 ticks
- **Touches:** 0 | **Sweeps:** 0
- **Reversal Rate:** 0.00 | **Continuation Rate:** 0.00
- **Avg Excursion (after touch):** $0.00
- **Avg Time-to-Touch:** 0.0s
- **Decay Score:** 0.0000
- **Quality Score:** 0.1050
- **Distance from Price:** 5000 bps

## Time-Based Evolution Summary

Zone evolution across the capture period:

### BTC
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-05-31 10:21:20 UTC
- **Duration:** 2.6 hours
- **Snapshots:** 8
- **First Zones:** 1 | **Last Zones:** 1
- **Confidence Range:** 0.30 – 0.30

### ETH
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-05-31 10:21:20 UTC
- **Duration:** 2.6 hours
- **Snapshots:** 8
- **First Zones:** 1 | **Last Zones:** 1
- **Confidence Range:** 0.30 – 0.30

### SOL
- **First Capture:** 2026-05-31 07:46:02 UTC
- **Last Capture:** 2026-05-31 10:21:20 UTC
- **Duration:** 2.6 hours
- **Snapshots:** 8
- **First Zones:** 1 | **Last Zones:** 1
- **Confidence Range:** 0.30 – 0.30

## Decay Curves

Zone quality decay over time based on touch frequency and age:

| Symbol | Zone Price | Initial Quality | Current Quality | Decay Score | Status |
|--------|-----------|----------------|-----------------|-------------|--------|
| BTC | $110,654.25 | 0.3000 | 0.1050 | 0.0000 | Active |
| ETH | $3,024.68 | 0.3000 | 0.1050 | 0.0000 | Active |
| SOL | $41.28 | 0.3000 | 0.1050 | 0.0000 | Active |

## Data Source Coverage

All zones in this report are sourced from the following captured data:

- **Snapshots:** `data/liquidation-zones/`
- **Total Snapshots Processed:** 24
- **Symbols:** BTC, ETH, SOL
- **Primary Source:** OI imbalance (100% of zones)
- **Multi-source Zones:** 0 (all zones are single-source)

### Data Limitations

1. **Capture duration:** Only ~2.6 hours of continuous data captured
2. **Single-source dependency:** All zones derived from OI imbalance only
3. **No fill burst data:** HL fills were not captured (no wallet watchlist active)
4. **Limited lifecycle data:** Zone touch/sweep counts based on 8 capture cycles
5. **Low confidence:** All zones at 0.30-0.40 confidence (moderate)

---
*Report generated by `scripts/generate_validation_reports.py`*
*Data source: `data/liquidation-zones/`*