//! Liquidity memory map module — zone lifecycle engine.
//!
//! Tracks liquidation zones over time, recording how price interacts with each
//! zone (touches, sweeps, reversals, continuations). Zones are classified as
//! Magnet, Reversal, or Inactive based on their interaction history. A decay
//! scoring system reduces zone quality over time when zones go untouched.
//!
//! **Constraints:**
//! - Intelligence-layer only: no Signal emissions, no trading logic.
//! - No imports from engine, executor, flash_api, or strategy.
//! - Uses `tracing` for all logging (never `println`).
//! - All data persistence uses atomic writes (.tmp → rename).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::liquidation::{LiquidationZone, LiquidationZoneSnapshot};

// ---------------------------------------------------------------------------
// Zone Type Classification
// ---------------------------------------------------------------------------

/// Classification of a memory zone based on its interaction history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneType {
    /// Price is attracted to this zone — high touch count, low sweep count,
    /// moderate-to-high reversal rate, price tends to bounce off it.
    Magnet,
    /// Price sweeps through and then reverses — high sweep count with high
    /// reversal rate. Depth tends to refill after sweep events.
    Reversal,
    /// Zone has decayed — no recent touches, low confidence, effectively
    /// dead and should not be traded.
    Inactive,
}

impl std::fmt::Display for ZoneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoneType::Magnet => write!(f, "magnet"),
            ZoneType::Reversal => write!(f, "reversal"),
            ZoneType::Inactive => write!(f, "inactive"),
        }
    }
}

// ---------------------------------------------------------------------------
// Memory Zone
// ---------------------------------------------------------------------------

/// A liquidation zone tracked over its entire lifecycle.
///
/// Extends the base `LiquidationZone` with lifecycle tracking fields:
/// touch count, sweep count, reversal/continuation rates, excursion stats,
/// decay scoring, and zone classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryZone {
    // --- Price range ---
    /// Lower bound of the zone price range.
    pub low: f64,
    /// Upper bound of the zone price range.
    pub high: f64,

    // --- Identity ---
    /// Side at risk: "long" (longs get liquidated when price drops here)
    /// or "short" (shorts get liquidated when price rises here).
    pub side_at_risk: String,

    // --- Quality ---
    /// Confidence score in [0.0, 1.0].
    pub confidence: f64,

    // --- Provenance ---
    /// Sources that contributed to this zone (e.g., "hyperliquid_positions",
    /// "hyperliquid_fills", "oi_imbalance", "depth_fragility").
    pub source_mix: Vec<String>,

    // --- Lifecycle ---
    /// Age in ticks (number of `update_from_snapshot` calls since creation).
    pub age: u64,

    /// Number of times price approached the zone (within proximity threshold).
    pub touch_count: u64,

    /// Number of times price crossed through the zone.
    pub sweep_count: u64,

    /// Number of times price reversed after touching the zone.
    pub reversal_success_count: u64,

    /// Number of times price continued through after touching/sweeping.
    pub continuation_success_count: u64,

    /// Average maximum price excursion (in USD) away from zone after a touch.
    pub avg_excursion_after_touch: f64,

    /// Average time (in seconds) between consecutive touches.
    pub avg_time_to_touch: f64,

    // --- Quality decay ---
    /// Decay score in [0.0, 1.0]. Higher = more decayed. A decay score of 1.0
    /// means the zone is effectively dead. Updated by `classify_zones()`.
    pub decay_score: f64,

    // --- Classification ---
    /// Current classification of this zone.
    pub zone_type: ZoneType,

    // --- Timestamps ---
    /// Timestamp (ms) when this zone was first created.
    pub created_at_ms: i64,

    /// Timestamp (ms) of the most recent touch or update.
    pub last_updated_ms: i64,

    // --- Internal tracking ---
    /// Sum of all excursion values (for computing average).
    #[serde(default)]
    excursion_sum: f64,

    /// Count of excursion observations (for computing average).
    #[serde(default)]
    excursion_count: u64,

    /// Timestamps of individual touches (for computing avg_time_to_touch).
    #[serde(default)]
    touch_timestamps_ms: Vec<i64>,

    /// Cumulative estimated notional from source zones.
    #[serde(default)]
    pub estimated_notional_usd: f64,
}

impl MemoryZone {
    /// Create a new MemoryZone from a liquidation zone snapshot entry.
    ///
    /// The zone's price range is derived from the point price ± a small margin
    /// based on the `distance_bps` field, or from explicit low/high if provided.
    /// The margin defaults to `range_bps / 2.0` where range_bps is the configurable
    /// zone width.
    pub fn from_liquidation_zone(zone: &LiquidationZone, created_at_ms: i64, range_bps: f64) -> Self {
        let half_range = zone.price * (range_bps / 2.0) / 10_000.0;
        let low = (zone.price - half_range).max(0.0);
        let high = zone.price + half_range;

        Self {
            low,
            high,
            side_at_risk: zone.side_at_risk.clone(),
            confidence: zone.confidence,
            source_mix: zone.source_mix.clone(),
            age: 0,
            touch_count: 0,
            sweep_count: 0,
            reversal_success_count: 0,
            continuation_success_count: 0,
            avg_excursion_after_touch: 0.0,
            avg_time_to_touch: 0.0,
            decay_score: 0.0,
            zone_type: ZoneType::Inactive, // Will be classified later
            created_at_ms,
            last_updated_ms: created_at_ms,
            excursion_sum: 0.0,
            excursion_count: 0,
            touch_timestamps_ms: Vec::new(),
            estimated_notional_usd: zone.estimated_notional_usd,
        }
    }

    /// Record a touch event (price approached within proximity).
    ///
    /// `current_price` is used to compute the excursion from the zone midpoint.
    /// `timestamp_ms` is used for time-to-touch statistics.
    pub fn record_touch(&mut self, current_price: f64, timestamp_ms: i64) {
        self.touch_count += 1;
        self.last_updated_ms = timestamp_ms;

        // Compute excursion from zone midpoint
        let midpoint = (self.low + self.high) / 2.0;
        let excursion = (current_price - midpoint).abs();
        self.excursion_sum += excursion;
        self.excursion_count += 1;
        self.avg_excursion_after_touch = self.excursion_sum / self.excursion_count as f64;

        // Track time between touches
        self.touch_timestamps_ms.push(timestamp_ms);
        if self.touch_timestamps_ms.len() >= 2 {
            let intervals: Vec<f64> = self.touch_timestamps_ms
                .windows(2)
                .map(|w| (w[1] - w[0]) as f64 / 1000.0) // Convert ms to seconds
                .collect();
            self.avg_time_to_touch = intervals.iter().sum::<f64>() / intervals.len() as f64;
        }
    }

