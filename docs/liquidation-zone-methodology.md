# Liquidation Zone Methodology

**Status:** Complete (Milestones M3–M4)
**Date:** 2026-05-31
**Modules:** `src/liquidation.rs`, `src/strategy.rs` (liquidation-cascade-hunter), `src/replay.rs`

## Overview

The Liquidation Zone Intelligence layer detects, captures, and exploits liquidation cascades across Solana perps markets. The system is **paper-only** and **capture-first**: data is collected and persisted before any trading signals are generated.

## Data Model

### LiquidationZoneSnapshot
```
{
  "symbol": "BTC",
  "timestamp_ms": 1770000000000,
  "mark_price": 100000.0,
  "zones": [...]
}
```

### LiquidationZone
```
{
  "price": 98000.0,
  "side_at_risk": "long",
  "estimated_notional_usd": 5000000.0,
  "wallet_count": 42,
  "distance_bps": 200.0,
  "confidence": 0.75,
  "source_mix": ["hyperliquid_positions", "oi_imbalance"]
}
```

## Data Sources

### 1. Hyperliquid Positions (`hyperliquid_positions`)
- **Method:** `clearinghouseState` for known wallets
- **Output:** Aggregate liquidation prices clustered by price level
- **Confidence base:** 0.25
- **Clusters:** Positions within `cluster_threshold_bps` (default 200 bps) merged into zones
- **Strengths:** Direct observation of liquidation levels
- **Limitations:** Only covers known wallets, not all positions

### 2. Hyperliquid Fills (`hyperliquid_fills`)
- **Method:** `userFills`/`userFillsByTime` burst detection
- **Output:** Zones from rapid forced-liquidation fill sequences
- **Confidence base:** 0.25
- **Detection:** 10+ fills within 60 seconds at similar prices
- **Strengths:** Captures actual liquidation events
- **Limitations:** Retrospective only — detects after the fact

### 3. Imperial OI Imbalance (`oi_imbalance`)
- **Method:** `GET /api/v1/stats/markets` — long/short OI asymmetry
- **Output:** Directional zones based on crowded side
- **Confidence base:** 0.25
- **Threshold:** OI imbalance > 20% of dominant side
- **Strengths:** Market-wide positioning view
- **Limitations:** No price-level specificity — only direction

### 4. Imperial Depth Fragility (`depth_fragility`)
- **Method:** `GET /api/v1/phoenix/depth` — thin orderbook detection
- **Output:** Zones at price levels where liquidity drops off
- **Confidence base:** 0.25
- **Detection:** Zero-notional gaps in the orderbook
- **Strengths:** Identifies structural fragility
- **Limitations:** Phoenix depth only, not all venues

## Confidence Scoring

```
confidence = base_per_source
           + multi_source_bonus (0.15 per additional source)
           + notional_bonus (log10(notional / 1_000_000).max(0.0))
           - staleness_penalty (if source data is stale)
```

- **Range:** Clamped to [0.0, 1.0]
- **Max (4 sources):** 0.25 + 0.15×3 + notional bonus ≈ 0.70+
- **Min threshold:** Configurable, default `min_confidence = 0.3`

## Capture Process

1. **Interval:** Configurable, default 30 seconds per cycle
2. **Fetch:** Query all configured sources for each symbol
3. **Fuse:** Merge zones from different sources at similar prices (within `merge_threshold_bps`)
4. **Score:** Compute confidence per zone
5. **Persist:** Atomic write to `data/liquidation-zones/{symbol}_{timestamp_ms}.json`
6. **Cleanup:** Delete snapshots older than `retention_days` (default 7)

## Strategy: liquidation-cascade-hunter

### Setup Type 1: Cascade Continuation
- Price approaches a high-confidence liquidation zone
- Forced-flow velocity spikes (cascading liquidations)
- Direction aligns with cascade flow
- Entry in the direction of the cascade

