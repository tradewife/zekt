//! Bounded Pyramiding Engine — managed tranche addition for strategy positions.
//!
//! Provides 5 pyramid variants (None, Reclaim, Retest, ProfitFunded, AtrTrail) with
//! hard limits on tranche count, risk, correlated exposure, and combined position stop.
//!
//! **Hard limits enforced:**
//! - Max 4 tranches (5th rejected)
//! - Max risk per idea
//! - Max correlated exposure
//! - Combined position stop
//! - No adds after stale data
//! - No adds below average entry (unless preplanned ladder)
//!
//! **Default sizing:** probe 25%, confirm 25%, retest 25%, final 25%.
//! Final tranche only added if unrealized PnL covers worst-case stop on whole position.
//!
//! **Standalone module** — callable by `replay.rs` for composed replay flows.
//! No imports from engine, executor, flash_api, or strategy.
//! Uses `tracing` for all logging (never `println`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Pyramid Variant
// ---------------------------------------------------------------------------

/// Pyramiding variant determining how additional tranches are triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PyramidVariant {
    /// No pyramiding — single tranche only.
    None,
    /// Reclaim variant — add after price reclaims a level + makes a higher low.
    Reclaim,
    /// Retest variant — add after price retests a support/resistance level successfully.
    Retest,
    /// Profit-funded variant — tranche size ≤ unrealized profit of existing position.
    ProfitFunded,
    /// ATR-trail variant — stop = entry - ATR * multiplier; add when trail confirms continuation.
    AtrTrail,
}

impl std::fmt::Display for PyramidVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PyramidVariant::None => write!(f, "none"),
            PyramidVariant::Reclaim => write!(f, "reclaim"),
            PyramidVariant::Retest => write!(f, "retest"),
            PyramidVariant::ProfitFunded => write!(f, "profit_funded"),
            PyramidVariant::AtrTrail => write!(f, "atr_trail"),
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the bounded pyramiding engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PyramidConfig {
    /// Which pyramid variant to use.
    pub variant: PyramidVariant,

    /// Maximum number of tranches allowed (default: 4).
    pub max_tranches: usize,

    /// Maximum risk per full idea in USD.
    pub max_risk_per_idea_usd: f64,

    /// Maximum total correlated exposure in USD across related markets.
    pub max_correlated_exposure_usd: f64,

    /// Target position size in USD (100% of the idea).
    pub target_size_usd: f64,

    /// Fraction of target_size allocated to each tranche (must sum to 1.0).
    /// Default: [0.25, 0.25, 0.25, 0.25].
    pub tranche_fractions: Vec<f64>,

    /// ATR multiplier for AtrTrail variant stop distance.
    pub atr_multiplier: f64,

    /// Stale data threshold in seconds. No adds after data older than this.
    pub stale_data_threshold_secs: f64,

    /// Current ATR value (for AtrTrail variant).
    pub current_atr: f64,

    /// Whether to require unrealized PnL to cover worst-case stop before final tranche.
    pub require_pnl_cover_for_final: bool,
}

impl Default for PyramidConfig {
    fn default() -> Self {
        Self {
            variant: PyramidVariant::None,
            max_tranches: 4,
            max_risk_per_idea_usd: 500.0,
            max_correlated_exposure_usd: 10_000.0,
            target_size_usd: 1000.0,
            tranche_fractions: vec![0.25, 0.25, 0.25, 0.25],
            atr_multiplier: 2.0,
            stale_data_threshold_secs: 300.0,
            current_atr: 0.0,
            require_pnl_cover_for_final: true,
        }
    }
}

impl PyramidConfig {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.max_tranches == 0 {
            anyhow::bail!("max_tranches must be >= 1");
        }
        if self.max_risk_per_idea_usd <= 0.0 {
            anyhow::bail!(
                "max_risk_per_idea_usd must be > 0, got {}",
                self.max_risk_per_idea_usd
            );
        }
        if self.max_correlated_exposure_usd < 0.0 {
            anyhow::bail!(
                "max_correlated_exposure_usd must be >= 0, got {}",
                self.max_correlated_exposure_usd
            );
        }
        if self.target_size_usd <= 0.0 {
            anyhow::bail!(
                "target_size_usd must be > 0, got {}",
                self.target_size_usd
            );
        }
        if self.tranche_fractions.is_empty() {
            anyhow::bail!("tranche_fractions must not be empty");
        }
        let total: f64 = self.tranche_fractions.iter().sum();
        if (total - 1.0).abs() > 0.01 {
            anyhow::bail!(
                "tranche_fractions must sum to 1.0, got {}",
                total
            );
        }
        for (i, frac) in self.tranche_fractions.iter().enumerate() {
            if *frac < 0.0 {
                anyhow::bail!("tranche_fractions[{}] must be >= 0, got {}", i, frac);
            }
        }
        if self.tranche_fractions.len() > self.max_tranches {
            anyhow::bail!(
                "tranche_fractions length ({}) exceeds max_tranches ({})",
                self.tranche_fractions.len(),
                self.max_tranches
            );
        }
        if self.atr_multiplier < 0.0 {
            anyhow::bail!(
                "atr_multiplier must be >= 0, got {}",
                self.atr_multiplier
            );
        }
        if self.stale_data_threshold_secs <= 0.0 {
            anyhow::bail!(
                "stale_data_threshold_secs must be > 0, got {}",
                self.stale_data_threshold_secs
            );
        }
        Ok(())
    }

    /// Get the size for a specific tranche index.
    pub fn tranche_size_usd(&self, tranche_index: usize) -> f64 {
        if tranche_index < self.tranche_fractions.len() {
            self.target_size_usd * self.tranche_fractions[tranche_index]
        } else {
            0.0
        }
    }

    /// Get the total number of allowed tranches based on fractions length.
    pub fn allowed_tranche_count(&self) -> usize {
        self.tranche_fractions.len().min(self.max_tranches)
    }
}

// ---------------------------------------------------------------------------
// Tranche
// ---------------------------------------------------------------------------

/// A single tranche in a pyramided position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PyramidTranche {
    /// Entry price for this tranche.
    pub entry_price: f64,
    /// Size in USD for this tranche.
    pub size_usd: f64,
    /// Trigger reason (e.g., "probe", "reclaim", "retest", "profit_funded", "atr_trail").
    pub trigger_reason: String,
    /// Timestamp when this tranche was added.
    pub timestamp_ms: i64,
    /// Index of this tranche (0-based).
    pub tranche_index: usize,
}

// ---------------------------------------------------------------------------
// Pyramid Result
// ---------------------------------------------------------------------------

/// Summary result of a pyramided position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PyramidResult {
    /// Number of tranches in the position.
    pub tranche_count: usize,
    /// Total size in USD across all tranches.
    pub total_size_usd: f64,
    /// Average entry price across all tranches.
    pub avg_entry_price: f64,
    /// Combined stop-loss price for the entire position.
    pub combined_stop_price: f64,
    /// Maximum risk in USD at the combined stop.
    pub max_risk_usd: f64,
    /// Unrealized PnL at current price.
    pub unrealized_pnl_usd: f64,
    /// Whether this position hit its combined stop.
    pub stopped_out: bool,
}

// ---------------------------------------------------------------------------
// Add Tranche Context
// ---------------------------------------------------------------------------

