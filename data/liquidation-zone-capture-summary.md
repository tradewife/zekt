# Liquidation Zone Capture Summary

**Date:** 2026-05-31
**Module:** `src/liquidation.rs`
**Status:** Infrastructure complete, capture not yet run

## Capture Engine

The liquidation zone capture engine is a tokio async task that runs at a configurable interval (default 30 seconds). Each cycle:

1. Fetches data from all configured sources (HL positions, HL fills, Imperial OI, Imperial depth)
2. Fuses multi-source data into `LiquidationZoneSnapshot` per symbol
3. Scores confidence per zone using multi-source corroboration
4. Persists snapshots to `data/liquidation-zones/{symbol}_{timestamp_ms}.json`

### Source Configuration

| Source | Config Name | Method | Default |
|--------|-------------|--------|---------|
| Hyperliquid Positions | `hl_positions` | `clearinghouseState` for known wallets | Enabled |
| Hyperliquid Fills | `hl_fills` | `userFillsByTime` burst detection | Enabled |
| Imperial OI Imbalance | `imperial_oi` | `GET /api/v1/stats/markets` | Enabled |
| Imperial Depth | `imperial_depth` | `GET /api/v1/phoenix/depth` | Enabled |

### Capture Statistics

Capture has not been run yet. The infrastructure is ready for a 24-72 hour capture run:

- **Symbols configured:** BTC, SOL, ETH
- **Interval:** 30 seconds
- **Retention:** 7 days
- **Snapshot directory:** `data/liquidation-zones/`

### Expected Capture Output

After a 24-hour capture run at 30-second intervals:
- **Snapshots per symbol:** ~2,880
- **Total snapshots (3 symbols):** ~8,640
- **Average snapshot size:** ~2-5 KB (depends on zone count)
- **Total disk usage:** ~20-40 MB per day

### Zone Confidence Distribution (Expected)

Based on the confidence scoring model:
- **1 source (base 0.25):** Low confidence — typically filtered out by `min_confidence = 0.3`
- **2 sources (0.25 + 0.15 = 0.40):** Moderate — passing minimum threshold
- **3 sources (0.25 + 0.30 = 0.55):** Good — usable for strategy entries
- **4 sources (0.25 + 0.45 = 0.70+):** High confidence — strongest signals

### Replay Validation

The `ReplayPipeline` (in `src/replay.rs`) can load these captured snapshots and replay them through the `liquidation-cascade-hunter` strategy. The promotion gate checks all criteria before the strategy can be promoted for paper trading.

## Files

- Snapshots: `data/liquidation-zones/{SYMBOL}_{timestamp_ms}.json`
- Replay reports: `data/replay-promotion-report.json` / `.md`
- Capture summary: This file (`data/liquidation-zone-capture-summary.md`)

## Running the Capture

```bash
# Enable capture in config/perps.toml:
# [liquidation]
# enabled = true
# capture_interval_secs = 30
# symbols = ["BTC", "SOL", "ETH"]

# Run the pipeline with capture enabled:
cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48
```

## Replay After Capture

```rust
use zekt::replay::ReplayPipeline;

// Load snapshots from capture
let snapshots = ReplayPipeline::load_snapshots("data/liquidation-zones/")?;
let points = ReplayPipeline::snapshots_to_replay_points(&snapshots);

// Run replay with strategy params
let pipeline = ReplayPipeline::new(params, gate_config);
let result = pipeline.run(&points);

// Generate promotion report
ReplayPipeline::write_markdown_report(&result, "data/replay-promotion-report.md")?;
```
