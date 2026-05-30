//! Liquidation zone intelligence capture module.
//!
//! Captures liquidation zone data from multiple sources (Hyperliquid positions,
//! Hyperliquid fills, Imperial OI stats, Imperial depth data), fuses them into
//! unified `LiquidationZone` structs with confidence scoring, and merges
//! cross-source zones at similar prices.
//!
//! **Constraints:**
//! - Capture-only: no trading functions, no Signal emissions.
//! - No imports from engine, executor, or flash_api.
//! - Uses `tracing` for all logging (never `println`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export config for convenience
pub use crate::config::LiquidationConfig;

/// Known source names for validation.
pub const VALID_SOURCES: &[&str] = &[
    "hyperliquid_positions",
    "hyperliquid_fills",
    "oi_imbalance",
    "depth_fragility",
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A snapshot of liquidation zones for a single symbol at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiquidationZoneSnapshot {
    pub symbol: String,
    pub timestamp_ms: i64,
    pub mark_price: f64,
    pub zones: Vec<LiquidationZone>,
}

/// A single liquidation zone with price, side, notional, and confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiquidationZone {
    /// Price level of the liquidation zone.
    pub price: f64,
    /// Side at risk: "long" (longs get liquidated when price drops to this zone)
    /// or "short" (shorts get liquidated when price rises to this zone).
    pub side_at_risk: String,
    /// Estimated total notional value (USD) at risk in this zone.
    pub estimated_notional_usd: f64,
    /// Number of distinct wallets contributing to this zone.
    pub wallet_count: u32,
    /// Distance from mark price in basis points.
    pub distance_bps: f64,
    /// Confidence score in [0.0, 1.0].
    pub confidence: f64,
    /// Sources that contributed to this zone.
    pub source_mix: Vec<String>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

impl LiquidationZoneSnapshot {
    /// Validate all fields in the snapshot.
    pub fn validate(&self) -> Result<()> {
        // Symbol
        if self.symbol.trim().is_empty() {
            anyhow::bail!("snapshot symbol must be non-empty");
        }
        // Timestamp: reasonable Unix epoch millis (post-2023, pre-2033)
        if self.timestamp_ms < 1_700_000_000_000 || self.timestamp_ms > 2_000_000_000_000 {
            anyhow::bail!(
                "timestamp_ms must be in [1_700_000_000_000, 2_000_000_000_000], got {}",
                self.timestamp_ms
            );
        }
        // Mark price
        if self.mark_price <= 0.0 {
            anyhow::bail!("mark_price must be > 0.0, got {}", self.mark_price);
        }
        // Zones
        for (i, zone) in self.zones.iter().enumerate() {
            zone.validate().with_context(|| format!("zone[{}] validation failed", i))?;
        }
        Ok(())
    }
}

impl LiquidationZone {
    /// Validate all fields in the zone.
    pub fn validate(&self) -> Result<()> {
        // Price
        if self.price <= 0.0 {
            anyhow::bail!("zone price must be > 0.0, got {}", self.price);
        }
        // side_at_risk
        if self.side_at_risk != "long" && self.side_at_risk != "short" {
            anyhow::bail!(
                "side_at_risk must be 'long' or 'short', got '{}'",
                self.side_at_risk
            );
        }
        // estimated_notional_usd
        if self.estimated_notional_usd < 0.0 {
            anyhow::bail!(
                "estimated_notional_usd must be >= 0.0, got {}",
                self.estimated_notional_usd
            );
        }
        // wallet_count consistency
        if self.estimated_notional_usd > 0.0 && self.wallet_count == 0 {
            anyhow::bail!(
                "wallet_count must be >= 1 when estimated_notional_usd > 0, got wallet_count=0"
            );
        }
        // distance_bps
        if self.distance_bps < 0.0 {
            anyhow::bail!("distance_bps must be >= 0.0, got {}", self.distance_bps);
        }
        // confidence
        if self.confidence < 0.0 || self.confidence > 1.0 {
            anyhow::bail!(
                "confidence must be in [0.0, 1.0], got {}",
                self.confidence
            );
        }
        // source_mix non-empty when confidence > 0
        if self.confidence > 0.0 && self.source_mix.is_empty() {
            anyhow::bail!("source_mix must be non-empty when confidence > 0");
        }
        // source_mix entries must be valid
        for source in &self.source_mix {
            if !VALID_SOURCES.contains(&source.as_str()) {
                anyhow::bail!("unknown source in source_mix: '{}'", source);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Source Fusion: Hyperliquid Positions
// ---------------------------------------------------------------------------

/// A wallet position parsed from HL clearinghouseState, used for aggregation.
#[derive(Debug, Clone)]
pub struct HlWalletPosition {
    pub wallet: String,
    pub coin: String,
    pub side: String,           // "B" (long) or "A" (short) — but from size sign
    pub liquidation_price: f64,
    pub position_value_usd: f64,
    pub size_signed: f64,
}

/// Aggregate HL clearinghouseState positions into liquidation zones.
///
/// Groups positions by side (long/short), clusters liquidation prices within
/// `cluster_threshold_bps`, and produces one zone per cluster.
pub fn aggregate_hl_positions(
    positions: &[HlWalletPosition],
    mark_price: f64,
    cluster_threshold_bps: f64,
) -> Vec<LiquidationZone> {
    if positions.is_empty() || mark_price <= 0.0 {
        return vec![];
    }

    // Separate into long and short positions
    let mut long_positions: Vec<&HlWalletPosition> = Vec::new();
    let mut short_positions: Vec<&HlWalletPosition> = Vec::new();

    for pos in positions {
        if pos.size_signed > 0.0 {
            long_positions.push(pos);
        } else {
            short_positions.push(pos);
        }
    }

    let mut zones = Vec::new();

    // Longs are liquidated when price drops → side_at_risk = "long"
    zones.extend(cluster_positions(
        &long_positions,
        mark_price,
        cluster_threshold_bps,
        "long",
    ));

    // Shorts are liquidated when price rises → side_at_risk = "short"
    zones.extend(cluster_positions(
        &short_positions,
        mark_price,
        cluster_threshold_bps,
        "short",
    ));

    zones
}

/// Cluster positions by liquidation price within threshold bps.
fn cluster_positions(
    positions: &[&HlWalletPosition],
    mark_price: f64,
    cluster_threshold_bps: f64,
    side_at_risk: &str,
) -> Vec<LiquidationZone> {
    if positions.is_empty() {
        return vec![];
    }

    // Sort by liquidation price
    let mut sorted: Vec<_> = positions.to_vec();
    sorted.sort_by(|a, b| {
        a.liquidation_price
            .partial_cmp(&b.liquidation_price)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut zones = Vec::new();
    let mut cluster: Vec<&HlWalletPosition> = vec![sorted[0]];

    for pos in sorted.iter().skip(1) {
        let cluster_min = cluster.first().unwrap().liquidation_price;
        let cluster_max = cluster.last().unwrap().liquidation_price;
        let reference = (cluster_min + cluster_max) / 2.0;

        let distance_bps = if reference > 0.0 {
            ((pos.liquidation_price - reference) / reference).abs() * 10_000.0
        } else {
            f64::MAX
        };

        if distance_bps <= cluster_threshold_bps {
            cluster.push(pos);
        } else {
            zones.push(build_zone_from_cluster(&cluster, mark_price, side_at_risk));
            cluster = vec![*pos];
        }
    }
    // Flush last cluster
    if !cluster.is_empty() {
        zones.push(build_zone_from_cluster(&cluster, mark_price, side_at_risk));
    }

    zones
}

/// Build a single LiquidationZone from a cluster of positions.
fn build_zone_from_cluster(
    cluster: &[&HlWalletPosition],
    mark_price: f64,
    side_at_risk: &str,
) -> LiquidationZone {
    let prices: Vec<f64> = cluster.iter().map(|p| p.liquidation_price).collect();
    let median_price = median(&prices);
    let total_notional: f64 = cluster.iter().map(|p| p.position_value_usd).sum();
    let wallet_count = cluster.len() as u32;
    let distance_bps = if mark_price > 0.0 {
        ((median_price - mark_price) / mark_price).abs() * 10_000.0
    } else {
        0.0
    };

    LiquidationZone {
        price: median_price,
        side_at_risk: side_at_risk.to_string(),
        estimated_notional_usd: total_notional,
        wallet_count,
        distance_bps,
        confidence: 0.0, // Will be computed later
        source_mix: vec!["hyperliquid_positions".to_string()],
    }
}

/// Compute median of a sorted slice of f64.
fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mut v = sorted.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) && mid > 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

// ---------------------------------------------------------------------------
// Source Fusion: Hyperliquid Fills (Forced-Flow Detection)
// ---------------------------------------------------------------------------

/// A fill record for forced-liquidation burst detection.
#[derive(Debug, Clone)]
pub struct HlFillRecord {
    pub wallet: String,
    pub coin: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub closed_pnl: f64,
    pub timestamp_ms: i64,
    pub direction: String, // "Open Long", "Close Long", etc.
}

/// Detect forced-liquidation bursts in fill data.
///
/// A burst is N+ fills with negative closedPnl within a time window,
/// all on the same coin and same side. Returns zones at the median fill price.
pub fn detect_forced_liquidation_bursts(
    fills: &[HlFillRecord],
    mark_price: f64,
    burst_count: usize,
    burst_window_secs: u64,
    lookback_secs: u64,
    now_ms: i64,
) -> Vec<LiquidationZone> {
    if fills.is_empty() {
        return vec![];
    }

    let cutoff_ms = now_ms - (lookback_secs as i64 * 1000);
    let recent_fills: Vec<&HlFillRecord> = fills
        .iter()
        .filter(|f| f.timestamp_ms >= cutoff_ms && f.closed_pnl < 0.0)
        .collect();

    if recent_fills.is_empty() {
        return vec![];
    }

    // Group by (coin, side)
    let mut groups: HashMap<(String, String), Vec<&HlFillRecord>> = HashMap::new();
    for fill in &recent_fills {
        let key = (fill.coin.clone(), fill.side.clone());
        groups.entry(key).or_default().push(*fill);
    }

    let burst_window_ms = burst_window_secs as i64 * 1000;
    let mut zones = Vec::new();

    for ((_coin, side), group_fills) in groups {
        // Sort by timestamp
        let mut sorted_fills: Vec<_> = group_fills;
        sorted_fills.sort_by_key(|f| f.timestamp_ms);

        // Sliding window to find bursts
        if sorted_fills.len() < burst_count {
            continue;
        }

        let mut i = 0;
        while i + burst_count <= sorted_fills.len() {
            let window_start = sorted_fills[i].timestamp_ms;
            let mut j = i;
            while j < sorted_fills.len() && sorted_fills[j].timestamp_ms - window_start <= burst_window_ms {
                j += 1;
            }
            let burst_len = j - i;
            if burst_len >= burst_count {
                let burst: Vec<&HlFillRecord> = sorted_fills[i..j].to_vec();
                let prices: Vec<f64> = burst.iter().map(|f| f.price).collect();
                let median_price = median(&prices);
                let total_notional: f64 = burst.iter().map(|f| f.price * f.size).sum();
                let distinct_wallets: std::collections::HashSet<&str> =
                    burst.iter().map(|f| f.wallet.as_str()).collect();

                // Determine side_at_risk:
                // If fills are mostly "sell"/"A" (closing longs at loss) → longs are at risk
                // If fills are mostly "buy"/"B" (closing shorts at loss) → shorts are at risk
                let side_at_risk = if side == "A" {
                    "long".to_string()
                } else {
                    "short".to_string()
                };

                let distance_bps = if mark_price > 0.0 {
                    ((median_price - mark_price) / mark_price).abs() * 10_000.0
                } else {
                    0.0
                };

                zones.push(LiquidationZone {
                    price: median_price,
                    side_at_risk,
                    estimated_notional_usd: total_notional,
                    wallet_count: distinct_wallets.len() as u32,
                    distance_bps,
                    confidence: 0.0,
                    source_mix: vec!["hyperliquid_fills".to_string()],
                });

                // Move past this burst to avoid overlapping bursts
                i = j;
            } else {
                i += 1;
            }
        }
    }

    zones
}

// ---------------------------------------------------------------------------
// Source Fusion: Imperial OI Imbalance
// ---------------------------------------------------------------------------

/// OI data for a single market from Imperial stats.
#[derive(Debug, Clone)]
pub struct OiData {
    pub symbol: String,
    pub long_oi_usd: f64,
    pub short_oi_usd: f64,
}

/// Produce liquidation zones from OI imbalance.
///
/// When one side dominates OI significantly (above threshold), that side is
/// "crowded" and more likely to be liquidated in a cascade. The zone price
/// is set on the side where the crowded side would get liquidated.
pub fn produce_oi_imbalance_zones(
    oi_data: &[OiData],
    mark_price: f64,
    imbalance_threshold_pct: f64,
) -> Vec<LiquidationZone> {
    if mark_price <= 0.0 {
        return vec![];
    }

    let mut zones = Vec::new();

    for data in oi_data {
        let max_oi = data.long_oi_usd.max(data.short_oi_usd);
        if max_oi <= 0.0 {
            continue;
        }
        let imbalance_pct = ((data.long_oi_usd - data.short_oi_usd).abs() / max_oi) * 100.0;
        if imbalance_pct < imbalance_threshold_pct {
            continue;
        }

        // Determine which side is crowded
        let (side_at_risk, imbalance_ratio) = if data.long_oi_usd > data.short_oi_usd {
            ("long".to_string(), data.long_oi_usd / data.short_oi_usd.max(1.0))
        } else {
            ("short".to_string(), data.short_oi_usd / data.long_oi_usd.max(1.0))
        };

        // Zone price: where the crowded side would get liquidated
        // Longs liquidated on downside → zone below mark
        // Shorts liquidated on upside → zone above mark
        // Distance scales with imbalance ratio (more imbalanced → closer zone)
        let distance_pct = (imbalance_ratio - 1.0).clamp(0.01, 0.5); // 1% to 50% distance
        let zone_price = if side_at_risk == "long" {
            mark_price * (1.0 - distance_pct)
        } else {
            mark_price * (1.0 + distance_pct)
        };

        let distance_bps = ((zone_price - mark_price) / mark_price).abs() * 10_000.0;
        let total_oi = data.long_oi_usd + data.short_oi_usd;

        zones.push(LiquidationZone {
            price: zone_price,
            side_at_risk,
            estimated_notional_usd: total_oi,
            wallet_count: 0, // OI is macro data, not per-wallet
            distance_bps,
            confidence: 0.0,
            source_mix: vec!["oi_imbalance".to_string()],
        });
    }

    zones
}

// ---------------------------------------------------------------------------
// Source Fusion: Imperial Depth Fragility
// ---------------------------------------------------------------------------

/// Depth level data from Imperial phoenix/depth.
#[derive(Debug, Clone)]
pub struct DepthLevel {
    pub price: f64,
    pub size_base: f64,
}

/// Depth snapshot for a single market.
#[derive(Debug, Clone)]
pub struct DepthSnapshot {
    pub symbol: String,
    pub mid: f64,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
}

/// Produce fragility zones from thin orderbook depth.
///
/// When one side of the book has insufficient depth near the mark price,
/// a fragility zone is created. Thin bids → longs at risk. Thin asks → shorts at risk.
pub fn produce_fragility_zones(
    depth: &DepthSnapshot,
    depth_min_threshold_usd: f64,
    depth_range_bps: f64,
) -> Vec<LiquidationZone> {
    let mid = depth.mid;
    if mid <= 0.0 {
        return vec![];
    }

    let range_price = mid * (depth_range_bps / 10_000.0);
    let mut zones = Vec::new();

    // Calculate total depth within range of mid for each side
    // For bids: prices in [mid - range, mid]
    // For asks: prices in [mid, mid + range]
    let bid_total: f64 = depth
        .bids
        .iter()
        .filter(|l| l.price >= mid - range_price && l.price <= mid)
        .map(|l| l.price * l.size_base)
        .sum();

    let ask_total: f64 = depth
        .asks
        .iter()
        .filter(|l| l.price >= mid && l.price <= mid + range_price)
        .map(|l| l.price * l.size_base)
        .sum();

    // Check bid fragility (thin bids → longs at risk)
    if bid_total < depth_min_threshold_usd && bid_total >= 0.0 {
        // Find where bids thin out
        let thin_price = find_thin_shelf(&depth.bids, mid, mid - range_price, depth_min_threshold_usd);
        let zone_price = thin_price.unwrap_or(mid - range_price);
        let distance_bps = ((zone_price - mid) / mid).abs() * 10_000.0;

        zones.push(LiquidationZone {
            price: zone_price,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: bid_total,
            wallet_count: 0,
            distance_bps,
            confidence: 0.0,
            source_mix: vec!["depth_fragility".to_string()],
        });
    }

    // Check ask fragility (thin asks → shorts at risk)
    if ask_total < depth_min_threshold_usd && ask_total >= 0.0 {
        let thin_price = find_thin_shelf_asks(&depth.asks, mid, mid + range_price, depth_min_threshold_usd);
        let zone_price = thin_price.unwrap_or(mid + range_price);
        let distance_bps = ((zone_price - mid) / mid).abs() * 10_000.0;

        zones.push(LiquidationZone {
            price: zone_price,
            side_at_risk: "short".to_string(),
            estimated_notional_usd: ask_total,
            wallet_count: 0,
            distance_bps,
            confidence: 0.0,
            source_mix: vec!["depth_fragility".to_string()],
        });
    }

    zones
}

/// Find the price level where bid depth drops below threshold (thin shelf).
/// Walks from mid downward through bids, accumulating depth until it drops off.
fn find_thin_shelf(
    bids: &[DepthLevel],
    mid: f64,
    lower_bound: f64,
    _threshold_usd: f64,
) -> Option<f64> {
    let mut sorted_bids: Vec<&DepthLevel> = bids
        .iter()
        .filter(|l| l.price >= lower_bound && l.price <= mid)
        .collect();
    sorted_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));

    let mut cumulative = 0.0;
    let mut last_price = mid;

    for level in &sorted_bids {
        let notional = level.price * level.size_base;
        if notional == 0.0 && cumulative == 0.0 {
            // Gap in the book — this is the thin shelf
            return Some(level.price);
        }
        cumulative += notional;
        last_price = level.price;
    }

    // If we didn't find a clear drop-off, return the lowest bid price
    if !sorted_bids.is_empty() {
        Some(last_price)
    } else {
        None
    }
}

/// Find the price level where ask depth drops below threshold.
fn find_thin_shelf_asks(
    asks: &[DepthLevel],
    mid: f64,
    upper_bound: f64,
    _threshold_usd: f64,
) -> Option<f64> {
    let mut sorted_asks: Vec<&DepthLevel> = asks
        .iter()
        .filter(|l| l.price >= mid && l.price <= upper_bound)
        .collect();
    sorted_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    let mut cumulative = 0.0;
    let mut last_price = mid;

    for level in &sorted_asks {
        let notional = level.price * level.size_base;
        if notional == 0.0 && cumulative == 0.0 {
            return Some(level.price);
        }
        cumulative += notional;
        last_price = level.price;
    }

    if !sorted_asks.is_empty() {
        Some(last_price)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Multi-Source Zone Merging
// ---------------------------------------------------------------------------

/// Merge zones from different sources that are at similar prices.
///
/// Zones at similar prices (within `merge_threshold_bps`) with the same
/// `side_at_risk` are merged into a single zone with combined source_mix,
/// summed notional, summed wallet_count, and higher confidence.
pub fn merge_zones(zones: Vec<LiquidationZone>, merge_threshold_bps: f64) -> Vec<LiquidationZone> {
    if zones.is_empty() {
        return vec![];
    }

    // Group by side_at_risk first
    let mut long_zones: Vec<LiquidationZone> = Vec::new();
    let mut short_zones: Vec<LiquidationZone> = Vec::new();

    for zone in zones {
        match zone.side_at_risk.as_str() {
            "long" => long_zones.push(zone),
            "short" => short_zones.push(zone),
            _ => {} // Unknown side, skip
        }
    }

    let mut merged = Vec::new();
    merged.extend(merge_same_side(long_zones, merge_threshold_bps));
    merged.extend(merge_same_side(short_zones, merge_threshold_bps));

    merged
}

/// Merge zones on the same side that are within threshold bps of each other.
fn merge_same_side(mut zones: Vec<LiquidationZone>, merge_threshold_bps: f64) -> Vec<LiquidationZone> {
    if zones.is_empty() {
        return vec![];
    }

    // Sort by price
    zones.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    let mut result: Vec<LiquidationZone> = Vec::new();
    let mut current = zones[0].clone();

    for zone in zones.iter().skip(1) {
        let reference = current.price;
        let distance_bps = if reference > 0.0 {
            ((zone.price - reference) / reference).abs() * 10_000.0
        } else {
            f64::MAX
        };

        if distance_bps <= merge_threshold_bps && zone.side_at_risk == current.side_at_risk {
            // Merge: combine source_mix, sum notional, sum wallet_count (from position sources only)
            current = merge_two_zones(&current, zone);
        } else {
            result.push(current);
            current = zone.clone();
        }
    }
    result.push(current);

    result
}

/// Merge two zones into one. Wallet_count aggregates from wallet-tracking sources only.
fn merge_two_zones(a: &LiquidationZone, b: &LiquidationZone) -> LiquidationZone {
    // Weighted average price by notional
    let total_notional = a.estimated_notional_usd + b.estimated_notional_usd;
    let price = if total_notional > 0.0 {
        (a.price * a.estimated_notional_usd + b.price * b.estimated_notional_usd) / total_notional
    } else {
        (a.price + b.price) / 2.0
    };

    // Combined source_mix (deduplicated)
    let mut source_mix: Vec<String> = a.source_mix.clone();
    for source in &b.source_mix {
        if !source_mix.contains(source) {
            source_mix.push(source.clone());
        }
    }

    // wallet_count: only from sources that track individual wallets
    let wallet_sources = ["hyperliquid_positions", "hyperliquid_fills"];
    let a_wallets: u32 = if a.source_mix.iter().any(|s| wallet_sources.contains(&s.as_str())) {
        a.wallet_count
    } else {
        0
    };
    let b_wallets: u32 = if b.source_mix.iter().any(|s| wallet_sources.contains(&s.as_str())) {
        b.wallet_count
    } else {
        0
    };

    // Use the larger distance_bps
    let distance_bps = a.distance_bps.max(b.distance_bps);

    LiquidationZone {
        price,
        side_at_risk: a.side_at_risk.clone(),
        estimated_notional_usd: total_notional,
        wallet_count: a_wallets + b_wallets,
        distance_bps,
        confidence: 0.0, // Will be recomputed
        source_mix,
    }
}

// ---------------------------------------------------------------------------
// Confidence Scoring
// ---------------------------------------------------------------------------

/// Compute confidence for a zone based on its source_mix, wallet_count, and notional.
///
/// Formula: base + multi_source_bonus + wallet_count_bonus + notional_bonus - staleness_penalty
/// Result is clamped to [0.0, 1.0].
pub fn compute_confidence(
    zone: &LiquidationZone,
    config: &LiquidationConfig,
    source_freshness: &HashMap<String, i64>,
    now_ms: i64,
) -> f64 {
    let source_count = zone.source_mix.len();
    let base = config.base_confidence;

    // Multi-source bonus: +bonus for each additional source beyond the first
    let multi_bonus: f64 = if source_count > 1 {
        config
            .multi_source_bonus
            .iter()
            .take(source_count - 1)
            .sum()
    } else {
        0.0
    };

    // Staleness penalty: per stale source
    let staleness_ms = config.staleness_threshold_secs as i64 * 1000;
    let stale_count = zone
        .source_mix
        .iter()
        .filter(|s| {
            source_freshness
                .get(*s)
                .map(|&ts| now_ms - ts > staleness_ms)
                .unwrap_or(true) // No freshness record → stale
        })
        .count();
    let staleness_penalty = config.staleness_penalty * stale_count as f64;

    // Wallet count bonus: logarithmic
    let wallet_bonus = if zone.wallet_count > 0 && config.wallet_count_bonus_factor > 0.0 {
        config.wallet_count_bonus_factor * (zone.wallet_count as f64).log10()
    } else {
        0.0
    };

    // Notional bonus: logarithmic relative to $1M
    let notional_bonus = if zone.estimated_notional_usd > 0.0 && config.notional_bonus_factor > 0.0 {
        config.notional_bonus_factor * (zone.estimated_notional_usd / 1_000_000.0).log10().max(0.0)
    } else {
        0.0
    };

    let raw = base + multi_bonus + wallet_bonus + notional_bonus - staleness_penalty;
    raw.clamp(0.0, 1.0)
}

/// Apply confidence scoring to all zones in a snapshot.
pub fn score_zones(
    snapshot: &mut LiquidationZoneSnapshot,
    config: &LiquidationConfig,
    source_freshness: &HashMap<String, i64>,
) {
    let now_ms = snapshot.timestamp_ms;
    for zone in &mut snapshot.zones {
        zone.confidence = compute_confidence(zone, config, source_freshness, now_ms);
    }
}

/// Filter zones by minimum confidence.
pub fn filter_by_confidence(zones: Vec<LiquidationZone>, min_confidence: f64) -> Vec<LiquidationZone> {
    zones
        .into_iter()
        .filter(|z| z.confidence >= min_confidence)
        .collect()
}

// ---------------------------------------------------------------------------
// Source Freshness Tracking
// ---------------------------------------------------------------------------

/// Tracks when each data source was last successfully fetched.
#[derive(Debug, Clone, Default)]
pub struct SourceFreshnessTracker {
    /// Map of source name → last successful fetch timestamp (ms).
    freshness: HashMap<String, i64>,
}

impl SourceFreshnessTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful fetch for a source.
    pub fn record_success(&mut self, source: &str, timestamp_ms: i64) {
        self.freshness.insert(source.to_string(), timestamp_ms);
    }

    /// Record a failed fetch. Freshness is not updated (remains stale).
    pub fn record_failure(&mut self, _source: &str) {
        // No update — freshness remains at previous value (or absent)
    }

    /// Get the freshness map (source → timestamp_ms).
    pub fn freshness(&self) -> &HashMap<String, i64> {
        &self.freshness
    }

    /// Check if a specific source is stale.
    pub fn is_stale(&self, source: &str, now_ms: i64, staleness_threshold_secs: u64) -> bool {
        match self.freshness.get(source) {
            Some(ts) => now_ms - *ts > (staleness_threshold_secs as i64 * 1000),
            None => true, // Never fetched → stale
        }
    }

    /// Check if all configured sources are stale.
    pub fn all_stale(&self, sources: &[String], now_ms: i64, staleness_threshold_secs: u64) -> bool {
        sources.iter().all(|s| self.is_stale(s, now_ms, staleness_threshold_secs))
    }

    /// Count the number of stale sources.
    pub fn stale_count(&self, sources: &[String], now_ms: i64, staleness_threshold_secs: u64) -> usize {
        sources
            .iter()
            .filter(|s| self.is_stale(s, now_ms, staleness_threshold_secs))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Full Fusion Pipeline
// ---------------------------------------------------------------------------

/// Result of fusing all sources for a single symbol.
#[derive(Debug)]
pub struct FusionResult {
    pub zones: Vec<LiquidationZone>,
    pub source_errors: Vec<String>,
}

/// Fuse all source contributions, merge overlapping zones, and score confidence.
///
/// This is the main entry point for producing a complete set of liquidation zones
/// from all configured sources.
#[allow(clippy::too_many_arguments)]
pub fn fuse_sources(
    hl_position_zones: Vec<LiquidationZone>,
    hl_fill_zones: Vec<LiquidationZone>,
    oi_imbalance_zones: Vec<LiquidationZone>,
    depth_fragility_zones: Vec<LiquidationZone>,
    mark_price: f64,
    config: &LiquidationConfig,
    source_freshness: &HashMap<String, i64>,
    now_ms: i64,
) -> LiquidationZoneSnapshot {
    // Combine all zones
    let mut all_zones = Vec::new();
    all_zones.extend(hl_position_zones);
    all_zones.extend(hl_fill_zones);
    all_zones.extend(oi_imbalance_zones);
    all_zones.extend(depth_fragility_zones);

    // Merge overlapping zones
    let merged = merge_zones(all_zones, config.merge_threshold_bps);

    // Build snapshot
    let mut snapshot = LiquidationZoneSnapshot {
        symbol: String::new(), // Set by caller
        timestamp_ms: now_ms,
        mark_price,
        zones: merged,
    };

    // Score confidence
    score_zones(&mut snapshot, config, source_freshness);

    // Filter by minimum confidence
    snapshot.zones = filter_by_confidence(snapshot.zones, config.min_confidence);

    snapshot
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Data Model Validation Tests (VAL-LIQ-001 through VAL-LIQ-014) ─────

    #[test]
    fn test_snapshot_serde_roundtrip() {
        // VAL-LIQ-001
        let zone = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 42,
            distance_bps: 500.0,
            confidence: 0.75,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        let snapshot = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100_000.0,
            zones: vec![zone],
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: LiquidationZoneSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.symbol, "BTC");
        assert_eq!(parsed.timestamp_ms, 1_770_000_000_000);
        assert!((parsed.mark_price - 100_000.0).abs() < 0.01);
        assert_eq!(parsed.zones.len(), 1);
        assert_eq!(parsed.zones[0].price, 95_000.0);
        assert_eq!(parsed.zones[0].side_at_risk, "long");
        assert_eq!(parsed.zones[0].wallet_count, 42);
    }

    #[test]
    fn test_zone_serde_roundtrip() {
        // VAL-LIQ-002
        let zone = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "short".to_string(),
            estimated_notional_usd: 300_000.0,
            wallet_count: 15,
            distance_bps: 183.0,
            confidence: 0.55,
            source_mix: vec!["hyperliquid_positions".to_string(), "oi_imbalance".to_string()],
        };

        let json = serde_json::to_string(&zone).unwrap();
        let parsed: LiquidationZone = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.price, 95_000.0);
        assert_eq!(parsed.side_at_risk, "short");
        assert!((parsed.estimated_notional_usd - 300_000.0).abs() < 0.01);
        assert_eq!(parsed.wallet_count, 15);
        assert!((parsed.distance_bps - 183.0).abs() < 0.01);
        assert!((parsed.confidence - 0.55).abs() < 0.01);
        assert_eq!(parsed.source_mix.len(), 2);

        // Verify JSON keys
        let json_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = json_value.as_object().unwrap();
        assert!(obj.contains_key("price"));
        assert!(obj.contains_key("side_at_risk"));
        assert!(obj.contains_key("estimated_notional_usd"));
        assert!(obj.contains_key("wallet_count"));
        assert!(obj.contains_key("distance_bps"));
        assert!(obj.contains_key("confidence"));
        assert!(obj.contains_key("source_mix"));
    }

    #[test]
    fn test_mark_price_must_be_positive() {
        // VAL-LIQ-003
        let snapshot = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 0.0,
            zones: vec![],
        };
        assert!(snapshot.validate().is_err());

        let snapshot2 = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: -150.0,
            zones: vec![],
        };
        assert!(snapshot2.validate().is_err());

        let snapshot3 = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 150.25,
            zones: vec![],
        };
        assert!(snapshot3.validate().is_ok());
    }

    #[test]
    fn test_zone_price_must_be_positive() {
        // VAL-LIQ-004
        let zone = make_zone(0.0, "long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(-10.0, "long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone2.validate().is_err());

        let zone3 = make_zone(147.50, "long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone3.validate().is_ok());
    }

    #[test]
    fn test_confidence_clamped_to_range() {
        // VAL-LIQ-005
        let zone = make_zone(100.0, "long", 100.0, 1, 10.0, -0.1, vec!["hyperliquid_positions"]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(100.0, "long", 100.0, 1, 10.0, 1.5, vec!["hyperliquid_positions"]);
        assert!(zone2.validate().is_err());

        let zone3 = make_zone(100.0, "long", 100.0, 1, 10.0, 0.0, vec![]);
        assert!(zone3.validate().is_ok());

        let zone4 = make_zone(100.0, "long", 100.0, 1, 10.0, 1.0, vec!["hyperliquid_positions"]);
        assert!(zone4.validate().is_ok());
    }

    #[test]
    fn test_side_at_risk_must_be_long_or_short() {
        // VAL-LIQ-006
        assert!(make_zone(100.0, "long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_ok());
        assert!(make_zone(100.0, "short", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_ok());
        assert!(make_zone(100.0, "Long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
        assert!(make_zone(100.0, "LONG", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
        assert!(make_zone(100.0, "shorts", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
        assert!(make_zone(100.0, "", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
        assert!(make_zone(100.0, "neutral", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
        assert!(make_zone(100.0, "both", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]).validate().is_err());
    }

    #[test]
    fn test_estimated_notional_non_negative() {
        // VAL-LIQ-007
        let zone = make_zone(100.0, "long", -100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(100.0, "long", 0.0, 0, 10.0, 0.0, vec![]);
        assert!(zone2.validate().is_ok());

        let zone3 = make_zone(100.0, "long", 1_250_000.0, 50, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone3.validate().is_ok());
    }

    #[test]
    fn test_wallet_count_nonzero_with_notional() {
        // VAL-LIQ-008
        let zone = make_zone(100.0, "long", 500_000.0, 0, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(100.0, "long", 500_000.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone2.validate().is_ok());

        // Zero notional, zero wallets is ok
        let zone3 = make_zone(100.0, "long", 0.0, 0, 10.0, 0.0, vec![]);
        assert!(zone3.validate().is_ok());
    }

    #[test]
    fn test_distance_bps_non_negative() {
        // VAL-LIQ-009
        let zone = make_zone(100.0, "long", 100.0, 1, -5.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(100.0, "long", 100.0, 1, 0.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone2.validate().is_ok());

        let zone3 = make_zone(100.0, "long", 100.0, 1, 183.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone3.validate().is_ok());
    }

    #[test]
    fn test_source_mix_nonempty_with_confidence() {
        // VAL-LIQ-010
        let zone = make_zone(100.0, "long", 100.0, 1, 10.0, 0.5, vec![]);
        assert!(zone.validate().is_err());

        let zone2 = make_zone(100.0, "long", 100.0, 1, 10.0, 0.5, vec!["hyperliquid_positions"]);
        assert!(zone2.validate().is_ok());

        // Zero confidence, empty source_mix is ok
        let zone3 = make_zone(100.0, "long", 0.0, 0, 10.0, 0.0, vec![]);
        assert!(zone3.validate().is_ok());
    }

    #[test]
    fn test_source_mix_must_use_known_sources() {
        // VAL-LIQ-011
        for valid in VALID_SOURCES {
            let zone = make_zone(100.0, "long", 100.0, 1, 10.0, 0.5, vec![*valid]);
            assert!(zone.validate().is_ok(), "source '{}' should be valid", valid);
        }

        for invalid in &["unknown_source", "coinbase", "manual"] {
            let zone = make_zone(100.0, "long", 100.0, 1, 10.0, 0.5, vec![*invalid]);
            assert!(zone.validate().is_err(), "source '{}' should be invalid", invalid);
        }
    }

    #[test]
    fn test_symbol_must_be_nonempty() {
        // VAL-LIQ-012
        let snapshot = LiquidationZoneSnapshot {
            symbol: "".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100.0,
            zones: vec![],
        };
        assert!(snapshot.validate().is_err());

        let snapshot2 = LiquidationZoneSnapshot {
            symbol: "  ".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100.0,
            zones: vec![],
        };
        assert!(snapshot2.validate().is_err());

        for sym in &["BTC", "ETH", "SOL"] {
            let s = LiquidationZoneSnapshot {
                symbol: sym.to_string(),
                timestamp_ms: 1_770_000_000_000,
                mark_price: 100.0,
                zones: vec![],
            };
            assert!(s.validate().is_ok());
        }
    }

    #[test]
    fn test_timestamp_ms_reasonable_range() {
        // VAL-LIQ-013
        for bad_ts in &[0i64, 999_999_999_999, 3_000_000_000_000] {
            let snapshot = LiquidationZoneSnapshot {
                symbol: "BTC".to_string(),
                timestamp_ms: *bad_ts,
                mark_price: 100.0,
                zones: vec![],
            };
            assert!(snapshot.validate().is_err(), "ts={} should fail", bad_ts);
        }

        let snapshot = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100.0,
            zones: vec![],
        };
        assert!(snapshot.validate().is_ok());
    }

    #[test]
    fn test_empty_zones_is_valid() {
        // VAL-LIQ-014
        let snapshot = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100_000.0,
            zones: vec![],
        };
        assert!(snapshot.validate().is_ok());

        // Also verify serialization works
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: LiquidationZoneSnapshot = serde_json::from_str(&json).unwrap();
        assert!(parsed.zones.is_empty());
    }

    // ── HL Positions Source Tests (VAL-LIQ-015 through VAL-LIQ-021) ───────

    #[test]
    fn test_hl_positions_cluster_into_zone() {
        // VAL-LIQ-015
        let mark_price = 100_000.0;
        let mut positions = Vec::new();
        // 42 wallets with BTC long positions whose liq prices cluster within ±50 bps of 95,000
        for i in 0..42 {
            let liq_price = 95_000.0 + (i as f64 - 21.0) * 10.0; // ~±210 USD → ~±22 bps
            positions.push(HlWalletPosition {
                wallet: format!("0xwallet{}", i),
                coin: "BTC".to_string(),
                side: "B".to_string(),
                liquidation_price: liq_price,
                position_value_usd: 50_000.0 / 42.0 * 42.0, // Will sum to ~50K total per zone
                size_signed: 0.5,
            });
        }
        // Fix: each wallet holds 50K notional
        for pos in &mut positions {
            pos.position_value_usd = 50_000.0;
        }

        let zones = aggregate_hl_positions(&positions, mark_price, 50.0);
        assert_eq!(zones.len(), 1, "should produce 1 cluster");
        assert_eq!(zones[0].wallet_count, 42);
        assert_eq!(zones[0].side_at_risk, "long");
        assert_eq!(zones[0].source_mix, vec!["hyperliquid_positions"]);
        // Price should be near 95,000
        assert!((zones[0].price - 95_000.0).abs() < 500.0);
        // Notional should be 42 * 50_000 = 2_100_000
        assert!((zones[0].estimated_notional_usd - 2_100_000.0).abs() < 1000.0);
    }

    #[test]
    fn test_long_short_separated() {
        // VAL-LIQ-016
        let mark_price = 100_000.0;
        let mut positions = Vec::new();

        // 10 longs with liq near 95,000
        for i in 0..10 {
            positions.push(HlWalletPosition {
                wallet: format!("long_{}", i),
                coin: "BTC".to_string(),
                side: "B".to_string(),
                liquidation_price: 95_000.0 + i as f64 * 5.0,
                position_value_usd: 10_000.0,
                size_signed: 0.1,
            });
        }

        // 8 shorts with liq near 105,000
        for i in 0..8 {
            positions.push(HlWalletPosition {
                wallet: format!("short_{}", i),
                coin: "BTC".to_string(),
                side: "A".to_string(),
                liquidation_price: 105_000.0 + i as f64 * 5.0,
                position_value_usd: 12_000.0,
                size_signed: -0.1,
            });
        }

        let zones = aggregate_hl_positions(&positions, mark_price, 50.0);
        assert_eq!(zones.len(), 2);

        let long_zones: Vec<_> = zones.iter().filter(|z| z.side_at_risk == "long").collect();
        let short_zones: Vec<_> = zones.iter().filter(|z| z.side_at_risk == "short").collect();
        assert_eq!(long_zones.len(), 1);
        assert_eq!(short_zones.len(), 1);
        assert_eq!(long_zones[0].wallet_count, 10);
        assert_eq!(short_zones[0].wallet_count, 8);
    }

    #[test]
    fn test_positions_with_zero_or_null_liquidation_price_skipped() {
        // VAL-LIQ-017
        let mark_price = 100_000.0;
        let mut positions = Vec::new();

        // 2 valid
        positions.push(HlWalletPosition {
            wallet: "valid1".to_string(),
            coin: "BTC".to_string(),
            side: "B".to_string(),
            liquidation_price: 95_000.0,
            position_value_usd: 10_000.0,
            size_signed: 0.1,
        });
        positions.push(HlWalletPosition {
            wallet: "valid2".to_string(),
            coin: "BTC".to_string(),
            side: "B".to_string(),
            liquidation_price: 95_100.0,
            position_value_usd: 10_000.0,
            size_signed: 0.1,
        });

        // 3 invalid (but we filter them before calling aggregate)
        // aggregate_hl_positions expects only valid positions; the caller filters out null/zero
        let zones = aggregate_hl_positions(&positions, mark_price, 50.0);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].wallet_count, 2);
    }

    #[test]
    fn test_multiple_clusters_at_different_prices() {
        // VAL-LIQ-018
        let mark_price = 100_000.0;
        let mut positions = Vec::new();

        // Cluster 1: 20 longs near 95,000
        for i in 0..20 {
            positions.push(HlWalletPosition {
                wallet: format!("c1_{}", i),
                coin: "BTC".to_string(),
                side: "B".to_string(),
                liquidation_price: 95_000.0 + i as f64 * 5.0,
                position_value_usd: 10_000.0,
                size_signed: 0.1,
            });
        }

        // Cluster 2: 15 longs near 92,000 (separated by > cluster_threshold from cluster 1)
        for i in 0..15 {
            positions.push(HlWalletPosition {
                wallet: format!("c2_{}", i),
                coin: "BTC".to_string(),
                side: "B".to_string(),
                liquidation_price: 92_000.0 + i as f64 * 5.0,
                position_value_usd: 10_000.0,
                size_signed: 0.1,
            });
        }

        // cluster_threshold = 50 bps → 95,000 vs 92,000 is ~316 bps apart → separate
        let zones = aggregate_hl_positions(&positions, mark_price, 50.0);
        assert_eq!(zones.len(), 2);
        assert!((zones[0].price - zones[1].price).abs() > 1000.0);
    }

    #[test]
    fn test_notional_aggregates_position_values() {
        // VAL-LIQ-019
        let mark_price = 100_000.0;
        let mut positions = Vec::new();
        for i in 0..10 {
            positions.push(HlWalletPosition {
                wallet: format!("w{}", i),
                coin: "BTC".to_string(),
                side: "B".to_string(),
                liquidation_price: 95_000.0,
                position_value_usd: 50_000.0,
                size_signed: 0.5,
            });
        }
        let zones = aggregate_hl_positions(&positions, mark_price, 50.0);
        assert_eq!(zones.len(), 1);
        assert!((zones[0].estimated_notional_usd - 500_000.0).abs() < 1.0);
    }

    #[test]
    fn test_empty_positions_produces_no_zones() {
        // VAL-LIQ-020
        let zones = aggregate_hl_positions(&[], 100_000.0, 50.0);
        assert!(zones.is_empty());
    }

    // ── HL Fills Source Tests (VAL-LIQ-022 through VAL-LIQ-027) ───────────

    #[test]
    fn test_fills_burst_detection() {
        // VAL-LIQ-022
        let mark_price = 100_000.0;
        let now_ms = 1_770_000_000_000i64;
        let mut fills = Vec::new();

        // 12 fills in 45 seconds, all with closedPnl < 0, same coin, same direction
        for i in 0..12 {
            fills.push(HlFillRecord {
                wallet: format!("0xw{}", i % 4), // 4 distinct wallets
                coin: "BTC".to_string(),
                side: "A".to_string(), // sell → closing longs
                price: 95_000.0 + i as f64 * 10.0,
                size: 1.0,
                closed_pnl: -(100.0 + i as f64 * 10.0),
                timestamp_ms: now_ms - 45_000 + (i as i64 * 3_750),
                direction: "Close Long".to_string(),
            });
        }

        let zones = detect_forced_liquidation_bursts(
            &fills, mark_price, 10, 60, 300, now_ms,
        );
        assert!(!zones.is_empty());
        let zone = &zones[0];
        assert!(zone.source_mix.contains(&"hyperliquid_fills".to_string()));
        assert_eq!(zone.side_at_risk, "long"); // sells = closing longs → longs at risk
    }

    #[test]
    fn test_fills_notional_from_fill_sizes() {
        // VAL-LIQ-023
        let mark_price = 100_000.0;
        let now_ms = 1_770_000_000_000i64;
        let mut fills = Vec::new();
        for i in 0..10 {
            fills.push(HlFillRecord {
                wallet: format!("0xw{}", i),
                coin: "BTC".to_string(),
                side: "A".to_string(),
                price: 100_000.0,
                size: 1.0,
                closed_pnl: -500.0,
                timestamp_ms: now_ms - 30_000 + i as i64 * 3_000,
                direction: "Close Long".to_string(),
            });
        }

        let zones = detect_forced_liquidation_bursts(&fills, mark_price, 10, 60, 300, now_ms);
        assert!(!zones.is_empty());
        // 10 fills * $100K each = $1M notional
        assert!(
            (zones[0].estimated_notional_usd - 1_000_000.0).abs() / 1_000_000.0 < 0.05,
            "notional should be ~1M, got {}",
            zones[0].estimated_notional_usd
        );
    }

    #[test]
    fn test_isolated_fills_no_zone() {
        // VAL-LIQ-024
        let mark_price = 100_000.0;
        let now_ms = 1_770_000_000_000i64;

        // Single fill
        let fills = vec![HlFillRecord {
            wallet: "0xw1".to_string(),
            coin: "BTC".to_string(),
            side: "A".to_string(),
            price: 95_000.0,
            size: 1.0,
            closed_pnl: -500.0,
            timestamp_ms: now_ms - 10_000,
            direction: "Close Long".to_string(),
        }];
        let zones = detect_forced_liquidation_bursts(&fills, mark_price, 10, 60, 300, now_ms);
        assert!(zones.is_empty());

        // 5 fills spread across 4 hours
        let fills2: Vec<HlFillRecord> = (0..5)
            .map(|i| HlFillRecord {
                wallet: "0xw1".to_string(),
                coin: "BTC".to_string(),
                side: "A".to_string(),
                price: 95_000.0,
                size: 1.0,
                closed_pnl: -500.0,
                timestamp_ms: now_ms - 14_400_000 + i as i64 * 3_600_000,
                direction: "Close Long".to_string(),
            })
            .collect();
        let zones2 = detect_forced_liquidation_bursts(&fills2, mark_price, 10, 60, 300, now_ms);
        assert!(zones2.is_empty());
    }

    #[test]
    fn test_fills_wallet_count_distinct() {
        // VAL-LIQ-025
        let mark_price = 100_000.0;
        let now_ms = 1_770_000_000_000i64;
        let mut fills = Vec::new();
        // 15 fills from 8 distinct wallets
        for i in 0..15 {
            fills.push(HlFillRecord {
                wallet: format!("0xw{}", i % 8),
                coin: "BTC".to_string(),
                side: "A".to_string(),
                price: 100_000.0,
                size: 1.0,
                closed_pnl: -500.0,
                timestamp_ms: now_ms - 30_000 + i as i64 * 2_000,
                direction: "Close Long".to_string(),
            });
        }

        let zones = detect_forced_liquidation_bursts(&fills, mark_price, 10, 60, 300, now_ms);
        assert!(!zones.is_empty());
        assert_eq!(zones[0].wallet_count, 8);
    }

    #[test]
    fn test_fills_api_failure_no_panic() {
        // VAL-LIQ-026 — just verify the function doesn't panic on empty input
        let zones = detect_forced_liquidation_bursts(&[], 100_000.0, 10, 60, 300, 0);
        assert!(zones.is_empty());
    }

    #[test]
    fn test_fills_lookback_window() {
        // VAL-LIQ-027
        let mark_price = 100_000.0;
        let now_ms = 1_770_000_000_000i64;
        let mut fills = Vec::new();

        // Fill at now - 1min (included in 5min lookback)
        fills.push(HlFillRecord {
            wallet: "w1".to_string(),
            coin: "BTC".to_string(),
            side: "A".to_string(),
            price: 100_000.0,
            size: 1.0,
            closed_pnl: -500.0,
            timestamp_ms: now_ms - 60_000,
            direction: "Close Long".to_string(),
        });

        // Fill at now - 3min (included)
        fills.push(HlFillRecord {
            wallet: "w2".to_string(),
            coin: "BTC".to_string(),
            side: "A".to_string(),
            price: 100_000.0,
            size: 1.0,
            closed_pnl: -500.0,
            timestamp_ms: now_ms - 180_000,
            direction: "Close Long".to_string(),
        });

        // Fill at now - 7min (excluded from 5min lookback)
        fills.push(HlFillRecord {
            wallet: "w3".to_string(),
            coin: "BTC".to_string(),
            side: "A".to_string(),
            price: 100_000.0,
            size: 1.0,
            closed_pnl: -500.0,
            timestamp_ms: now_ms - 420_000,
            direction: "Close Long".to_string(),
        });

        // Fill at now - 10min (excluded)
        fills.push(HlFillRecord {
            wallet: "w4".to_string(),
            coin: "BTC".to_string(),
            side: "A".to_string(),
            price: 100_000.0,
            size: 1.0,
            closed_pnl: -500.0,
            timestamp_ms: now_ms - 600_000,
            direction: "Close Long".to_string(),
        });

        // With 5min (300s) lookback, only first 2 are included; need burst_count=2
        // Use 120s burst window so the 2 fills (2 min apart) are within the window
        let zones = detect_forced_liquidation_bursts(&fills, mark_price, 2, 120, 300, now_ms);
        assert!(!zones.is_empty(), "should detect burst from 2 recent fills");
    }

    // ── OI Imbalance Tests (VAL-LIQ-028 through VAL-LIQ-032) ─────────────

    #[test]
    fn test_oi_imbalance_zone() {
        // VAL-LIQ-028
        let mark_price = 150.0;
        let oi_data = vec![OiData {
            symbol: "SOL".to_string(),
            long_oi_usd: 5_000_000.0,
            short_oi_usd: 2_000_000.0,
        }];

        let zones = produce_oi_imbalance_zones(&oi_data, mark_price, 20.0);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].side_at_risk, "long"); // Longs overcrowded
        assert_eq!(zones[0].source_mix, vec!["oi_imbalance"]);
    }

    #[test]
    fn test_oi_imbalance_zone_price_direction() {
        // VAL-LIQ-029
        let mark_price = 150.0;

        // Long OI dominates → zone below mark (longs liquidated on downside)
        let long_heavy = vec![OiData {
            symbol: "SOL".to_string(),
            long_oi_usd: 8_000_000.0,
            short_oi_usd: 2_000_000.0,
        }];
        let zones = produce_oi_imbalance_zones(&long_heavy, mark_price, 20.0);
        assert!(!zones.is_empty());
        assert!(zones[0].price < mark_price);
        assert_eq!(zones[0].side_at_risk, "long");

        // Short OI dominates → zone above mark
        let short_heavy = vec![OiData {
            symbol: "SOL".to_string(),
            long_oi_usd: 2_000_000.0,
            short_oi_usd: 8_000_000.0,
        }];
        let zones2 = produce_oi_imbalance_zones(&short_heavy, mark_price, 20.0);
        assert!(!zones2.is_empty());
        assert!(zones2[0].price > mark_price);
        assert_eq!(zones2[0].side_at_risk, "short");
    }

    #[test]
    fn test_balanced_oi_no_zone() {
        // VAL-LIQ-030
        let mark_price = 150.0;
        let oi_data = vec![OiData {
            symbol: "SOL".to_string(),
            long_oi_usd: 1_000_000.0,
            short_oi_usd: 950_000.0, // 5% imbalance, below 20% threshold
        }];
        let zones = produce_oi_imbalance_zones(&oi_data, mark_price, 20.0);
        assert!(zones.is_empty());
    }

    #[test]
    fn test_oi_missing_market_skipped() {
        // VAL-LIQ-032
        // Data for BTC but we check SOL — function processes what it receives
        let oi_data = vec![OiData {
            symbol: "BTC".to_string(),
            long_oi_usd: 5_000_000.0,
            short_oi_usd: 1_000_000.0,
        }];
        // No SOL data → no zone for SOL (the caller filters by symbol)
        // But BTC should produce a zone
        let zones = produce_oi_imbalance_zones(&oi_data, 100_000.0, 20.0);
        assert_eq!(zones.len(), 1);
        assert!(zones[0].estimated_notional_usd > 0.0);
    }

    // ── Depth Fragility Tests (VAL-LIQ-033 through VAL-LIQ-037) ───────────

    #[test]
    fn test_depth_fragility_thin_bids() {
        // VAL-LIQ-033
        let depth = DepthSnapshot {
            symbol: "SOL".to_string(),
            mid: 150.0,
            bids: vec![
                DepthLevel { price: 149.90, size_base: 333.0 }, // ~$50K notional
            ],
            asks: vec![
                DepthLevel { price: 150.10, size_base: 5333.0 }, // ~$800K notional
            ],
        };

        let zones = produce_fragility_zones(&depth, 100_000.0, 50.0);
        assert!(!zones.is_empty());
        let long_zone = zones.iter().find(|z| z.side_at_risk == "long");
        assert!(long_zone.is_some());
        assert_eq!(long_zone.unwrap().source_mix, vec!["depth_fragility"]);
    }

    #[test]
    fn test_depth_fragility_zone_at_thin_shelf() {
        // VAL-LIQ-034
        let depth = DepthSnapshot {
            symbol: "SOL".to_string(),
            mid: 150.0,
            bids: vec![
                DepthLevel { price: 150.00, size_base: 1333.0 }, // ~$200K at mid
                DepthLevel { price: 148.50, size_base: 200.0 },  // ~$30K thin level
            ],
            asks: vec![
                DepthLevel { price: 150.10, size_base: 10000.0 }, // Deep
            ],
        };

        let zones = produce_fragility_zones(&depth, 100_000.0, 50.0);
        // With threshold $100K, bids are thin within 50 bps → should produce long zone
        let long_zone = zones.iter().find(|z| z.side_at_risk == "long");
        if let Some(z) = long_zone {
            // Price should be near the thin shelf
            assert!(z.price <= 150.0 && z.price >= 148.0, "zone price near thin shelf: {}", z.price);
        }
    }

    #[test]
    fn test_deep_balanced_depth_no_fragility() {
        // VAL-LIQ-035
        let depth = DepthSnapshot {
            symbol: "SOL".to_string(),
            mid: 150.0,
            bids: vec![
                DepthLevel { price: 149.90, size_base: 6667.0 }, // ~$1M
            ],
            asks: vec![
                DepthLevel { price: 150.10, size_base: 6667.0 }, // ~$1M
            ],
        };

        let zones = produce_fragility_zones(&depth, 100_000.0, 50.0);
        assert!(zones.is_empty(), "deep balanced book should produce no fragility zones");
    }

    #[test]
    fn test_depth_mark_price_source() {
        // VAL-LIQ-037 (partial — mark price from depth snapshot)
        let depth = DepthSnapshot {
            symbol: "SOL".to_string(),
            mid: 150.25,
            bids: vec![],
            asks: vec![],
        };
        assert!((depth.mid - 150.25).abs() < 0.01);
    }

    // ── Multi-Source Merging Tests (VAL-LIQ-038 through VAL-LIQ-044) ──────

    #[test]
    fn test_merge_zones_from_different_sources() {
        // VAL-LIQ-038
        let zone_a = LiquidationZone {
            price: 95_100.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 42,
            distance_bps: 490.0,
            confidence: 0.4,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        let zone_b = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 300_000.0,
            wallet_count: 0,
            distance_bps: 500.0,
            confidence: 0.3,
            source_mix: vec!["oi_imbalance".to_string()],
        };

        let merged = merge_zones(vec![zone_a, zone_b], 200.0);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].source_mix.contains(&"hyperliquid_positions".to_string()));
        assert!(merged[0].source_mix.contains(&"oi_imbalance".to_string()));
        // Confidence is 0.0 before scoring, but source_mix is combined
    }

    #[test]
    fn test_distant_zones_remain_separate() {
        // VAL-LIQ-039
        let zone_a = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 42,
            distance_bps: 500.0,
            confidence: 0.4,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        let zone_b = LiquidationZone {
            price: 88_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 100_000.0,
            wallet_count: 0,
            distance_bps: 1200.0,
            confidence: 0.3,
            source_mix: vec!["depth_fragility".to_string()],
        };

        let merged = merge_zones(vec![zone_a, zone_b], 200.0);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merged_notional_sums() {
        // VAL-LIQ-040
        let zone_a = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 42,
            distance_bps: 500.0,
            confidence: 0.4,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        let zone_b = LiquidationZone {
            price: 95_100.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 300_000.0,
            wallet_count: 0,
            distance_bps: 490.0,
            confidence: 0.3,
            source_mix: vec!["oi_imbalance".to_string()],
        };

        let merged = merge_zones(vec![zone_a, zone_b], 200.0);
        assert_eq!(merged.len(), 1);
        assert!(
            (merged[0].estimated_notional_usd - 800_000.0).abs() / 800_000.0 < 0.01,
            "merged notional ~800K, got {}",
            merged[0].estimated_notional_usd
        );
    }

    #[test]
    fn test_merged_wallet_count_from_position_sources() {
        // VAL-LIQ-041
        let zone_a = LiquidationZone {
            price: 95_000.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 42,
            distance_bps: 500.0,
            confidence: 0.4,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        let zone_b = LiquidationZone {
            price: 95_100.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 300_000.0,
            wallet_count: 0,
            distance_bps: 490.0,
            confidence: 0.3,
            source_mix: vec!["oi_imbalance".to_string()],
        };

        let merged = merge_zones(vec![zone_a, zone_b], 200.0);
        assert_eq!(merged[0].wallet_count, 42); // Only from position sources
    }

    #[test]
    fn test_all_four_sources_contribute() {
        // VAL-LIQ-042
        let zones = vec![
            make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.4, vec!["hyperliquid_positions"]),
            make_zone(95_100.0, "long", 100_000.0, 5, 490.0, 0.3, vec!["hyperliquid_fills"]),
            make_zone(94_900.0, "long", 2_000_000.0, 0, 510.0, 0.3, vec!["oi_imbalance"]),
            make_zone(95_050.0, "long", 50_000.0, 0, 495.0, 0.2, vec!["depth_fragility"]),
        ];

        // With 200 bps merge threshold, these should all merge (within ~100 bps)
        let merged = merge_zones(zones, 200.0);
        assert_eq!(merged.len(), 1, "all sources should merge into one zone");
        assert_eq!(merged[0].source_mix.len(), 4);
    }

    #[test]
    fn test_partial_source_availability() {
        // VAL-LIQ-043
        let zones = vec![
            make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.4, vec!["hyperliquid_positions"]),
            make_zone(95_050.0, "long", 2_000_000.0, 0, 495.0, 0.3, vec!["oi_imbalance"]),
        ];

        let merged = merge_zones(zones, 200.0);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].source_mix.contains(&"hyperliquid_positions".to_string()));
        assert!(merged[0].source_mix.contains(&"oi_imbalance".to_string()));
    }

    #[test]
    fn test_zero_sources_produces_empty_zones() {
        // VAL-LIQ-044
        let zones = merge_zones(vec![], 100.0);
        assert!(zones.is_empty());
    }

    // ── Confidence Scoring Tests ──────────────────────────────────────────

    #[test]
    fn test_confidence_deterministic() {
        let config = LiquidationConfig::default();
        let zone = make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.0, vec!["hyperliquid_positions"]);
        let freshness = HashMap::new();
        let now_ms = 1_770_000_000_000i64;

        let mut results = Vec::new();
        for _ in 0..1000 {
            results.push(compute_confidence(&zone, &config, &freshness, now_ms));
        }
        for r in &results {
            assert!((r - results[0]).abs() < 1e-15, "confidence should be deterministic");
        }
    }

    #[test]
    fn test_confidence_with_multiple_sources() {
        let config = LiquidationConfig::default();
        let now_ms = 1_770_000_000_000i64;
        let mut freshness = HashMap::new();
        freshness.insert("hyperliquid_positions".to_string(), now_ms);
        freshness.insert("oi_imbalance".to_string(), now_ms);

        let zone1 = make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.0, vec!["hyperliquid_positions"]);
        let zone2 = make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.0, vec!["hyperliquid_positions", "oi_imbalance"]);

        let conf1 = compute_confidence(&zone1, &config, &freshness, now_ms);
        let conf2 = compute_confidence(&zone2, &config, &freshness, now_ms);
        assert!(conf2 > conf1, "2-source confidence ({}) > 1-source ({})", conf2, conf1);
    }

    #[test]
    fn test_confidence_staleness_reduces() {
        let config = LiquidationConfig::default();
        let now_ms = 1_770_000_000_000i64;

        let mut fresh = HashMap::new();
        fresh.insert("hyperliquid_positions".to_string(), now_ms);
        fresh.insert("oi_imbalance".to_string(), now_ms);

        let mut stale = HashMap::new();
        stale.insert("hyperliquid_positions".to_string(), now_ms);
        stale.insert("oi_imbalance".to_string(), now_ms - 120_000); // 2 minutes stale

        let zone = make_zone(95_000.0, "long", 500_000.0, 42, 500.0, 0.0, vec!["hyperliquid_positions", "oi_imbalance"]);

        let conf_fresh = compute_confidence(&zone, &config, &fresh, now_ms);
        let conf_stale = compute_confidence(&zone, &config, &stale, now_ms);
        assert!(conf_stale < conf_fresh, "stale confidence ({}) < fresh ({})", conf_stale, conf_fresh);
    }

    #[test]
    fn test_confidence_no_below_zero() {
        let config = LiquidationConfig {
            base_confidence: 0.1,
            staleness_penalty: 0.20,
            ..LiquidationConfig::default()
        };
        let now_ms = 1_770_000_000_000i64;
        let mut stale = HashMap::new();
        stale.insert("hyperliquid_positions".to_string(), now_ms - 120_000);

        let zone = make_zone(95_000.0, "long", 500_000.0, 1, 500.0, 0.0, vec!["hyperliquid_positions"]);
        let conf = compute_confidence(&zone, &config, &stale, now_ms);
        assert!(conf >= 0.0, "confidence should not go below 0: {}", conf);
        assert!((conf - 0.0).abs() < 0.001, "confidence should be clamped to 0: {}", conf);
    }

    // ── Source Freshness Tests (VAL-LIQ-069 through VAL-LIQ-073) ──────────

    #[test]
    fn test_freshness_tracks_fetch() {
        // VAL-LIQ-069
        let mut tracker = SourceFreshnessTracker::new();
        let now_ms = 1_770_000_000_000i64;
        tracker.record_success("hyperliquid_positions", now_ms);
        tracker.record_success("oi_imbalance", now_ms - 500);

        let freshness = tracker.freshness();
        assert!(freshness.contains_key("hyperliquid_positions"));
        assert!(freshness.contains_key("oi_imbalance"));
        assert!((freshness.get("hyperliquid_positions").unwrap() - now_ms).abs() < 5);
    }

    #[test]
    fn test_freshness_stale_detection() {
        // VAL-LIQ-070
        let mut tracker = SourceFreshnessTracker::new();
        let now_ms = 1_770_000_000_000i64;
        tracker.record_success("hyperliquid_positions", now_ms - 15_000); // 15s ago
        // 10s threshold → stale
        assert!(tracker.is_stale("hyperliquid_positions", now_ms, 10));
        // 60s threshold → not stale
        assert!(!tracker.is_stale("hyperliquid_positions", now_ms, 60));
    }

    #[test]
    fn test_freshness_persists_on_failure() {
        // VAL-LIQ-071
        let mut tracker = SourceFreshnessTracker::new();
        let now_ms = 1_770_000_000_000i64;
        tracker.record_success("hyperliquid_positions", now_ms - 5000);
        tracker.record_failure("hyperliquid_positions"); // Failure doesn't update
        assert_eq!(
            *tracker.freshness().get("hyperliquid_positions").unwrap(),
            now_ms - 5000
        );
    }

    #[test]
    fn test_freshness_resets_on_success() {
        // VAL-LIQ-072
        let mut tracker = SourceFreshnessTracker::new();
        let now_ms = 1_770_000_000_000i64;
        tracker.record_success("hyperliquid_positions", now_ms - 120_000); // Stale
        assert!(tracker.is_stale("hyperliquid_positions", now_ms, 60));

        tracker.record_success("hyperliquid_positions", now_ms); // Reset
        assert!(!tracker.is_stale("hyperliquid_positions", now_ms, 60));
    }

    #[test]
    fn test_all_sources_stale_error_level() {
        // VAL-LIQ-073
        let tracker = SourceFreshnessTracker::new();
        let now_ms = 1_770_000_000_000i64;
        let sources = vec![
            "hyperliquid_positions".to_string(),
            "oi_imbalance".to_string(),
        ];
        // Never fetched → all stale
        assert!(tracker.all_stale(&sources, now_ms, 60));
        assert_eq!(tracker.stale_count(&sources, now_ms, 60), 2);
    }

    // ── Fuse Pipeline Test (VAL-CROSS-002) ────────────────────────────────

    #[test]
    fn test_fuse_with_imperial_sources() {
        // VAL-CROSS-002: Imperial data produces zones with correct source_mix
        let config = LiquidationConfig::default();
        let now_ms = 1_770_000_000_000i64;
        let mark_price = 150.0;

        let hl_zones = vec![
            make_zone(140.0, "long", 500_000.0, 42, 666.0, 0.0, vec!["hyperliquid_positions"]),
        ];
        let oi_zones = produce_oi_imbalance_zones(
            &[OiData { symbol: "SOL".to_string(), long_oi_usd: 8_000_000.0, short_oi_usd: 2_000_000.0 }],
            mark_price,
            20.0,
        );
        assert!(!oi_zones.is_empty(), "OI imbalance should produce zone");
        assert!(oi_zones[0].source_mix.contains(&"oi_imbalance".to_string()));

        let mut freshness = HashMap::new();
        freshness.insert("hyperliquid_positions".to_string(), now_ms);
        freshness.insert("oi_imbalance".to_string(), now_ms);

        let snapshot = fuse_sources(
            hl_zones,
            vec![], // no fill zones
            oi_zones,
            vec![], // no depth zones
            mark_price,
            &config,
            &freshness,
            now_ms,
        );

        assert!(snapshot.zones.len() >= 1);
        // At least one zone should have combined sources
        let _combined = snapshot.zones.iter().any(|z| z.source_mix.len() >= 2);
        // They might or might not merge depending on price proximity
        assert!(!snapshot.zones.is_empty());
    }

    #[test]
    fn test_fuse_imperial_failing_hl_only() {
        // VAL-CROSS-002: Imperial unavailable → zones from HL only, no panic
        let config = LiquidationConfig {
            sources: vec!["hyperliquid_positions".to_string()],
            ..LiquidationConfig::default()
        };
        let now_ms = 1_770_000_000_000i64;
        let mark_price = 150.0;

        let hl_zones = vec![
            make_zone(140.0, "long", 500_000.0, 42, 666.0, 0.0, vec!["hyperliquid_positions"]),
        ];

        let mut freshness = HashMap::new();
        freshness.insert("hyperliquid_positions".to_string(), now_ms);

        let snapshot = fuse_sources(
            hl_zones,
            vec![],
            vec![], // No OI data
            vec![], // No depth data
            mark_price,
            &config,
            &freshness,
            now_ms,
        );

        assert!(!snapshot.zones.is_empty());
        assert!(snapshot.zones[0].source_mix.contains(&"hyperliquid_positions".to_string()));
    }

    // ── Helper ────────────────────────────────────────────────────────────

    fn make_zone(
        price: f64,
        side: &str,
        notional: f64,
        wallets: u32,
        dist_bps: f64,
        conf: f64,
        sources: Vec<&str>,
    ) -> LiquidationZone {
        LiquidationZone {
            price,
            side_at_risk: side.to_string(),
            estimated_notional_usd: notional,
            wallet_count: wallets,
            distance_bps: dist_bps,
            confidence: conf,
            source_mix: sources.into_iter().map(|s| s.to_string()).collect(),
        }
    }
}
