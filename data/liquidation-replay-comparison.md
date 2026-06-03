# Liquidation Replay Comparison Report — Rust Binary Results

**Generated:** 2026-06-03T08:50:00Z
**Binary:** `target/release/zekt --liquidation-replay`
**Data:** 4,740 snapshots from `data/liquidation-zones/` (~72h capture, BTC/ETH/SOL)
**Starting Balance:** $1,000

## Executive Summary

**All 4 liquidation-zone strategies produce ZERO trades** when run through the actual Rust binary replay pipeline with real strategy entry/exit logic. The previous Python-based replay evaluation (reported in `data/liquidation-event-replay.md`) bypassed the strategies' strict entry gates, producing simulated results that do not reflect the real strategy behavior.

This is the definitive replay result using production Rust strategy implementations.

## Strategy Results

| Strategy | Data Points | Trades | Win Rate | Net PnL | Sharpe | Sortino | Calmar | Max DD | Gate |
|----------|-------------|--------|----------|---------|--------|---------|--------|--------|------|
| liquidation-cascade-continuation | 4,734 | **0** | 0.0% | $0.00 | 0.0000 | 0.0000 | 0.0000 | 0.00% | ❌ Denied (6/12) |
| sweep-reclaim | 4,740 | **0** | 0.0% | $0.00 | 0.0000 | 0.0000 | 0.0000 | 0.00% | ❌ Denied (6/12) |
| liquidity-memory-fisher | 4,740 | **0** | 0.0% | $0.00 | 0.0000 | 0.0000 | 0.0000 | 0.00% | ❌ Denied (6/12) |
| liquidation-zone-arbiter | 4,740 | **0** | 0.0% | $0.00 | 0.0000 | 0.0000 | 0.0000 | 0.00% | ❌ Denied (6/12) |

### Promotion Gate Results (identical across all 4 strategies)

| # | Criterion | Result |
|---|-----------|--------|
| 1 | Positive net expectancy after fees | ❌ $0.00 (no trades) |
| 2 | Max drawdown ≤ 10% | ✅ 0.00% |
| 3 | Zero stale-data trades | ✅ 0 |
| 4 | Zero duplicate pending trades | ✅ 0 |
| 5 | ≥ 30 signal events | ❌ 0 (no signals) |
| 6 | Sharpe ≥ 1.0 | ❌ 0.0000 |
| 7 | Fee/gross < 35% | ✅ 0.00% |
| 8 | No single event > 25% profit | ✅ OK |
| 9 | Fishing improves expectancy | ✅ +0.05 delta |
| 10 | Pyramiding improves risk-adjusted return | ❌ -$239.89 unrealized |
| 11 | Route cost doesn't consume edge | ❌ 0.00% |
| 12 | Liquidation distance safe | ❌ 0.0 bps |

## Root Cause Analysis

### Why Zero Signals Fire

All 4 strategies enforce multi-gate entry logic. The cascade-continuation strategy requires ALL of:
1. `confidence_min ≥ 0.5` — **FAILS**: max captured zone confidence is 0.468
2. `max_distance_bps ≤ 200` — Only 2.8% of zones qualify
3. `volume_zscore_min ≥ 1.5` — Default replay value is 2.5 (passes)
4. `max_spread_bps ≤ 20` — Default replay value is 0.1 (passes)
5. `min_depth_usd ≥ 50,000` — Default replay value is 50,000 (passes)
6. Regime compatibility — Default is "Trending" (passes for cascade-continuation)

**Primary blocker: Zone confidence.** No captured zone achieves the minimum confidence threshold (0.5 for cascade, 0.6 for sweep-reclaim).

### Captured Data Quality

Analysis of 207 sampled snapshots (4620 zones):

| Metric | Value | Assessment |
|--------|-------|------------|
| Total zones sampled | 4,620 | Plentiful |
| Mean zone confidence | 0.398 | Below all strategy thresholds |
| Max zone confidence | 0.468 | Below even the lowest threshold (0.5) |
| Zones with confidence ≥ 0.5 | **0** (0%) | Critical gap |
| Near-price zones (≤200 bps) | 134 (2.8%) | All from HL fills, all confidence 0.41-0.43 |
| Multi-source zones | 25 (0.5%) | Insufficient corroboration |
| Source distribution | 93% HL positions, 3.7% OI, 2.8% fills | Single-source dominated |