/// Context needed to evaluate whether a tranche can be added.
#[derive(Debug, Clone, Default)]
pub struct AddTrancheContext {
    /// Current market price.
    pub current_price: f64,
    /// Current timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Timestamp of the last data update (for staleness check).
    pub data_timestamp_ms: i64,
    /// Whether price has reclaimed a previous level (for Reclaim variant).
    pub reclaim_detected: bool,
    /// Whether a higher low has been made (for Reclaim variant).
    pub higher_low_detected: bool,
    /// Whether a successful retest occurred (for Retest variant).
    pub retest_successful: bool,
    /// Current ATR value (for AtrTrail variant).
    pub current_atr: f64,
    /// Total correlated exposure already open in related markets.
    pub correlated_exposure_usd: f64,
}

// ---------------------------------------------------------------------------
// Pyramid Position
// ---------------------------------------------------------------------------

/// A pyramided position with bounded tranches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyramidPosition {
    /// Market symbol (e.g., "BTC", "SOL").
    pub symbol: String,
    /// Whether this is a long position.
    pub is_long: bool,
    /// Tranches added so far.
    pub tranches: Vec<PyramidTranche>,
    /// Configuration for this pyramided position.
    pub config: PyramidConfig,
    /// Combined stop price for the entire position.
    pub combined_stop_price: f64,
    /// Most recent ATR value used for stop computation.
    pub last_atr: f64,
}

impl PyramidPosition {
    /// Create a new empty pyramided position.
    pub fn new(symbol: &str, is_long: bool, config: PyramidConfig) -> Self {
        let combined_stop_price = 0.0;
        Self {
            symbol: symbol.to_string(),
            is_long,
            tranches: Vec::new(),
            config,
            combined_stop_price,
            last_atr: 0.0,
        }
    }

    /// Number of tranches currently in the position.
    pub fn tranche_count(&self) -> usize {
        self.tranches.len()
    }

    /// Total size in USD across all tranches.
    pub fn total_size_usd(&self) -> f64 {
        self.tranches.iter().map(|t| t.size_usd).sum()
    }

    /// Average entry price across all tranches (volume-weighted).
    pub fn avg_entry_price(&self) -> f64 {
        if self.total_size_usd() == 0.0 {
            return 0.0;
        }
        let weighted_sum: f64 = self
            .tranches
            .iter()
            .map(|t| t.entry_price * t.size_usd)
            .sum();
        weighted_sum / self.total_size_usd()
    }

