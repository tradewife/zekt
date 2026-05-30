# Validation Contract: Liquidation-Zone Intelligence Capture

Feature area of the "Imperial Route Oracle + Liquidation-Zone Alpha Validation" mission (Workstream 4).
Covers: LiquidationZone data model, source fusion, confidence scoring, snapshot persistence, capture interval, source freshness, edge cases, and configuration.

> This module is **capture-only**. No assertions authorize trading decisions.

---

## LiquidationZone Data Model

### VAL-LIQ-001: LiquidationZoneSnapshot contains all required fields
When a `LiquidationZoneSnapshot` is constructed, it contains exactly the fields: `symbol` (String), `timestamp_ms` (i64), `mark_price` (f64), and `zones` (Vec\<LiquidationZone\>). All fields are present and correctly typed after serde serialization and deserialization.
Tool: cargo-test
Evidence: Construct a `LiquidationZoneSnapshot` with known values, serialize to JSON via `serde_json::to_string()`, deserialize back, and assert every field matches the original. Assert JSON keys are exactly `["symbol", "timestamp_ms", "mark_price", "zones"]`.

### VAL-LIQ-002: LiquidationZone contains all required fields
Each `LiquidationZone` in a snapshot contains: `price` (f64), `side_at_risk` (String: "long" or "short"), `estimated_notional_usd` (f64), `wallet_count` (u32), `distance_bps` (f64), `confidence` (f64, 0.0–1.0), `source_mix` (Vec\<String\>). All fields are present, correctly typed, and survive a serde round-trip.
Tool: cargo-test
Evidence: Construct a `LiquidationZone` with all fields set, serialize/deserialize, assert equality. Assert JSON keys include all seven required fields.

### VAL-LIQ-003: mark_price is strictly positive
`LiquidationZoneSnapshot.mark_price` must be > 0.0. Construction or deserialization of a snapshot with `mark_price <= 0.0` returns a validation error or is rejected by a `validate()` method.
Tool: cargo-test
Evidence: Attempt to construct a snapshot with `mark_price = 0.0`, `mark_price = -150.0`. Assert `validate()` returns `Err`. Construct with `mark_price = 150.25` and assert `validate()` returns `Ok(())`.

### VAL-LIQ-004: zone price is strictly positive
Each `LiquidationZone.price` must be > 0.0. A zone with `price <= 0.0` fails validation.
Tool: cargo-test
Evidence: Construct zones with `price = 0.0` and `price = -10.0`. Assert `validate()` returns `Err`. Construct with `price = 147.50` and assert `Ok(())`.

### VAL-LIQ-005: confidence is clamped to [0.0, 1.0]
`LiquidationZone.confidence` must be in the range `[0.0, 1.0]`. Values outside this range are either rejected at construction or clamped.
Tool: cargo-test
Evidence: Construct zones with `confidence = -0.1`, `confidence = 1.5`. Assert either rejection via `validate()` or clamping to 0.0 and 1.0 respectively. Assert `confidence = 0.0` and `confidence = 1.0` are accepted as-is.

### VAL-LIQ-006: side_at_risk is exactly "long" or "short"
`LiquidationZone.side_at_risk` must be either `"long"` or `"short"`. Any other value fails validation.
Tool: cargo-test
Evidence: Assert `validate()` returns `Ok` for `"long"` and `"short"`. Assert `validate()` returns `Err` for `"Long"`, `"LONG"`, `"shorts"`, `""`, `"neutral"`, `"both"`.

### VAL-LIQ-007: estimated_notional_usd is non-negative
`LiquidationZone.estimated_notional_usd` must be >= 0.0. A negative value fails validation.
Tool: cargo-test
Evidence: Assert `validate()` returns `Err` for `estimated_notional_usd = -100.0`. Assert `Ok` for `0.0` and `1_250_000.0`.

### VAL-LIQ-008: wallet_count is non-zero when estimated_notional_usd > 0
When `estimated_notional_usd > 0`, `wallet_count` must be >= 1. A zone with notional value but zero wallets is inconsistent and fails validation.
Tool: cargo-test
Evidence: Construct zone with `estimated_notional_usd = 500_000.0, wallet_count = 0`. Assert `validate()` returns `Err`. Construct with `wallet_count = 1` and assert `Ok`. Construct with `estimated_notional_usd = 0.0, wallet_count = 0` and assert `Ok` (empty zone is valid).

### VAL-LIQ-009: distance_bps is non-negative
`LiquidationZone.distance_bps` must be >= 0.0. A negative distance fails validation.
Tool: cargo-test
Evidence: Assert `validate()` returns `Err` for `distance_bps = -5.0`. Assert `Ok` for `0.0` and `183.0`.

### VAL-LIQ-010: source_mix is non-empty for any zone with confidence > 0
When `LiquidationZone.confidence > 0.0`, `source_mix` must contain at least one entry. A confident zone with no sources is invalid.
Tool: cargo-test
Evidence: Construct zone with `confidence = 0.5, source_mix = vec![]`. Assert `validate()` returns `Err`. Construct with `source_mix = vec!["hyperliquid_positions".to_string()]` and assert `Ok`. Construct with `confidence = 0.0, source_mix = vec![]` and assert `Ok` (zero-confidence empty zone is valid).

### VAL-LIQ-011: source_mix entries are drawn from a known source set
Every string in `source_mix` must be one of: `"hyperliquid_positions"`, `"hyperliquid_fills"`, `"oi_imbalance"`, `"depth_fragility"`. Unknown source names fail validation.
Tool: cargo-test
Evidence: Assert `validate()` returns `Ok` for all four known sources. Assert `validate()` returns `Err` for `"unknown_source"`, `"coinbase"`, `"manual"`.