    /// Record a sweep event (price crossed through the zone).
    pub fn record_sweep(&mut self, timestamp_ms: i64) {
        self.sweep_count += 1;
        self.last_updated_ms = timestamp_ms;
    }

    /// Record a reversal outcome — price touched/swept the zone then reversed.
    pub fn record_reversal(&mut self) {
        self.reversal_success_count += 1;
    }

    /// Record a continuation outcome — price continued through after zone interaction.
    pub fn record_continuation(&mut self) {
        self.continuation_success_count += 1;
    }

    /// Compute the reversal rate: reversals / (reversals + continuations).
    ///
    /// Returns 0.0 if no interactions have been recorded.
    pub fn reversal_rate(&self) -> f64 {
        let total = self.reversal_success_count + self.continuation_success_count;
        if total == 0 {
            return 0.0;
        }
        self.reversal_success_count as f64 / total as f64
    }

    /// Compute the continuation rate: continuations / (reversals + continuations).
    ///
    /// Returns 0.0 if no interactions have been recorded.
    pub fn continuation_rate(&self) -> f64 {
        let total = self.reversal_success_count + self.continuation_success_count;
        if total == 0 {
            return 0.0;
        }
        self.continuation_success_count as f64 / total as f64
    }

    /// Check if a price is within the zone range.
    pub fn contains_price(&self, price: f64) -> bool {
        price >= self.low && price <= self.high
    }

    /// Check if a price is near the zone (within `proximity_bps` basis points).
    pub fn is_near(&self, price: f64, proximity_bps: f64) -> bool {
        let midpoint = (self.low + self.high) / 2.0;
        if midpoint <= 0.0 {
            return false;
        }
        let distance_bps = ((price - midpoint) / midpoint).abs() * 10_000.0;
        distance_bps <= proximity_bps
    }

    /// Distance from a price to the nearest edge of this zone, in bps.
    /// Returns 0.0 if the price is inside the zone.
    pub fn distance_bps(&self, price: f64) -> f64 {
        if self.contains_price(price) {
            return 0.0;
        }
        let reference = (self.low + self.high) / 2.0;
        if reference <= 0.0 {
            return f64::MAX;
        }
        let nearest_edge = if price < self.low { self.low } else { self.high };
        ((price - nearest_edge) / reference).abs() * 10_000.0
    }

    /// Validate all fields in the memory zone.
    pub fn validate(&self) -> Result<()> {
        if self.low <= 0.0 {
            anyhow::bail!("zone low must be > 0.0, got {}", self.low);
        }
        if self.high <= 0.0 {
            anyhow::bail!("zone high must be > 0.0, got {}", self.high);
        }
        if self.low >= self.high {
            anyhow::bail!(
                "zone low ({}) must be < zone high ({})",
                self.low,
                self.high
            );
        }
        if self.side_at_risk != "long" && self.side_at_risk != "short" {
            anyhow::bail!(
                "side_at_risk must be 'long' or 'short', got '{}'",
                self.side_at_risk
            );
        }
        if self.confidence < 0.0 || self.confidence > 1.0 {
            anyhow::bail!(
                "confidence must be in [0.0, 1.0], got {}",
                self.confidence
            );
        }
        if self.decay_score < 0.0 || self.decay_score > 1.0 {
            anyhow::bail!(
                "decay_score must be in [0.0, 1.0], got {}",
                self.decay_score
            );
        }
        Ok(())
    }

    /// Merge another zone's data into this one. Combines source_mix, updates
    /// confidence to the higher of the two, and sums notional.
    pub fn merge_from(&mut self, other: &LiquidationZone) {
        // Update confidence: take the higher value
        self.confidence = self.confidence.max(other.confidence);

        // Merge source_mix (deduplicate)
        for source in &other.source_mix {
            if !self.source_mix.contains(source) {
                self.source_mix.push(source.clone());
            }
        }

        // Sum notional
        self.estimated_notional_usd += other.estimated_notional_usd;

        // Widen price range if needed
        self.low = self.low.min(other.price);
        self.high = self.high.max(other.price);
    }
}

// ---------------------------------------------------------------------------
// Liquidity Memory Map
// ---------------------------------------------------------------------------

/// Configuration for the liquidity memory map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryMapConfig {
    /// Zone price range width in basis points (default: 50 = 0.5%).
    pub zone_range_bps: f64,
    /// Proximity threshold for touch detection in basis points (default: 100).
    pub touch_proximity_bps: f64,
    /// Decay half-life in seconds — time for decay score to reach 0.5 (default: 86400 = 24h).
    pub decay_half_life_secs: f64,
    /// Minimum touches to be classified as Magnet (default: 3).
    pub magnet_min_touches: u64,
    /// Maximum sweep-to-touch ratio for Magnet classification (default: 0.3).
    pub magnet_max_sweep_ratio: f64,
    /// Minimum reversal rate for Reversal classification (default: 0.5).
    pub reversal_min_reversal_rate: f64,
    /// Minimum sweeps to be classified as Reversal (default: 2).
    pub reversal_min_sweeps: u64,
    /// Maximum age (in ticks) before a zone becomes Inactive (default: 100).
    pub inactive_max_age: u64,
    /// Decay score threshold above which zones are Inactive (default: 0.8).
    pub inactive_decay_threshold: f64,
    /// Minimum confidence for a zone to be eligible for fishing (default: 0.3).
    pub fishing_min_confidence: f64,
    /// Maximum decay score for a zone to be eligible for fishing (default: 0.5).
    pub fishing_max_decay: f64,
}

impl Default for MemoryMapConfig {
    fn default() -> Self {
        Self {
            zone_range_bps: 50.0,
            touch_proximity_bps: 100.0,
            decay_half_life_secs: 86400.0,
            magnet_min_touches: 3,
            magnet_max_sweep_ratio: 0.3,
            reversal_min_reversal_rate: 0.5,
            reversal_min_sweeps: 2,
            inactive_max_age: 100,
            inactive_decay_threshold: 0.8,
            fishing_min_confidence: 0.3,
            fishing_max_decay: 0.5,
        }
    }
}