### Setup Type 2: Exhaustion Reversal
- Liquidation burst has occurred (detected from fills)
- Price reclaims VWAP (smart money re-entering)
- Forced-flow velocity decays (cascade exhausting)
- Depth refills at support/resistance
- Entry against the cascade direction (reversal)

### Entry Gates (All Must Pass)
1. **Confidence gate:** Zone confidence ≥ `confidence_min` (default 0.6)
2. **Volume z-score:** ≥ `volume_z_score_threshold` (default 2.0)
3. **Distance gate:** Price within `max_distance_to_zone_pct` (default 5%)
4. **VWAP filter:** Price on correct side of VWAP
5. **Spread filter:** Spread ≤ `spread_max_pct` (default 0.5%)
6. **Depth filter:** Depth ≥ `depth_min_usd` (default $10K)
7. **Regime filter:** Compatible regime (Trending/HighVol OK, Choppy/LowVol blocked)
8. **Route cost veto:** Cost ≤ `route_cost_max_bps` (default 5 bps)
9. **Duplicate guard:** Max one pending per symbol/side
10. **Cooldown:** After any loss, wait `cooldown_after_loss_secs` (default 300s)

### Exit Logic
- **Take-profit:** `take_profit_pct` (default 1.5%)
- **Stop-loss:** `stop_loss_pct` (default 0.75%)
- **Trailing stop:** `trailing_stop_pct` (default 0.5%) after `trailing_activation_pct` (default 1.0%)
- **Time stop:** `max_hold_secs` (default 1800s)
- **Stale data exit:** Zone data older than `stale_data_threshold_secs` (default 300s)

## Replay Validation Pipeline

### Capture Phase
1. Run read-only capture for 24–72 hours
2. Collect: zones, route costs, mark prices, depth, OI stats
3. Persist to `data/liquidation-zones/`

### Replay Phase
1. Load captured snapshots from disk
2. Construct `MomentumSnapshot` with `MarketExtension` for each data point
3. Replay through `LiquidationCascadeHunter` strategy
4. Track: entries, exits, PnL, drawdown, stale trades, duplicates

### Promotion Gate (ALL Must Pass)
| Criterion | Threshold | Description |
|-----------|-----------|-------------|
| Net expectancy | > 0 USD | (win_rate × avg_win) - (loss_rate × avg_loss) - avg_route_cost |
| Max drawdown | ≤ 10% | Of starting balance |
| Stale-data trades | = 0 | No trades on stale zone data |
| Duplicate pending | = 0 | No duplicate symbol/side pending trades |
| Signal events | ≥ 30 | Minimum for statistical validity |
| Sharpe ratio | ≥ 1.0 | Annualized |

## Safety Constraints

- **Paper-only:** Strategy blocked from live engine (`paper_only = true`)
- **Disabled by default:** Must be explicitly enabled in config
- **No knife catching:** Requires confirmation filters (volume, VWAP, velocity)
- **Stale data protection:** Blocks entries and forces exits on stale zone data
- **Atomic writes:** All file persistence uses write-to-tmp + rename pattern
- **No live trading:** `--keypair` never used in any replay or paper mode

## Configuration

```toml
[liquidation]
enabled = false
capture_interval_secs = 30
symbols = ["BTC", "SOL", "ETH"]
snapshot_dir = "data/liquidation-zones"
retention_days = 7
sources = ["hl_positions", "hl_fills", "imperial_oi", "imperial_depth"]
cluster_threshold_bps = 200.0
merge_threshold_bps = 100.0
min_confidence = 0.3

[strategy.liquidation-cascade-hunter]
enabled = false
confidence_min = 0.6
volume_z_score_threshold = 2.0
max_distance_to_zone_pct = 5.0
spread_max_pct = 0.5
depth_min_usd = 10000.0
route_cost_max_bps = 5.0
stale_data_threshold_secs = 300
take_profit_pct = 1.5
stop_loss_pct = 0.75
trailing_stop_pct = 0.5
trailing_activation_pct = 1.0
max_hold_secs = 1800
clip_size_usd = 100.0
```