### VAL-LIQ-012: snapshot symbol is a recognized market string
`LiquidationZoneSnapshot.symbol` must be a non-empty string matching a known market identifier (e.g., "BTC", "ETH", "SOL"). Empty strings or whitespace-only strings fail validation.
Tool: cargo-test
Evidence: Assert `validate()` returns `Err` for `""`, `"  "`. Assert `Ok` for `"BTC"`, `"ETH"`, `"SOL"`.

### VAL-LIQ-013: timestamp_ms is a reasonable Unix epoch millis value
`LiquidationZoneSnapshot.timestamp_ms` must be > 1_700_000_000_000 (post-2023) and < 2_000_000_000_000 (pre-2033). Values outside this range suggest a parsing error or uninitialized field.
Tool: cargo-test
Evidence: Assert `validate()` returns `Err` for `0`, `999_999_999_999`, `3_000_000_000_000`. Assert `Ok` for `1_770_000_000_000`.

### VAL-LIQ-014: zones vector can be empty
A `LiquidationZoneSnapshot` with `zones = vec![]` is valid. This represents a capture cycle where no liquidation clusters were detected. The snapshot is still persisted.
Tool: cargo-test
Evidence: Construct snapshot with `zones: vec![]`. Assert `validate()` returns `Ok`. Assert serialization succeeds and deserialization round-trips correctly.

---

## Source Fusion: Hyperliquid Positions

### VAL-LIQ-015: HL clearinghouseState positions are parsed into liquidation zones
Given a `clearinghouseState` response containing a wallet with an open BTC long position (entry price 100,000, leverage 5x), the liquidation price is extracted. If 42 wallets share a liquidation price within a configurable cluster threshold (e.g., ±50 bps), a single merged zone is produced at the median liquidation price with `wallet_count = 42`, `side_at_risk = "long"`, and `source_mix = ["hyperliquid_positions"]`.
Tool: cargo-test
Evidence: Provide synthetic `clearinghouseState` JSON for 42 wallets with BTC long positions whose liquidation prices cluster within ±50 bps of $95,000. Call the position aggregation function. Assert output zones has length 1. Assert `zones[0].wallet_count == 42`, `zones[0].side_at_risk == "long"`, `zones[0].source_mix == ["hyperliquid_positions"]`, `zones[0].price` is within ±50 bps of $95,000.

### VAL-LIQ-016: Long and short liquidation zones are separated
Wallets with long positions produce zones with `side_at_risk = "long"` (longs are liquidated when price drops). Wallets with short positions produce zones with `side_at_risk = "short"` (shorts are liquidated when price rises). A long and a short liquidation price at the same level produce two separate zones.
Tool: cargo-test
Evidence: Provide synthetic data: 10 wallets with long BTC liquidation near $95,000 and 8 wallets with short BTC liquidation near $105,000. Assert output has 2 zones. Assert one has `side_at_risk == "long"` and `wallet_count == 10`, the other has `side_at_risk == "short"` and `wallet_count == 8`.

### VAL-LIQ-017: Positions with missing or zero liquidation price are skipped
If a wallet's position in the `clearinghouseState` response has `liquidationPrice == null`, `""`, or `"0"`, it is silently skipped and does not contribute to any zone.
Tool: cargo-test
Evidence: Provide 5 wallets, 2 with valid liquidation prices and 3 with null/zero/empty. Assert output has zones contributed only by the 2 valid wallets. Assert no panic or error.

### VAL-LIQ-018: Multiple position clusters at different price levels produce distinct zones
If 20 wallets have BTC long liquidations near $95,000 and another 15 have liquidations near $92,000 (outside the cluster threshold), the output contains two separate zones at different prices.
Tool: cargo-test
Evidence: Provide 35 wallets in two distinct clusters (20 at ~$95,000, 15 at ~$92,000, separated by > cluster threshold). Assert output zones has length 2. Assert `zones[0].price` and `zones[1].price` differ by more than the cluster threshold.

### VAL-LIQ-019: estimated_notional_usd aggregates position notional values
For a zone with 10 wallets each holding $50,000 notional, `estimated_notional_usd` equals approximately $500,000. The aggregation sums the `position.value` (or `szi * mark_px`) from each contributing wallet.
Tool: cargo-test
Evidence: Provide 10 synthetic wallets each with BTC position size 0.5 BTC at $100,000 mark price ($50,000 notional each). Assert resulting zone has `estimated_notional_usd == 500_000.0`.

### VAL-LIQ-020: Empty clearinghouseState produces no zones
When the HL API returns an empty list of wallets or all wallets have no open positions, the position aggregation function returns an empty zones vector without error.
Tool: cargo-test
Evidence: Call aggregation with `vec![]` (no wallets). Assert result is `zones: vec![]`. Call with 3 wallets all having no open positions. Assert result is `zones: vec![]`.

### VAL-LIQ-021: API failure during position fetch returns error without panic
When the HL API call for `clearinghouseState` returns a network error, HTTP 5xx, or malformed JSON, the aggregation function returns an `Err` (logged at warn level) but does not panic. Other sources in the same capture cycle continue.
Tool: cargo-test
Evidence: Mock HL client to return 500 error. Assert function returns `Err`. Assert `tracing` capture at warn level contains relevant error context. Assert no panic.

---

## Source Fusion: Hyperliquid Fills (Forced-Flow Detection)