impl MemoryMapConfig {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.zone_range_bps <= 0.0 {
            anyhow::bail!("zone_range_bps must be > 0, got {}", self.zone_range_bps);
        }
        if self.touch_proximity_bps <= 0.0 {
            anyhow::bail!("touch_proximity_bps must be > 0, got {}", self.touch_proximity_bps);
        }
        if self.decay_half_life_secs <= 0.0 {
            anyhow::bail!("decay_half_life_secs must be > 0, got {}", self.decay_half_life_secs);
        }
        if self.fishing_min_confidence < 0.0 || self.fishing_min_confidence > 1.0 {
            anyhow::bail!(
                "fishing_min_confidence must be in [0.0, 1.0], got {}",
                self.fishing_min_confidence
            );
        }
        if self.fishing_max_decay < 0.0 || self.fishing_max_decay > 1.0 {
            anyhow::bail!(
                "fishing_max_decay must be in [0.0, 1.0], got {}",
                self.fishing_max_decay
            );
        }
        Ok(())
    }
}

/// Per-symbol liquidity memory map tracking zone lifecycles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiquidityMemoryMap {
    /// Symbol this map tracks (e.g., "BTC", "ETH", "SOL").
    pub symbol: String,
    /// Active memory zones.
    pub zones: Vec<MemoryZone>,
    /// Configuration for classification and decay.
    #[serde(default)]
    pub config: MemoryMapConfig,
    /// Timestamp of the last update.
    pub last_update_ms: i64,
    /// Timestamp of map creation.
    pub created_at_ms: i64,
    /// Merge threshold in bps for combining nearby zones.
    #[serde(default = "default_merge_threshold_bps")]
    pub merge_threshold_bps: f64,
}

fn default_merge_threshold_bps() -> f64 {
    50.0
}

impl LiquidityMemoryMap {
    /// Create a new empty memory map for a symbol.
    pub fn new(symbol: &str, config: MemoryMapConfig, created_at_ms: i64) -> Self {
        Self {
            symbol: symbol.to_string(),
            zones: Vec::new(),
            config,
            last_update_ms: created_at_ms,
            created_at_ms,
            merge_threshold_bps: 50.0,
        }
    }

    // -----------------------------------------------------------------------
    // Update from snapshot
    // -----------------------------------------------------------------------

    /// Update the memory map from a new liquidation zone snapshot.
    ///
    /// For each zone in the snapshot:
    /// 1. Check if a matching zone exists (within `merge_threshold_bps`).
    /// 2. If yes, merge data into the existing zone (confidence, source_mix, notional).
    /// 3. If no, create a new MemoryZone from the snapshot entry.
    /// 4. Increment the age of all zones.
    pub fn update_from_snapshot(&mut self, snapshot: &LiquidationZoneSnapshot) {
        if snapshot.symbol != self.symbol {
            return;
        }

        self.last_update_ms = snapshot.timestamp_ms;

        for zone in &snapshot.zones {
            // Try to find an existing zone to merge with
            if let Some(existing) = self.find_matching_zone_mut(zone.price, &zone.side_at_risk) {
                existing.merge_from(zone);
                existing.last_updated_ms = snapshot.timestamp_ms;
            } else {
                // Create new memory zone
                let memory_zone = MemoryZone::from_liquidation_zone(
                    zone,
                    snapshot.timestamp_ms,
                    self.config.zone_range_bps,
                );
                self.zones.push(memory_zone);
            }
        }

        // Increment age of all zones
        for zone in &mut self.zones {
            zone.age += 1;
        }
    }

    /// Process a price update — detect touches and sweeps against all zones.
    ///
    /// Returns a vector of zone touch/sweep events that occurred.
    pub fn process_price(&mut self, price: f64, timestamp_ms: i64) -> Vec<ZoneEvent> {
        let mut events = Vec::new();

        for zone in &mut self.zones {
            let was_near = zone.is_near(price, self.config.touch_proximity_bps);
            let crossed = zone.contains_price(price);

            if crossed {
                // Price is inside the zone → sweep
                zone.record_sweep(timestamp_ms);
                events.push(ZoneEvent {
                    zone_low: zone.low,
                    zone_high: zone.high,
                    side_at_risk: zone.side_at_risk.clone(),
                    event_type: ZoneEventType::Sweep,
                    price,
                    timestamp_ms,
                });
            } else if was_near {
                // Price is near but not inside → touch
                zone.record_touch(price, timestamp_ms);
                events.push(ZoneEvent {
                    zone_low: zone.low,
                    zone_high: zone.high,
                    side_at_risk: zone.side_at_risk.clone(),
                    event_type: ZoneEventType::Touch,
                    price,
                    timestamp_ms,
                });
            }
        }

        events
    }

    /// Record a reversal outcome for a zone near the given price.
    pub fn record_reversal_at(&mut self, price: f64) {
        for zone in &mut self.zones {
            if zone.contains_price(price) || zone.is_near(price, self.config.touch_proximity_bps) {
                zone.record_reversal();
            }
        }
    }

