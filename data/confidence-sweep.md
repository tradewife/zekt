# Confidence Threshold Sweep Report

**Date:** 2026-06-03
**Data:** 4,848 liquidation zone snapshots (BTC: 1,618, ETH: 1,615, SOL: 1,615)
**Span:** ~72 hours of continuous capture
**Binary:** `target/release/zekt --liquidation-replay`
**Purpose:** Find confidence thresholds where strategies start producing trades.

## Executive Summary

A parameter sweep across confidence thresholds [0.10–0.45] was run for all 4 liquidation zone strategies, with additional gate relaxation as needed. **Only 2 of 4 strategies produce any trades**, and neither achieves positive net expectancy.

| Strategy | Best Config | Trades | Win Rate | Net PnL | Sharpe | Gate |
|----------|-------------|--------|----------|---------|--------|------|
| cascade-continuation | conf=0.30, dist=50% | 5 | 40.0% | -$3,521 | -0.44 | 5/12 |
| liquidity-memory-fisher | conf=0.25, cascade_cancel=false | 4 | 50.0% | -$144 | -0.04 | 5/12 |
| sweep-reclaim | conf=0.05, all gates off | 0 | — | $0 | 0.00 | 6/12 |
| liquidation-zone-arbiter | conf=0.05, all gates off | 0 | — | $0 | 0.00 | 6/12 |

**Verdict: No strategy/threshold combo achieves positive net expectancy.** Confidence relaxation alone is insufficient. The fundamental issue is insufficient data quality (max confidence 0.47, mostly single-source zones far from current price).

---

## 1. Methodology

### Sweep Parameters

- **Confidence thresholds:** [0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.45]
- **Strategies:** cascade-continuation, sweep-reclaim, liquidity-memory-fisher, liquidation-zone-arbiter
- **Total runs:** 28 (8 thresholds × 2 producing strategies + 8 thresholds × 2 non-producing strategies with relaxed gates)

### Confidence Field Names Per Strategy

Each strategy uses a different field name for confidence threshold:

| Strategy | Field Name | Default |
|----------|-----------|---------|
| cascade-continuation | `confidence_min` | 0.60 |
| sweep-reclaim | `min_confidence` | 0.60 |
| liquidity-memory-fisher | `min_confidence` | 0.30 |
| liquidation-zone-arbiter | `min_zone_confidence` | 0.60 |

### Critical Discovery: `enabled: true` Required

All liquidation strategies default to `enabled: false`. The `--param-override` must include `"enabled": true` or the strategy immediately returns `NoSignal` regardless of other parameters. This was the first blocker discovered.

### Additional Gate Relaxation

For strategies producing 0 trades with confidence relaxation alone, additional gates were relaxed:

| Strategy | Additional Overrides | Why |
|----------|---------------------|-----|
| cascade-continuation | `max_distance_to_zone_pct: 50.0` | Zones are 16-50% from current price |
| liquidity-memory-fisher | `cascade_cancel_enabled: false` | Zones trigger cascade cancel, killing all orders |
| sweep-reclaim | `forced_flow_spike_threshold: 0.001`, `oi_contraction_required: false`, `vwap_reclaim_required: false`, `regime_filter: false`, `volume_z_score_threshold: 0.0` | Multi-phase state machine requires specific patterns |
| liquidation-zone-arbiter | `forced_flow_threshold: 0.0`, `exhaustion_required: false`, `regime_filter_enabled: false` | Routes to blocked sub-strategies |

---

## 2. Heatmap: Strategies × Thresholds

### Trade Count

| Strategy \ Conf | 0.10 | 0.15 | 0.20 | 0.25 | 0.30 | 0.35 | 0.40 | 0.45 |
|----------------|------|------|------|------|------|------|------|------|
| cascade (dist=5%) | 2 | 2 | 2 | 2 | 2 | 2 | 2 | 1 |
| cascade (dist=50%) | **5** | **5** | **5** | **5** | **5** | **5** | **5** | 2 |
| fisher (cascade_cancel=false) | **4** | **4** | **4** | **4** | **4** | **4** | **4** | 0 |
| sweep-reclaim | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| zone-arbiter | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

### Net PnL ($)