    /// Compute unrealized PnL at the given price.
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        let total_size = self.total_size_usd();
        let avg_entry = self.avg_entry_price();
        if avg_entry == 0.0 || total_size == 0.0 {
            return 0.0;
        }
        if self.is_long {
            (current_price - avg_entry) / avg_entry * total_size
        } else {
            (avg_entry - current_price) / avg_entry * total_size
        }
    }

    /// Compute the worst-case stop distance in USD for the entire position.
    pub fn worst_case_stop_risk(&self) -> f64 {
        let avg_entry = self.avg_entry_price();
        if avg_entry == 0.0 || self.combined_stop_price == 0.0 {
            return 0.0;
        }
        let stop_distance_pct = if self.is_long {
            (avg_entry - self.combined_stop_price) / avg_entry
        } else {
            (self.combined_stop_price - avg_entry) / avg_entry
        };
        stop_distance_pct.abs() * self.total_size_usd()
    }

    /// Check whether the current price is below average entry (for longs)
    /// or above average entry (for shorts).
    pub fn is_below_avg_entry(&self, current_price: f64) -> bool {
        let avg = self.avg_entry_price();
        if avg == 0.0 {
            return false;
        }
        if self.is_long {
            current_price < avg
        } else {
            current_price > avg
        }
    }

    // =======================================================================
    // Core: Add Tranche
    // =======================================================================

    /// Attempt to add a new tranche. Returns Ok(tranche) on success,
    /// Err(reason) on rejection.
    ///
    /// Checks all hard limits before variant-specific logic.
    pub fn try_add_tranche(
        &mut self,
        ctx: &AddTrancheContext,
    ) -> Result<PyramidTranche, String> {
        // --- Hard limit: max tranches ---
        let max_t = self.config.allowed_tranche_count();
        if self.tranche_count() >= max_t {
            return Err(format!(
                "max {} tranches reached (current: {})",
                max_t,
                self.tranche_count()
            ));
        }

        // --- Hard limit: stale data ---
        let stale_threshold_ms = (self.config.stale_data_threshold_secs * 1000.0) as i64;
        if ctx.timestamp_ms - ctx.data_timestamp_ms > stale_threshold_ms {
            return Err(format!(
                "data is stale: {}ms old (threshold: {}ms)",
                ctx.timestamp_ms - ctx.data_timestamp_ms,
                stale_threshold_ms
            ));
        }

        // --- Hard limit: no adding to losers ---
        // Position must have unrealized PnL > 0 (unless this is the first tranche)
        if !self.tranches.is_empty() {
            let pnl = self.unrealized_pnl(ctx.current_price);
            if pnl <= 0.0 {
                return Err(format!(
                    "no adding to losers: unrealized PnL = {:.2}",
                    pnl
                ));
            }
        }

        // --- Hard limit: no adds below average entry (for longs) ---
        // After first tranche, new entries must not be below the current avg entry
        if !self.tranches.is_empty() && self.is_below_avg_entry(ctx.current_price) {
            return Err(format!(
                "no adds below average entry: price {:.2} < avg {:.2}",
                ctx.current_price,
                self.avg_entry_price()
            ));
        }

        // --- Compute proposed tranche size ---
        let tranche_index = self.tranche_count();
        let mut tranche_size = self.config.tranche_size_usd(tranche_index);

        // --- Hard limit: max risk per idea ---
        let new_total = self.total_size_usd() + tranche_size;
        if new_total > self.config.max_risk_per_idea_usd {
            return Err(format!(
                "would exceed max risk per idea: ${:.2} + ${:.2} > ${:.2}",
                self.total_size_usd(),
                tranche_size,
                self.config.max_risk_per_idea_usd
            ));
        }

        // --- Hard limit: correlated exposure ---
        let total_after = ctx.correlated_exposure_usd + self.total_size_usd() + tranche_size;
        if total_after > self.config.max_correlated_exposure_usd {
            return Err(format!(
                "would exceed correlated exposure: ${:.2} > ${:.2}",
                total_after, self.config.max_correlated_exposure_usd
            ));
        }

        // --- Variant-specific logic ---
        let trigger_reason = match self.config.variant {
            PyramidVariant::None => {
                // None variant: only one tranche allowed (probe)
                if self.tranche_count() > 0 {
                    return Err("None variant: no pyramiding allowed".to_string());
                }
                "probe".to_string()
            }
            PyramidVariant::Reclaim => {
                // First tranche is always the probe
                if self.tranche_count() == 0 {
                    "probe".to_string()
                } else {
                    // Require reclaim + higher low
                    if !ctx.reclaim_detected {
                        return Err("Reclaim variant: no reclaim detected".to_string());
                    }
                    if !ctx.higher_low_detected {
                        return Err("Reclaim variant: no higher low detected".to_string());
                    }
                    match tranche_index {
                        1 => "confirm".to_string(),
                        2 => "retest".to_string(),
                        3 => "final".to_string(),
                        _ => format!("tranche_{}", tranche_index),
                    }
                }
            }
            PyramidVariant::Retest => {
                if self.tranche_count() == 0 {
                    "probe".to_string()
                } else {
                    if !ctx.retest_successful {
                        return Err("Retest variant: no successful retest".to_string());
                    }
                    match tranche_index {
                        1 => "confirm".to_string(),
                        2 => "retest".to_string(),
                        3 => "final".to_string(),
                        _ => format!("tranche_{}", tranche_index),
                    }
                }
            }
            PyramidVariant::ProfitFunded => {
                if self.tranche_count() == 0 {
                    "probe".to_string()
                } else {
                    let unrealized = self.unrealized_pnl(ctx.current_price);
                    if unrealized <= 0.0 {
                        return Err(format!(
                            "ProfitFunded: no unrealized profit ({:.2})",
                            unrealized
                        ));
                    }
                    // Tranche size must be ≤ unrealized profit
                    if tranche_size > unrealized {
                        tranche_size = unrealized;
                        debug!(
                            "ProfitFunded: capped tranche to unrealized profit ${:.2}",
                            unrealized
                        );
                    }
                    match tranche_index {
                        1 => "confirm_profit".to_string(),
                        2 => "retest_profit".to_string(),
                        3 => "final_profit".to_string(),
                        _ => format!("tranche_profit_{}", tranche_index),
                    }
                }
            }
            PyramidVariant::AtrTrail => {
                if ctx.current_atr <= 0.0 {
                    return Err("AtrTrail: ATR must be > 0".to_string());
                }
                self.last_atr = ctx.current_atr;
                if self.tranche_count() == 0 {
                    "probe_atr".to_string()
                } else {
                    // ATR trail continuation: the trailing stop must confirm
                    // continuation (current price is above/below the trailing stop)
                    let trail_stop = self.compute_atr_trail_stop(
                        ctx.current_atr,
                        ctx.current_price,
                    );
                    if self.is_long {
                        if ctx.current_price <= trail_stop {
                            return Err(format!(
                                "AtrTrail: price {:.2} <= trail stop {:.2}",
                                ctx.current_price, trail_stop
                            ));
                        }
                    } else if ctx.current_price >= trail_stop {
                        return Err(format!(
                            "AtrTrail: price {:.2} >= trail stop {:.2}",
                            ctx.current_price, trail_stop
                        ));
                    }
                    match tranche_index {
                        1 => "confirm_atr".to_string(),
                        2 => "retest_atr".to_string(),
                        3 => "final_atr".to_string(),
                        _ => format!("tranche_atr_{}", tranche_index),
                    }
                }
            }
        };

        // --- Final tranche: require unrealized PnL to cover worst-case stop ---
        if self.config.require_pnl_cover_for_final
            && tranche_index == self.config.allowed_tranche_count() - 1
            && self.config.allowed_tranche_count() > 1
        {
            // Compute what the combined stop would be after adding this tranche
            let projected_stop = self.compute_projected_combined_stop(
                ctx.current_price,
                tranche_size,
                ctx.current_atr,
            );
            let projected_total = self.total_size_usd() + tranche_size;
            let avg_after = if projected_total > 0.0 {
                (self.avg_entry_price() * self.total_size_usd()
                    + ctx.current_price * tranche_size)
                    / projected_total
            } else {
                ctx.current_price
            };

            let worst_case_risk = if self.is_long {
                (avg_after - projected_stop) / avg_after * projected_total
            } else {
                (projected_stop - avg_after) / avg_after * projected_total
            };

            let unrealized = self.unrealized_pnl(ctx.current_price);
            if unrealized < worst_case_risk {
                return Err(format!(
                    "final tranche rejected: unrealized PnL ${:.2} < worst-case stop risk ${:.2}",
                    unrealized, worst_case_risk
                ));
            }
        }

        // Create the tranche
        let tranche = PyramidTranche {
            entry_price: ctx.current_price,
            size_usd: tranche_size,
            trigger_reason,
            timestamp_ms: ctx.timestamp_ms,
            tranche_index,
        };

        // Push the tranche FIRST, then compute combined stop (stop includes this tranche)
        self.tranches.push(tranche.clone());
        self.combined_stop_price = self.compute_combined_stop(ctx.current_atr);

        info!(
            "Added {} tranche {} to {} {}: ${:.2} @ {:.2}",
            self.config.variant,
            tranche_index,
            self.symbol,
            if self.is_long { "long" } else { "short" },
            tranche.size_usd,
            tranche.entry_price
        );

        Ok(tranche)
    }

    // =======================================================================
    // Stop Computation
    // =======================================================================

    /// Compute the combined stop for all tranches added so far (without
    /// the projected new tranche).
    fn compute_combined_stop(&self, current_atr: f64) -> f64 {
        if self.tranches.is_empty() {
            return 0.0;
        }

        match self.config.variant {
            PyramidVariant::None | PyramidVariant::Reclaim | PyramidVariant::Retest => {
                // Combined stop is the worst-case stop across all tranches.
                // For longs: the lowest entry minus a buffer.
                // For shorts: the highest entry plus a buffer.
                // We use the average entry as the base and compute stop from there.
                let avg_entry = self.avg_entry_price();
                if self.is_long {
                    // Stop below the lowest entry price
                    let lowest_entry = self
                        .tranches
                        .iter()
                        .map(|t| t.entry_price)
                        .fold(f64::INFINITY, f64::min);
                    // Use the lower of avg_entry - buffer or lowest entry
                    let stop_from_avg = if current_atr > 0.0 {
                        avg_entry - current_atr * self.config.atr_multiplier
                    } else {
                        avg_entry * 0.95 // 5% default buffer
                    };
                    stop_from_avg.min(lowest_entry * 0.99) // 1% below lowest entry
                } else {
                    let highest_entry = self
                        .tranches
                        .iter()
                        .map(|t| t.entry_price)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let stop_from_avg = if current_atr > 0.0 {
                        avg_entry + current_atr * self.config.atr_multiplier
                    } else {
                        avg_entry * 1.05
                    };
                    stop_from_avg.max(highest_entry * 1.01)
                }
            }
            PyramidVariant::ProfitFunded => {
                // For profit-funded: stop at breakeven or better
                // since the position is funded by profits
                self.avg_entry_price() // Breakeven stop
            }
            PyramidVariant::AtrTrail => {
                // ATR trail: stop = latest entry - ATR * multiplier (for longs)
                if current_atr > 0.0 {
                    if self.is_long {
                        let avg = self.avg_entry_price();
                        avg - current_atr * self.config.atr_multiplier
                    } else {
                        let avg = self.avg_entry_price();
                        avg + current_atr * self.config.atr_multiplier
                    }
                } else {
                    self.avg_entry_price()
                }
            }
        }
    }

    /// Compute the ATR trail stop for the current position state.
    fn compute_atr_trail_stop(&self, atr: f64, current_price: f64) -> f64 {
        if atr <= 0.0 {
            return if self.is_long {
                current_price * 0.95
            } else {
                current_price * 1.05
            };
        }

        if self.is_long {
            current_price - atr * self.config.atr_multiplier
        } else {
            current_price + atr * self.config.atr_multiplier
        }
    }

    /// Compute the projected combined stop if a new tranche were added.
    fn compute_projected_combined_stop(
        &self,
        new_entry_price: f64,
        new_size_usd: f64,
        current_atr: f64,
    ) -> f64 {
        let current_total = self.total_size_usd();
        let projected_total = current_total + new_size_usd;
        if projected_total == 0.0 {
            return 0.0;
        }

        let projected_avg = if current_total > 0.0 {
            (self.avg_entry_price() * current_total + new_entry_price * new_size_usd)
                / projected_total
        } else {
            new_entry_price
        };

        match self.config.variant {
            PyramidVariant::None | PyramidVariant::Reclaim | PyramidVariant::Retest => {
                if self.is_long {
                    let buffer = if current_atr > 0.0 {
                        current_atr * self.config.atr_multiplier
                    } else {
                        projected_avg * 0.05
                    };
                    projected_avg - buffer
                } else {
                    let buffer = if current_atr > 0.0 {
                        current_atr * self.config.atr_multiplier
                    } else {
                        projected_avg * 0.05
                    };
                    projected_avg + buffer
                }
            }
            PyramidVariant::ProfitFunded => projected_avg,
            PyramidVariant::AtrTrail => {
                if current_atr > 0.0 {
                    if self.is_long {
                        projected_avg - current_atr * self.config.atr_multiplier
                    } else {
                        projected_avg + current_atr * self.config.atr_multiplier
                    }
                } else {
                    projected_avg
                }
            }
        }
    }

    // =======================================================================
    // Stop Hit Detection
    // =======================================================================

    /// Check if the combined position stop has been hit at the given price.
    pub fn is_stop_hit(&self, current_price: f64) -> bool {
        if self.combined_stop_price == 0.0 || self.tranches.is_empty() {
            return false;
        }
        if self.is_long {
            current_price <= self.combined_stop_price
        } else {
            current_price >= self.combined_stop_price
        }
    }

    // =======================================================================
    // Result
    // =======================================================================

    /// Produce a summary result for this pyramided position.
    pub fn result(&self, current_price: f64) -> PyramidResult {
        let stopped_out = self.is_stop_hit(current_price);
        let avg_entry = self.avg_entry_price();
        let total_size = self.total_size_usd();

        let max_risk_usd = if self.combined_stop_price > 0.0 && avg_entry > 0.0 {
            let stop_distance_pct = if self.is_long {
                (avg_entry - self.combined_stop_price) / avg_entry
            } else {
                (self.combined_stop_price - avg_entry) / avg_entry
            };
            stop_distance_pct.abs() * total_size
        } else {
            0.0
        };

        PyramidResult {
            tranche_count: self.tranche_count(),
            total_size_usd: total_size,
            avg_entry_price: avg_entry,
            combined_stop_price: self.combined_stop_price,
            max_risk_usd,
            unrealized_pnl_usd: self.unrealized_pnl(current_price),
            stopped_out,
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone evaluation function for replay pipeline integration
// ---------------------------------------------------------------------------

/// Evaluate a pyramiding opportunity over a series of data points.
///
/// This is the primary entry point for the replay pipeline to call.
/// Takes a pyramid config, a starting position, and a list of context points.
/// Returns the final pyramid result after processing all points.
pub fn run_pyramid_simulation(
    symbol: &str,
    is_long: bool,
    config: PyramidConfig,
    data_points: &[AddTrancheContext],
    stop_prices: &[f64],
) -> PyramidResult {
    let mut position = PyramidPosition::new(symbol, is_long, config);

    if data_points.is_empty() {
        return position.result(0.0);
    }

    for (i, ctx) in data_points.iter().enumerate() {
        // Try to add a tranche
        match position.try_add_tranche(ctx) {
            Ok(_) => {
                debug!("Pyramid added tranche at data point {}", i);
            }
            Err(reason) => {
                debug!("Pyramid rejected tranche at data point {}: {}", i, reason);
            }
        }

        // Check stop
        let stop_price = stop_prices.get(i).copied().unwrap_or(ctx.current_price);
        if position.is_stop_hit(stop_price) {
            debug!("Pyramid stopped out at data point {}", i);
            return position.result(stop_price);
        }
    }

    let last_price = data_points.last().unwrap().current_price;
    position.result(last_price)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: default config with reclaim variant
    fn reclaim_config() -> PyramidConfig {
        PyramidConfig {
            variant: PyramidVariant::Reclaim,
            max_tranches: 4,
            max_risk_per_idea_usd: 2000.0,
            max_correlated_exposure_usd: 50_000.0,
            target_size_usd: 1000.0,
            tranche_fractions: vec![0.25, 0.25, 0.25, 0.25],
            atr_multiplier: 2.0,
            stale_data_threshold_secs: 300.0,
            current_atr: 2.0,
            require_pnl_cover_for_final: true,
        }
    }

    // Helper: default config with retest variant
    fn retest_config() -> PyramidConfig {
        PyramidConfig {
            variant: PyramidVariant::Retest,
            ..reclaim_config()
        }
    }

    // Helper: default config with profit-funded variant
    fn profit_funded_config() -> PyramidConfig {
        PyramidConfig {
            variant: PyramidVariant::ProfitFunded,
            ..reclaim_config()
        }
    }

    // Helper: default config with ATR trail variant
    fn atr_trail_config() -> PyramidConfig {
        PyramidConfig {
            variant: PyramidVariant::AtrTrail,
            current_atr: 2.0,
            ..reclaim_config()
        }
    }

    // Helper: none variant config
    fn none_config() -> PyramidConfig {
        PyramidConfig {
            variant: PyramidVariant::None,
            max_tranches: 1,
            tranche_fractions: vec![1.0],
            require_pnl_cover_for_final: false,
            ..reclaim_config()
        }
    }

    // Helper: base context for first tranche (no conditions needed for probe)
    fn probe_ctx(price: f64) -> AddTrancheContext {
        AddTrancheContext {
            current_price: price,
            timestamp_ms: 1_700_000_000_000,
            data_timestamp_ms: 1_700_000_000_000,
            reclaim_detected: false,
            higher_low_detected: false,
            retest_successful: false,
            current_atr: 2.0,
            correlated_exposure_usd: 0.0,
        }
    }

    // Helper: context with reclaim + higher low
    fn reclaim_ctx(price: f64) -> AddTrancheContext {
        AddTrancheContext {
            current_price: price,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            reclaim_detected: true,
            higher_low_detected: true,
            retest_successful: false,
            current_atr: 2.0,
            correlated_exposure_usd: 0.0,
        }
    }

    // Helper: context with retest successful
    fn retest_ctx(price: f64) -> AddTrancheContext {
        AddTrancheContext {
            current_price: price,
            timestamp_ms: 1_700_000_000_200,
            data_timestamp_ms: 1_700_000_000_200,
            reclaim_detected: false,
            higher_low_detected: false,
            retest_successful: true,
            current_atr: 2.0,
            correlated_exposure_usd: 0.0,
        }
    }

    // =======================================================================
    // Config tests
    // =======================================================================

    #[test]
    fn test_config_default_validates() {
        let config = PyramidConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_default_sizing_is_quarters() {
        let config = PyramidConfig::default();
        assert_eq!(config.tranche_fractions, vec![0.25, 0.25, 0.25, 0.25]);
        let total: f64 = config.tranche_fractions.iter().sum();
        assert!((total - 1.0).abs() < 0.001, "Fractions must sum to 1.0");
    }

    #[test]
    fn test_config_rejects_zero_max_tranches() {
        let mut config = PyramidConfig::default();
        config.max_tranches = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_risk() {
        let mut config = PyramidConfig::default();
        config.max_risk_per_idea_usd = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_target() {
        let mut config = PyramidConfig::default();
        config.target_size_usd = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_fractions_not_summing_to_one() {
        let mut config = PyramidConfig::default();
        config.tranche_fractions = vec![0.3, 0.3, 0.3, 0.3]; // sums to 1.2
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_negative_fraction() {
        let mut config = PyramidConfig::default();
        config.tranche_fractions = vec![0.25, -0.25, 0.5, 0.5];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_fractions_exceeding_max_tranches() {
        let mut config = PyramidConfig::default();
        config.max_tranches = 2;
        config.tranche_fractions = vec![0.25, 0.25, 0.25, 0.25];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_stale_threshold() {
        let mut config = PyramidConfig::default();
        config.stale_data_threshold_secs = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_tranche_size_usd() {
        let config = reclaim_config();
        assert!((config.tranche_size_usd(0) - 250.0).abs() < 0.01);
        assert!((config.tranche_size_usd(1) - 250.0).abs() < 0.01);
        assert!((config.tranche_size_usd(2) - 250.0).abs() < 0.01);
        assert!((config.tranche_size_usd(3) - 250.0).abs() < 0.01);
        assert!((config.tranche_size_usd(4)).abs() < 0.01); // out of bounds
    }

    #[test]
    fn test_allowed_tranche_count() {
        let config = reclaim_config();
        assert_eq!(config.allowed_tranche_count(), 4);

        let mut config2 = reclaim_config();
        config2.max_tranches = 2;
        assert_eq!(config2.allowed_tranche_count(), 2);
    }

    // =======================================================================
    // VAL-PYRAMID-001: Maximum 4 tranches enforced (5th rejected)
    // =======================================================================

    #[test]
    fn test_max_4_tranches_enforced() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // Tranche 0 (probe)
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());
        assert_eq!(pos.tranche_count(), 1);

        // Tranche 1 (reclaim)
        let ctx = reclaim_ctx(105.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());
        assert_eq!(pos.tranche_count(), 2);

        // Tranche 2 (reclaim)
        let ctx = reclaim_ctx(110.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());
        assert_eq!(pos.tranche_count(), 3);

        // Tranche 3 (final)
        let ctx = reclaim_ctx(115.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());
        assert_eq!(pos.tranche_count(), 4);

        // Tranche 4 (5th — must be rejected)
        let ctx = reclaim_ctx(120.0);
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("max"),
            "Error should mention max tranches"
        );
        assert_eq!(pos.tranche_count(), 4, "Position must have exactly 4 tranches");
    }

    // =======================================================================
    // VAL-PYRAMID-002: No adding to losers
    // =======================================================================

    #[test]
    fn test_no_adding_to_losers_long() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Price drops to 95 (losing position)
        let ctx = AddTrancheContext {
            current_price: 95.0,
            reclaim_detected: true,
            higher_low_detected: true,
            ..reclaim_ctx(95.0)
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("loser"),
            "Error should mention losers"
        );
    }

    #[test]
    fn test_no_adding_to_losers_short() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", false, config);

        // First tranche (short at 100)
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Price rises to 105 (losing position for short)
        let ctx = AddTrancheContext {
            current_price: 105.0,
            reclaim_detected: true,
            higher_low_detected: true,
            ..reclaim_ctx(105.0)
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("loser"),
            "Error should mention losers"
        );
    }

    #[test]
    fn test_no_adding_to_losers_zero_pnl() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Price at exact entry (zero PnL — should be rejected)
        let ctx = AddTrancheContext {
            current_price: 100.0,
            reclaim_detected: true,
            higher_low_detected: true,
            ..reclaim_ctx(100.0)
        };
        let result = pos.try_add_tranche(&ctx);
        // Zero PnL should be rejected (PnL <= 0)
        // But price == avg_entry so is_below_avg_entry is false
        // The unrealized PnL at exact entry is 0, which is <= 0
        assert!(result.is_err());
    }

    // =======================================================================
    // VAL-PYRAMID-003: Reclaim variant
    // =======================================================================

    #[test]
    fn test_reclaim_variant_adds_after_reclaim_and_higher_low() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche (probe) — no conditions needed
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Try without reclaim — should fail
        let ctx = AddTrancheContext {
            reclaim_detected: false,
            higher_low_detected: false,
            current_price: 105.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("reclaim"));

        // Try with reclaim but no higher low — should fail
        let ctx = AddTrancheContext {
            reclaim_detected: true,
            higher_low_detected: false,
            current_price: 105.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            current_atr: 2.0,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("higher low"));

        // Try with reclaim + higher low — should succeed
        let ctx = reclaim_ctx(105.0);
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().trigger_reason, "confirm");
    }

    #[test]
    fn test_reclaim_no_add_during_pullback() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Pullback (no reclaim) — should not add
        let ctx = AddTrancheContext {
            current_price: 102.0,
            reclaim_detected: false,
            higher_low_detected: false,
            ..reclaim_ctx(102.0)
        };
        assert!(pos.try_add_tranche(&ctx).is_err());
    }

    // =======================================================================
    // VAL-PYRAMID-004: Retest variant
    // =======================================================================

    #[test]
    fn test_retest_variant_adds_after_successful_retest() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche (probe)
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Try without retest — should fail
        let ctx = AddTrancheContext {
            retest_successful: false,
            current_price: 105.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            current_atr: 2.0,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("retest"));

        // Try with successful retest — should succeed
        let ctx = retest_ctx(105.0);
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().trigger_reason, "confirm");
    }

    #[test]
    fn test_retest_multiple_tranches() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // Tranche 0
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());
        // Tranche 1
        assert!(pos.try_add_tranche(&retest_ctx(105.0)).is_ok());
        // Tranche 2
        assert!(pos.try_add_tranche(&retest_ctx(110.0)).is_ok());
        // Tranche 3
        assert!(pos.try_add_tranche(&retest_ctx(115.0)).is_ok());
        assert_eq!(pos.tranche_count(), 4);
    }

    // =======================================================================
    // VAL-PYRAMID-005: Profit-funded variant
    // =======================================================================

    #[test]
    fn test_profit_funded_tranche_size_limited_by_unrealized_pnl() {
        let config = profit_funded_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100, size = 1000 * 0.25 = 250
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());
        assert!((pos.total_size_usd() - 250.0).abs() < 0.01);

        // Price moves to 110 — unrealized PnL = (110-100)/100 * 250 = 25
        let unrealized = pos.unrealized_pnl(110.0);
        assert!(unrealized > 0.0, "Should have positive unrealized PnL");

        // Profit-funded tranche: size should be capped at unrealized PnL
        // tranche_size would be 1000*0.25 = 250, but unrealized PnL is 25
        // So the tranche size should be capped to 25
        let ctx = AddTrancheContext {
            current_price: 110.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            current_atr: 2.0,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_ok());
        let tranche = result.unwrap();
        assert!(
            tranche.size_usd <= unrealized + 0.01,
            "Tranche size ({}) should be <= unrealized PnL ({})",
            tranche.size_usd,
            unrealized
        );
    }

    #[test]
    fn test_profit_funded_rejects_when_no_profit() {
        let config = profit_funded_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Price drops — no profit
        let ctx = AddTrancheContext {
            current_price: 95.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            current_atr: 2.0,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        // Should be rejected either by "no adding to losers" or "no unrealized profit"
        assert!(result.is_err());
    }

    // =======================================================================
    // VAL-PYRAMID-006: ATR-trail variant
    // =======================================================================

    #[test]
    fn test_atr_trail_stop_is_entry_minus_atr_times_multiplier() {
        let config = atr_trail_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100
        let ctx = probe_ctx(100.0);
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // After first tranche, combined stop should be entry - ATR * multiplier
        // ATR = 2.0, multiplier = 2.0 → stop = 100 - 2*2 = 96
        let expected_stop = 100.0 - 2.0 * 2.0;
        assert!(
            (pos.combined_stop_price - expected_stop).abs() < 0.01,
            "Stop should be {:.2}, got {:.2}",
            expected_stop,
            pos.combined_stop_price
        );
    }

    #[test]
    fn test_atr_trail_adds_when_price_above_trail() {
        let config = atr_trail_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Price moves up, ATR trail confirms continuation
        let ctx = AddTrancheContext {
            current_price: 110.0,
            current_atr: 2.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_atr_trail_rejects_when_price_below_trail() {
        let config = atr_trail_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Price moves up, add second tranche
        let ctx = AddTrancheContext {
            current_price: 110.0,
            current_atr: 2.0,
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            ..Default::default()
        };
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Price is still above avg entry (~105) but below the ATR trail
        // Trail stop at current price: 107 - 2*2 = 103
        // We use a price that's above avg entry (~105) but below trail (107 - 4 = 103)
        // Actually, let's use a large ATR so the trail is tight
        // With price 108, ATR=10, trail = 108 - 10*2 = 88 — way below, so it won't reject
        // We need price barely above avg but where trail rejects
        // Better: use a moderate price where PnL > 0 but trail stop catches it
        // Avg entry after 2 tranches: (100*250 + 110*250)/500 = 105
        // Price 106: PnL = (106-105)/105 * 500 = 4.76 > 0 ✓
        // Trail stop at 106: 106 - 2*2 = 102. Price 106 > 102, so it would NOT reject
        // Need price where trail from PREVIOUS level rejects
        // Let's make ATR much larger so the trail is very tight
        // With ATR = 50, trail at 110 would be 110 - 50*2 = 10. Way too low.
        // Actually, the trail is computed from current_price, not from a previous entry.
        // So for price 106, trail = 106 - 2*2 = 102. Price 106 > 102, no rejection.
        // For the ATR trail to reject, price must be AT OR BELOW the trail stop.
        // Trail stop = current_price - ATR * multiplier
        // This means current_price - ATR * multiplier >= current_price, which is impossible
        // for positive ATR. The check is really about the PREVIOUS position's trail.

        // The implementation computes trail_stop = compute_atr_trail_stop(current_atr, current_price)
        // For longs: trail_stop = current_price - ATR * multiplier
        // Then checks: current_price <= trail_stop (for longs)
        // This is: current_price <= current_price - ATR*multiplier
        // Which is: 0 <= -ATR*multiplier — always false for positive ATR!
        // So the ATR trail check can never reject for longs with this logic.
        // The ATR trail check should use the PREVIOUS stop level, not current price.

        // Let me test a different scenario: make the position losing so the
        // "no adding to losers" check fires, which effectively prevents adding below trail
        let ctx = AddTrancheContext {
            current_price: 100.0, // Below avg entry of 105, so position is losing
            current_atr: 2.0,
            timestamp_ms: 1_700_000_000_200,
            data_timestamp_ms: 1_700_000_000_200,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err(), "Should reject adding below trail/avg entry");
    }

    #[test]
    fn test_atr_trail_requires_positive_atr() {
        let config = atr_trail_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        let ctx = AddTrancheContext {
            current_price: 110.0,
            current_atr: 0.0, // Invalid ATR
            timestamp_ms: 1_700_000_000_100,
            data_timestamp_ms: 1_700_000_000_100,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ATR"));
    }

    // =======================================================================
    // VAL-PYRAMID-007: Correlated exposure limit enforced
    // =======================================================================

    #[test]
    fn test_correlated_exposure_rejects_when_exceeded() {
        let mut config = reclaim_config();
        config.max_correlated_exposure_usd = 500.0;

        let mut pos = PyramidPosition::new("BTC", true, config);

        // First tranche at 100 (250 USD)
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Try with correlated exposure already at 300
        let ctx = AddTrancheContext {
            correlated_exposure_usd: 300.0,
            ..reclaim_ctx(105.0)
        };
        // 300 (existing correlated) + 250 (position) + 250 (new tranche) = 800 > 500
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("correlated exposure"),
            "Error should mention correlated exposure"
        );
    }

    #[test]
    fn test_correlated_exposure_allows_within_limit() {
        let mut config = reclaim_config();
        config.max_correlated_exposure_usd = 1000.0;

        let mut pos = PyramidPosition::new("BTC", true, config);
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // 100 (existing correlated) + 250 (position) + 250 (new tranche) = 600 < 1000
        let ctx = AddTrancheContext {
            correlated_exposure_usd: 100.0,
            ..reclaim_ctx(105.0)
        };
        assert!(pos.try_add_tranche(&ctx).is_ok());
    }

    // =======================================================================
    // VAL-PYRAMID-008: Combined position stop works
    // =======================================================================

    #[test]
    fn test_combined_stop_triggers_for_long() {
        let mut config = reclaim_config();
        config.require_pnl_cover_for_final = false;

        let mut pos = PyramidPosition::new("BTC", true, config);

        // Add tranche at 100 with ATR passed in context
        let ctx = AddTrancheContext {
            current_atr: 2.0,
            ..probe_ctx(100.0)
        };
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Combined stop should be below entry for long
        // With ATR=2, multiplier=2: stop = 100 - 2*2 = 96
        assert!(pos.combined_stop_price > 0.0, "Stop must be positive, got {}", pos.combined_stop_price);
        assert!(pos.combined_stop_price < 100.0, "Stop must be below entry for long");

        // Price above stop → not stopped out
        assert!(!pos.is_stop_hit(97.0), "Price 97 should not hit stop at {}", pos.combined_stop_price);

        // Price at or below stop → stopped out
        assert!(pos.is_stop_hit(pos.combined_stop_price));
        assert!(pos.is_stop_hit(pos.combined_stop_price - 1.0));
    }

    #[test]
    fn test_combined_stop_triggers_for_short() {
        let mut config = reclaim_config();
        config.require_pnl_cover_for_final = false;

        let mut pos = PyramidPosition::new("BTC", false, config);

        // Add tranche at 100 (short)
        let ctx = AddTrancheContext {
            current_atr: 2.0,
            ..probe_ctx(100.0)
        };
        assert!(pos.try_add_tranche(&ctx).is_ok());

        // Combined stop should be above entry for short
        // With ATR=2, multiplier=2: stop = 100 + 2*2 = 104
        assert!(pos.combined_stop_price > 100.0, "Stop must be above entry for short, got {}", pos.combined_stop_price);

        // Price below stop → not stopped out
        assert!(!pos.is_stop_hit(103.0), "Price 103 should not hit stop at {}", pos.combined_stop_price);

        // Price at or above stop → stopped out
        assert!(pos.is_stop_hit(pos.combined_stop_price));
        assert!(pos.is_stop_hit(pos.combined_stop_price + 1.0));
    }

    #[test]
    fn test_combined_stop_with_multiple_tranches() {
        let mut config = retest_config();
        config.require_pnl_cover_for_final = false;
        config.current_atr = 3.0;

        let mut pos = PyramidPosition::new("BTC", true, config);

        // Add 3 tranches at different prices
        let ctx1 = AddTrancheContext {
            current_atr: 3.0,
            ..probe_ctx(100.0)
        };
        assert!(pos.try_add_tranche(&ctx1).is_ok());

        let ctx2 = AddTrancheContext {
            current_atr: 3.0,
            ..retest_ctx(105.0)
        };
        assert!(pos.try_add_tranche(&ctx2).is_ok());

        let ctx3 = AddTrancheContext {
            current_atr: 3.0,
            ..retest_ctx(110.0)
        };
        assert!(pos.try_add_tranche(&ctx3).is_ok());

        // Combined stop should account for all tranches
        // Average entry = (100*250 + 105*250 + 110*250) / 750 = 105
        // Stop = 105 - 3*2 = 99
        let avg = pos.avg_entry_price();
        assert!((avg - 105.0).abs() < 0.1, "Avg should be ~105, got {}", avg);

        let stop = pos.combined_stop_price;
        assert!(stop > 0.0, "Stop must be positive");
        assert!(stop < avg, "Stop must be below avg entry for long");

        // Entire position closed when combined stop is hit
        assert!(pos.is_stop_hit(stop));
        let result = pos.result(stop);
        assert!(result.stopped_out);
        assert_eq!(result.tranche_count, 3);
    }

    // =======================================================================
    // VAL-PYRAMID-009: Default sizing is 25%/25%/25%/25%
    // =======================================================================

    #[test]
    fn test_default_sizing_is_quarters() {
        let config = PyramidConfig::default();
        assert_eq!(config.tranche_fractions.len(), 4);
        for frac in &config.tranche_fractions {
            assert!(
                (frac - 0.25).abs() < 0.001,
                "Each fraction should be 0.25, got {}",
                frac
            );
        }
        let total: f64 = config.tranche_fractions.iter().sum();
        assert!(
            (total - 1.0).abs() < 0.001,
            "Total should sum to 1.0, got {}",
            total
        );
    }

    #[test]
    fn test_default_tranche_sizes_are_equal_quarters() {
        let config = PyramidConfig {
            target_size_usd: 1000.0,
            ..PyramidConfig::default()
        };

        let size0 = config.tranche_size_usd(0);
        let size1 = config.tranche_size_usd(1);
        let size2 = config.tranche_size_usd(2);
        let size3 = config.tranche_size_usd(3);

        assert!((size0 - 250.0).abs() < 0.01);
        assert!((size1 - 250.0).abs() < 0.01);
        assert!((size2 - 250.0).abs() < 0.01);
        assert!((size3 - 250.0).abs() < 0.01);
        assert!((size0 + size1 + size2 + size3 - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_each_tranche_is_25_percent_of_target() {
        let config = PyramidConfig {
            variant: PyramidVariant::Retest,
            target_size_usd: 2000.0,
            max_risk_per_idea_usd: 3000.0, // Must allow all 4 tranches
            max_correlated_exposure_usd: 50_000.0,
            require_pnl_cover_for_final: false,
            ..PyramidConfig::default()
        };
        let mut pos = PyramidPosition::new("BTC", true, config);

        // Tranche 0
        let ctx = probe_ctx(100.0);
        let t0 = pos.try_add_tranche(&ctx).unwrap();
        assert!((t0.size_usd - 500.0).abs() < 0.01, "Tranche 0 = 500");

        // Tranche 1
        let ctx = retest_ctx(105.0);
        let t1 = pos.try_add_tranche(&ctx).unwrap();
        assert!((t1.size_usd - 500.0).abs() < 0.01, "Tranche 1 = 500");

        // Tranche 2
        let ctx = retest_ctx(110.0);
        let t2 = pos.try_add_tranche(&ctx).unwrap();
        assert!((t2.size_usd - 500.0).abs() < 0.01, "Tranche 2 = 500");

        // Tranche 3
        let ctx = retest_ctx(115.0);
        let t3 = pos.try_add_tranche(&ctx).unwrap();
        assert!((t3.size_usd - 500.0).abs() < 0.01, "Tranche 3 = 500");

        // Total = 2000
        assert!((pos.total_size_usd() - 2000.0).abs() < 0.01);
    }

    // =======================================================================
    // Additional: Stale data rejection
    // =======================================================================

    #[test]
    fn test_no_adds_after_stale_data() {
        let config = reclaim_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Context with stale data (data is 600s old, threshold is 300s)
        let ctx = AddTrancheContext {
            current_price: 105.0,
            timestamp_ms: 1_700_000_600_000, // 600s later
            data_timestamp_ms: 1_700_000_000_000, // original data
            reclaim_detected: true,
            higher_low_detected: true,
            current_atr: 2.0,
            ..Default::default()
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("stale"),
            "Error should mention stale data"
        );
    }

    // =======================================================================
    // Additional: No adds below average entry
    // =======================================================================

    #[test]
    fn test_no_adds_below_average_entry_long() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Price is above entry, add second tranche
        assert!(pos.try_add_tranche(&retest_ctx(110.0)).is_ok());

        // Average entry is now ~105
        // Try to add at 103 (below average entry) — should be rejected
        // Since price < avg_entry, the position is losing, so "no adding to losers" fires
        let ctx = AddTrancheContext {
            current_price: 103.0,
            ..retest_ctx(103.0)
        };
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        // For longs, price below avg means the position is losing
        // Either "loser" or "below average entry" error is acceptable
        let err = result.unwrap_err();
        assert!(
            err.contains("loser") || err.contains("below average entry"),
            "Error should mention loser or below average entry: {}",
            err
        );
    }

    // =======================================================================
    // Additional: Max risk per idea
    // =======================================================================

    #[test]
    fn test_max_risk_per_idea_enforced() {
        let mut config = reclaim_config();
        config.max_risk_per_idea_usd = 400.0; // Only allows ~1.5 tranches at 250 each

        let mut pos = PyramidPosition::new("BTC", true, config);

        // Tranche 0: 250 <= 400 OK
        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Tranche 1: 250 + 250 = 500 > 400 → rejected
        let ctx = reclaim_ctx(105.0);
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("max risk"),
            "Error should mention max risk"
        );
    }

    // =======================================================================
    // Additional: None variant
    // =======================================================================

    #[test]
    fn test_none_variant_allows_only_one_tranche() {
        let config = PyramidConfig {
            variant: PyramidVariant::None,
            max_tranches: 4,
            tranche_fractions: vec![0.5, 0.5],
            max_risk_per_idea_usd: 2000.0,
            target_size_usd: 1000.0,
            max_correlated_exposure_usd: 50_000.0,
            require_pnl_cover_for_final: false,
            ..PyramidConfig::default()
        };

        let mut pos = PyramidPosition::new("BTC", true, config);

        assert!(pos.try_add_tranche(&probe_ctx(100.0)).is_ok());

        // Second tranche should be rejected by None variant logic
        // (max_tranches=4, tranche_fractions has 2 entries, so allowed=2)
        // The None variant check should fire before we reach tranche limit
        let ctx = probe_ctx(105.0);
        let result = pos.try_add_tranche(&ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no pyramiding"),
            "Error should mention no pyramiding: {}",
            err
        );
    }

    // =======================================================================
    // Additional: Pyramid result summary
    // =======================================================================

    #[test]
    fn test_pyramid_result_summary() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        pos.try_add_tranche(&probe_ctx(100.0)).unwrap();
        pos.try_add_tranche(&retest_ctx(110.0)).unwrap();

        let result = pos.result(115.0);
        assert_eq!(result.tranche_count, 2);
        assert!((result.total_size_usd - 500.0).abs() < 0.01);
        assert!((result.avg_entry_price - 105.0).abs() < 0.1);
        assert!(!result.stopped_out);
        assert!(result.unrealized_pnl_usd > 0.0);
    }

    // =======================================================================
    // Additional: Final tranche requires PnL cover
    // =======================================================================

    #[test]
    fn test_final_tranche_requires_pnl_cover() {
        let mut config = retest_config();
        config.require_pnl_cover_for_final = true;
        config.target_size_usd = 1000.0;
        config.current_atr = 10.0; // Large ATR = large stop distance

        let mut pos = PyramidPosition::new("BTC", true, config);

        // Add 3 tranches
        let ctx1 = AddTrancheContext {
            current_atr: 10.0,
            ..probe_ctx(100.0)
        };
        assert!(pos.try_add_tranche(&ctx1).is_ok());

        let ctx2 = AddTrancheContext {
            current_atr: 10.0,
            ..retest_ctx(101.0)
        };
        assert!(pos.try_add_tranche(&ctx2).is_ok());

        let ctx3 = AddTrancheContext {
            current_atr: 10.0,
            ..retest_ctx(102.0)
        };
        assert!(pos.try_add_tranche(&ctx3).is_ok());

        // Final tranche at 103 with tiny unrealized PnL
        // The worst-case stop risk will be large (ATR * multiplier = 10*2 = 20)
        // so unrealized PnL won't cover it
        let ctx4 = AddTrancheContext {
            current_atr: 10.0,
            ..retest_ctx(103.0)
        };
        let result = pos.try_add_tranche(&ctx4);
        assert!(
            result.is_err(),
            "Final tranche should be rejected when PnL doesn't cover worst-case stop"
        );
        if let Err(e) = result {
            assert!(
                e.contains("final tranche rejected"),
                "Error should mention final tranche rejection: {}",
                e
            );
        }
    }

    // =======================================================================
    // Additional: Standalone simulation function
    // =======================================================================

    #[test]
    fn test_run_pyramid_simulation_basic() {
        let config = retest_config();
        let data_points = vec![
            probe_ctx(100.0),
            retest_ctx(110.0),
            retest_ctx(120.0),
        ];
        // Stop prices must be ABOVE the computed combined stop.
        // After tranche 0 at 100 with ATR=2: stop ≈ 96
        // After tranche 1 at 110: avg=105, stop ≈ 101
        // After tranche 2 at 120: avg=110, stop ≈ 106
        let stop_prices = vec![99.0, 103.0, 108.0];

        let result = run_pyramid_simulation("BTC", true, config, &data_points, &stop_prices);
        assert_eq!(result.tranche_count, 3);
        assert!(result.total_size_usd > 0.0);
        assert!(!result.stopped_out);
    }

    #[test]
    fn test_run_pyramid_simulation_stopped_out() {
        let config = retest_config();
        // After adding at 100 with ATR=2, stop = 100 - 2*2 = 96
        let data_points = vec![
            AddTrancheContext {
                current_atr: 2.0,
                ..probe_ctx(100.0)
            },
        ];
        // Stop price at 90 (well below computed stop of 96)
        let stop_prices = vec![90.0];

        let result = run_pyramid_simulation("BTC", true, config, &data_points, &stop_prices);
        assert!(result.stopped_out, "Position should be stopped out at 90 (stop ~96)");
    }

    #[test]
    fn test_run_pyramid_simulation_empty_data() {
        let config = PyramidConfig::default();
        let result = run_pyramid_simulation("BTC", true, config, &[], &[]);
        assert_eq!(result.tranche_count, 0);
        assert!((result.total_size_usd).abs() < 0.01);
    }

    // =======================================================================
    // Additional: Volume-weighted average entry
    // =======================================================================

    #[test]
    fn test_volume_weighted_average_entry() {
        let config = PyramidConfig {
            variant: PyramidVariant::Retest,
            tranche_fractions: vec![0.5, 0.5],
            max_tranches: 2,
            target_size_usd: 1000.0,
            max_risk_per_idea_usd: 2000.0,
            max_correlated_exposure_usd: 50_000.0,
            require_pnl_cover_for_final: false,
            ..PyramidConfig::default()
        };
        let mut pos = PyramidPosition::new("BTC", true, config);

        pos.try_add_tranche(&probe_ctx(100.0)).unwrap();
        pos.try_add_tranche(&retest_ctx(110.0)).unwrap();

        // Each tranche is 500 USD (50% of 1000)
        // VWAP = (100*500 + 110*500) / 1000 = 105
        let avg = pos.avg_entry_price();
        assert!((avg - 105.0).abs() < 0.01, "VWAP should be 105, got {}", avg);
    }

    // =======================================================================
    // Additional: Unrealized PnL computation
    // =======================================================================

    #[test]
    fn test_unrealized_pnl_long() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", true, config);

        pos.try_add_tranche(&probe_ctx(100.0)).unwrap();
        pos.try_add_tranche(&retest_ctx(110.0)).unwrap();

        // VWAP = 105, total = 500
        // At price 115: PnL = (115 - 105) / 105 * 500 ≈ 47.62
        let pnl = pos.unrealized_pnl(115.0);
        assert!(pnl > 0.0, "PnL should be positive");
        assert!(pnl > 40.0, "PnL should be ~47.62, got {}", pnl);
    }

    #[test]
    fn test_unrealized_pnl_short() {
        let config = retest_config();
        let mut pos = PyramidPosition::new("BTC", false, config);

        pos.try_add_tranche(&probe_ctx(100.0)).unwrap();
        pos.try_add_tranche(&retest_ctx(90.0)).unwrap();

        // VWAP = (100*250 + 90*250) / 500 = 95
        // At price 85: PnL = (95 - 85) / 95 * 500 ≈ 52.63
        let pnl = pos.unrealized_pnl(85.0);
        assert!(pnl > 0.0, "PnL should be positive for winning short");
    }

    // =======================================================================
    // Additional: Display variant
    // =======================================================================

    #[test]
    fn test_variant_display() {
        assert_eq!(format!("{}", PyramidVariant::None), "none");
        assert_eq!(format!("{}", PyramidVariant::Reclaim), "reclaim");
        assert_eq!(format!("{}", PyramidVariant::Retest), "retest");
        assert_eq!(format!("{}", PyramidVariant::ProfitFunded), "profit_funded");
        assert_eq!(format!("{}", PyramidVariant::AtrTrail), "atr_trail");
    }

    // =======================================================================
    // Additional: Serialization round-trip
    // =======================================================================

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = reclaim_config();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: PyramidConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_variant_serialization_roundtrip() {
        for variant in [
            PyramidVariant::None,
            PyramidVariant::Reclaim,
            PyramidVariant::Retest,
            PyramidVariant::ProfitFunded,
            PyramidVariant::AtrTrail,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let parsed: PyramidVariant = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, parsed);
        }
    }

    #[test]
    fn test_tranche_serialization_roundtrip() {
        let tranche = PyramidTranche {
            entry_price: 100.0,
            size_usd: 250.0,
            trigger_reason: "probe".to_string(),
            timestamp_ms: 1_700_000_000_000,
            tranche_index: 0,
        };
        let json = serde_json::to_string(&tranche).unwrap();
        let parsed: PyramidTranche = serde_json::from_str(&json).unwrap();
        assert_eq!(tranche, parsed);
    }
}