### VAL-LIQ-022: HL fills are scanned for forced-liquidation bursts
`userFills` / `userFillsByTime` responses are scanned for fill patterns indicating forced liquidations: rapid sequence of fills on the same side, with `closedPnl < 0`, within a short time window. A burst of 10+ liquidation fills within 60 seconds at similar prices generates a zone with `source_mix = ["hyperliquid_fills"]`.
Tool: cargo-test
Evidence: Provide synthetic fills: 12 fills in 45 seconds, all with `closedPnl < 0`, same coin, same direction. Assert aggregation produces a zone with `source_mix` containing `"hyperliquid_fills"`. Assert `side_at_risk` matches the liquidation direction.

### VAL-LIQ-023: Fill source zones include estimated_notional_usd from fill sizes
The notional value for fill-derived zones is estimated as the sum of `px * sz` across fills in the burst. For 10 fills averaging 1 BTC each at $100,000: `estimated_notional_usd ≈ 1_000_000`.
Tool: cargo-test
Evidence: Provide 10 fills at $100,000 with size 1 BTC each. Assert resulting zone has `estimated_notional_usd` within 5% of 1_000_000.

### VAL-LIQ-024: Isolated fills do not create zones
A single fill with `closedPnl < 0` or scattered fills at different prices over hours do not create a zone. Only clustered burst patterns qualify.
Tool: cargo-test
Evidence: Provide 1 fill with negative PnL. Assert no zones produced. Provide 5 fills spread across 4 hours. Assert no zones produced.

### VAL-LIQ-025: wallet_count for fill-derived zones reflects distinct wallets
When fills from multiple wallets contribute to a burst zone, `wallet_count` equals the number of distinct wallet addresses. Fills from the same wallet count once.
Tool: cargo-test
Evidence: Provide 15 fills from 8 distinct wallets (some wallets contribute multiple fills). Assert `wallet_count == 8`.

### VAL-LIQ-026: Fill API failure returns error without panic
When `userFills` or `userFillsByTime` returns a network error or malformed response, the function returns `Err` without panicking. Other sources continue.
Tool: cargo-test
Evidence: Mock fill client to return timeout error. Assert `Err` returned, no panic. Verify warn-level log emitted.

### VAL-LIQ-027: userFillsByTime respects configurable lookback window
The fill capture function uses a configurable lookback duration (e.g., last 5 minutes). Only fills within this window are considered. Older fills are excluded.
Tool: cargo-test
Evidence: Provide fills at `now - 1min`, `now - 3min`, `now - 7min`, `now - 10min`. With a 5-minute lookback, only the first two are included. Assert zones derived only from recent fills.

---

## Source Fusion: Imperial OI Imbalance

### VAL-LIQ-028: Imperial /api/v1/stats/markets OI imbalance is parsed
The Imperial client fetches `/api/v1/stats/markets` and extracts `longOiUsd` and `shortOiUsd` per market. When `longOiUsd` significantly exceeds `shortOiUsd` (or vice versa), an OI imbalance zone is created. `side_at_risk` is the side with the larger OI (more crowded side gets liquidated first in a cascade). `source_mix = ["oi_imbalance"]`.
Tool: cargo-test
Evidence: Mock Imperial response with `longOiUsd = 5_000_000, shortOiUsd = 2_000_000` for SOL. Assert resulting zone has `side_at_risk == "long"` (longs are overcrowded). Assert `source_mix == ["oi_imbalance"]`.

### VAL-LIQ-029: OI imbalance zone price is derived from mark price and imbalance direction
When long OI dominates, the zone price is set below mark price (long liquidations happen on downside). When short OI dominates, the zone price is set above mark price. The distance reflects the imbalance magnitude.
Tool: cargo-test
Evidence: With `mark_price = 150.0`, `longOiUsd = 8_000_000`, `shortOiUsd = 2_000_000`, assert `zone.price < 150.0` and `zone.side_at_risk == "long"`. With reversed OI, assert `zone.price > 150.0` and `zone.side_at_risk == "short"`.

### VAL-LIQ-030: Balanced OI does not produce a zone
When `abs(longOiUsd - shortOiUsd) / max(longOiUsd, shortOiUsd) < imbalance_threshold` (configurable, default 20%), no OI imbalance zone is created.
Tool: cargo-test
Evidence: Mock Imperial response with `longOiUsd = 1_000_000, shortOiUsd = 950_000` (5% imbalance, below 20% threshold). Assert no OI zone produced.

### VAL-LIQ-031: Imperial stats API failure returns error without panic
When `/api/v1/stats/markets` returns an error, the function returns `Err` without panic. Other sources continue processing.
Tool: cargo-test
Evidence: Mock Imperial client to return 503. Assert `Err` returned, warn-level log emitted, no panic.

### VAL-LIQ-032: Missing market in Imperial stats is skipped
If the Imperial response does not include data for a target market (e.g., "SOL" is absent), that market is skipped for this source. Other markets with data continue normally.
Tool: cargo-test
Evidence: Mock Imperial response with only BTC and ETH data. Request SOL. Assert no OI zone for SOL. Assert no error (silent skip, logged at debug level).

---

## Source Fusion: Imperial Depth / Orderbook Fragility

### VAL-LIQ-033: Imperial /api/v1/phoenix/depth thin orderbook creates fragility zone
When the Phoenix depth response shows thin bid liquidity below the current price (or thin ask liquidity above), a fragility zone is created. Thin is defined as total bid/ask depth within N bps of mark falling below a configurable threshold (e.g., < $100K within 50 bps). `source_mix = ["depth_fragility"]`.
Tool: cargo-test
Evidence: Mock depth response with bids: total $50K within 50 bps of mark, asks: total $800K within 50 bps. Assert zone created with `side_at_risk == "long"` (thin bids → longs at risk). Assert `source_mix == ["depth_fragility"]`.