| Strategy \ Conf | 0.10 | 0.15 | 0.20 | 0.25 | 0.30 | 0.35 | 0.40 | 0.45 |
|----------------|------|------|------|------|------|------|------|------|
| cascade (dist=50%) | -3521 | -3521 | -3521 | -3521 | -3521 | -3521 | -3521 | -3478 |
| fisher (cascade_cancel=false) | -144 | -144 | -144 | -144 | -144 | -144 | -144 | $0 |

### Sharpe Ratio

| Strategy \ Conf | 0.10 | 0.15 | 0.20 | 0.25 | 0.30 | 0.35 | 0.40 | 0.45 |
|----------------|------|------|------|------|------|------|------|------|
| cascade (dist=50%) | -0.44 | -0.44 | -0.44 | -0.44 | -0.44 | -0.44 | -0.44 | -33.62 |
| fisher (cascade_cancel=false) | -0.04 | -0.04 | -0.04 | -0.04 | -0.04 | -0.04 | -0.04 | 0.00 |

---

## 3. Per-Strategy Analysis

### 3.1 Cascade-Continuation

**Producing config:** `confidence_min: 0.30, max_distance_to_zone_pct: 50.0, enabled: true`

| Metric | Value |
|--------|-------|
| Trades | 5 (2W / 3L) |
| Win rate | 40.0% |
| Net PnL | -$3,521.08 |
| Sharpe | -0.4412 |
| Sortino | -0.6238 |
| Calmar | -0.0098 |
| Max drawdown | -$93,206 (9,320%) |
| Fee/gross | 0.03% |
| Gate | 5/12 ❌ Denied |

**Key findings:**
- Distance gate is the primary limiter. Default `max_distance_to_zone_pct: 5.0` only allows zones within 5% of current price, but most liquidation zones are 16-50% away.
- Increasing to 50% unlocks 3 additional trades (from 2 to 5).
- All trades at conf 0.10-0.40 are identical — the same zones trigger regardless of threshold because all zones in the 0.30-0.47 range pass at any threshold ≤ 0.40.
- At conf 0.45, only 2 zones qualify (those with confidence ≥ 0.45), reducing trade count.
- The strategy's Sharpe of -0.44 is negative but not catastrophic — the sample is too small (5 trades) for statistical significance.
- **Max drawdown of 9,320%** indicates the strategy opens positions far exceeding the starting balance ($1,000). The clip size ($100) is reasonable but cumulative PnL tracking produces extreme drawdown percentages.

**Promotion criteria passed:**
- ✅ Zero stale-data trades
- ✅ Zero duplicate pending trades
- ✅ Fee/gross ratio < 35% (0.03%)
- ✅ No single-trade dominance
- ✅ Fishing improves expectancy

**Promotion criteria failed:**
- ❌ Positive net expectancy (-$704/trade)
- ❌ Max drawdown ≤ 10%
- ❌ ≥ 30 signal events (only 5)
- ❌ Sharpe ≥ 1.0 (-0.44)
- ❌ Pyramiding improves risk-adjusted return
- ❌ Route cost check
- ❌ Liquidation distance check

### 3.2 Liquidity-Memory-Fisher

**Producing config:** `min_confidence: 0.25, cascade_cancel_enabled: false, regime_filter: false, enabled: true`

| Metric | Value |
|--------|-------|
| Trades | 4 (2W / 2L) |
| Win rate | 50.0% |
| Net PnL | -$144.18 |
| Sharpe | -0.0404 |
| Sortino | -0.0570 |
| Calmar | -0.0008 |
| Max drawdown | -$44,911 (4,491%) |
| Fee/gross | 0.32% |
| Gate | 5/12 ❌ Denied |

**Key findings:**
- `cascade_cancel_enabled` is the critical gate. Default `true` causes all fishing orders to be cancelled whenever liquidation zones exist (which is always in the captured data).
- With cascade cancel disabled, the fisher places passive orders at zone offsets and catches fills when price moves toward zones.
- 50% win rate is encouraging but the sample is too small (4 trades).
- Net loss of -$144 on 4 trades = -$36/trade average, driven by fees + adverse selection.
- Sharpe of -0.04 is near-zero — the strategy is roughly breakeven after fees, leaning slightly negative.