    /// Record a continuation outcome for a zone near the given price.
    pub fn record_continuation_at(&mut self, price: f64) {
        for zone in &mut self.zones {
            if zone.contains_price(price) || zone.is_near(price, self.config.touch_proximity_bps) {
                zone.record_continuation();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Classification
    // -----------------------------------------------------------------------

    /// Classify all zones based on their lifecycle statistics.
    ///
    /// Also updates decay scores based on time since last touch.
    pub fn classify_zones(&mut self, now_ms: i64) {
        for zone in &mut self.zones {
            // Compute decay score based on time since last update
            let elapsed_secs = (now_ms - zone.last_updated_ms).max(0) as f64 / 1000.0;
            let half_life = self.config.decay_half_life_secs;
            // Exponential decay: decay_score = 1 - 0.5^(elapsed / half_life)
            // At half_life seconds, decay is 0.5. At 2*half_life, decay is 0.75.
            zone.decay_score = (1.0 - 0.5_f64.powf(elapsed_secs / half_life)).clamp(0.0, 1.0);

            // Classify based on interaction history
            let total_interactions = zone.touch_count + zone.sweep_count;

            // First check: Inactive (decayed or too old with no activity)
            if zone.decay_score >= self.config.inactive_decay_threshold {
                zone.zone_type = ZoneType::Inactive;
                continue;
            }

            if zone.age > self.config.inactive_max_age && total_interactions == 0 {
                zone.zone_type = ZoneType::Inactive;
                continue;
            }

            // Need some interactions to be classified as non-Inactive
            if total_interactions == 0 {
                zone.zone_type = ZoneType::Inactive;
                continue;
            }

            let sweep_ratio = if zone.touch_count > 0 {
                zone.sweep_count as f64 / zone.touch_count as f64
            } else if zone.sweep_count > 0 {
                1.0 // All sweeps, no touches
            } else {
                0.0
            };

            let reversal_rate = zone.reversal_rate();

            // Reversal zone: swept multiple times with high reversal rate
            if zone.sweep_count >= self.config.reversal_min_sweeps
                && reversal_rate >= self.config.reversal_min_reversal_rate
            {
                zone.zone_type = ZoneType::Reversal;
                continue;
            }

            // Magnet zone: high touch count, low sweep ratio
            if zone.touch_count >= self.config.magnet_min_touches
                && sweep_ratio <= self.config.magnet_max_sweep_ratio
            {
                zone.zone_type = ZoneType::Magnet;
                continue;
            }

            // If we have interactions but don't fit Magnet or Reversal,
            // classify based on which dimension is stronger
            if zone.sweep_count > 0 && reversal_rate > 0.3 {
                zone.zone_type = ZoneType::Reversal;
            } else if zone.touch_count > 0 {
                zone.zone_type = ZoneType::Magnet;
            } else {
                zone.zone_type = ZoneType::Inactive;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Get zones suitable for passive fishing orders.
    ///
    /// Returns Magnet and Reversal zones with confidence above the minimum
    /// and decay below the maximum threshold. Inactive zones are excluded.
    pub fn get_fishing_zones(&self) -> Vec<&MemoryZone> {
        self.zones
            .iter()
            .filter(|z| {
                z.zone_type != ZoneType::Inactive
                    && z.confidence >= self.config.fishing_min_confidence
                    && z.decay_score <= self.config.fishing_max_decay
            })
            .collect()
    }

    /// Get the nearest zone to a given price.
    ///
    /// Returns None if the map is empty.
    pub fn get_nearest_zone(&self, price: f64) -> Option<&MemoryZone> {
        self.zones
            .iter()
            .min_by(|a, b| {
                a.distance_bps(price)
                    .partial_cmp(&b.distance_bps(price))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Get the top N zones sorted by quality (confidence - decay_score).
    ///
    /// Returns fewer than `n` zones if the map has fewer entries.
    pub fn top_zones(&self, n: usize) -> Vec<&MemoryZone> {
        let mut scored: Vec<(f64, &MemoryZone)> = self
            .zones
            .iter()
            .map(|z| {
                let quality = z.confidence * (1.0 - z.decay_score);
                (quality, z)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(n).map(|(_, z)| z).collect()
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Persist the memory map to a JSON file using atomic writes.
    pub fn persist(&self, path: &Path) -> Result<()> {
        let parent = path.parent().with_context(|| {
            format!("path has no parent directory: {}", path.display())
        })?;
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create directory: {}", parent.display())
        })?;

        let tmp_path = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).with_context(|| {
            format!("failed to serialize memory map for {}", self.symbol)
        })?;

        std::fs::write(&tmp_path, &json).with_context(|| {
            format!("failed to write tmp file: {}", tmp_path.display())
        })?;

        std::fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "failed to rename {} -> {}",
                tmp_path.display(),
                path.display()
            )
        })?;

        tracing::debug!(
            symbol = %self.symbol,
            path = %path.display(),
            zone_count = self.zones.len(),
            "persisted liquidity memory map"
        );

        Ok(())
    }

    /// Load a memory map from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path).with_context(|| {
            format!("failed to read memory map file: {}", path.display())
        })?;

        let map: LiquidityMemoryMap = serde_json::from_str(&data).with_context(|| {
            format!("failed to parse memory map JSON: {}", path.display())
        })?;

        tracing::debug!(
            symbol = %map.symbol,
            path = %path.display(),
            zone_count = map.zones.len(),
            "loaded liquidity memory map"
        );

        Ok(map)
    }

    /// Export the memory map as machine-readable JSON string.
    ///
    /// Produces a JSON object with all required fields for each zone.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).with_context(|| {
            format!("failed to serialize memory map for {}", self.symbol)
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Find a zone that matches the given price and side, within merge threshold.
    fn find_matching_zone_mut(
        &mut self,
        price: f64,
        side_at_risk: &str,
    ) -> Option<&mut MemoryZone> {
        self.zones.iter_mut().find(|z| {
            if z.side_at_risk != side_at_risk {
                return false;
            }
            let midpoint = (z.low + z.high) / 2.0;
            if midpoint <= 0.0 {
                return false;
            }
            let distance_bps = ((price - midpoint) / midpoint).abs() * 10_000.0;
            distance_bps <= self.merge_threshold_bps
        })
    }
}

// ---------------------------------------------------------------------------
// Zone Events
// ---------------------------------------------------------------------------

/// An event generated when price interacts with a zone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZoneEvent {
    pub zone_low: f64,
    pub zone_high: f64,
    pub side_at_risk: String,
    pub event_type: ZoneEventType,
    pub price: f64,
    pub timestamp_ms: i64,
}

/// Type of zone interaction event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneEventType {
    /// Price approached within proximity but did not cross.
    Touch,
    /// Price crossed through the zone.
    Sweep,
}

// ---------------------------------------------------------------------------
// JSON output helpers
// ---------------------------------------------------------------------------

/// A simplified JSON-serializable representation of a zone for machine-readable output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneMapEntry {
    pub low: f64,
    pub high: f64,
    pub side_at_risk: String,
    pub confidence: f64,
    pub source_mix: Vec<String>,
    pub age: u64,
    pub touch_count: u64,
    pub sweep_count: u64,
    pub reversal_rate: f64,
    pub continuation_rate: f64,
    pub avg_excursion_after_touch: f64,
    pub avg_time_to_touch: f64,
    pub decay_score: f64,
    pub zone_type: String,
}

impl From<&MemoryZone> for ZoneMapEntry {
    fn from(z: &MemoryZone) -> Self {
        Self {
            low: z.low,
            high: z.high,
            side_at_risk: z.side_at_risk.clone(),
            confidence: z.confidence,
            source_mix: z.source_mix.clone(),
            age: z.age,
            touch_count: z.touch_count,
            sweep_count: z.sweep_count,
            reversal_rate: z.reversal_rate(),
            continuation_rate: z.continuation_rate(),
            avg_excursion_after_touch: z.avg_excursion_after_touch,
            avg_time_to_touch: z.avg_time_to_touch,
            decay_score: z.decay_score,
            zone_type: z.zone_type.to_string(),
        }
    }
}

/// Machine-readable zone map output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneMapOutput {
    pub symbol: String,
    pub zones: Vec<ZoneMapEntry>,
    pub generated_at_ms: i64,
}

impl ZoneMapOutput {
    /// Create a zone map output from a LiquidityMemoryMap.
    pub fn from_map(map: &LiquidityMemoryMap, now_ms: i64) -> Self {
        Self {
            symbol: map.symbol.clone(),
            zones: map.zones.iter().map(ZoneMapEntry::from).collect(),
            generated_at_ms: now_ms,
        }
    }