### VAL-LIQ-034: Depth fragility zone price is set at the thin shelf
The zone price is set where liquidity drops off (the price level where cumulative depth falls below threshold). If bids thin out sharply at $148.50 for a $150 mark, zone price ≈ $148.50.
Tool: cargo-test
Evidence: Mock depth with bids: $200K at $150.00, $30K at $148.50, then nothing. Assert `zone.price` is near $148.50.

### VAL-LIQ-035: Deep balanced orderbook produces no fragility zone
When both bid and ask sides have sufficient depth (above threshold), no fragility zone is created.
Tool: cargo-test
Evidence: Mock depth with $1M bids and $1M asks within 50 bps. Assert no fragility zone produced.

### VAL-LIQ-036: Depth API failure returns error without panic
When `/api/v1/phoenix/depth` returns an error, the function returns `Err` without panic.
Tool: cargo-test
Evidence: Mock depth endpoint returning timeout. Assert `Err`, warn log, no panic.

### VAL-LIQ-037: Imperial /api/v1/mark-prices provides mark price for snapshot
The mark price in `LiquidationZoneSnapshot.mark_price` is sourced from Imperial `/api/v1/mark-prices` when available. If Imperial is unavailable, the HL mark price is used as fallback.
Tool: cargo-test
Evidence: Mock Imperial mark price at $150.25. Assert snapshot `mark_price == 150.25`. Mock Imperial failure, HL mark at $150.30. Assert `mark_price == 150.30` (fallback).

---

## Multi-Source Fusion and Zone Merging

### VAL-LIQ-038: Zones from different sources at similar prices are merged
When `hyperliquid_positions` produces a long-liquidation zone at $95,100 and `oi_imbalance` produces a long-liquidation zone at $95,000 (within merge threshold), they are merged into a single zone at the weighted-average price with `source_mix = ["hyperliquid_positions", "oi_imbalance"]` and `confidence` higher than either individual zone.
Tool: cargo-test
Evidence: Produce zones at $95,100 (HL positions) and $95,000 (OI imbalance). After merge, assert exactly 1 zone. Assert `source_mix` contains both sources. Assert `confidence > max(individual confidences)`.

### VAL-LIQ-039: Zones from different sources at distant prices remain separate
A zone from `hyperliquid_positions` at $95,000 and a zone from `depth_fragility` at $88,000 (outside merge threshold) remain as two distinct zones.
Tool: cargo-test
Evidence: Produce zones at $95,000 and $88,000. After merge, assert 2 zones remain, each with their original source.

### VAL-LIQ-040: Merged zone estimated_notional_usd is the sum of contributing sources
When HL positions contribute $500K notional and OI imbalance contributes $300K notional at a similar price, the merged zone has `estimated_notional_usd ≈ $800K`.
Tool: cargo-test
Evidence: Merge a $500K HL positions zone with a $300K OI imbalance zone. Assert merged `estimated_notional_usd` is within 1% of $800K.

### VAL-LIQ-041: Merged zone wallet_count sums from position sources only
`wallet_count` aggregates from sources that track individual wallets (`hyperliquid_positions`, `hyperliquid_fills`). Macro sources (`oi_imbalance`, `depth_fragility`) do not contribute to `wallet_count`.
Tool: cargo-test
Evidence: Merge a zone from HL positions (wallet_count=42) with OI imbalance (no wallet_count). Assert merged `wallet_count == 42`.

### VAL-LIQ-042: All four sources can contribute simultaneously
In a single capture cycle, all four sources (HL positions, HL fills, OI imbalance, depth fragility) produce data. The fusion layer merges overlapping zones and preserves distinct zones. No source blocks another.
Tool: cargo-test
Evidence: Provide synthetic data from all 4 sources. Assert fusion completes without error. Assert each source's contribution appears in at least one zone's `source_mix`.

### VAL-LIQ-043: Partial source availability produces zones from available sources
If only 2 of 4 sources are available (e.g., HL positions + OI imbalance), zones are still produced from those 2 sources. Missing sources are logged at debug level but do not block the capture cycle.
Tool: cargo-test
Evidence: Configure 4 sources, mock 2 as failing. Assert zones produced from the 2 working sources. Assert warn/debug logs for failures. Assert no panic.

### VAL-LIQ-044: Capture with zero available sources produces empty zones
If all configured sources fail in a single cycle, the snapshot is still created with `zones: vec![]` and persisted. An error-level log summarizes total source failure.
Tool: cargo-test
Evidence: Mock all 4 sources as failing. Assert snapshot produced with empty zones. Assert error-level log emitted. Assert no panic.

---

## Confidence Scoring

### VAL-LIQ-045: Single-source zone starts at base confidence
A zone derived from exactly one source (e.g., `hyperliquid_positions` alone) has confidence equal to the base confidence for that source (configurable, e.g., 0.4 for positions). No multi-source bonus is applied.
Tool: cargo-test
Evidence: Produce a zone from HL positions only. Assert `confidence == base_confidence_for_source("hyperliquid_positions")`.

### VAL-LIQ-046: Two-source corroboration increases confidence
When a zone is corroborated by 2 sources, confidence increases by a configurable bonus (e.g., +0.15). For base 0.4 + 2 sources: `confidence = 0.4 + 0.15 = 0.55`.
Tool: cargo-test
Evidence: Produce a zone from 2 sources. Assert `confidence == base + bonus_2source`. Assert `confidence <= 1.0`.

### VAL-LIQ-047: Three-source corroboration increases confidence further
With 3 corroborating sources: confidence = base + bonus_2source + bonus_3source (e.g., 0.4 + 0.15 + 0.10 = 0.65).
Tool: cargo-test
Evidence: Produce a zone from 3 sources. Assert confidence exceeds the 2-source value by the configurable 3-source bonus.