**Promotion criteria passed:**
- ✅ Zero stale-data trades
- ✅ Zero duplicate pending trades
- ✅ Fee/gross ratio < 35% (0.32%)
- ✅ No single-trade dominance
- ✅ Fishing improves expectancy

**Promotion criteria failed:**
- ❌ Positive net expectancy (-$36/trade)
- ❌ Max drawdown ≤ 10%
- ❌ ≥ 30 signal events (only 4)
- ❌ Sharpe ≥ 1.0 (-0.04)
- ❌ Pyramiding improves risk-adjusted return
- ❌ Route cost check
- ❌ Liquidation distance check

### 3.3 Sweep-Reclaim

**No trades produced at any threshold or parameter combination.**

The sweep-reclaim strategy uses a multi-phase state machine:
1. **Idle → Fishing**: Requires zone sweep detection (price crosses zone + reversal velocity) + forced-flow spike
2. **Fishing → Confirmation**: Passive orders placed, waiting for fill
3. **Confirmation → Entry**: VWAP reclaim + depth refill + spread normalization + OI contraction

Even with ALL gates disabled (forced_flow_spike_threshold=0.001, oi_contraction_required=false, vwap_reclaim_required=false, volume_z_score_threshold=0.0, spread_max_pct=100, depth_min_usd=0, regime_filter=false, velocity_deceleration_threshold=100), the strategy produces 0 trades.

**Root cause:** The `detect_zone_sweep` method requires price to cross through a zone and then show reversal velocity. The replay data's price velocity (computed from MomentumDetector's price history) doesn't exhibit the sweep-and-reverse pattern that the strategy looks for. The prices change slowly between snapshots (avg 0.04%, max 0.38%), so the velocity never indicates a sharp reversal.

### 3.4 Liquidation-Zone-Arbiter

**No trades produced at any threshold or parameter combination.**

The arbiter routes to sub-strategies based on regime + zone state:
- Trending + forced flow → cascade-continuation
- HighVol + exhaustion → sweep-reclaim
- LowVol + quality zones → memory-fisher

The arbiter itself has relaxed gates, but the sub-strategies it delegates to still have their own entry gates. Since cascade-continuation requires specific zone proximity + velocity patterns, sweep-reclaim requires sweep detection, and fisher requires cascade cancel disabled, the arbiter cannot route to any producing sub-strategy.

---

## 4. Data Quality Assessment

### Zone Confidence Distribution

| Range | Count | Percentage |
|-------|-------|------------|
| 0.30–0.40 | 97 | 3.9% |
| 0.40–0.50 | 2,413 | 96.1% |
| **Total** | **2,510** | **100%** |

No zones exceed 0.47 confidence. All zones are single-source (primarily HL positions or HL fills).

### Zone Distance from Current Price

| Symbol | Near-price zones (within 5%) | Total zones |
|--------|------------------------------|-------------|
| BTC | 1,699 | ~4,800 |
| ETH | 598 | ~4,800 |
| SOL | 716 | ~4,800 |

Only 3,013 of ~14,400 zones (21%) are within 5% of the current price — the actionable range for default strategy parameters.

### Side Distribution

| Side | Count | Percentage |
|------|-------|------------|
| short | 1,826 | 72.7% |
| long | 684 | 27.3% |

Most zones indicate shorts are at risk (price may rise through these liquidation levels).

---

## 5. Parameter Sensitivity

### Confidence Threshold

Confidence threshold has a **step-function** effect, not gradual:
- Thresholds 0.10–0.40: All zones with confidence 0.30–0.47 pass, producing the same trades
- Threshold 0.45: Only zones with confidence ≥ 0.45 pass, reducing trade count
- This is because the captured confidence distribution is narrow (0.30–0.47)

### Distance Threshold (cascade-continuation only)

| max_distance_to_zone_pct | Trades | Net PnL |
|--------------------------|--------|---------|
| 5% (default) | 2 | -$3,478 |
| 50% | 5 | -$3,521 |
| 50% + no regime | 5 | -$3,521 |

Increasing distance from 5% to 50% adds 3 trades but makes the net PnL slightly worse.