    /// Serialize to pretty JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .with_context(|| format!("failed to serialize zone map for {}", self.symbol))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper: create a basic LiquidationZone for testing
    fn make_zone(price: f64, side: &str, confidence: f64) -> LiquidationZone {
        LiquidationZone {
            price,
            side_at_risk: side.to_string(),
            estimated_notional_usd: 1_000_000.0,
            wallet_count: 5,
            distance_bps: 500.0,
            confidence,
            source_mix: vec!["hyperliquid_positions".to_string()],
        }
    }

    fn make_snapshot(symbol: &str, timestamp_ms: i64, zones: Vec<LiquidationZone>) -> LiquidationZoneSnapshot {
        LiquidationZoneSnapshot {
            symbol: symbol.to_string(),
            timestamp_ms,
            mark_price: 100_000.0,
            zones,
        }
    }

    fn default_config() -> MemoryMapConfig {
        MemoryMapConfig::default()
    }

    fn make_map(symbol: &str) -> LiquidityMemoryMap {
        LiquidityMemoryMap::new(symbol, default_config(), 1_700_000_000_000)
    }

    // ===================================================================
    // VAL-MEMORY-001 — Touch count tracking
    // ===================================================================

    #[test]
    fn test_touch_count_increments_when_price_approaches() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Price near zone (within 100 bps of midpoint ~99_000)
        // 99_000 * (1 + 0.01) = 99_990, which is within 100 bps
        let events = map.process_price(99_500.0, 1_700_000_001_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ZoneEventType::Touch);
        assert_eq!(map.zones[0].touch_count, 1);

        // Another touch
        let events = map.process_price(99_600.0, 1_700_000_002_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ZoneEventType::Touch);
        assert_eq!(map.zones[0].touch_count, 2);