### VAL-LIQ-048: Four-source corroboration yields maximum confidence
All 4 sources corroborating a single zone yields the highest confidence: base + bonus_2 + bonus_3 + bonus_4 (e.g., 0.4 + 0.15 + 0.10 + 0.10 = 0.75). Confidence never exceeds 1.0 regardless of bonuses.
Tool: cargo-test
Evidence: Produce a zone from all 4 sources. Assert `confidence <= 1.0`. Assert `confidence == min(base + sum_of_bonuses, 1.0)`.

### VAL-LIQ-049: Stale data reduces confidence
Each source tracks when its data was last fetched. If a source's data is older than a configurable staleness threshold (e.g., 60 seconds), a staleness penalty is applied (e.g., -0.10). A zone derived from 2 sources, one stale, has lower confidence than the same zone with both sources fresh.
Tool: cargo-test
Evidence: Produce a zone from 2 fresh sources (confidence = X). Mark one source as stale (last fetch 120s ago). Assert new confidence < X by exactly the staleness penalty.

### VAL-LIQ-050: Confidence does not go below 0.0 after staleness penalty
If staleness penalties would reduce confidence below 0.0, confidence is clamped to 0.0.
Tool: cargo-test
Evidence: Start with base confidence 0.1, apply staleness penalty of -0.20. Assert `confidence == 0.0`.

### VAL-LIQ-051: Confidence computation is deterministic
Given the same source contributions and timestamps, confidence is computed identically across calls. No randomness or floating-point drift.
Tool: cargo-test
Evidence: Call confidence computation 1000 times with identical inputs. Assert all 1000 results are bitwise equal.

### VAL-LIQ-052: High wallet_count increases confidence marginally
A zone with 50 contributing wallets has slightly higher confidence than an equivalent zone with 5 wallets. The wallet-count bonus is logarithmic (e.g., `+0.02 * log10(wallet_count)`) and capped.
Tool: cargo-test
Evidence: Produce two otherwise-identical zones, one with `wallet_count = 5`, one with `wallet_count = 50`. Assert the 50-wallet zone has higher confidence. Assert the difference is within the expected logarithmic bonus.

### VAL-LIQ-053: High estimated_notional_usd increases confidence marginally
A zone with $10M notional has slightly higher confidence than an equivalent zone with $100K. The notional bonus is logarithmic (e.g., `+0.01 * log10(notional / 1_000_000)`) and capped.
Tool: cargo-test
Evidence: Produce two otherwise-identical zones, one with $100K notional, one with $10M. Assert the $10M zone has higher confidence.

---

## Snapshot Persistence

### VAL-LIQ-054: Snapshots are persisted as valid JSON files
Each capture cycle writes a `LiquidationZoneSnapshot` to `data/liquidation-zones/{symbol}_{timestamp_ms}.json`. The file is valid JSON parseable by `serde_json::from_str()`.
Tool: cargo-test
Evidence: Run a capture cycle with known data. Read the output file. Parse with `serde_json::from_str::<LiquidationZoneSnapshot>()`. Assert success. Assert all fields match the in-memory snapshot.

### VAL-LIQ-055: Snapshot files are written atomically
Snapshot files use the atomic write pattern: write to `<path>.tmp`, then `std::fs::rename` to the final path. After clean completion, no `.tmp` files remain. If the process crashes mid-write, the final path contains either the complete previous snapshot or nothing (no partial files).
Tool: cargo-test
Evidence: Inspect the persistence function. Assert it writes to `.tmp` first. Assert it calls `std::fs::rename`. After a successful write, assert `!path.ends_with(".tmp")` exists and `path.ends_with(".tmp")` does not.

### VAL-LIQ-056: Snapshot directory is created if absent
If `data/liquidation-zones/` does not exist at startup, it is created automatically. No error or panic.
Tool: cargo-test
Evidence: Point snapshot_dir to a non-existent `/tmp/test-liq-{pid}/`. Run capture. Assert directory was created. Assert file exists inside it.

### VAL-LIQ-057: Multiple snapshots for same symbol at different timestamps are distinct files
Two capture cycles for SOL at different timestamps produce two separate files: `SOL_1770000000000.json` and `SOL_1770000060000.json`. Neither overwrites the other.
Tool: cargo-test
Evidence: Run two capture cycles with different timestamps for the same symbol. Assert both files exist. Assert contents differ.