### Confidence Gap Detail

The confidence scoring formula:
- Base confidence: 0.40 (single source)
- Multi-source bonus: +0.15 (2nd source), +0.10 (3rd), +0.10 (4th)
- Max achievable without multi-source: 0.40 (base) + wallet/notional bonuses (~0.02-0.05)
- Near-price zones from HL fills: 0.41-0.43 (single source + small wallet bonus)
- **Required for any strategy entry**: ≥ 0.50 (cascade) or ≥ 0.60 (sweep-reclaim)
- **Gap**: 0.07-0.19 below threshold

The only way to reach 0.5+ confidence is with multi-source corroboration, but only 0.5% of zones have 2+ sources.

### Near-Price Zones

The 134 near-price zones (within 200 bps) are exclusively from HL fills data:
- All single-source (`hyperliquid_fills` only)
- Confidence range: 0.41-0.43
- Side: 87% long-side risk, 13% short-side risk
- Symbol: ~70% BTC, ~20% ETH, ~10% SOL

These zones are close enough to price to be actionable, but their confidence is too low to pass any strategy's entry gate.

## Comparison: Rust Binary vs Python Simulation

| Metric | Python Simulation | Rust Binary |
|--------|------------------|-------------|
| cascade-continuation trades | 108 | **0** |
| sweep-reclaim trades | 117 | **0** |
| liquidity-memory-fisher trades | 100 | **0** |
| liquidation-zone-arbiter trades | 81 | **0** |
| Best net PnL (sweep-reclaim) | +$981 | **$0** |
| Best Sharpe (sweep-reclaim) | 0.48 | **0.00** |
| Best gate result | 11/12 (sweep, arbiter) | **6/12 (all)** |

The Python simulation (`scripts/generate_validation_reports.py`) used synthetic strategy logic that bypassed the multi-gate entry conditions. It simulated entries based on generic signal conditions rather than the actual Rust strategy implementations. The Rust binary correctly enforces all entry gates (confidence, distance, volume z-score, spread, depth, regime, route cost), resulting in zero signal generation.

## Recommendation

### **REJECT** — No strategy is promotable

The Rust binary replay definitively shows that:

1. **No strategy produces any trades** with the current captured data quality
2. **Zone confidence is the universal blocker** — max captured confidence (0.47) is below all strategy thresholds (0.5-0.6)
3. **Multi-source corroboration is insufficient** — only 0.5% of zones have 2+ sources, which is the only path to confidence ≥ 0.5
4. **Near-price zones exist** (134 from HL fills) but are single-source with insufficient confidence

### Required Changes Before Re-evaluation

1. **Improve multi-source fusion**: The capture pipeline must produce zones corroborated by 2+ sources (positions + fills, or fills + OI, etc.) to achieve confidence ≥ 0.5
2. **Expand HL fills watchlist**: All 134 near-price zones come from HL fills. More fills wallets = more near-price zones = more potential multi-source fusion
3. **Lower confidence thresholds** (not recommended — would weaken strategy safety): Reduce `min_confidence` to 0.35 to match captured data range
4. **Add depth fragility source**: Currently contributing 0% of zones. Depth fragility could corroborate near-price fill zones to push confidence above threshold

### Verdict

The liquidation zone exploitation architecture is sound — the strategies are correctly implemented with proper safety gates. The problem is data quality, not strategy logic. Without multi-source zone corroboration, the confidence scores remain too low for any strategy to fire.

**This is the correct outcome for a safety-first design: the strategies correctly refuse to trade when data quality is insufficient.**

---
*Report generated by Rust binary replay: `./target/release/zekt --liquidation-replay --strategy <name> --snapshot-dir data/liquidation-zones/ --starting-balance 1000`*
*JSON results: `data/liquidation-replay-results/`*