        // Third touch
        let _events = map.process_price(99_400.0, 1_700_000_003_000);
        assert_eq!(map.zones[0].touch_count, 3);
    }

    #[test]
    fn test_no_touch_when_price_far_away() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Price far from zone
        let events = map.process_price(105_000.0, 1_700_000_001_000);
        assert!(events.is_empty());
        assert_eq!(map.zones[0].touch_count, 0);
    }

    // ===================================================================
    // VAL-MEMORY-002 — Sweep count tracking
    // ===================================================================

    #[test]
    fn test_sweep_count_increments_when_price_crosses_zone() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        let _zone = &map.zones[0];
        // Zone range: price ± 0.25% = 99_000 * 0.0025 = 247.5
        // low = 98752.5, high = 99247.5

        // Price inside zone → sweep
        let events = map.process_price(99_100.0, 1_700_000_001_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ZoneEventType::Sweep);
        assert_eq!(map.zones[0].sweep_count, 1);

        // Another sweep
        let _events = map.process_price(99_000.0, 1_700_000_002_000);
        assert_eq!(map.zones[0].sweep_count, 2);
    }

    #[test]
    fn test_sweep_vs_touch_distinction() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Touch: price near but outside zone
        let events = map.process_price(99_500.0, 1_700_000_001_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ZoneEventType::Touch);
        assert_eq!(map.zones[0].touch_count, 1);
        assert_eq!(map.zones[0].sweep_count, 0);

        // Sweep: price inside zone
        let _events = map.process_price(99_000.0, 1_700_000_002_000);
        assert_eq!(map.zones[0].sweep_count, 1);
    }

    // ===================================================================
    // VAL-MEMORY-003 — Reversal rate tracking
    // ===================================================================

    #[test]
    fn test_reversal_rate_computed_correctly() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Record 3 reversals and 1 continuation → rate = 0.75
        map.record_reversal_at(99_000.0);
        map.record_reversal_at(99_000.0);
        map.record_reversal_at(99_000.0);
        map.record_continuation_at(99_000.0);

        let rate = map.zones[0].reversal_rate();
        assert!((rate - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_reversal_rate_zero_when_no_interactions() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        assert_eq!(map.zones[0].reversal_rate(), 0.0);
    }

    #[test]
    fn test_continuation_rate_computed_correctly() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        map.record_reversal_at(99_000.0);
        map.record_continuation_at(99_000.0);
        map.record_continuation_at(99_000.0);

        let reversal_rate = map.zones[0].reversal_rate();
        let continuation_rate = map.zones[0].continuation_rate();

        // reversal_rate + continuation_rate = 1.0
        assert!((reversal_rate + continuation_rate - 1.0).abs() < 0.001);
        assert!((reversal_rate - 1.0 / 3.0).abs() < 0.001);
        assert!((continuation_rate - 2.0 / 3.0).abs() < 0.001);
    }

    // ===================================================================
    // VAL-MEMORY-004 — Decay scoring
    // ===================================================================

    #[test]
    fn test_decay_score_increases_over_time() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        let config = default_config();

        // At creation time, no decay
        map.classify_zones(1_700_000_000_000);
        assert_eq!(map.zones[0].decay_score, 0.0);

        // After half_life (86400s = 24h), decay should be ~0.5
        let half_life_later = 1_700_000_000_000 + (config.decay_half_life_secs as i64 * 1000);
        map.classify_zones(half_life_later);
        assert!((map.zones[0].decay_score - 0.5).abs() < 0.01);

        // After 2x half_life, decay should be ~0.75
        let two_half_lives = 1_700_000_000_000 + (2 * config.decay_half_life_secs as i64 * 1000);
        map.classify_zones(two_half_lives);
        assert!((map.zones[0].decay_score - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_decay_score_lower_when_recently_touched() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Touch the zone at T+1000ms
        map.process_price(99_500.0, 1_700_000_001_000);

        // Decay at T+1000ms should be low
        map.classify_zones(1_700_000_001_000);
        let decay_after_touch = map.zones[0].decay_score;

        // At T+86400000ms without touching, decay should be higher
        map.classify_zones(1_700_000_000_000 + 86_400_000);
        let decay_later = map.zones[0].decay_score;

        assert!(decay_later > decay_after_touch);
    }

    // ===================================================================
    // VAL-MEMORY-005 — Zone classification: magnet/reversal/inactive
    // ===================================================================

    #[test]
    fn test_classify_magnet_zone() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // High touch count, low sweep ratio → Magnet
        for i in 0..5 {
            map.process_price(99_500.0, 1_700_000_000_000 + (i + 1) * 1000);
        }

        map.classify_zones(1_700_000_005_000);
        assert_eq!(map.zones[0].zone_type, ZoneType::Magnet);
    }

    #[test]
    fn test_classify_reversal_zone() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // High sweep count with reversals → Reversal
        for i in 0..3 {
            map.process_price(99_000.0, 1_700_000_000_000 + (i + 1) * 1000); // sweep
            map.record_reversal_at(99_000.0); // reversed each time
        }

        map.classify_zones(1_700_000_003_000);
        assert_eq!(map.zones[0].zone_type, ZoneType::Reversal);
    }

    #[test]
    fn test_classify_inactive_zone() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // No interactions, classify far in the future → decayed → Inactive
        let far_future = 1_700_000_000_000 + (86400 * 10 * 1000); // 10 days later
        map.classify_zones(far_future);
        assert_eq!(map.zones[0].zone_type, ZoneType::Inactive);
    }

    #[test]
    fn test_classify_magnet_with_touches_and_low_sweeps() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // 5 touches, 0 sweeps → Magnet
        for i in 0..5 {
            map.process_price(99_600.0, 1_700_000_000_000 + (i + 1) * 1000);
        }

        map.classify_zones(1_700_000_005_000);
        assert_eq!(map.zones[0].zone_type, ZoneType::Magnet);
        assert_eq!(map.zones[0].touch_count, 5);
        assert_eq!(map.zones[0].sweep_count, 0);
    }

    // ===================================================================
    // VAL-MEMORY-006 — Persistence: save/load round-trip
    // ===================================================================

    #[test]
    fn test_persist_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("btc_memory.json");

        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);
        map.process_price(99_000.0, 1_700_000_001_000);
        map.record_reversal_at(99_000.0);
        map.classify_zones(1_700_000_002_000);

        map.persist(&path).unwrap();
        let loaded = LiquidityMemoryMap::load(&path).unwrap();

        assert_eq!(loaded, map);
    }

    #[test]
    fn test_persist_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("dir").join("map.json");

        let map = make_map("BTC");
        map.persist(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_persist_atomic_no_partial_on_failure() {
        // Verify no .json.tmp file remains after successful persist
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("map.json");

        let map = make_map("BTC");
        map.persist(&path).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    // ===================================================================
    // VAL-MEMORY-007 — Machine-readable JSON zone map output
    // ===================================================================

    #[test]
    fn test_json_output_includes_all_fields() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);
        map.process_price(99_500.0, 1_700_000_001_000);
        map.process_price(99_000.0, 1_700_000_002_000);
        map.record_reversal_at(99_000.0);
        map.classify_zones(1_700_000_003_000);

        let output = ZoneMapOutput::from_map(&map, 1_700_000_003_000);
        let json = output.to_json().unwrap();

        // Parse back to verify all fields are present
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let zone = &parsed["zones"][0];

        // Required fields from VAL-MEMORY-007/007a-007e
        assert!(zone["low"].is_number(), "missing low");
        assert!(zone["high"].is_number(), "missing high");
        assert!(zone["side_at_risk"].is_string(), "missing side_at_risk");
        assert!(zone["confidence"].is_number(), "missing confidence");
        assert!(zone["source_mix"].is_array(), "missing source_mix");
        assert!(zone["age"].is_number(), "missing age");
        assert!(zone["touch_count"].is_number(), "missing touch_count");
        assert!(zone["sweep_count"].is_number(), "missing sweep_count");
        assert!(zone["reversal_rate"].is_number(), "missing reversal_rate");
        assert!(zone["continuation_rate"].is_number(), "missing continuation_rate");
        assert!(zone["avg_excursion_after_touch"].is_number(), "missing avg_excursion_after_touch");
        assert!(zone["avg_time_to_touch"].is_number(), "missing avg_time_to_touch");
        assert!(zone["decay_score"].is_number(), "missing decay_score");
        assert!(zone["zone_type"].is_string(), "missing zone_type");
    }

    // ===================================================================
    // VAL-MEMORY-007a — Price range (low, high)
    // ===================================================================

    #[test]
    fn test_zone_uses_price_range() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        let zone = &map.zones[0];
        assert!(zone.low > 0.0, "low must be positive");
        assert!(zone.high > 0.0, "high must be positive");
        assert!(zone.low < zone.high, "low must be < high");
    }

    #[test]
    fn test_price_range_from_zone() {
        let config = default_config();
        let zone = make_zone(100_000.0, "short", 0.8);
        let memory_zone = MemoryZone::from_liquidation_zone(&zone, 1_700_000_000_000, config.zone_range_bps);

        // zone_range_bps = 50 → half_range = 100_000 * 0.0025 = 250
        assert!((memory_zone.low - 99_750.0).abs() < 0.01);
        assert!((memory_zone.high - 100_250.0).abs() < 0.01);
    }

    // ===================================================================
    // VAL-MEMORY-007b — Continuation success rate
    // ===================================================================

    #[test]
    fn test_continuation_rate_tracked() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // 2 reversals, 3 continuations → continuation rate = 0.6
        map.record_reversal_at(99_000.0);
        map.record_reversal_at(99_000.0);
        map.record_continuation_at(99_000.0);
        map.record_continuation_at(99_000.0);
        map.record_continuation_at(99_000.0);

        let zone = &map.zones[0];
        assert!((zone.continuation_rate() - 0.6).abs() < 0.001);
        assert!((zone.reversal_rate() + zone.continuation_rate() - 1.0).abs() < 0.001);
    }

    // ===================================================================
    // VAL-MEMORY-007c — Average excursion after touch
    // ===================================================================

    #[test]
    fn test_avg_excursion_after_touch() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Zone midpoint ≈ 99000
        // Touch at 99500 → excursion = |99500 - 99000| = 500
        map.process_price(99_500.0, 1_700_000_001_000);
        assert!((map.zones[0].avg_excursion_after_touch - 500.0).abs() < 0.01);

        // Touch at 98700 → excursion = |98700 - 99000| = 300
        // (This is inside the zone, so it's a sweep, not a touch)
        // Let's use a price far enough to be near but outside
        map.process_price(99_800.0, 1_700_000_002_000);
        // excursion = |99800 - 99000| = 800
        // avg = (500 + 800) / 2 = 650
        assert!((map.zones[0].avg_excursion_after_touch - 650.0).abs() < 0.01);
    }

    // ===================================================================
    // VAL-MEMORY-007d — Average time-to-touch
    // ===================================================================

    #[test]
    fn test_avg_time_to_touch_computed() {
        let mut map = make_map("BTC");
        let snapshot = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snapshot);

        // Touch at T+1000ms
        map.process_price(99_500.0, 1_700_000_001_000);
        // avg_time_to_touch needs 2+ touches
        assert_eq!(map.zones[0].avg_time_to_touch, 0.0);

        // Touch at T+3000ms
        map.process_price(99_600.0, 1_700_000_003_000);
        // Interval: (3000 - 1000) / 1000 = 2.0 seconds
        assert!((map.zones[0].avg_time_to_touch - 2.0).abs() < 0.001);

        // Touch at T+7000ms
        map.process_price(99_400.0, 1_700_000_007_000);
        // Intervals: 2.0s, 4.0s → avg = 3.0s
        assert!((map.zones[0].avg_time_to_touch - 3.0).abs() < 0.001);
    }

    // ===================================================================
    // VAL-MEMORY-007e — All MISSION-SPEC fields present
    // ===================================================================

    #[test]
    fn test_all_required_fields_present() {
        let config = default_config();
        let zone = make_zone(99_000.0, "long", 0.7);
        let memory_zone = MemoryZone::from_liquidation_zone(&zone, 1_700_000_000_000, config.zone_range_bps);

        // Serialize and check all fields present in JSON
        let json = serde_json::to_value(&memory_zone).unwrap();

        // 13+ required fields from MISSION-SPEC
        let required_fields = [
            "low", "high",                          // price range
            "side_at_risk",                          // side
            "confidence",                            // confidence
            "source_mix",                            // provenance
            "age",                                   // age
            "touch_count",                           // touches
            "sweep_count",                           // sweeps
            "reversal_success_count",                // reversal tracking
            "continuation_success_count",            // continuation tracking
            "avg_excursion_after_touch",             // excursion
            "avg_time_to_touch",                     // time-to-touch
            "decay_score",                           // decay
            "zone_type",                             // classification
        ];

        for field in &required_fields {
            assert!(
                json.get(field).is_some(),
                "missing required field: {}",
                field
            );
        }

        // Verify no null values for critical fields
        for field in &["low", "high", "side_at_risk", "confidence", "decay_score", "zone_type"] {
            assert!(
                !json[field].is_null(),
                "field {} is null",
                field
            );
        }
    }

    // ===================================================================
    // Additional coverage tests
    // ===================================================================

    #[test]
    fn test_update_from_snapshot_merges_nearby_zones() {
        let mut map = make_map("BTC");

        // First snapshot with zone at 99000
        let snap1 = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.5)]);
        map.update_from_snapshot(&snap1);
        assert_eq!(map.zones.len(), 1);

        // Second snapshot with nearby zone at 99020 (within merge threshold)
        let snap2 = make_snapshot("BTC", 1_700_000_001_000, vec![make_zone(99_020.0, "long", 0.8)]);
        map.update_from_snapshot(&snap2);
        // Should merge, not create a new zone
        assert_eq!(map.zones.len(), 1);
        // Confidence should be max(0.5, 0.8) = 0.8
        assert!((map.zones[0].confidence - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_update_from_snapshot_creates_new_zone_when_far() {
        let mut map = make_map("BTC");

        let snap1 = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.5)]);
        map.update_from_snapshot(&snap1);
        assert_eq!(map.zones.len(), 1);

        // Far away zone
        let snap2 = make_snapshot("BTC", 1_700_000_001_000, vec![make_zone(95_000.0, "long", 0.6)]);
        map.update_from_snapshot(&snap2);
        assert_eq!(map.zones.len(), 2);
    }

    #[test]
    fn test_update_from_snapshot_ignores_wrong_symbol() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("ETH", 1_700_000_000_000, vec![make_zone(3000.0, "long", 0.7)]);
        map.update_from_snapshot(&snap);
        assert!(map.zones.is_empty());
    }

    #[test]
    fn test_update_from_snapshot_increments_age() {
        let mut map = make_map("BTC");

        let snap1 = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.5)]);
        map.update_from_snapshot(&snap1);
        assert_eq!(map.zones[0].age, 1);

        let snap2 = make_snapshot("BTC", 1_700_000_001_000, vec![]);
        map.update_from_snapshot(&snap2);
        assert_eq!(map.zones[0].age, 2);
    }

    #[test]
    fn test_get_fishing_zones_excludes_inactive() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![
            make_zone(99_000.0, "long", 0.7),
            make_zone(95_000.0, "long", 0.2), // Low confidence
        ]);
        map.update_from_snapshot(&snap);

        // Add touches to first zone to make it Magnet
        for i in 0..5 {
            map.process_price(99_500.0, 1_700_000_000_000 + (i + 1) * 1000);
        }

        map.classify_zones(1_700_000_005_000);

        let fishing = map.get_fishing_zones();
        // Only the high-confidence zone should be included
        for z in &fishing {
            assert!(z.confidence >= map.config.fishing_min_confidence);
            assert!(z.decay_score <= map.config.fishing_max_decay);
            assert_ne!(z.zone_type, ZoneType::Inactive);
        }
    }

    #[test]
    fn test_get_nearest_zone() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![
            make_zone(99_000.0, "long", 0.7),
            make_zone(95_000.0, "long", 0.6),
            make_zone(105_000.0, "short", 0.8),
        ]);
        map.update_from_snapshot(&snap);

        // Nearest to 100_000 should be 99_000 or 105_000 — whichever is closer
        let nearest = map.get_nearest_zone(100_000.0).unwrap();
        assert!((nearest.low - 99_000.0).abs() < 500.0); // Within zone range

        // Nearest to 94_000 should be 95_000
        let nearest = map.get_nearest_zone(94_000.0).unwrap();
        assert!((nearest.low - 95_000.0).abs() < 500.0);
    }

    #[test]
    fn test_get_nearest_zone_empty_map() {
        let map = make_map("BTC");
        assert!(map.get_nearest_zone(100_000.0).is_none());
    }

    #[test]
    fn test_top_zones_sorted_by_quality() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![
            make_zone(99_000.0, "long", 0.3),
            make_zone(95_000.0, "long", 0.9),
            make_zone(105_000.0, "short", 0.6),
        ]);
        map.update_from_snapshot(&snap);
        map.classify_zones(1_700_000_000_000);

        let top2 = map.top_zones(2);
        assert_eq!(top2.len(), 2);
        // Highest quality (confidence * (1 - decay)) should be first
        assert!((top2[0].confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_top_zones_fewer_than_n() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snap);

        let top5 = map.top_zones(5);
        assert_eq!(top5.len(), 1);
    }

    #[test]
    fn test_zone_contains_price() {
        let config = default_config();
        let zone = make_zone(100_000.0, "long", 0.5);
        let mz = MemoryZone::from_liquidation_zone(&zone, 1_700_000_000_000, config.zone_range_bps);

        // low ≈ 99750, high ≈ 100250
        assert!(mz.contains_price(100_000.0));
        assert!(mz.contains_price(99_800.0));
        assert!(mz.contains_price(100_200.0));
        assert!(!mz.contains_price(99_000.0));
        assert!(!mz.contains_price(101_000.0));
    }

    #[test]
    fn test_zone_distance_bps() {
        let config = default_config();
        let zone = make_zone(100_000.0, "long", 0.5);
        let mz = MemoryZone::from_liquidation_zone(&zone, 1_700_000_000_000, config.zone_range_bps);

        // Inside zone → 0
        assert_eq!(mz.distance_bps(100_000.0), 0.0);

        // Outside zone
        let dist = mz.distance_bps(101_000.0);
        assert!(dist > 0.0);
    }

    #[test]
    fn test_zone_validate_ok() {
        let config = default_config();
        let zone = make_zone(100_000.0, "long", 0.5);
        let mz = MemoryZone::from_liquidation_zone(&zone, 1_700_000_000_000, config.zone_range_bps);
        assert!(mz.validate().is_ok());
    }

    #[test]
    fn test_zone_validate_rejects_low_ge_high() {
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(100_000.0, "long", 0.5),
            1_700_000_000_000,
            50.0,
        );
        mz.low = 100_000.0;
        mz.high = 99_000.0;
        assert!(mz.validate().is_err());
    }

    #[test]
    fn test_zone_validate_rejects_invalid_side() {
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(100_000.0, "long", 0.5),
            1_700_000_000_000,
            50.0,
        );
        mz.side_at_risk = "invalid".to_string();
        assert!(mz.validate().is_err());
    }

    #[test]
    fn test_zone_validate_rejects_out_of_range_confidence() {
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(100_000.0, "long", 0.5),
            1_700_000_000_000,
            50.0,
        );
        mz.confidence = 1.5;
        assert!(mz.validate().is_err());
    }

    #[test]
    fn test_config_validate_ok() {
        assert!(default_config().validate().is_ok());
    }

    #[test]
    fn test_config_validate_rejects_zero_range() {
        let mut c = default_config();
        c.zone_range_bps = 0.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_config_validate_rejects_zero_proximity() {
        let mut c = default_config();
        c.touch_proximity_bps = 0.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_config_validate_rejects_zero_decay() {
        let mut c = default_config();
        c.decay_half_life_secs = 0.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_merge_from_combines_sources() {
        let config = default_config();
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(99_000.0, "long", 0.5),
            1_700_000_000_000,
            config.zone_range_bps,
        );

        let other = LiquidationZone {
            price: 99_010.0,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 3,
            distance_bps: 400.0,
            confidence: 0.9,
            source_mix: vec!["hyperliquid_fills".to_string()],
        };

        mz.merge_from(&other);
        assert!((mz.confidence - 0.9).abs() < 0.001);
        assert!(mz.source_mix.contains(&"hyperliquid_positions".to_string()));
        assert!(mz.source_mix.contains(&"hyperliquid_fills".to_string()));
        assert!((mz.estimated_notional_usd - 1_500_000.0).abs() < 0.01);
    }

    #[test]
    fn test_zone_event_types() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snap);

        // Touch event
        let events = map.process_price(99_500.0, 1_700_000_001_000);
        assert_eq!(events[0].event_type, ZoneEventType::Touch);
        assert!((events[0].zone_low - map.zones[0].low).abs() < 0.01);

        // Sweep event
        let events = map.process_price(99_000.0, 1_700_000_002_000);
        assert_eq!(events[0].event_type, ZoneEventType::Sweep);
    }

    #[test]
    fn test_multiple_zones_independent_tracking() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![
            make_zone(99_000.0, "long", 0.7),
            make_zone(105_000.0, "short", 0.6),
        ]);
        map.update_from_snapshot(&snap);

        // Touch zone 1 only
        map.process_price(99_500.0, 1_700_000_001_000);
        assert_eq!(map.zones[0].touch_count, 1);
        assert_eq!(map.zones[1].touch_count, 0);

        // Sweep zone 2 only
        map.process_price(105_000.0, 1_700_000_002_000);
        assert_eq!(map.zones[0].sweep_count, 0);
        assert_eq!(map.zones[1].sweep_count, 1);
    }

    #[test]
    fn test_zone_map_output_serialization() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snap);

        let output = ZoneMapOutput::from_map(&map, 1_700_000_000_000);
        let json = output.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["symbol"], "BTC");
        assert!(parsed["zones"].is_array());
        assert_eq!(parsed["generated_at_ms"], 1_700_000_000_000i64);
    }

    #[test]
    fn test_load_nonexistent_file_returns_error() {
        let result = LiquidityMemoryMap::load(Path::new("/nonexistent/path.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_json_returns_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.json");
        std::fs::write(&path, "not valid json").unwrap();
        let result = LiquidityMemoryMap::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_different_sides_not_merged() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![
            make_zone(99_000.0, "long", 0.7),
            make_zone(99_000.0, "short", 0.6),
        ]);
        map.update_from_snapshot(&snap);
        // Two zones at same price but different sides → should not merge
        assert_eq!(map.zones.len(), 2);
        assert_eq!(map.zones[0].side_at_risk, "long");
        assert_eq!(map.zones[1].side_at_risk, "short");
    }

    #[test]
    fn test_decay_score_clamped_to_range() {
        let mut map = make_map("BTC");
        let snap = make_snapshot("BTC", 1_700_000_000_000, vec![make_zone(99_000.0, "long", 0.7)]);
        map.update_from_snapshot(&snap);

        // Very far future → decay should still be clamped to 1.0
        map.classify_zones(1_700_000_000_000 + 1_000_000_000_000);
        assert!(map.zones[0].decay_score <= 1.0);
        assert!(map.zones[0].decay_score >= 0.0);
    }

    #[test]
    fn test_zone_type_display() {
        assert_eq!(ZoneType::Magnet.to_string(), "magnet");
        assert_eq!(ZoneType::Reversal.to_string(), "reversal");
        assert_eq!(ZoneType::Inactive.to_string(), "inactive");
    }

    #[test]
    fn test_process_price_no_zones() {
        let mut map = make_map("BTC");
        let events = map.process_price(100_000.0, 1_700_000_001_000);
        assert!(events.is_empty());
    }

    #[test]
    fn test_memory_zone_validate_rejects_negative_decay() {
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(100_000.0, "long", 0.5),
            1_700_000_000_000,
            50.0,
        );
        mz.decay_score = -0.1;
        assert!(mz.validate().is_err());
    }

    #[test]
    fn test_memory_zone_validate_rejects_decay_above_1() {
        let mut mz = MemoryZone::from_liquidation_zone(
            &make_zone(100_000.0, "long", 0.5),
            1_700_000_000_000,
            50.0,
        );
        mz.decay_score = 1.5;
        assert!(mz.validate().is_err());
    }
}