### VAL-LIQ-058: Snapshot file names are safe and deterministic
File names follow the pattern `{symbol}_{timestamp_ms}.json`. The symbol is sanitized (no `/`, `\`, `:` characters). Timestamp is zero-padded if needed. File names are valid on Linux filesystems.
Tool: cargo-test
Evidence: Assert file name generation for symbol "BTC/USD" produces a sanitized name (e.g., "BTC-USD_*.json"). Assert no path traversal characters in the file name.

### VAL-LIQ-059: File write failure is logged and does not crash
If the file system is read-only or the disk is full, the write fails gracefully. An error is logged. The capture cycle continues (next cycle retries). No panic.
Tool: cargo-test
Evidence: Point snapshot_dir to a read-only path (`/proc/fake-liq/`). Assert write returns `Err`. Assert error logged. Assert no panic. Assert capture loop continues.

### VAL-LIQ-060: Empty snapshot (no zones) is still persisted
A capture cycle that produces zero zones writes a snapshot with `zones: []`. The file still contains `symbol`, `timestamp_ms`, `mark_price`, and the empty zones array.
Tool: cargo-test
Evidence: Run capture with all sources returning no actionable data. Assert output file exists. Assert `zones == []` in JSON. Assert `symbol` and `mark_price` are populated.

### VAL-LIQ-061: Snapshot JSON is human-readable (pretty-printed)
The persisted JSON uses `serde_json::to_string_pretty()` (or equivalent) with 2-space indentation. The file is directly readable by a human without a formatter.
Tool: cargo-test
Evidence: Read a persisted snapshot file. Assert it contains newlines and indentation. Assert it is not a single-line minified JSON blob.

---

## Capture Interval and Loop

### VAL-LIQ-062: Capture runs at configurable interval
The capture loop runs at a configurable interval (default: 30 seconds). Each cycle fetches from all configured sources, fuses data, and persists a snapshot. The interval is read from config.
Tool: cargo-test
Evidence: Construct capture engine with `interval_secs = 5`. Run for 3 cycles. Assert 3 snapshot files created. Assert timestamps are approximately 5 seconds apart.

### VAL-LIQ-063: Capture cycle duration shorter than interval does not drift
If a capture cycle takes 2 seconds and the interval is 30 seconds, the next cycle starts at `start + 30s`, not `end + 30s`. Long-term timing does not drift.
Tool: cargo-test
Evidence: Mock a source that takes 2 seconds. Run 3 cycles with 5-second interval. Assert cycle start times are at T+0, T+5, T+10 (not T+0, T+7, T+14).

### VAL-LIQ-064: Capture cycle longer than interval logs a warning
If a capture cycle takes longer than the configured interval (e.g., 35s with 30s interval), a warning is logged: "capture cycle exceeded interval". The next cycle starts immediately.
Tool: cargo-test
Evidence: Mock a source that takes 35 seconds with a 30-second interval. Assert warn log contains "exceeded interval" or similar. Assert no cycle is skipped.

### VAL-LIQ-065: Graceful shutdown on SIGINT/SIGTERM
When a SIGINT is received during a capture cycle, the current cycle completes (including file persistence), and the loop exits cleanly. No partial writes. No orphan `.tmp` files.
Tool: cargo-test
Evidence: Start capture loop in background. Send SIGINT. Assert process exits with code 0. Assert last snapshot file is complete (valid JSON). Assert no `.tmp` files remain.

### VAL-LIQ-066: No resource leaks across cycles
Running 1000 capture cycles does not leak memory or file handles. The number of open file descriptors at cycle 1000 is no more than at cycle 2 (plus the snapshot files themselves).
Tool: cargo-test
Evidence: Run 100 capture cycles in a test. Track `std::mem::size_of_val(&engine)` or use a leak-detection approach. Assert no unbounded growth. (A full leak test may require `valgrind` or similar — at minimum, assert the engine struct size does not grow across cycles.)

### VAL-LIQ-067: Multiple symbols can be captured in a single cycle
When configured for `["BTC", "ETH", "SOL"]`, each capture cycle produces a separate snapshot per symbol. Each snapshot is independently persisted to its own file.
Tool: cargo-test
Evidence: Configure 3 symbols. Run 1 cycle. Assert 3 snapshot files created. Assert each file's `symbol` field matches the expected market.

### VAL-LIQ-068: Per-symbol capture is independent
If the BTC source fails but ETH succeeds, the ETH snapshot is still persisted. A BTC failure does not block ETH persistence.
Tool: cargo-test
Evidence: Mock BTC source as failing, ETH as succeeding. Run 1 cycle. Assert ETH snapshot persisted. Assert BTC snapshot either persisted with empty zones or logged as failed.

---

## Source Freshness Tracking

### VAL-LIQ-069: Each source tracks its last successful fetch timestamp
The capture engine maintains a `HashMap<String, i64>` mapping source name to last successful fetch timestamp in milliseconds. After a successful fetch, the timestamp is updated.
Tool: cargo-test
Evidence: Run 1 cycle with all sources succeeding. Assert freshness map has entries for all configured sources. Assert each timestamp is within 5 seconds of `now`.

### VAL-LIQ-070: Stale source triggers degradation log
If a source has not been successfully fetched for longer than the staleness threshold (e.g., 120 seconds), a warn-level log is emitted: "source {name} is stale (last fetch: Xs ago)".
Tool: cargo-test
Evidence: Configure staleness threshold at 10 seconds. Mock a source as failing for 15 seconds. Assert warn log emitted with "stale" and the source name.

### VAL-LIQ-071: Staleness does not remove the source from config
A stale source is still attempted on each cycle. Staleness affects confidence scoring and logging, not source selection. The source is retried indefinitely.
Tool: cargo-test
Evidence: Mock a source that fails for 5 cycles then succeeds on the 6th. Assert it was attempted on every cycle. Assert freshness timestamp updated on cycle 6.

### VAL-LIQ-072: Freshness timestamp resets on successful fetch
After a failed fetch (stale source), a subsequent successful fetch resets the freshness timestamp to the current time. Staleness penalty is removed from confidence scoring.
Tool: cargo-test
Evidence: Source fails for 3 cycles (stale). Source succeeds on cycle 4. Assert freshness timestamp is now cycle 4's time. Assert confidence no longer includes staleness penalty.

### VAL-LIQ-073: All-sources-stale condition is logged at error level
When all configured sources are stale simultaneously, an error-level log is emitted. This indicates a systemic capture degradation.
Tool: cargo-test
Evidence: Mock all sources as failing for longer than the staleness threshold. Assert error-level log emitted. Assert capture continues (does not halt).

---

## Edge Cases

### VAL-LIQ-074: No known wallets produces empty HL position zones
If the wallet list for HL position scraping is empty, the HL positions source returns zero zones gracefully. No error.
Tool: cargo-test
Evidence: Pass empty wallet list to HL position aggregator. Assert result is `zones: vec![]`. Assert no error.

### VAL-LIQ-075: All wallets have no open positions
If all known wallets exist but have zero open positions, the HL positions source returns zero zones.
Tool: cargo-test
Evidence: Pass 100 wallets all with no open positions. Assert `zones: vec![]`.

### VAL-LIQ-076: Extremely large position creates a zone with correct notional
A single wallet with a $50M BTC position at 2x leverage has a liquidation price. This creates a zone with `estimated_notional_usd = 50_000_000.0` and `wallet_count = 1`.
Tool: cargo-test
Evidence: Provide 1 wallet with $50M position. Assert zone has `estimated_notional_usd == 50_000_000.0`, `wallet_count == 1`.

### VAL-LIQ-077: Malformed API JSON is handled gracefully
When an API returns invalid JSON (e.g., truncated, wrong types, unexpected structure), the parser returns an error without panicking. The error is logged with the raw response excerpt.
Tool: cargo-test
Evidence: Provide malformed JSON to each source parser. Assert `Err` returned for each. Assert no panic. Assert error log contains context.

### VAL-LIQ-078: Rate-limited API responses are retried
When an API returns HTTP 429 (rate limited), the capture engine waits and retries once before marking the source as failed for this cycle.
Tool: cargo-test
Evidence: Mock a 429 response followed by a 200 on retry. Assert the source eventually succeeds. Assert data captured from the successful retry.

### VAL-LIQ-079: Concurrent capture cycles do not overlap
The capture loop ensures only one cycle runs at a time. If a cycle is still running when the next interval fires, the new cycle is skipped (not queued). A debug log notes the skip.
Tool: cargo-test
Evidence: Mock a cycle that takes 45 seconds with a 30-second interval. Assert no concurrent execution. Assert debug log "skipping cycle, previous still running" or equivalent.

### VAL-LIQ-080: Zero-confidence zones are included in snapshot
Zones with `confidence = 0.0` (e.g., all sources stale) are still included in the snapshot for auditability. Downstream consumers filter by confidence.
Tool: cargo-test
Evidence: Produce a zone where all sources are stale (confidence = 0.0). Assert zone is in snapshot. Assert `confidence == 0.0` in persisted file.

### VAL-LIQ-081: Very small distance_bps zones are captured
A liquidation zone within 1 bp of the mark price is valid and captured. `distance_bps` can be 0.0 (zone at mark price).
Tool: cargo-test
Evidence: Produce zone at mark price exactly. Assert `distance_bps == 0.0`. Assert zone is in snapshot.

### VAL-LIQ-082: Very large distance_bps zones are captured
A liquidation zone 10,000 bps (100%) away from the mark price is valid and captured (extremely leveraged or unusual position).
Tool: cargo-test
Evidence: Produce zone with `distance_bps = 10000.0`. Assert zone is in snapshot.

### VAL-LIQ-083: Symbol case is normalized
If the config specifies `["btc", "ETH", "Sol"]`, symbols are normalized to uppercase canonical form (`["BTC", "ETH", "SOL"]`) at initialization. Snapshot files use the normalized form.
Tool: cargo-test
Evidence: Configure with lowercase symbols. Assert snapshots use uppercase. Assert `symbol` field in JSON is uppercase.

---

## Configuration

### VAL-LIQ-084: Capture config has sensible defaults
When no `[liquidation-zone]` section exists in the TOML config, the capture module uses defaults: `interval_secs = 30`, `staleness_threshold_secs = 60`, `min_confidence = 0.0`, `snapshot_dir = "data/liquidation-zones"`, `sources = ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance", "depth_fragility"]`, `cluster_threshold_bps = 50.0`, `merge_threshold_bps = 100.0`.
Tool: cargo-test
Evidence: Load config without `[liquidation-zone]` section. Assert all defaults match. Assert `validate()` returns `Ok`.

### VAL-LIQ-085: Partial config overrides merge with defaults
If the TOML has `[liquidation-zone]` with only `interval_secs = 10`, all other fields use defaults. `interval_secs = 10` is respected; `snapshot_dir` remains the default.
Tool: cargo-test
Evidence: Load config with partial override. Assert `interval_secs == 10`. Assert `snapshot_dir == "data/liquidation-zones"`. Assert `staleness_threshold_secs == 60`.

### VAL-LIQ-086: Invalid config values are rejected
`interval_secs = 0`, `staleness_threshold_secs = -1`, `min_confidence = 1.5`, `cluster_threshold_bps = -10.0` all fail config validation with descriptive error messages.
Tool: cargo-test
Evidence: Load config with each invalid value. Assert `Err` with message containing the field name and the constraint violated.

### VAL-LIQ-087: sources list can be subset of all available sources
Configuring `sources = ["hyperliquid_positions", "oi_imbalance"]` enables only those 2 sources. The other 2 are never called. No error for omitted sources.
Tool: cargo-test
Evidence: Configure with subset. Run capture. Assert only the configured sources are queried. Assert `source_mix` in zones contains only the configured sources.

### VAL-LIQ-088: Unknown source names in config are rejected
`sources = ["hyperliquid_positions", "magic_8_ball"]` fails validation: "unknown source: magic_8_ball".
Tool: cargo-test
Evidence: Load config with unknown source. Assert `Err` with message containing "unknown source" and "magic_8_ball".

### VAL-LIQ-089: snapshot_dir with relative path resolves from CWD
`snapshot_dir = "data/liquidation-zones"` resolves relative to the current working directory, not relative to the binary.
Tool: cargo-test
Evidence: Set `snapshot_dir = "data/liquidation-zones"`. Assert resolved path is `{cwd}/data/liquidation-zones`.

### VAL-LIQ-090: snapshot_dir with absolute path is used as-is
`snapshot_dir = "/tmp/liq-zekt"` is used exactly as specified.
Tool: cargo-test
Evidence: Set absolute path. Assert resolved path is `/tmp/liq-zekt`.

### VAL-LIQ-091: min_confidence filters zones in persistence
When `min_confidence = 0.3`, zones with `confidence < 0.3` are excluded from the persisted snapshot. Zones with `confidence >= 0.3` are included.
Tool: cargo-test
Evidence: Produce zones with confidence 0.1, 0.3, 0.5. Configure `min_confidence = 0.3`. Assert persisted snapshot contains only the 0.3 and 0.5 zones.

### VAL-LIQ-092: min_confidence = 0.0 includes all zones
The default `min_confidence = 0.0` includes even zero-confidence zones for full auditability.
Tool: cargo-test
Evidence: Produce zones with confidence 0.0, 0.5, 1.0. Assert all 3 are in snapshot.

### VAL-LIQ-093: cluster_threshold_bps controls position aggregation sensitivity
With `cluster_threshold_bps = 100`, positions whose liquidation prices are within 100 bps of each other are clustered into one zone. At `cluster_threshold_bps = 10`, only very close prices merge.
Tool: cargo-test
Evidence: Provide 10 positions with liquidation prices spread over 80 bps. With threshold 100: assert 1 zone. With threshold 10: assert multiple zones.

### VAL-LIQ-094: merge_threshold_bps controls cross-source zone merging
With `merge_threshold_bps = 200`, a HL-positions zone at $95,000 and an OI-imbalance zone at $96,500 (158 bps apart for BTC) are merged. At `merge_threshold_bps = 50`, they remain separate.
Tool: cargo-test
Evidence: Produce zones 150 bps apart. With threshold 200: assert merged. With threshold 50: assert separate.

---

## Capture-Only Constraint

### VAL-LIQ-095: Capture module has no trading entry/exit functions
The `liquidation.rs` module does not contain any function that constructs, signs, or submits a trade. No `Signal::MomentumLong`, `Signal::MomentumShort`, or `Signal::Exit*` is emitted by the capture module. The module's public API is limited to: start capture, stop capture, get latest snapshot, get source freshness.
Tool: code-review
Evidence: Grep `src/liquidation.rs` for `open_position`, `close_position`, `Signal::Momentum`, `sign`, `submit`, `execute`, `place_order`, `order`. Assert zero matches.

### VAL-LIQ-096: Capture module does not import executor or engine
`src/liquidation.rs` does not `use crate::engine` or `use crate::executor`. It has no dependency on trading infrastructure.
Tool: code-review
Evidence: Grep imports in `src/liquidation.rs`. Assert no `use crate::engine`, `use crate::executor`, or `use crate::flash_api`.

### VAL-LIQ-097: Capture module output is read-only data
The capture module's output types (`LiquidationZoneSnapshot`, `LiquidationZone`) derive `Serialize` and `Clone` but never `Into<Signal>` or any trading-related trait. They are pure data containers.
Tool: code-review
Evidence: Inspect struct definitions. Assert no `impl Into<Signal>` or trading trait implementations.

---

## Integration

### VAL-LIQ-098: Capture module is gated behind config flag
When `[liquidation-zone]` is absent from config, the capture module is not started. No background task, no file writes, no API calls.
Tool: cargo-test
Evidence: Load config without `[liquidation-zone]`. Assert capture engine returns `None` or is not initialized. Assert no file writes.

### VAL-LIQ-099: Capture runs in its own async task
The capture loop runs as a spawned `tokio::task` that does not block the main trading loop or pipeline. Graceful shutdown via `tokio::CancellationToken` or shared `AtomicBool`.
Tool: cargo-test
Evidence: Inspect pipeline/launch code. Assert capture runs in `tokio::spawn`. Assert it reads from shared `AtomicBool` for shutdown.

### VAL-LIQ-100: Capture module logs all API calls at debug level
Each API call (URL, response status, duration_ms) is logged at debug level. Successful parses at info level summarize zones found. Failures at warn level.
Tool: code-review
Evidence: Assert `tracing::debug!` calls for each API fetch. Assert `tracing::info!` for cycle summary. Assert `tracing::warn!` for failures.

### VAL-LIQ-101: Capture module uses tracing (never println)
All output from the capture module uses `tracing` macros (`info!`, `warn!`, `error!`, `debug!`). No `println!`, `eprintln!`, or `dbg!` calls.
Tool: code-review
Evidence: Grep `src/liquidation.rs` for `println`, `eprintln`, `dbg!`, `print!`. Assert zero matches.

### VAL-LIQ-102: Snapshot files are cleaned up after configurable retention period
Snapshots older than a configurable retention period (e.g., 7 days) are deleted at the start of each capture cycle. This prevents unbounded disk growth.
Tool: cargo-test
Evidence: Create snapshot files with timestamps 8 days old. Run capture with `retention_days = 7`. Assert old files deleted. Assert current files preserved.

### VAL-LIQ-103: Snapshot retention cleanup handles missing directory gracefully
If the snapshot directory does not exist during cleanup, no error is raised (directory will be created when needed).
Tool: cargo-test
Evidence: Delete snapshot directory. Run cleanup function. Assert no error, no panic.

### VAL-LIQ-104: Capture module compiles as part of `cargo build --release`
The `liquidation.rs` module is included in the crate and compiles without errors or warnings in release mode.
Tool: cargo-build
Evidence: Run `cargo build --release 2>&1`. Assert exit code 0. Assert no warnings referencing `liquidation`.

### VAL-LIQ-105: All liquidation capture tests pass
All unit tests for the liquidation capture module pass with `cargo test liquidation`.
Tool: cargo-test
Evidence: Run `cargo test liquidation`. Assert all tests pass (exit code 0). Assert test count >= number of VAL-LIQ assertions that specify cargo-test.