### Cascade Cancel (fisher only)

| cascade_cancel_enabled | Trades | Net PnL |
|------------------------|--------|---------|
| true (default) | 0 | $0 |
| false | 4 | -$144 |

Disabling cascade cancel is essential for the fisher to produce any trades at all.

---

## 6. Optimal Threshold Per Strategy

| Strategy | Optimal Threshold | Additional Overrides Needed | Trades | Net PnL | Sharpe |
|----------|-------------------|----------------------------|--------|---------|--------|
| cascade-continuation | 0.30 | max_distance=50%, enabled=true | 5 | -$3,521 | -0.44 |
| liquidity-memory-fisher | 0.25 | cascade_cancel=false, enabled=true | 4 | -$144 | -0.04 |
| sweep-reclaim | — | No config produces trades | 0 | $0 | 0.00 |
| liquidation-zone-arbiter | — | No config produces trades | 0 | $0 | 0.00 |

**The fisher at conf=0.25 is the closest to breakeven** (Sharpe -0.04, loss of only $144 on 4 trades). However, it still fails the promotion gate on 7/12 criteria.

---

## 7. Recommendations

### Immediate (Current Data)

1. **Liquidity-memory-fisher at conf=0.25** is the most promising configuration, with near-zero Sharpe and small losses. With more data and longer capture, this strategy might reach breakeven or slight profitability.

2. **Do not promote any strategy** to paper trading. No configuration achieves positive net expectancy.

### Data Quality Improvements

3. **Multi-source fusion is the #1 priority.** All zones are single-source with confidence ≤ 0.47. Achieving confidence ≥ 0.5 requires 2+ source corroboration. This would unlock the default thresholds.

4. **Near-price zone capture.** Only 21% of zones are within 5% of the current price. Capturing zones closer to the action (e.g., from HL fill analysis with expanded watchlist) would increase signal density.

5. **Longer capture (168h+).** More data points increase the chance of observing sweep-and-reverse patterns that sweep-reclaim needs. The current 72h capture may be insufficient for event-driven strategies.

### Architecture Improvements

6. **Fisher: Add cascade cancel exemption for single-source zones.** Currently, ALL fishing orders are cancelled when any zone exists. A smarter approach would only cancel on high-confidence multi-source cascade signals.

7. **Replay pipeline: Use actual L2 depth data from snapshots** instead of hardcoded 50,000 USD. Some snapshots have L2 book data that could be used for more realistic depth checks.

8. **Sweep-reclaim: Consider synthetic sweep injection.** The current replay data doesn't produce the sweep pattern. An alternative validation approach would inject synthetic sweep events into the replay pipeline.

---

## 8. Command Reference

### Producing Configurations

```bash
# Cascade-continuation (best: 5 trades)
./target/release/zekt --liquidation-replay \
    --strategy cascade-continuation \
    --param-override '{"enabled": true, "confidence_min": 0.30, "max_distance_to_zone_pct": 50.0}'

# Liquidity-memory-fisher (best: 4 trades, smallest loss)
./target/release/zekt --liquidation-replay \
    --strategy liquidity-memory-fisher \
    --param-override '{"enabled": true, "min_confidence": 0.25, "cascade_cancel_enabled": false, "regime_filter": false}'
```

### Sweep-Reclaim (0 trades even with all gates disabled)

```bash
./target/release/zekt --liquidation-replay \
    --strategy sweep-reclaim \
    --param-override '{"enabled": true, "min_confidence": 0.05, "forced_flow_spike_threshold": 0.001, "oi_contraction_required": false, "volume_z_score_threshold": 0.0, "spread_max_pct": 100.0, "depth_min_usd": 0.0, "regime_filter": false, "vwap_reclaim_required": false, "velocity_deceleration_threshold": 100.0}'
```

---

## 9. Files Generated

| File | Description |
|------|-------------|
| `data/confidence-sweep-results/*.txt` | Individual run outputs (20 initial + expanded runs) |
| `data/confidence-sweep-results/best_configs.txt` | Detailed output for best cascade and fisher configs |
| `data/confidence-sweep.md` | This report |
| `scripts/confidence-sweep.sh` | Sweep runner script |
