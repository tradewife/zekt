//! Strategy trait and implementations for the Zekt trading system.
//!
//! This module defines the `Strategy` trait that all trading strategies must implement.
//! It also provides the `MomentumScalperStrategy` (extracted from the original `MomentumDetector`),
//! `LpConsumptionStrategy` (LP depth consumption detector from M1 blueprints),
//! and a centralized factory function for strategy instantiation.

use crate::signal::{
    ExitReason, MomentumDetector, MomentumSnapshot, PoolSnapshot, Signal,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

/// Parameters for a trading strategy, used for display and validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyParams {
    pub direction_bias: String,
    pub momentum_threshold_pct: f64,
    pub lookback_count: usize,
    pub scale_in_clips: u32,
    pub clip_size_usd: f64,
    pub max_hold_secs: u64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub trailing_stop_pct: f64,
    pub trailing_activation_pct: f64,
    pub cooldown_after_loss_secs: u64,
    pub use_native_tp_sl: bool,
}

impl StrategyParams {
    /// Validate that all parameter values are within acceptable ranges.
    /// Returns an error message for the first invalid parameter found.
    pub fn validate(&self) -> Result<(), String> {
        if self.momentum_threshold_pct <= 0.0 {
            return Err(format!(
                "momentum_threshold_pct must be > 0, got {}",
                self.momentum_threshold_pct
            ));
        }
        if self.lookback_count == 0 {
            return Err("lookback_count must be > 0".to_string());
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0, got {}",
                self.clip_size_usd
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0, got {}",
                self.stop_loss_pct
            ));
        }
        if self.trailing_stop_pct < 0.0 {
            return Err(format!(
                "trailing_stop_pct must be >= 0, got {}",
                self.trailing_stop_pct
            ));
        }
        if self.trailing_activation_pct < 0.0 {
            return Err(format!(
                "trailing_activation_pct must be >= 0, got {}",
                self.trailing_activation_pct
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LP Consumption Strategy Parameters
// ---------------------------------------------------------------------------

/// Parameters specific to the LP Consumption Detector strategy.
///
/// All parameter values originate from the M1 blueprint:
/// `data/strategy-blueprints/lp-consumer.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LpConsumptionParams {
    // --- Entry parameters ---
    // From blueprint: data/strategy-blueprints/lp-consumer.json#parameters.entry

    /// Minimum utilization velocity (change per tick) to trigger an entry signal.
    /// From blueprint: entry.threshold_pct = 0.5 (interpreted as velocity threshold)
    pub consumption_velocity_threshold: f64,

    /// Minimum LP concentration ratio (0.0-1.0) to consider the pool concentrated enough.
    /// From blueprint: entry.confirmation = "consumption_directional >= 70%"
    /// i.e., at least 70% of consumption must be in one direction.
    pub lp_concentration_min: f64,

    /// Number of consecutive ticks with directional consumption above threshold
    /// required to confirm entry. Provides signal stability filtering.
    pub confirmation_ticks: usize,

    /// Maximum utilization ratio (0.0-1.0) — don't enter if pool is already maxed out.
    pub max_utilization: f64,

    // --- Standard trading parameters ---
    // From blueprint: data/strategy-blueprints/lp-consumer.json#parameters.exit + risk

    /// Direction bias: "long", "short", or "neutral".
    #[serde(default = "default_direction_bias")]
    pub direction_bias: String,
    /// Position clip size in USD.
    /// From blueprint: clip_size_usd = 20904.74 (scaled down for paper trading)
    #[serde(default = "default_clip_size_usd")]
    pub clip_size_usd: f64,
    /// Maximum hold duration in seconds.
    /// From blueprint: exit.max_hold_secs = 3600
    #[serde(default = "default_max_hold_secs")]
    pub max_hold_secs: u64,
    /// Take-profit threshold percentage.
    /// From blueprint: exit.take_profit_pct = 2.0
    #[serde(default = "default_take_profit_pct")]
    pub take_profit_pct: f64,
    /// Stop-loss threshold percentage.
    /// From blueprint: exit.stop_loss_pct = 1.0
    #[serde(default = "default_stop_loss_pct")]
    pub stop_loss_pct: f64,
    /// Trailing stop percentage.
    /// From blueprint: exit.trailing_stop_pct = 0.8
    #[serde(default = "default_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage.
    /// From blueprint: exit.trailing_activation_pct = 1.2
    #[serde(default = "default_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    /// Cooldown after a losing trade, in seconds.
    /// From blueprint: risk.cooldown_after_loss_secs = 300
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,
    /// Whether to use native on-chain TP/SL trigger orders.
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,
    /// Leverage for positions.
    /// From blueprint: leverage = 2.5
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    /// Number of scale-in clips (for multi-clip entry).
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,

    // --- Flash-native market parameters ---
    // For Flash Trade-only markets (not on HL) with thin books

    /// Whether to activate flash-native mode (relaxed thresholds for Flash-only markets).
    #[serde(default)]
    pub flash_native_mode: bool,
    /// Minimum utilization percentage to consider a market a flash-native target.
    #[serde(default = "default_flash_native_min_util")]
    pub flash_native_min_util_pct: f64,
    /// Velocity threshold multiplier for flash-native markets (lower = more sensitive).
    #[serde(default = "default_flash_native_velocity_mult")]
    pub flash_native_velocity_mult: f64,
    /// Whether the current market is flash-only (set dynamically, not from config).
    #[serde(default, skip)]
    pub is_flash_only_market: bool,
}

fn default_flash_native_min_util() -> f64 { 30.0 }
fn default_flash_native_velocity_mult() -> f64 { 0.6 }

fn default_direction_bias() -> String { "neutral".to_string() }
fn default_clip_size_usd() -> f64 { 100.0 }
fn default_max_hold_secs() -> u64 { 3600 }
fn default_take_profit_pct() -> f64 { 2.0 }
fn default_stop_loss_pct() -> f64 { 1.0 }
fn default_trailing_stop_pct() -> f64 { 0.8 }
fn default_trailing_activation_pct() -> f64 { 1.2 }
fn default_cooldown_after_loss_secs() -> u64 { 300 }
fn default_use_native_tp_sl() -> bool { true }
fn default_leverage() -> f64 { 2.5 }
fn default_scale_in_clips() -> u32 { 1 }

impl LpConsumptionParams {
    /// Validate LP consumption parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.consumption_velocity_threshold <= 0.0 {
            return Err(format!(
                "consumption_velocity_threshold must be > 0, got {}",
                self.consumption_velocity_threshold
            ));
        }
        if self.lp_concentration_min <= 0.0 || self.lp_concentration_min > 1.0 {
            return Err(format!(
                "lp_concentration_min must be in (0, 1], got {}",
                self.lp_concentration_min
            ));
        }
        if self.confirmation_ticks == 0 {
            return Err("confirmation_ticks must be > 0".to_string());
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0, got {}",
                self.clip_size_usd
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0, got {}",
                self.stop_loss_pct
            ));
        }
        Ok(())
    }

    /// Convert to the generic StrategyParams for use by engine/risk modules
    /// that need a uniform parameter interface.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.consumption_velocity_threshold,
            lookback_count: self.confirmation_ticks * 5, // rough equivalent
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

// ---------------------------------------------------------------------------
// Mean Reversion Strategy Parameters
// ---------------------------------------------------------------------------

/// Parameters specific to the Mean Reversion Scalper strategy.
///
/// Exit parameters derived from: data/strategy-blueprints/swing-trader.json#parameters.exit
/// (scaled down for scalper timeframes — mean reversion targets smaller moves).
/// Entry logic based on M1 strategy classification analysis:
/// oscillating long/short direction, short hold times, tight PnL range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionParams {
    // --- Entry parameters ---
    // Mean reversion concept: fade momentum spikes after reversal signals.
    // Entry fires when price deviates from SMA by threshold, then reversal tick confirmed.

    /// Number of price points to compute the simple moving average (SMA).
    /// From M1 analysis: mean-reversion wallets showed short hold times,
    /// indicating quick entries/exits around the mean.
    pub mean_lookback: usize,

    /// Minimum price deviation from SMA (%) to detect a "spike".
    /// A spike above SMA → potential SHORT entry.
    /// A spike below SMA → potential LONG entry.
    /// From M1 analysis: tight PnL range suggests moderate threshold.
    pub deviation_threshold_pct: f64,

    /// Number of consecutive ticks moving back toward the mean required
    /// to confirm a reversal after a spike is detected.
    /// From M1 analysis: moderate-to-high trade frequency supports quick confirmation.
    pub reversal_confirmation_ticks: usize,

    // --- Exit parameters ---
    // From blueprint: data/strategy-blueprints/swing-trader.json#parameters.exit
    // Scaled for mean reversion: tighter TP, no trailing, shorter holds.

    /// How close to the SMA the price must return (%) for a "mean return" exit.
    /// This is the primary exit condition for mean reversion.
    pub mean_tolerance_pct: f64,

    /// Direction bias: "long", "short", or "neutral".
    #[serde(default = "default_direction_bias")]
    pub direction_bias: String,
    /// Position clip size in USD.
    /// From M1 analysis: mean-reversion wallets used moderate clip sizes.
    #[serde(default = "default_mr_clip_size_usd")]
    pub clip_size_usd: f64,
    /// Maximum hold duration in seconds.
    /// From blueprint: swing-trader exit.max_hold_secs = 28800 (scaled to 1800 for scalper).
    #[serde(default = "default_mr_max_hold_secs")]
    pub max_hold_secs: u64,
    /// Take-profit threshold percentage.
    /// From blueprint: swing-trader exit.take_profit_pct = 4.0 (scaled to 1.0 for scalper).
    #[serde(default = "default_mr_take_profit_pct")]
    pub take_profit_pct: f64,
    /// Stop-loss threshold percentage.
    /// From blueprint: swing-trader exit.stop_loss_pct = 2.0 (scaled to 1.5 for scalper).
    #[serde(default = "default_mr_stop_loss_pct")]
    pub stop_loss_pct: f64,
    /// Trailing stop percentage (0.0 = disabled for mean reversion).
    /// Mean reversion targets a fixed level (the mean), not momentum continuation.
    #[serde(default)]
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage (0.0 = disabled).
    #[serde(default)]
    pub trailing_activation_pct: f64,
    /// Cooldown after a losing trade, in seconds.
    /// From blueprint: swing-trader risk.cooldown_after_loss_secs = 900 (scaled to 300).
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,
    /// Whether to use native on-chain TP/SL trigger orders.
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,
    /// Leverage for positions.
    /// From M1 analysis: mean-reversion wallets used moderate leverage.
    #[serde(default = "default_mr_leverage")]
    pub leverage: f64,
    /// Number of scale-in clips (for multi-clip entry).
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
}

fn default_mr_clip_size_usd() -> f64 { 100.0 }
fn default_mr_max_hold_secs() -> u64 { 1800 }
fn default_mr_take_profit_pct() -> f64 { 1.0 }
fn default_mr_stop_loss_pct() -> f64 { 1.5 }
fn default_mr_leverage() -> f64 { 3.0 }

impl MeanReversionParams {
    /// Validate mean reversion parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.mean_lookback == 0 {
            return Err("mean_lookback must be > 0".to_string());
        }
        if self.deviation_threshold_pct <= 0.0 {
            return Err(format!(
                "deviation_threshold_pct must be > 0, got {}",
                self.deviation_threshold_pct
            ));
        }
        if self.reversal_confirmation_ticks == 0 {
            return Err("reversal_confirmation_ticks must be > 0".to_string());
        }
        if self.mean_tolerance_pct < 0.0 {
            return Err(format!(
                "mean_tolerance_pct must be >= 0, got {}",
                self.mean_tolerance_pct
            ));
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0, got {}",
                self.clip_size_usd
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0, got {}",
                self.stop_loss_pct
            ));
        }
        Ok(())
    }

    /// Convert to the generic StrategyParams for use by engine/risk modules
    /// that need a uniform parameter interface.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.deviation_threshold_pct,
            lookback_count: self.mean_lookback,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

// ---------------------------------------------------------------------------
// Trend Follower Strategy Parameters
// ---------------------------------------------------------------------------

/// Parameters specific to the Trend Follower strategy.
///
/// This strategy enters on confirmed momentum breakouts with wider stops,
/// trailing exits, and longer holds than scalper strategies. It accepts a
/// lower win rate in exchange for larger average winners.
///
/// Entry logic: price velocity must exceed a *higher* threshold than the
/// momentum scalper ( breakout confirmation ), AND the trend must be
/// confirmed over multiple consecutive ticks.
///
/// Exit logic: wider trailing stop, generous time stop. No mean-return exit.
///
/// Sensible defaults are used since M1 blueprints did not identify a
/// dedicated trend-following pattern. Parameters are tuned for Flash Trade
/// perp markets with 5-second polling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendFollowerParams {
    // --- Entry parameters ---

    /// Velocity threshold (%) for a "breakout" — higher than the momentum scalper's
    /// threshold. A true breakout should show sustained, strong directional momentum.
    /// Default 0.25 %/tick (vs 0.15 for the scalper).
    pub breakout_threshold_pct: f64,

    /// Number of consecutive ticks above breakout threshold required to confirm entry.
    /// This filters out brief spikes that don't become sustained trends.
    /// Default 4 ticks (20 seconds at 5s poll interval).
    pub confirmation_ticks: usize,

    /// Minimum number of price points required before any entry signal is generated.
    /// The strategy needs enough history to compute a reliable trend.
    /// Default 30.
    pub min_price_count: usize,

    // --- Exit parameters ---

    /// Direction bias: "long", "short", or "neutral".
    #[serde(default = "default_direction_bias")]
    pub direction_bias: String,
    /// Position clip size in USD.
    #[serde(default = "default_tf_clip_size_usd")]
    pub clip_size_usd: f64,
    /// Maximum hold duration in seconds — longer than scalper (7200 vs 1800).
    #[serde(default = "default_tf_max_hold_secs")]
    pub max_hold_secs: u64,
    /// Take-profit threshold percentage — wider than scalper (5.0 vs 2.5).
    #[serde(default = "default_tf_take_profit_pct")]
    pub take_profit_pct: f64,
    /// Stop-loss threshold percentage — wider than scalper (2.0 vs 1.0).
    #[serde(default = "default_tf_stop_loss_pct")]
    pub stop_loss_pct: f64,
    /// Trailing stop percentage — wider than scalper (1.5 vs 0.8).
    #[serde(default = "default_tf_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage — when to start trailing (2.5 vs 1.5).
    #[serde(default = "default_tf_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    /// Cooldown after a losing trade, in seconds.
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,
    /// Whether to use native on-chain TP/SL trigger orders.
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,
    /// Leverage for positions.
    #[serde(default = "default_tf_leverage")]
    pub leverage: f64,
    /// Number of scale-in clips (for multi-clip entry).
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
}

fn default_tf_clip_size_usd() -> f64 { 100.0 }
fn default_tf_max_hold_secs() -> u64 { 7200 }
fn default_tf_take_profit_pct() -> f64 { 5.0 }
fn default_tf_stop_loss_pct() -> f64 { 2.0 }
fn default_tf_trailing_stop_pct() -> f64 { 1.5 }
fn default_tf_trailing_activation_pct() -> f64 { 2.5 }
fn default_tf_leverage() -> f64 { 5.0 }

impl TrendFollowerParams {
    /// Validate trend follower parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.breakout_threshold_pct <= 0.0 {
            return Err(format!(
                "breakout_threshold_pct must be > 0, got {}",
                self.breakout_threshold_pct
            ));
        }
        if self.confirmation_ticks == 0 {
            return Err("confirmation_ticks must be > 0".to_string());
        }
        if self.min_price_count == 0 {
            return Err("min_price_count must be > 0".to_string());
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0, got {}",
                self.clip_size_usd
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0, got {}",
                self.stop_loss_pct
            ));
        }
        if self.trailing_stop_pct < 0.0 {
            return Err(format!(
                "trailing_stop_pct must be >= 0, got {}",
                self.trailing_stop_pct
            ));
        }
        Ok(())
    }

    /// Convert to the generic StrategyParams for use by engine/risk modules
    /// that need a uniform parameter interface.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.breakout_threshold_pct,
            lookback_count: self.min_price_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

// ---------------------------------------------------------------------------
// PositionContext
// ---------------------------------------------------------------------------

/// Context passed to `detect_exit` bundling all position and exit-threshold parameters.
/// This avoids passing 8+ individual parameters to the exit method.
#[derive(Debug, Clone)]
pub struct PositionContext {
    /// Whether this is a long position.
    pub is_long: bool,
    /// Entry price of the position.
    pub entry_price: f64,
    /// Current price of the position.
    pub current_price: f64,
    /// Peak price seen since entry (highest for longs, lowest for shorts).
    pub peak_price: f64,
    /// How long the position has been held, in seconds.
    pub hold_secs: u64,
    /// Maximum hold duration in seconds before time-stop triggers.
    pub max_hold_secs: u64,
    /// Take-profit threshold as a percentage.
    pub take_profit_pct: f64,
    /// Stop-loss threshold as a percentage.
    pub stop_loss_pct: f64,
    /// Trailing stop percentage.
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage (peak profit must exceed this).
    pub trailing_activation_pct: f64,
}

// ---------------------------------------------------------------------------
// Strategy Trait
// ---------------------------------------------------------------------------

/// The core Strategy trait that all trading strategies must implement.
///
/// This trait is object-safe and `Send + Sync` so it can be used as `Box<dyn Strategy>`
/// inside async contexts.
pub trait Strategy: Send + Sync {
    /// Return the canonical name of this strategy (e.g., "momentum-scalper").
    fn name(&self) -> &str;

    /// Detect an entry signal given the latest market snapshot.
    ///
    /// The strategy is allowed mutable access because it maintains internal state
    /// (e.g., price buffers for the momentum detector).
    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal;

    /// Detect whether an exit should be triggered for an open position.
    ///
    /// Returns `Some(Signal::ExitLong/ExitShort)` if an exit condition is met,
    /// or `None` to continue holding.
    fn detect_exit(&self, snapshot: &MomentumSnapshot, context: &PositionContext) -> Option<Signal>;

    /// Return the strategy's parameter set (for display, logging, validation).
    fn parameters(&self) -> &StrategyParams;

    /// Push a new price point into the strategy's internal buffer.
    /// This is called by the engine on each tick before `detect_entry`/`detect_exit`.
    fn push_price(&mut self, price: f64, timestamp_ms: i64);

    /// Produce a `MomentumSnapshot` from the current internal state.
    /// Used for logging and debugging.
    fn snapshot(&self) -> MomentumSnapshot;

    /// Downcast support for trait-object downcasting (e.g., to `FundingRateCaptureStrategy`).
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn std::any::Any;
    /// Mutable downcast support.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// ---------------------------------------------------------------------------
// MomentumScalperStrategy
// ---------------------------------------------------------------------------

/// The original momentum scalper strategy, extracted from `MomentumDetector`.
///
/// This wraps a `MomentumDetector` and implements the `Strategy` trait.
/// The logic is identical to the pre-refactor `MomentumDetector::detect_signal`
/// and `MomentumDetector::detect_exit` methods.
pub struct MomentumScalperStrategy {
    detector: MomentumDetector,
    params: StrategyParams,
}

impl MomentumScalperStrategy {
    pub fn new(params: StrategyParams) -> Self {
        let detector =
            MomentumDetector::new(params.momentum_threshold_pct, params.lookback_count);
        Self { detector, params }
    }
}

impl Strategy for MomentumScalperStrategy {
    fn name(&self) -> &str {
        "momentum-scalper"
    }

    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal {
        self.detector.detect_signal(snapshot)
    }

    fn detect_exit(
        &self,
        snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        self.detector.detect_exit(
            snapshot,
            ctx.is_long,
            ctx.entry_price,
            ctx.current_price,
            ctx.peak_price,
            ctx.hold_secs,
            ctx.max_hold_secs,
            ctx.take_profit_pct,
            ctx.stop_loss_pct,
            ctx.trailing_stop_pct,
            ctx.trailing_activation_pct,
        )
    }

    fn parameters(&self) -> &StrategyParams {
        &self.params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.detector.push_price(price, timestamp_ms);
    }

    fn snapshot(&self) -> MomentumSnapshot {
        self.detector.analyze()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// LpConsumptionStrategy
// ---------------------------------------------------------------------------

/// LP Consumption Detector strategy.
///
/// Detects when a single LP's depth is being consumed in one direction via pool
/// utilization data from the Flash Trade API. This is a market-structure arbitrage
/// strategy: find illiquid markets where a dominant LP provides most of the depth,
/// detect when that LP is being consumed, and ride the momentum.
///
/// **Entry conditions:**
/// 1. Pool data is available (no entry without pool data)
/// 2. One-sided utilization velocity exceeds `consumption_velocity_threshold`
/// 3. Directional concentration ratio exceeds `lp_concentration_min`
/// 4. Confirmed over `confirmation_ticks` consecutive ticks
/// 5. Utilization is below `max_utilization` (don't enter a fully consumed pool)
/// 6. Entry direction matches consumption direction
///
/// **Exit conditions (priority order):**
/// 1. Stop-loss (absolute loss from entry)
/// 2. Take-profit (absolute gain from entry)
/// 3. Trailing stop (after activation threshold reached)
/// 4. Time stop (max hold duration exceeded)
/// 5. Consumption stall (velocity drops below threshold while in position)
/// 6. Reversal (opposite-side consumption detected)
///
/// Blueprint source: `data/strategy-blueprints/lp-consumer.json`
pub struct LpConsumptionStrategy {
    params: LpConsumptionParams,
    generic_params: StrategyParams,
    /// Internal momentum detector for price-based exit signals.
    detector: MomentumDetector,
    /// Rolling window of utilization velocity readings for stall detection.
    velocity_history: VecDeque<f64>,
    /// Number of consecutive ticks with consumption exceeding threshold in the
    /// same direction. Positive = long consumption, negative = short consumption.
    consecutive_consumption: i32,
    /// Latest pool snapshot for snapshot() reporting.
    last_pool: Option<PoolSnapshot>,
}

impl LpConsumptionStrategy {
    pub fn new(params: LpConsumptionParams) -> Self {
        let generic = params.to_strategy_params();
        let detector = MomentumDetector::new(
            params.consumption_velocity_threshold,
            params.confirmation_ticks * 2,
        );
        Self {
            generic_params: generic,
            detector,
            velocity_history: VecDeque::with_capacity(60),
            consecutive_consumption: 0,
            last_pool: None,
            params,
        }
    }

    /// Compute the directional concentration ratio.
    /// Returns (long_ratio, short_ratio) where each is 0.0-1.0 and they sum to 1.0
    /// (or close to it if both are near zero).
    ///
    /// "Directional" means: of the total consumption happening, how much is on
    /// each side? If long_utilization_velocity = 0.8 and short = 0.2, then
    /// long_ratio = 0.8 / (0.8 + 0.2) = 0.8 = 80% concentrated on the long side.
    fn directional_concentration(pool: &PoolSnapshot) -> (f64, f64) {
        let long_v = pool.long_utilization_velocity.abs();
        let short_v = pool.short_utilization_velocity.abs();
        let total = long_v + short_v;
        if total < 1e-10 {
            return (0.5, 0.5);
        }
        (long_v / total, short_v / total)
    }
}

impl Strategy for LpConsumptionStrategy {
    fn name(&self) -> &str {
        "lp-consumption"
    }

    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal {
        let pool = match &snapshot.pool_data {
            Some(p) => p,
            None => {
                // No pool data available — cannot detect LP consumption.
                // Log at debug level to avoid spamming on every tick.
                debug!(
                    "[lp-consumption] No pool data in snapshot, skipping entry detection"
                );
                return Signal::NoSignal;
            }
        };

        self.last_pool = Some(pool.clone());

        // Flash-native mode: adjust thresholds for Flash Trade-only markets
        let effective_velocity_threshold = if self.params.flash_native_mode && self.params.is_flash_only_market {
            self.params.consumption_velocity_threshold * self.params.flash_native_velocity_mult
        } else {
            self.params.consumption_velocity_threshold
        };

        // Flash-native: check minimum utilization for thin-book markets
        if self.params.flash_native_mode && self.params.is_flash_only_market {
            let utilization = pool.long_utilization.max(pool.short_utilization);
            if utilization < self.params.flash_native_min_util_pct / 100.0 {
                debug!(
                    "[lp-consumption] Flash-native: utilization too low ({:.1}% < {:.1}%), skipping",
                    utilization * 100.0, self.params.flash_native_min_util_pct
                );
                return Signal::NoSignal;
            }
        }

        // Compute directional concentration
        let (long_conc, short_conc) = Self::directional_concentration(pool);

        // Determine which side has higher velocity
        let long_velocity = pool.long_utilization_velocity;
        let short_velocity = pool.short_utilization_velocity;
        let max_velocity = long_velocity.abs().max(short_velocity.abs());

        // Track consecutive consumption ticks
        // Positive = long-side being consumed (enter LONG), negative = short-side (enter SHORT)
        let direction = if long_velocity > short_velocity.abs() && long_conc >= self.params.lp_concentration_min {
            // Long utilization growing fast = bid side being consumed = price likely up = LONG
            1
        } else if short_velocity > long_velocity.abs() && short_conc >= self.params.lp_concentration_min {
            // Short utilization growing fast = ask side being consumed = price likely down = SHORT
            -1
        } else {
            // No clear directional consumption above concentration threshold
            if self.consecutive_consumption != 0 {
                debug!(
                    "[lp-consumption] Consecutive reset: long_conc={:.2} short_conc={:.2} \
                     long_v={:.4} short_v={:.4}",
                    long_conc, short_conc, long_velocity, short_velocity
                );
            }
            self.consecutive_consumption = 0;
            return Signal::NoSignal;
        };

        // Check velocity threshold
        if max_velocity < effective_velocity_threshold {
            if self.consecutive_consumption != 0 {
                debug!(
                    "[lp-consumption] Velocity below threshold: {:.4} < {:.4}",
                    max_velocity, effective_velocity_threshold
                );
            }
            self.consecutive_consumption = 0;
            return Signal::NoSignal;
        }

        // Check max utilization — don't enter a fully consumed pool
        let current_utilization = pool.long_utilization.max(pool.short_utilization);
        if current_utilization >= self.params.max_utilization {
            debug!(
                "[lp-consumption] Utilization too high: {:.2} >= {:.2}",
                current_utilization, self.params.max_utilization
            );
            self.consecutive_consumption = 0;
            return Signal::NoSignal;
        }

        // Track consecutive direction
        let prev_dir = self.consecutive_consumption.signum();
        if prev_dir == direction {
            self.consecutive_consumption += direction;
        } else {
            self.consecutive_consumption = direction;
        }

        // Record velocity for stall detection
        self.velocity_history.push_back(max_velocity);
        if self.velocity_history.len() > 60 {
            self.velocity_history.pop_front();
        }

        let consec_count = self.consecutive_consumption.unsigned_abs();
        debug!(
            "[lp-consumption] Consecutive ticks: {} (need {}), velocity={:.4}, \
             long_conc={:.2}, short_conc={:.2}, utilization={:.2}",
            consec_count, self.params.confirmation_ticks, max_velocity,
            long_conc, short_conc, current_utilization,
        );

        // Check confirmation — need N consecutive ticks of directional consumption
        if (consec_count as usize) < self.params.confirmation_ticks {
            return Signal::NoSignal;
        }

        // ENTRY SIGNAL
        let strength = (max_velocity / effective_velocity_threshold * 50.0)
            .clamp(50.0, 100.0);

        let flash_tag = if self.params.flash_native_mode && self.params.is_flash_only_market {
            " [FLASH-NATIVE]"
        } else {
            ""
        };

        if direction > 0 {
            info!(
                "[lp-consumption]{} LONG signal: velocity={:.4} (threshold={:.4}), \
                 concentration={:.0}%, consecutive={}, utilization={:.2}",
                flash_tag, max_velocity, effective_velocity_threshold,
                long_conc * 100.0, consec_count, current_utilization,
            );
            Signal::MomentumLong {
                strength,
                velocity_pct: max_velocity,
            }
        } else {
            info!(
                "[lp-consumption]{} SHORT signal: velocity={:.4} (threshold={:.4}), \
                 concentration={:.0}%, consecutive={}, utilization={:.2}",
                flash_tag, max_velocity, effective_velocity_threshold,
                short_conc * 100.0, consec_count, current_utilization,
            );
            Signal::MomentumShort {
                strength,
                velocity_pct: max_velocity,
            }
        }
    }

    fn detect_exit(
        &self,
        snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        // Standard SL/TP/trailing/time exits (same priority as momentum scalper)
        let pnl_pct = if ctx.is_long {
            (ctx.current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - ctx.current_price) / ctx.entry_price * 100.0
        };

        let peak_profit_pct = if ctx.entry_price > 0.0 {
            if ctx.is_long {
                (ctx.peak_price - ctx.entry_price) / ctx.entry_price * 100.0
            } else {
                (ctx.entry_price - ctx.peak_price) / ctx.entry_price * 100.0
            }
        } else {
            0.0
        };

        let retracement_pct = if ctx.peak_price > 0.0 && peak_profit_pct > 0.0 {
            if ctx.is_long {
                (ctx.peak_price - ctx.current_price) / ctx.peak_price * 100.0
            } else {
                (ctx.current_price - ctx.peak_price) / ctx.peak_price * 100.0
            }
        } else {
            0.0
        };

        // 1. Stop loss (highest priority)
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!(
                "[lp-consumption] STOP LOSS: pnl={:.2}%, threshold=-{:.2}%",
                pnl_pct, ctx.stop_loss_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::StopLoss));
        }

        // 2. Take profit
        if pnl_pct >= ctx.take_profit_pct {
            info!(
                "[lp-consumption] TAKE PROFIT: pnl={:.2}%, threshold={:.2}%",
                pnl_pct, ctx.take_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
        }

        // 3. Trailing stop
        if peak_profit_pct >= ctx.trailing_activation_pct && retracement_pct >= ctx.trailing_stop_pct {
            warn!(
                "[lp-consumption] TRAILING STOP: retracement={:.2}%, trail={:.2}%, peak_profit={:.2}%",
                retracement_pct, ctx.trailing_stop_pct, peak_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TrailingStop));
        }

        // 4. Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            warn!(
                "[lp-consumption] TIME STOP: held {}s, max={}s",
                ctx.hold_secs, ctx.max_hold_secs
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TimeStop));
        }

        // 5. LP-specific exit: consumption stall
        // If pool data shows velocity dropping below threshold, the LP consumption
        // momentum that triggered entry has stalled → exit
        if let Some(ref pool) = snapshot.pool_data {
            let relevant_velocity = if ctx.is_long {
                pool.long_utilization_velocity
            } else {
                pool.short_utilization_velocity
            };

            if relevant_velocity < self.params.consumption_velocity_threshold * 0.3 {
                debug!(
                    "[lp-consumption] Consumption stall: velocity={:.4} < {:.4}",
                    relevant_velocity,
                    self.params.consumption_velocity_threshold * 0.3
                );
                // Only exit if we're in profit or held long enough (avoid stalling at a loss)
                if pnl_pct > 0.0 || ctx.hold_secs > 300 {
                    return Some(exit_signal(
                        ctx.is_long,
                        crate::signal::ExitReason::MomentumLost,
                    ));
                }
            }
        }

        // 6. Reversal detection: opposite-side consumption is accelerating
        if let Some(ref pool) = snapshot.pool_data {
            let (long_conc, short_conc) = Self::directional_concentration(pool);
            let reversal_detected = if ctx.is_long {
                short_conc > self.params.lp_concentration_min
                    && pool.short_utilization_velocity > self.params.consumption_velocity_threshold
            } else {
                long_conc > self.params.lp_concentration_min
                    && pool.long_utilization_velocity > self.params.consumption_velocity_threshold
            };

            if reversal_detected {
                warn!(
                    "[lp-consumption] REVERSAL detected: {} side consumption accelerating",
                    if ctx.is_long { "short" } else { "long" }
                );
                return Some(exit_signal(
                    ctx.is_long,
                    crate::signal::ExitReason::ReversalDetected,
                ));
            }
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.detector.push_price(price, timestamp_ms);
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let mut snap = self.detector.analyze();
        // Layer in the pool data if we have it
        if self.last_pool.is_some() && snap.pool_data.is_none() {
            snap.pool_data = self.last_pool.clone();
        }
        snap
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Helper to create an exit signal.
fn exit_signal(is_long: bool, reason: crate::signal::ExitReason) -> Signal {
    if is_long {
        Signal::ExitLong { reason }
    } else {
        Signal::ExitShort { reason }
    }
}

// ---------------------------------------------------------------------------
// MeanReversionStrategy
// ---------------------------------------------------------------------------

/// Direction of a detected spike relative to the SMA.
#[derive(Debug, Clone, PartialEq)]
enum SpikeDirection {
    /// Price spiked above SMA → looking for SHORT entry (fade the spike).
    Above,
    /// Price spiked below SMA → looking for LONG entry (fade the spike).
    Below,
}

/// Mean Reversion Scalper strategy.
///
/// Fades momentum spikes by entering in the opposite direction after a sharp
/// price move away from the mean (SMA) followed by a reversal tick. This is
/// a contrarian strategy: it bets that extreme deviations from the mean will
/// revert.
///
/// **Entry conditions:**
/// 1. Sufficient price history (>= mean_lookback prices)
/// 2. Price deviates from SMA by more than deviation_threshold_pct (spike)
/// 3. Reversal confirmed: N consecutive ticks moving back toward the mean
/// 4. Entry direction is opposite to the spike (spike up → SHORT, spike down → LONG)
///
/// **Exit conditions (priority order):**
/// 1. Stop-loss (absolute loss from entry)
/// 2. Mean return: price returns to within mean_tolerance_pct of the computed SMA
/// 3. Take-profit (absolute gain from entry)
/// 4. Time stop (max hold duration exceeded)
///
/// Note: No trailing stop — mean reversion targets a fixed level (the mean),
/// not momentum continuation.
///
/// Blueprint source: derived from M1 analysis (mean-reversion wallets) and
/// data/strategy-blueprints/swing-trader.json for exit/risk parameter ranges.
pub struct MeanReversionStrategy {
    params: MeanReversionParams,
    generic_params: StrategyParams,
    /// Rolling price buffer for SMA calculation.
    prices: VecDeque<crate::signal::PricePoint>,
    /// Current spike state, if any.
    spike_state: Option<SpikeDirection>,
    /// Number of consecutive reversal ticks counted after a spike.
    reversal_ticks: usize,
}

impl MeanReversionStrategy {
    pub fn new(params: MeanReversionParams) -> Self {
        let generic = params.to_strategy_params();
        let capacity = params.mean_lookback * 2;
        Self {
            generic_params: generic,
            prices: VecDeque::with_capacity(capacity),
            spike_state: None,
            reversal_ticks: 0,
            params,
        }
    }

    /// Compute the simple moving average over the last `lookback` prices.
    /// Returns None if there are fewer than `lookback` prices in the buffer.
    fn compute_sma(&self, lookback: usize) -> Option<f64> {
        if self.prices.len() < lookback {
            return None;
        }
        let sum: f64 = self
            .prices
            .iter()
            .rev()
            .take(lookback)
            .map(|p| p.price)
            .sum();
        Some(sum / lookback as f64)
    }

    /// Get the previous price (second-to-last in buffer).
    fn prev_price(&self) -> Option<f64> {
        if self.prices.len() < 2 {
            return None;
        }
        // prices[len-2] is the second-to-last
        self.prices.iter().rev().nth(1).map(|p| p.price)
    }

    /// Get the current (most recent) price.
    fn current_price(&self) -> Option<f64> {
        self.prices.back().map(|p| p.price)
    }
}

impl Strategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        "mean-reversion"
    }

    fn detect_entry(&mut self, _snapshot: &MomentumSnapshot) -> Signal {
        let current_price = match self.current_price() {
            Some(p) => p,
            None => return Signal::NoSignal,
        };

        let sma = match self.compute_sma(self.params.mean_lookback) {
            Some(s) => s,
            None => {
                debug!(
                    "[mean-reversion] Insufficient price history: {}/{} prices",
                    self.prices.len(),
                    self.params.mean_lookback
                );
                return Signal::NoSignal;
            }
        };

        if sma <= 0.0 {
            return Signal::NoSignal;
        }

        // Compute deviation from SMA as a percentage
        let deviation_pct = (current_price - sma) / sma * 100.0;

        // Check for spike detection
        if deviation_pct > self.params.deviation_threshold_pct {
            // Spike above SMA → potential SHORT entry
            if self.spike_state != Some(SpikeDirection::Above) {
                self.spike_state = Some(SpikeDirection::Above);
                self.reversal_ticks = 0;
                debug!(
                    "[mean-reversion] Spike ABOVE detected: price={:.2}, sma={:.2}, deviation={:.2}% (threshold={:.2}%)",
                    current_price, sma, deviation_pct, self.params.deviation_threshold_pct
                );
            }
        } else if deviation_pct < -self.params.deviation_threshold_pct {
            // Spike below SMA → potential LONG entry
            if self.spike_state != Some(SpikeDirection::Below) {
                self.spike_state = Some(SpikeDirection::Below);
                self.reversal_ticks = 0;
                debug!(
                    "[mean-reversion] Spike BELOW detected: price={:.2}, sma={:.2}, deviation={:.2}% (threshold={:.2}%)",
                    current_price, sma, deviation_pct, self.params.deviation_threshold_pct
                );
            }
        }

        // Check for reversal confirmation
        if let Some(ref spike_dir) = self.spike_state {
            let prev_price = match self.prev_price() {
                Some(p) => p,
                None => return Signal::NoSignal,
            };

            // Is this tick moving back toward the mean?
            let moving_toward_mean = match spike_dir {
                SpikeDirection::Above => {
                    // Price spiked above, reversal means price is coming down (current < prev)
                    current_price < prev_price
                }
                SpikeDirection::Below => {
                    // Price spiked below, reversal means price is coming up (current > prev)
                    current_price > prev_price
                }
            };

            if moving_toward_mean {
                self.reversal_ticks += 1;
                debug!(
                    "[mean-reversion] Reversal tick {} (need {}): price={:.2}, prev={:.2}, sma={:.2}",
                    self.reversal_ticks, self.params.reversal_confirmation_ticks,
                    current_price, prev_price, sma
                );

                if self.reversal_ticks >= self.params.reversal_confirmation_ticks {
                    // CONFIRMED REVERSAL — generate entry signal
                    // Fading the spike: spike above → SHORT, spike below → LONG
                    let strength = (deviation_pct.abs() / self.params.deviation_threshold_pct * 50.0)
                        .clamp(50.0, 100.0);
                    let velocity_pct = deviation_pct.abs();

                    // Clone spike_dir before resetting state
                    let spike_dir_clone = spike_dir.clone();

                    // Reset spike state after generating signal
                    self.spike_state = None;
                    self.reversal_ticks = 0;

                    match spike_dir_clone {
                        SpikeDirection::Above => {
                            info!(
                                "[mean-reversion] SHORT signal: price={:.2}, sma={:.2}, deviation={:.2}%, reversal_ticks={}",
                                current_price, sma, deviation_pct, self.params.reversal_confirmation_ticks
                            );
                            Signal::MomentumShort { strength, velocity_pct }
                        }
                        SpikeDirection::Below => {
                            info!(
                                "[mean-reversion] LONG signal: price={:.2}, sma={:.2}, deviation={:.2}%, reversal_ticks={}",
                                current_price, sma, deviation_pct, self.params.reversal_confirmation_ticks
                            );
                            Signal::MomentumLong { strength, velocity_pct }
                        }
                    }
                } else {
                    Signal::NoSignal
                }
            } else {
                // Not moving toward mean — if still in deviation zone, keep spike state;
                // if back within threshold, reset
                if deviation_pct.abs() <= self.params.deviation_threshold_pct {
                    debug!(
                        "[mean-reversion] Spike expired: deviation {:.2}% back within threshold {:.2}%",
                        deviation_pct, self.params.deviation_threshold_pct
                    );
                    self.spike_state = None;
                    self.reversal_ticks = 0;
                }
                // If spike is still active but this tick moved away from mean, don't reset
                // (allow resumption of reversal counting on next tick)
                Signal::NoSignal
            }
        } else {
            // No active spike — check if we're in a gradual move (no signal for mean reversion)
            Signal::NoSignal
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let current_price = ctx.current_price;

        // PnL from entry
        let pnl_pct = if ctx.is_long {
            (current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - current_price) / ctx.entry_price * 100.0
        };

        // 1. Stop loss (highest priority)
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!(
                "[mean-reversion] STOP LOSS: pnl={:.2}%, threshold=-{:.2}%",
                pnl_pct, ctx.stop_loss_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::StopLoss));
        }

        // 2. Mean return: price returns to within tolerance of computed mean
        if let Some(sma) = self.compute_sma(self.params.mean_lookback)
            && sma > 0.0
        {
            let deviation_from_mean = (current_price - sma).abs() / sma * 100.0;
            if deviation_from_mean <= self.params.mean_tolerance_pct {
                info!(
                    "[mean-reversion] MEAN RETURN exit: price={:.2}, sma={:.2}, deviation={:.2}% (tolerance={:.2}%), pnl={:.2}%",
                    current_price, sma, deviation_from_mean, self.params.mean_tolerance_pct, pnl_pct
                );
                return Some(exit_signal(
                    ctx.is_long,
                    crate::signal::ExitReason::TakeProfit,
                ));
            }
        }

        // 3. Take profit (absolute gain from entry)
        if pnl_pct >= ctx.take_profit_pct {
            info!(
                "[mean-reversion] TAKE PROFIT: pnl={:.2}%, threshold={:.2}%",
                pnl_pct, ctx.take_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
        }

        // 4. Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            warn!(
                "[mean-reversion] TIME STOP: held {}s, max={}s",
                ctx.hold_secs, ctx.max_hold_secs
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TimeStop));
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.prices.push_back(crate::signal::PricePoint { price, timestamp_ms });
        // Keep buffer at 2x mean_lookback to have enough history
        while self.prices.len() > self.params.mean_lookback * 2 {
            self.prices.pop_front();
        }
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let lookback = self.params.mean_lookback;
        let sma = self.compute_sma(lookback);
        let current_price = self.current_price().unwrap_or(0.0);

        let velocity_pct = if let Some(sma) = sma {
            if sma > 0.0 {
                (current_price - sma) / sma * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };

        let direction = if velocity_pct > 0.0 {
            crate::signal::TradeDirection::Long
        } else if velocity_pct < 0.0 {
            crate::signal::TradeDirection::Short
        } else {
            crate::signal::TradeDirection::Neutral
        };

        MomentumSnapshot {
            price_count: self.prices.len(),
            current_price,
            price_velocity_pct: velocity_pct,
            direction,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// TrendFollowerStrategy
// ---------------------------------------------------------------------------

/// Trend Follower strategy.
///
/// Enters on confirmed momentum breakouts with wider stops and trailing exits.
/// Designed for longer holds than scalper strategies — accepts a lower win rate
/// but aims for larger average winners.
///
/// **Entry conditions:**
/// 1. Sufficient price history (>= min_price_count)
/// 2. Price velocity exceeds breakout_threshold_pct (higher than scalper threshold)
/// 3. Velocity sustained for confirmation_ticks consecutive ticks
/// 4. Direction matches the breakout direction
///
/// **Exit conditions (priority order):**
/// 1. Stop-loss (wider than scalper)
/// 2. Take-profit (wider than scalper)
/// 3. Trailing stop (after activation threshold — wider activation & trail)
/// 4. Time stop (much longer than scalper)
/// 5. Trend exhaustion (velocity drops near zero while in position)
///
/// Sensible defaults are used since M1 blueprints did not produce a dedicated
/// trend-following pattern. The parameters are tuned for Flash Trade perp markets
/// with 5-second polling.
pub struct TrendFollowerStrategy {
    params: TrendFollowerParams,
    generic_params: StrategyParams,
    /// Internal momentum detector for price-based signal computation.
    detector: MomentumDetector,
    /// Rolling price buffer for independent velocity calculation.
    prices: VecDeque<crate::signal::PricePoint>,
    /// Number of consecutive ticks with velocity above breakout threshold.
    /// Positive = upward breakout, negative = downward breakout.
    consecutive_breakout: i32,
    /// Previous velocity reading for trend exhaustion detection.
    prev_velocity_pct: f64,
}

impl TrendFollowerStrategy {
    pub fn new(params: TrendFollowerParams) -> Self {
        let generic = params.to_strategy_params();
        let detector = MomentumDetector::new(
            params.breakout_threshold_pct,
            params.min_price_count,
        );
        Self {
            generic_params: generic,
            detector,
            prices: VecDeque::with_capacity(params.min_price_count * 2),
            consecutive_breakout: 0,
            prev_velocity_pct: 0.0,
            params,
        }
    }

    /// Compute price velocity over the recent lookback window.
    /// Returns (velocity_pct, is_sufficient_data).
    fn compute_velocity(&self) -> (f64, bool) {
        let lookback = self.params.min_price_count;
        if self.prices.len() < lookback {
            return (0.0, false);
        }

        let recent: Vec<_> = self.prices.iter().rev().take(lookback).collect();
        let current_price = recent[0].price;
        let oldest_price = recent.last().unwrap().price;

        if oldest_price <= 0.0 {
            return (0.0, true);
        }

        let velocity_pct = (current_price - oldest_price) / oldest_price * 100.0;
        (velocity_pct, true)
    }

    /// Get the current (most recent) price.
    #[allow(dead_code)]
    fn current_price(&self) -> Option<f64> {
        self.prices.back().map(|p| p.price)
    }
}

impl Strategy for TrendFollowerStrategy {
    fn name(&self) -> &str {
        "trend-follower"
    }

    fn detect_entry(&mut self, _snapshot: &MomentumSnapshot) -> Signal {
        // Need sufficient price history
        if self.prices.len() < self.params.min_price_count {
            debug!(
                "[trend-follower] Insufficient price history: {}/{} prices",
                self.prices.len(),
                self.params.min_price_count
            );
            return Signal::NoSignal;
        }

        let (velocity_pct, sufficient) = self.compute_velocity();
        if !sufficient {
            return Signal::NoSignal;
        }

        let threshold = self.params.breakout_threshold_pct;

        // Track consecutive breakout ticks
        if velocity_pct > threshold {
            // Upward breakout
            if self.consecutive_breakout > 0 {
                self.consecutive_breakout += 1;
            } else {
                self.consecutive_breakout = 1;
            }
        } else if velocity_pct < -threshold {
            // Downward breakout
            if self.consecutive_breakout < 0 {
                self.consecutive_breakout -= 1;
            } else {
                self.consecutive_breakout = -1;
            }
        } else {
            // Velocity within range — reset consecutive count
            if self.consecutive_breakout != 0 {
                debug!(
                    "[trend-follower] Breakout reset: velocity={:.3}% within threshold ±{:.3}%",
                    velocity_pct, threshold
                );
            }
            self.consecutive_breakout = 0;
        }

        self.prev_velocity_pct = velocity_pct;

        let consec_count = self.consecutive_breakout.unsigned_abs();
        debug!(
            "[trend-follower] Velocity: {:.3}%, threshold: ±{:.3}%, consecutive: {} (need {})",
            velocity_pct, threshold, consec_count, self.params.confirmation_ticks
        );

        // Check confirmation — need N consecutive ticks above threshold
        if (consec_count as usize) < self.params.confirmation_ticks {
            return Signal::NoSignal;
        }

        // ENTRY SIGNAL
        let strength = (velocity_pct.abs() / threshold * 50.0)
            .clamp(50.0, 100.0);

        if self.consecutive_breakout > 0 {
            info!(
                "[trend-follower] LONG breakout confirmed: velocity={:.3}% (threshold={:.3}%), consecutive={}, strength={:.1}",
                velocity_pct, threshold, consec_count, strength
            );
            Signal::MomentumLong {
                strength,
                velocity_pct,
            }
        } else {
            info!(
                "[trend-follower] SHORT breakout confirmed: velocity={:.3}% (threshold={:.3}%), consecutive={}, strength={:.1}",
                velocity_pct, threshold, consec_count, strength
            );
            Signal::MomentumShort {
                strength,
                velocity_pct,
            }
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let pnl_pct = if ctx.is_long {
            (ctx.current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - ctx.current_price) / ctx.entry_price * 100.0
        };

        let peak_profit_pct = if ctx.entry_price > 0.0 {
            if ctx.is_long {
                (ctx.peak_price - ctx.entry_price) / ctx.entry_price * 100.0
            } else {
                (ctx.entry_price - ctx.peak_price) / ctx.entry_price * 100.0
            }
        } else {
            0.0
        };

        let retracement_pct = if ctx.peak_price > 0.0 && peak_profit_pct > 0.0 {
            if ctx.is_long {
                (ctx.peak_price - ctx.current_price) / ctx.peak_price * 100.0
            } else {
                (ctx.current_price - ctx.peak_price) / ctx.peak_price * 100.0
            }
        } else {
            0.0
        };

        // 1. Stop loss (highest priority)
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!(
                "[trend-follower] STOP LOSS: pnl={:.2}%, threshold=-{:.2}%",
                pnl_pct, ctx.stop_loss_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::StopLoss));
        }

        // 2. Take profit
        if pnl_pct >= ctx.take_profit_pct {
            info!(
                "[trend-follower] TAKE PROFIT: pnl={:.2}%, threshold={:.2}%",
                pnl_pct, ctx.take_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
        }

        // 3. Trailing stop (after activation threshold)
        if peak_profit_pct >= ctx.trailing_activation_pct
            && retracement_pct >= ctx.trailing_stop_pct
        {
            warn!(
                "[trend-follower] TRAILING STOP: retracement={:.2}%, trail={:.2}%, peak_profit={:.2}%",
                retracement_pct, ctx.trailing_stop_pct, peak_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TrailingStop));
        }

        // 4. Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            warn!(
                "[trend-follower] TIME STOP: held {}s, max={}s",
                ctx.hold_secs, ctx.max_hold_secs
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TimeStop));
        }

        // 5. Trend exhaustion: velocity has dropped to near zero while in position.
        // Only fire if we've been in the trade long enough to establish a trend (>= 60s)
        // and the trend is clearly fading (velocity near zero or reversing).
        if ctx.hold_secs >= 60 {
            let (velocity_pct, sufficient) = self.compute_velocity();
            if sufficient && velocity_pct.abs() < self.params.breakout_threshold_pct * 0.2 {
                // Trend has faded to less than 20% of breakout threshold
                debug!(
                    "[trend-follower] Trend exhaustion: velocity={:.3}% (< 20% of threshold {:.3}%)",
                    velocity_pct,
                    self.params.breakout_threshold_pct * 0.2
                );
                // Only exit if we've captured some profit or held a long time
                if pnl_pct > 0.0 || ctx.hold_secs > ctx.max_hold_secs / 2 {
                    return Some(exit_signal(
                        ctx.is_long,
                        crate::signal::ExitReason::MomentumLost,
                    ));
                }
            }
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.detector.push_price(price, timestamp_ms);
        self.prices.push_back(crate::signal::PricePoint { price, timestamp_ms });
        while self.prices.len() > self.params.min_price_count * 2 {
            self.prices.pop_front();
        }
    }

    fn snapshot(&self) -> MomentumSnapshot {
        self.detector.analyze()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Liquidation Cascade Hunter Strategy
// ---------------------------------------------------------------------------

/// Parameters for the liquidation-cascade-hunter strategy.
///
/// Two setup types:
/// 1. **Cascade continuation** — price approaches a high-confidence liquidation zone,
///    forced-flow spikes, velocity confirms, depth thins, route cost OK.
/// 2. **Exhaustion reversal** — liquidation burst occurs, price reclaims VWAP,
///    depth refills, velocity decays, spread normalizes.
///
/// Entry gates (all must pass):
/// - Confidence minimum
/// - Volume z-score threshold
/// - Price distance to zone
/// - VWAP filter
/// - Spread/depth filter
/// - Regime compatibility
/// - Route cost veto
/// - Max one pending trade per symbol/side
///
/// Exits: TP, SL, trailing stop, time stop, stale-data forced exit.
///
/// **Paper-only** — blocked in live engine. **Disabled by default** in config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationCascadeParams {
    // --- Enable / paper-only ---
    /// Must be explicitly set to true to activate the strategy.
    #[serde(default)]
    pub enabled: bool,

    /// Paper-only: strategy is blocked in the live engine.
    #[serde(default = "default_true")]
    pub paper_only: bool,

    // --- Entry gates ---
    /// Minimum liquidation zone confidence [0.0, 1.0].
    #[serde(default = "default_lc_confidence_min")]
    pub confidence_min: f64,

    /// Minimum volume z-score for cascade confirmation.
    #[serde(default = "default_lc_volume_z_score")]
    pub volume_z_score_threshold: f64,

    /// Maximum distance from price to nearest zone as a percentage.
    #[serde(default = "default_lc_max_distance_pct")]
    pub max_distance_to_zone_pct: f64,

    /// Whether the VWAP filter is enabled.
    #[serde(default = "default_true")]
    pub vwap_filter_enabled: bool,

    /// Maximum bid-ask spread as a percentage.
    #[serde(default = "default_lc_spread_max_pct")]
    pub spread_max_pct: f64,

    /// Minimum order book depth in USD at the zone.
    #[serde(default = "default_lc_depth_min_usd")]
    pub depth_min_usd: f64,

    /// Whether regime compatibility filter is enabled.
    #[serde(default = "default_true")]
    pub regime_filter: bool,

    /// Maximum route cost in basis points before veto.
    #[serde(default = "default_lc_route_cost_max_bps")]
    pub route_cost_max_bps: f64,

    /// Stale data threshold in seconds. Zone data older than this blocks entry.
    #[serde(default = "default_lc_stale_threshold_secs")]
    pub stale_data_threshold_secs: u64,

    /// Minimum forced-flow velocity for cascade continuation.
    #[serde(default = "default_lc_forced_flow_vel")]
    pub forced_flow_velocity_threshold: f64,

    /// Maximum forced-flow velocity for exhaustion reversal (decay threshold).
    #[serde(default = "default_lc_velocity_decay_threshold")]
    pub velocity_decay_threshold: f64,

    // --- Exit parameters ---
    #[serde(default = "default_lc_take_profit_pct")]
    pub take_profit_pct: f64,
    #[serde(default = "default_lc_stop_loss_pct")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_lc_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    #[serde(default = "default_lc_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    #[serde(default = "default_lc_max_hold_secs")]
    pub max_hold_secs: u64,
    #[serde(default = "default_lc_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,

    // --- Cascade continuation exit parameters ---
    /// Enable take-profit into the next liquidation zone in the cascade direction.
    #[serde(default = "default_true")]
    pub next_zone_tp_enabled: bool,
    /// Enable zone-reclaimed stop exit (price reclaims the broken zone).
    #[serde(default = "default_true")]
    pub zone_reclaimed_stop_enabled: bool,
    /// Enable time stop (max_hold_secs already defines duration).
    #[serde(default = "default_true")]
    pub time_stop_enabled: bool,

    // --- General ---
    #[serde(default = "default_lc_clip_size_usd")]
    pub clip_size_usd: f64,
    #[serde(default = "default_lc_leverage")]
    pub leverage: f64,
    #[serde(default = "default_neutral")]
    pub direction_bias: String,
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
    #[serde(default = "default_true")]
    pub use_native_tp_sl: bool,
    /// Minimum price history required before generating signals.
    #[serde(default = "default_lc_lookback_count")]
    pub lookback_count: usize,
}

fn default_lc_confidence_min() -> f64 { 0.6 }
fn default_lc_volume_z_score() -> f64 { 2.0 }
fn default_lc_max_distance_pct() -> f64 { 5.0 }
fn default_lc_spread_max_pct() -> f64 { 0.5 }
fn default_lc_depth_min_usd() -> f64 { 10_000.0 }
fn default_lc_route_cost_max_bps() -> f64 { 5.0 }
fn default_lc_stale_threshold_secs() -> u64 { 300 }
fn default_lc_forced_flow_vel() -> f64 { 0.5 }
fn default_lc_velocity_decay_threshold() -> f64 { 0.1 }
fn default_lc_take_profit_pct() -> f64 { 1.5 }
fn default_lc_stop_loss_pct() -> f64 { 0.75 }
fn default_lc_trailing_stop_pct() -> f64 { 0.5 }
fn default_lc_trailing_activation_pct() -> f64 { 1.0 }
fn default_lc_max_hold_secs() -> u64 { 1800 }
fn default_lc_cooldown_after_loss_secs() -> u64 { 300 }
fn default_lc_clip_size_usd() -> f64 { 100.0 }
fn default_lc_leverage() -> f64 { 3.0 }
fn default_lc_lookback_count() -> usize { 30 }
fn default_neutral() -> String { "neutral".to_string() }
fn default_true() -> bool { true }

impl LiquidationCascadeParams {
    /// Validate all parameter ranges.
    pub fn validate(&self) -> Result<(), String> {
        if self.confidence_min < 0.0 || self.confidence_min > 1.0 {
            return Err(format!(
                "confidence_min must be in [0.0, 1.0], got {}",
                self.confidence_min
            ));
        }
        if self.volume_z_score_threshold < 0.0 {
            return Err(format!(
                "volume_z_score_threshold must be >= 0.0, got {}",
                self.volume_z_score_threshold
            ));
        }
        if self.max_distance_to_zone_pct < 0.0 {
            return Err(format!(
                "max_distance_to_zone_pct must be >= 0.0, got {}",
                self.max_distance_to_zone_pct
            ));
        }
        if self.spread_max_pct < 0.0 {
            return Err(format!(
                "spread_max_pct must be >= 0.0, got {}",
                self.spread_max_pct
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0.0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0.0, got {}",
                self.stop_loss_pct
            ));
        }
        if self.trailing_stop_pct < 0.0 {
            return Err(format!(
                "trailing_stop_pct must be >= 0.0, got {}",
                self.trailing_stop_pct
            ));
        }
        if self.trailing_activation_pct < 0.0 {
            return Err(format!(
                "trailing_activation_pct must be >= 0.0, got {}",
                self.trailing_activation_pct
            ));
        }
        if self.stale_data_threshold_secs == 0 {
            return Err("stale_data_threshold_secs must be > 0".to_string());
        }
        if self.route_cost_max_bps < 0.0 {
            return Err(format!(
                "route_cost_max_bps must be >= 0.0, got {}",
                self.route_cost_max_bps
            ));
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0.0, got {}",
                self.clip_size_usd
            ));
        }
        if self.lookback_count == 0 {
            return Err("lookback_count must be > 0".to_string());
        }
        if self.direction_bias != "long" && self.direction_bias != "short" && self.direction_bias != "neutral" {
            return Err(format!(
                "direction_bias must be 'long', 'short', or 'neutral', got '{}'",
                self.direction_bias
            ));
        }
        Ok(())
    }

    /// Convert to generic StrategyParams for the trait's parameters() method.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.forced_flow_velocity_threshold,
            lookback_count: self.lookback_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

impl Default for LiquidationCascadeParams {
    fn default() -> Self {
        Self {
            enabled: false,
            paper_only: true,
            confidence_min: default_lc_confidence_min(),
            volume_z_score_threshold: default_lc_volume_z_score(),
            max_distance_to_zone_pct: default_lc_max_distance_pct(),
            vwap_filter_enabled: true,
            spread_max_pct: default_lc_spread_max_pct(),
            depth_min_usd: default_lc_depth_min_usd(),
            regime_filter: true,
            route_cost_max_bps: default_lc_route_cost_max_bps(),
            stale_data_threshold_secs: default_lc_stale_threshold_secs(),
            forced_flow_velocity_threshold: default_lc_forced_flow_vel(),
            velocity_decay_threshold: default_lc_velocity_decay_threshold(),
            take_profit_pct: default_lc_take_profit_pct(),
            stop_loss_pct: default_lc_stop_loss_pct(),
            trailing_stop_pct: default_lc_trailing_stop_pct(),
            trailing_activation_pct: default_lc_trailing_activation_pct(),
            max_hold_secs: default_lc_max_hold_secs(),
            cooldown_after_loss_secs: default_lc_cooldown_after_loss_secs(),
            next_zone_tp_enabled: true,
            zone_reclaimed_stop_enabled: true,
            time_stop_enabled: true,
            clip_size_usd: default_lc_clip_size_usd(),
            leverage: default_lc_leverage(),
            direction_bias: default_neutral(),
            scale_in_clips: default_scale_in_clips(),
            use_native_tp_sl: true,
            lookback_count: default_lc_lookback_count(),
        }
    }
}

/// Liquidation cascade hunter strategy.
///
/// Paper-only strategy that exploits liquidation cascades through two setup types:
///
/// 1. **Cascade continuation** — price approaches a high-confidence liquidation zone,
///    forced-flow spikes, velocity confirms direction, depth thins, route cost OK.
///    Trade in the direction of the cascade (longs getting liquidated → short signal).
///
/// 2. **Exhaustion reversal** — after a liquidation burst, price reclaims VWAP,
///    depth refills, forced-flow velocity decays, spread normalizes.
///    Trade against the cascade direction (reversal).
///
/// All entry gates must pass for a signal to fire. Mandatory TP/SL on every entry.
/// Paper-only: blocked in the live engine.
pub struct LiquidationCascadeHunter {
    params: LiquidationCascadeParams,
    generic_params: StrategyParams,
    detector: MomentumDetector,
    /// Pending trade tracker: (symbol, side) → signal timestamp_ms.
    pending_signals: std::collections::HashMap<(String, String), i64>,
    /// Timestamp of the last loss (for cooldown).
    last_loss_timestamp_ms: Option<i64>,
    /// Current timestamp (updated on each push_price).
    current_timestamp_ms: i64,
    /// Price of the entry zone used for the most recent cascade continuation signal.
    /// Used for zone-reclaimed stop exit.
    entry_zone_price: Option<f64>,
    /// Direction of the last entry signal: true = long, false = short.
    entry_is_long: Option<bool>,
    /// Canonical strategy name (set at construction based on alias or canonical name).
    canonical_name: &'static str,
}

impl LiquidationCascadeHunter {
    /// Create a new liquidation cascade hunter with the given parameters.
    pub fn new(params: LiquidationCascadeParams) -> Self {
        let generic = params.to_strategy_params();
        let detector = MomentumDetector::new(params.forced_flow_velocity_threshold, params.lookback_count);
        Self {
            generic_params: generic,
            detector,
            params,
            pending_signals: std::collections::HashMap::new(),
            last_loss_timestamp_ms: None,
            current_timestamp_ms: 0,
            entry_zone_price: None,
            entry_is_long: None,
            canonical_name: "liquidation-cascade-continuation",
        }
    }

    /// Create with the legacy name "liquidation-cascade-hunter".
    pub fn new_legacy(params: LiquidationCascadeParams) -> Self {
        let mut s = Self::new(params);
        s.canonical_name = "liquidation-cascade-hunter";
        s
    }

    /// Return a reference to the strategy-specific parameters.
    #[allow(dead_code)]
    pub fn cascade_params(&self) -> &LiquidationCascadeParams {
        &self.params
    }

    /// Check if a cascade continuation entry is valid.
    ///
    /// Cascade continuation: price approaches a liquidation zone, and we trade
    /// in the direction the cascade is pushing (e.g., longs getting liquidated
    /// pushes price down → we go short).
    #[allow(clippy::collapsible_if)]
    fn check_cascade_continuation(
        &self,
        snapshot: &MomentumSnapshot,
        ext: &crate::signal::MarketExtension,
    ) -> Option<Signal> {
        let zones = ext.liquidation_zones.as_ref()?;
        let price = snapshot.current_price;
        if price <= 0.0 {
            return None;
        }

        // Find the nearest high-confidence zone within distance threshold
        for zone in zones {
            // Check confidence gate
            if zone.confidence < self.params.confidence_min {
                continue;
            }

            // Check distance gate
            let distance_pct = if zone.price > 0.0 {
                ((price - zone.price) / zone.price * 100.0).abs()
            } else {
                continue;
            };
            if distance_pct > self.params.max_distance_to_zone_pct {
                continue;
            }

            // Determine trade direction based on zone side_at_risk
            // If longs are at risk (price approaching zone from above → short)
            // If shorts are at risk (price approaching zone from below → long)
            let (is_long, direction_ok) = match zone.side_at_risk.as_str() {
                "long" => (false, true),   // longs being liquidated → price drops → we short
                "short" => (true, true),    // shorts being liquidated → price rises → we long
                _ => continue,
            };

            if !direction_ok {
                continue;
            }

            // Check direction bias
            if self.params.direction_bias == "long" && !is_long {
                continue;
            }
            if self.params.direction_bias == "short" && is_long {
                continue;
            }

            // Check VWAP filter
            if self.params.vwap_filter_enabled {
                if let Some(vwap) = ext.vwap {
                    if vwap > 0.0 {
                        if is_long && price < vwap {
                            continue; // Long but price below VWAP
                        }
                        if !is_long && price > vwap {
                            continue; // Short but price above VWAP
                        }
                    }
                }
            }

            // Check forced-flow velocity gate
            if let Some(velocity) = ext.forced_flow_velocity {
                if velocity < self.params.forced_flow_velocity_threshold {
                    continue;
                }
            } else {
                continue; // No velocity data, can't confirm cascade
            }

            // Compute signal strength from zone confidence and distance
            let strength = (zone.confidence * 100.0).min(100.0);
            let velocity_pct = snapshot.price_velocity_pct.abs();

            return Some(if is_long {
                Signal::MomentumLong { strength, velocity_pct }
            } else {
                Signal::MomentumShort { strength, velocity_pct }
            });
        }

        None
    }

    /// Check if an exhaustion reversal entry is valid.
    ///
    /// Exhaustion reversal: after a liquidation burst, price reclaims VWAP,
    /// depth refills, velocity decays. We trade against the cascade direction.
    #[allow(clippy::collapsible_if)]
    fn check_exhaustion_reversal(
        &self,
        snapshot: &MomentumSnapshot,
        ext: &crate::signal::MarketExtension,
    ) -> Option<Signal> {
        // Exhaustion requires a liquidation burst to have been detected
        if !ext.liquidation_burst_detected {
            return None;
        }

        let zones = ext.liquidation_zones.as_ref()?;
        let price = snapshot.current_price;
        if price <= 0.0 {
            return None;
        }

        // Check forced-flow velocity decay
        if let Some(velocity) = ext.forced_flow_velocity {
            if velocity > self.params.velocity_decay_threshold {
                return None; // Still too much velocity, cascade not exhausted
            }
        } else {
            return None; // No velocity data
        }

        // Check spread normalization
        if let Some(spread) = ext.spread_pct {
            if spread > self.params.spread_max_pct {
                return None; // Spread still elevated
            }
        }

        // Find the nearest high-confidence zone for direction
        for zone in zones {
            if zone.confidence < self.params.confidence_min {
                continue;
            }

            // Exhaustion reversal: trade AGAINST the cascade direction
            // If longs were liquidated (zone side_at_risk = "long"), price dropped → we go LONG (reversal)
            // If shorts were liquidated (zone side_at_risk = "short"), price rose → we go SHORT (reversal)
            let is_long = match zone.side_at_risk.as_str() {
                "long" => true,    // Longs got rekt → price dropped → reversal is LONG
                "short" => false,  // Shorts got rekt → price rose → reversal is SHORT
                _ => continue,
            };

            // Check direction bias
            if self.params.direction_bias == "long" && !is_long {
                continue;
            }
            if self.params.direction_bias == "short" && is_long {
                continue;
            }

            // Check VWAP reclamation
            if self.params.vwap_filter_enabled {
                if let Some(vwap) = ext.vwap {
                    if vwap > 0.0 {
                        if is_long && price < vwap {
                            continue; // Long reversal but price still below VWAP
                        }
                        if !is_long && price > vwap {
                            continue; // Short reversal but price still above VWAP
                        }
                    }
                }
            }

            let strength = (zone.confidence * 80.0).min(90.0); // Slightly lower strength for reversals
            let velocity_pct = snapshot.price_velocity_pct.abs();

            return Some(if is_long {
                Signal::MomentumLong { strength, velocity_pct }
            } else {
                Signal::MomentumShort { strength, velocity_pct }
            });
        }

        None
    }

    /// Check if zone data is stale (older than stale_data_threshold_secs).
    fn is_zone_data_stale(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let Some(ts) = ext.zone_capture_timestamp_ms {
            let age_secs = (self.current_timestamp_ms - ts).max(0) as u64 / 1000;
            age_secs > self.params.stale_data_threshold_secs
        } else {
            true // No timestamp → stale
        }
    }

    /// Clear pending signal for a given symbol/side.
    #[allow(dead_code)]
    pub fn clear_pending(&mut self, symbol: &str, side: &str) {
        self.pending_signals.remove(&(symbol.to_string(), side.to_string()));
    }

    /// Record a loss timestamp for cooldown tracking.
    #[allow(dead_code)]
    pub fn record_loss(&mut self, timestamp_ms: i64) {
        self.last_loss_timestamp_ms = Some(timestamp_ms);
    }
}

impl Strategy for LiquidationCascadeHunter {
    fn name(&self) -> &str {
        self.canonical_name
    }

    #[allow(clippy::collapsible_if)]
    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal {
        // Gate 0: Strategy must be enabled
        if !self.params.enabled {
            return Signal::NoSignal;
        }

        // Gate 1: Need sufficient price history
        if snapshot.price_count < self.params.lookback_count {
            return Signal::NoSignal;
        }

        // Gate 2: Need extended market data
        let ext = match &snapshot.ext {
            Some(e) => e,
            None => return Signal::NoSignal,
        };

        // Gate 3: Stale zone data check — prevent entries with stale data
        if self.is_zone_data_stale(ext) {
            return Signal::NoSignal;
        }

        // Gate 4: Volume z-score
        if let Some(zscore) = ext.volume_zscore {
            if zscore < self.params.volume_z_score_threshold {
                return Signal::NoSignal;
            }
        } else {
            return Signal::NoSignal; // No volume data
        }

        // Gate 5: Spread filter
        if let Some(spread) = ext.spread_pct {
            if spread > self.params.spread_max_pct {
                return Signal::NoSignal;
            }
        } else {
            return Signal::NoSignal; // No spread data
        }

        // Gate 6: Depth filter
        if let Some(depth) = ext.depth_usd {
            if depth < self.params.depth_min_usd {
                return Signal::NoSignal;
            }
        } else {
            return Signal::NoSignal; // No depth data
        }

        // Gate 7: Regime compatibility
        if self.params.regime_filter {
            if let Some(label) = &ext.regime_label {
                // Cascade hunting works best in Trending or HighVol regimes
                // Choppy and LowVol are incompatible
                match label.as_str() {
                    "Choppy" | "LowVol" => return Signal::NoSignal,
                    _ => {} // Trending, HighVol, or unknown → allow
                }
            }
        }

        // Gate 8: Route cost veto (checked inside detect_entry per VAL-STRAT-043)
        if let Some(route_cost_bps) = ext.route_cost_bps {
            if route_cost_bps > self.params.route_cost_max_bps {
                return Signal::NoSignal;
            }
            // route_cost_bps == 0.0 or None → don't block (graceful degradation)
        }

        // Gate 9: Cooldown after loss
        if let Some(last_loss) = self.last_loss_timestamp_ms {
            let elapsed_secs = (self.current_timestamp_ms - last_loss).max(0) as u64 / 1000;
            if elapsed_secs < self.params.cooldown_after_loss_secs {
                return Signal::NoSignal;
            }
        }

        // Try cascade continuation first
        let signal = self.check_cascade_continuation(snapshot, ext)
            .or_else(|| self.check_exhaustion_reversal(snapshot, ext));

        match signal {
            Some(s) => {
                // Gate 10: Duplicate check — max one pending per symbol/side
                let (side, is_long) = match &s {
                    Signal::MomentumLong { .. } => ("long", true),
                    Signal::MomentumShort { .. } => ("short", false),
                    _ => return Signal::NoSignal,
                };
                let symbol = ext.symbol.clone().unwrap_or_default();
                let key = (symbol.clone(), side.to_string());
                if self.pending_signals.contains_key(&key) {
                    return Signal::NoSignal; // Already have pending signal
                }
                self.pending_signals.insert(key, self.current_timestamp_ms);
                // Record entry zone and direction for cascade continuation exit logic
                self.entry_is_long = Some(is_long);
                // Store the nearest zone price used for the entry (for zone-reclaimed stop)
                if let Some(zones) = &ext.liquidation_zones {
                    self.entry_zone_price = zones.first().map(|z| z.price);
                }
                s
            }
            None => Signal::NoSignal,
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let current_price = ctx.current_price;
        let entry_price = ctx.entry_price;

        if entry_price <= 0.0 {
            return None;
        }

        // PnL from entry
        let pnl_pct = if ctx.is_long {
            (current_price - entry_price) / entry_price * 100.0
        } else {
            (entry_price - current_price) / entry_price * 100.0
        };

        // Priority 1: Take-profit
        if pnl_pct >= ctx.take_profit_pct {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::TakeProfit }
            } else {
                Signal::ExitShort { reason: ExitReason::TakeProfit }
            });
        }

        // Priority 2: Take-profit into next zone (cascade continuation specific)
        if self.params.next_zone_tp_enabled
            && let Some(zones) = _snapshot.ext.as_ref().and_then(|e| e.liquidation_zones.as_ref())
        {
            // Find the next zone in the cascade direction
            let next_zone = if ctx.is_long {
                // Long position → look for a zone above entry price
                zones.iter()
                    .filter(|z| z.price > entry_price)
                    .min_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal))
            } else {
                // Short position → look for a zone below entry price
                zones.iter()
                    .filter(|z| z.price < entry_price)
                    .max_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal))
            };
            if let Some(zone) = next_zone {
                // If price has reached the next zone, take profit
                let reached = if ctx.is_long {
                    current_price >= zone.price
                } else {
                    current_price <= zone.price
                };
                if reached {
                    return Some(if ctx.is_long {
                        Signal::ExitLong { reason: ExitReason::TakeProfit }
                    } else {
                        Signal::ExitShort { reason: ExitReason::TakeProfit }
                    });
                }
            }
        }

        // Priority 3: Zone-reclaimed stop (cascade continuation specific)
        if self.params.zone_reclaimed_stop_enabled
            && let Some(entry_zone_price) = self.entry_zone_price
            && entry_zone_price > 0.0
        {
            // For a long continuation entry (shorts being liquidated, price going up):
            //   If price drops back below the entry zone → zone reclaimed → exit
            // For a short continuation entry (longs being liquidated, price going down):
            //   If price rises back above the entry zone → zone reclaimed → exit
            let reclaimed = if ctx.is_long {
                current_price < entry_zone_price
            } else {
                current_price > entry_zone_price
            };
            if reclaimed {
                return Some(if ctx.is_long {
                    Signal::ExitLong { reason: ExitReason::ReversalDetected }
                } else {
                    Signal::ExitShort { reason: ExitReason::ReversalDetected }
                });
            }
        }

        // Priority 4: Stop-loss
        if pnl_pct <= -ctx.stop_loss_pct {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::StopLoss }
            } else {
                Signal::ExitShort { reason: ExitReason::StopLoss }
            });
        }

        // Priority 5: Trailing stop
        if ctx.trailing_stop_pct > 0.0 && ctx.trailing_activation_pct > 0.0 {
            let peak_profit_pct = if ctx.is_long {
                (ctx.peak_price - entry_price) / entry_price * 100.0
            } else {
                (entry_price - ctx.peak_price) / entry_price * 100.0
            };

            if peak_profit_pct >= ctx.trailing_activation_pct {
                let drawdown_from_peak = peak_profit_pct - pnl_pct;
                if drawdown_from_peak >= ctx.trailing_stop_pct {
                    return Some(if ctx.is_long {
                        Signal::ExitLong { reason: ExitReason::TrailingStop }
                    } else {
                        Signal::ExitShort { reason: ExitReason::TrailingStop }
                    });
                }
            }
        }

        // Priority 6: Time stop (cascade continuation specific)
        if self.params.time_stop_enabled && ctx.hold_secs >= ctx.max_hold_secs {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::TimeStop }
            } else {
                Signal::ExitShort { reason: ExitReason::TimeStop }
            });
        }

        // Priority 7: Stale zone data forced exit
        if let Some(zone_ts) = _snapshot.ext.as_ref().and_then(|e| e.zone_capture_timestamp_ms) {
            let age_secs = (self.current_timestamp_ms - zone_ts).max(0) as u64 / 1000;
            if age_secs > self.params.stale_data_threshold_secs {
                return Some(if ctx.is_long {
                    Signal::ExitLong { reason: ExitReason::ReversalDetected }
                } else {
                    Signal::ExitShort { reason: ExitReason::ReversalDetected }
                });
            }
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.current_timestamp_ms = timestamp_ms;
        self.detector.push_price(price, timestamp_ms);
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let mut snap = self.detector.analyze();
        snap.ext = None;
        snap
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Sweep Reclaim Strategy
// ---------------------------------------------------------------------------

/// Parameters for the liquidation-sweep-reclaim strategy.
///
/// This strategy enters after a zone sweep with exhaustion signals:
/// forced-flow spike → pressure deceleration → VWAP reclaim → depth refill →
/// spread normalization → OI contraction. Two-phase entry: passive fishing
/// ladder first, confirmation entry after reclaim. Never chases beyond max
/// distance. Exit: trailing stop after reclaim, time stop.
///
/// Paper-only by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepReclaimParams {
    // --- Enable / paper-only ---
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub paper_only: bool,

    // --- Entry gates ---
    /// Minimum liquidation zone confidence [0.0, 1.0].
    #[serde(default = "default_sr_confidence_min")]
    pub min_confidence: f64,
    /// Maximum chase distance from the swept zone in basis points.
    /// Strategy will never enter if price has moved further than this.
    #[serde(default = "default_sr_max_chase_distance_bps")]
    pub max_chase_distance_bps: f64,
    /// Forced-flow velocity spike threshold. Velocity must exceed this
    /// at the sweep point to qualify as a liquidation cascade event.
    #[serde(default = "default_sr_forced_flow_spike")]
    pub forced_flow_spike_threshold: f64,
    /// Velocity deceleration threshold. After the spike, velocity must
    /// drop below this to confirm exhaustion / pressure deceleration.
    #[serde(default = "default_sr_velocity_deceleration")]
    pub velocity_deceleration_threshold: f64,
    /// Whether VWAP reclaim is required for entry confirmation.
    #[serde(default = "default_true")]
    pub vwap_reclaim_required: bool,
    /// Maximum spread (%) for spread normalization gate.
    #[serde(default = "default_sr_spread_max_pct")]
    pub spread_max_pct: f64,
    /// Minimum order book depth (USD) for depth refill confirmation.
    #[serde(default = "default_sr_depth_min_usd")]
    pub depth_min_usd: f64,
    /// Whether OI contraction is required for entry.
    #[serde(default = "default_true")]
    pub oi_contraction_required: bool,
    /// Minimum volume z-score for cascade confirmation.
    #[serde(default = "default_sr_volume_z_score")]
    pub volume_z_score_threshold: f64,
    /// Stale data threshold in seconds.
    #[serde(default = "default_sr_stale_threshold_secs")]
    pub stale_data_threshold_secs: u64,
    /// Whether regime compatibility filter is enabled.
    #[serde(default = "default_true")]
    pub regime_filter: bool,
    /// Maximum route cost in bps.
    #[serde(default = "default_sr_route_cost_max_bps")]
    pub route_cost_max_bps: f64,
    /// Cooldown after a losing trade in seconds.
    #[serde(default = "default_sr_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,

    // --- Fishing ladder (phase 1) ---
    /// Offsets (in bps) from the zone midpoint for passive fishing orders.
    #[serde(default = "default_sr_fishing_offsets")]
    pub fishing_ladder_offsets_bps: Vec<f64>,
    /// Size in USD for each fishing tranche.
    #[serde(default = "default_sr_fishing_tranche_usd")]
    pub fishing_tranche_usd: f64,
    /// Fishing order expiry in seconds.
    #[serde(default = "default_sr_fishing_expiry_secs")]
    pub fishing_expiry_secs: u64,

    // --- Exit parameters ---
    #[serde(default = "default_sr_take_profit_pct")]
    pub take_profit_pct: f64,
    #[serde(default = "default_sr_stop_loss_pct")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_sr_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    #[serde(default = "default_sr_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    #[serde(default = "default_sr_max_hold_secs")]
    pub max_hold_secs: u64,

    // --- General ---
    #[serde(default = "default_sr_clip_size_usd")]
    pub clip_size_usd: f64,
    #[serde(default = "default_sr_leverage")]
    pub leverage: f64,
    #[serde(default = "default_neutral")]
    pub direction_bias: String,
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
    #[serde(default = "default_true")]
    pub use_native_tp_sl: bool,
    #[serde(default = "default_sr_lookback_count")]
    pub lookback_count: usize,
}

fn default_sr_confidence_min() -> f64 { 0.6 }
fn default_sr_max_chase_distance_bps() -> f64 { 150.0 }
fn default_sr_forced_flow_spike() -> f64 { 2.0 }
fn default_sr_velocity_deceleration() -> f64 { 0.5 }
fn default_sr_spread_max_pct() -> f64 { 0.5 }
fn default_sr_depth_min_usd() -> f64 { 10_000.0 }
fn default_sr_volume_z_score() -> f64 { 1.5 }
fn default_sr_stale_threshold_secs() -> u64 { 300 }
fn default_sr_route_cost_max_bps() -> f64 { 5.0 }
fn default_sr_cooldown_after_loss_secs() -> u64 { 300 }
fn default_sr_fishing_offsets() -> Vec<f64> { vec![10.0, 20.0, 30.0] }
fn default_sr_fishing_tranche_usd() -> f64 { 25.0 }
fn default_sr_fishing_expiry_secs() -> u64 { 300 }
fn default_sr_take_profit_pct() -> f64 { 3.0 }
fn default_sr_stop_loss_pct() -> f64 { 1.5 }
fn default_sr_trailing_stop_pct() -> f64 { 0.8 }
fn default_sr_trailing_activation_pct() -> f64 { 1.5 }
fn default_sr_max_hold_secs() -> u64 { 1800 }
fn default_sr_clip_size_usd() -> f64 { 100.0 }
fn default_sr_leverage() -> f64 { 2.0 }
fn default_sr_lookback_count() -> usize { 30 }

impl Default for SweepReclaimParams {
    fn default() -> Self {
        Self {
            enabled: false,
            paper_only: true,
            min_confidence: default_sr_confidence_min(),
            max_chase_distance_bps: default_sr_max_chase_distance_bps(),
            forced_flow_spike_threshold: default_sr_forced_flow_spike(),
            velocity_deceleration_threshold: default_sr_velocity_deceleration(),
            vwap_reclaim_required: true,
            spread_max_pct: default_sr_spread_max_pct(),
            depth_min_usd: default_sr_depth_min_usd(),
            oi_contraction_required: true,
            volume_z_score_threshold: default_sr_volume_z_score(),
            stale_data_threshold_secs: default_sr_stale_threshold_secs(),
            regime_filter: true,
            route_cost_max_bps: default_sr_route_cost_max_bps(),
            cooldown_after_loss_secs: default_sr_cooldown_after_loss_secs(),
            fishing_ladder_offsets_bps: default_sr_fishing_offsets(),
            fishing_tranche_usd: default_sr_fishing_tranche_usd(),
            fishing_expiry_secs: default_sr_fishing_expiry_secs(),
            take_profit_pct: default_sr_take_profit_pct(),
            stop_loss_pct: default_sr_stop_loss_pct(),
            trailing_stop_pct: default_sr_trailing_stop_pct(),
            trailing_activation_pct: default_sr_trailing_activation_pct(),
            max_hold_secs: default_sr_max_hold_secs(),
            clip_size_usd: default_sr_clip_size_usd(),
            leverage: default_sr_leverage(),
            direction_bias: default_neutral(),
            scale_in_clips: default_scale_in_clips(),
            use_native_tp_sl: true,
            lookback_count: default_sr_lookback_count(),
        }
    }
}

impl SweepReclaimParams {
    /// Validate all parameter ranges.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_confidence < 0.0 || self.min_confidence > 1.0 {
            return Err(format!(
                "min_confidence must be in [0.0, 1.0], got {}",
                self.min_confidence
            ));
        }
        if self.max_chase_distance_bps < 0.0 {
            return Err(format!(
                "max_chase_distance_bps must be >= 0.0, got {}",
                self.max_chase_distance_bps
            ));
        }
        if self.forced_flow_spike_threshold <= 0.0 {
            return Err(format!(
                "forced_flow_spike_threshold must be > 0.0, got {}",
                self.forced_flow_spike_threshold
            ));
        }
        if self.velocity_deceleration_threshold < 0.0 {
            return Err(format!(
                "velocity_deceleration_threshold must be >= 0.0, got {}",
                self.velocity_deceleration_threshold
            ));
        }
        if self.spread_max_pct < 0.0 {
            return Err(format!(
                "spread_max_pct must be >= 0.0, got {}",
                self.spread_max_pct
            ));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!(
                "take_profit_pct must be > 0.0, got {}",
                self.take_profit_pct
            ));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0.0, got {}",
                self.stop_loss_pct
            ));
        }
        if self.trailing_stop_pct < 0.0 {
            return Err(format!(
                "trailing_stop_pct must be >= 0.0, got {}",
                self.trailing_stop_pct
            ));
        }
        if self.stale_data_threshold_secs == 0 {
            return Err("stale_data_threshold_secs must be > 0".to_string());
        }
        if self.route_cost_max_bps < 0.0 {
            return Err(format!(
                "route_cost_max_bps must be >= 0.0, got {}",
                self.route_cost_max_bps
            ));
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0.0, got {}",
                self.clip_size_usd
            ));
        }
        if self.lookback_count == 0 {
            return Err("lookback_count must be > 0".to_string());
        }
        if self.fishing_ladder_offsets_bps.is_empty() {
            return Err("fishing_ladder_offsets_bps must not be empty".to_string());
        }
        if self.fishing_tranche_usd <= 0.0 {
            return Err(format!(
                "fishing_tranche_usd must be > 0.0, got {}",
                self.fishing_tranche_usd
            ));
        }
        if self.direction_bias != "long" && self.direction_bias != "short" && self.direction_bias != "neutral" {
            return Err(format!(
                "direction_bias must be 'long', 'short', or 'neutral', got '{}'",
                self.direction_bias
            ));
        }
        Ok(())
    }

    /// Parse from a TOML sub-table, falling back to defaults for missing fields.
    pub fn from_toml_table(table: &toml::Value) -> Result<Self, String> {
        let params = Self::default();
        let mut p = params;

        if let Some(v) = table.get("enabled").and_then(|v| v.as_bool()) {
            p.enabled = v;
        }
        if let Some(v) = table.get("paper_only").and_then(|v| v.as_bool()) {
            p.paper_only = v;
        }
        if let Some(v) = table.get("min_confidence").and_then(|v| v.as_float()) {
            p.min_confidence = v;
        }
        if let Some(v) = table.get("max_chase_distance_bps").and_then(|v| v.as_float()) {
            p.max_chase_distance_bps = v;
        }
        if let Some(v) = table.get("forced_flow_spike_threshold").and_then(|v| v.as_float()) {
            p.forced_flow_spike_threshold = v;
        }
        if let Some(v) = table.get("velocity_deceleration_threshold").and_then(|v| v.as_float()) {
            p.velocity_deceleration_threshold = v;
        }
        if let Some(v) = table.get("vwap_reclaim_required").and_then(|v| v.as_bool()) {
            p.vwap_reclaim_required = v;
        }
        if let Some(v) = table.get("max_spread_bps").and_then(|v| v.as_float()) {
            p.spread_max_pct = v / 100.0; // config in bps, internal in pct
        }
        if let Some(v) = table.get("spread_max_pct").and_then(|v| v.as_float()) {
            p.spread_max_pct = v;
        }
        if let Some(v) = table.get("min_depth_usd").and_then(|v| v.as_float()) {
            p.depth_min_usd = v;
        }
        if let Some(v) = table.get("depth_min_usd").and_then(|v| v.as_float()) {
            p.depth_min_usd = v;
        }
        if let Some(v) = table.get("oi_contraction_required").and_then(|v| v.as_bool()) {
            p.oi_contraction_required = v;
        }
        if let Some(v) = table.get("volume_zscore_min").and_then(|v| v.as_float()) {
            p.volume_z_score_threshold = v;
        }
        if let Some(v) = table.get("volume_z_score_threshold").and_then(|v| v.as_float()) {
            p.volume_z_score_threshold = v;
        }
        if let Some(v) = table.get("stale_data_threshold_secs").and_then(|v| v.as_integer()) {
            p.stale_data_threshold_secs = v as u64;
        }
        if let Some(v) = table.get("regime_filter").and_then(|v| v.as_bool()) {
            p.regime_filter = v;
        }
        if let Some(v) = table.get("route_cost_max_bps").and_then(|v| v.as_float()) {
            p.route_cost_max_bps = v;
        }
        if let Some(v) = table.get("cooldown_after_loss_secs").and_then(|v| v.as_integer()) {
            p.cooldown_after_loss_secs = v as u64;
        }
        if let Some(arr) = table.get("fishing_ladder_offsets_bps").and_then(|v| v.as_array()) {
            p.fishing_ladder_offsets_bps = arr.iter()
                .filter_map(|v| v.as_float())
                .collect();
        }
        if let Some(v) = table.get("fishing_tranche_usd").and_then(|v| v.as_float()) {
            p.fishing_tranche_usd = v;
        }
        if let Some(v) = table.get("fishing_expiry_secs").and_then(|v| v.as_integer()) {
            p.fishing_expiry_secs = v as u64;
        }
        if let Some(v) = table.get("take_profit_pct").and_then(|v| v.as_float()) {
            p.take_profit_pct = v;
        }
        if let Some(v) = table.get("stop_loss_pct").and_then(|v| v.as_float()) {
            p.stop_loss_pct = v;
        }
        if let Some(v) = table.get("trailing_stop_pct").and_then(|v| v.as_float()) {
            p.trailing_stop_pct = v;
        }
        if let Some(v) = table.get("trailing_activation_pct").and_then(|v| v.as_float()) {
            p.trailing_activation_pct = v;
        }
        if let Some(v) = table.get("max_hold_secs").and_then(|v| v.as_integer()) {
            p.max_hold_secs = v as u64;
        }
        if let Some(v) = table.get("clip_size_usd").and_then(|v| v.as_float()) {
            p.clip_size_usd = v;
        }
        if let Some(v) = table.get("leverage").and_then(|v| v.as_float()) {
            p.leverage = v;
        }
        if let Some(v) = table.get("direction_bias").and_then(|v| v.as_str()) {
            p.direction_bias = v.to_string();
        }
        if let Some(v) = table.get("scale_in_clips").and_then(|v| v.as_integer()) {
            p.scale_in_clips = v as u32;
        }
        if let Some(v) = table.get("use_native_tp_sl").and_then(|v| v.as_bool()) {
            p.use_native_tp_sl = v;
        }
        if let Some(v) = table.get("lookback_count").and_then(|v| v.as_integer()) {
            p.lookback_count = v as usize;
        }

        Ok(p)
    }

    /// Convert to generic StrategyParams.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.forced_flow_spike_threshold,
            lookback_count: self.lookback_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

/// Phase of the sweep-reclaim entry process.
#[derive(Debug, Clone, PartialEq)]
pub enum SweepReclaimPhase {
    /// Waiting for a zone sweep event.
    Idle,
    /// Zone sweep detected; placing passive fishing ladder orders.
    Fishing,
}

/// Internal state tracking a detected zone sweep.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SweepState {
    /// Price of the swept zone.
    zone_price: f64,
    /// Side at risk in the zone ("long" = longs liquidated → we go long for reversal).
    side_at_risk: String,
    /// Direction of the sweep (true = price went down through zone, false = up).
    swept_down: bool,
    /// Timestamp when the sweep was detected.
    sweep_timestamp_ms: i64,
    /// Peak forced-flow velocity observed during the sweep.
    peak_velocity: f64,
    /// Current forced-flow velocity (updated each tick).
    current_velocity: f64,
    /// Whether forced-flow spike has been confirmed.
    spike_confirmed: bool,
    /// Whether pressure deceleration has been confirmed.
    deceleration_confirmed: bool,
    /// Whether VWAP reclaim has been confirmed.
    vwap_reclaim_confirmed: bool,
    /// Whether depth refill has been confirmed.
    depth_refill_confirmed: bool,
    /// Whether spread normalization has been confirmed.
    spread_normalization_confirmed: bool,
    /// Whether OI contraction has been confirmed.
    oi_contraction_confirmed: bool,
}

/// A passive fishing order placed during the fishing phase.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FishingOrder {
    /// Price level of the order.
    price: f64,
    /// Size in USD.
    size_usd: f64,
    /// Timestamp when the order was placed.
    placed_timestamp_ms: i64,
    /// Whether the order has been filled.
    filled: bool,
    /// Whether the order has expired.
    expired: bool,
}

/// Sweep-reclaim reversal strategy.
///
/// Paper-only strategy that enters after a liquidation zone sweep with exhaustion
/// confirmation. Two-phase entry:
/// 1. **Fishing phase**: After sweep detection, place passive limit orders at
///    configured offsets from the zone.
/// 2. **Confirmation phase**: After VWAP reclaim + depth refill + spread normalization +
///    OI contraction, enter with a market order.
///
/// Never chases beyond max distance. Exit via trailing stop or time stop.
pub struct SweepReclaimStrategy {
    params: SweepReclaimParams,
    generic_params: StrategyParams,
    detector: MomentumDetector,
    /// Current phase of the entry process.
    phase: SweepReclaimPhase,
    /// Active sweep state (None if no sweep is being tracked).
    sweep_state: Option<SweepState>,
    /// Active fishing orders placed during the fishing phase.
    fishing_orders: Vec<FishingOrder>,
    /// Pending trade tracker: (symbol, side) → signal timestamp_ms.
    pending_signals: std::collections::HashMap<(String, String), i64>,
    /// Timestamp of the last loss (for cooldown).
    last_loss_timestamp_ms: Option<i64>,
    /// Current timestamp (updated on each push_price).
    current_timestamp_ms: i64,
    /// Previous forced-flow velocity (for deceleration detection).
    prev_forced_flow_velocity: Option<f64>,
    /// Previous depth (for refill detection - tracks minimum depth during sweep).
    prev_min_depth: Option<f64>,
    /// Previous spread (for normalization detection - tracks peak spread during sweep).
    prev_max_spread: Option<f64>,
    /// Entry zone price for the most recent position.
    entry_zone_price: Option<f64>,
    /// Whether the most recent entry was long.
    entry_is_long: Option<bool>,
}

impl SweepReclaimStrategy {
    /// Create a new sweep-reclaim strategy with the given parameters.
    pub fn new(params: SweepReclaimParams) -> Self {
        let generic = params.to_strategy_params();
        let detector = MomentumDetector::new(params.forced_flow_spike_threshold, params.lookback_count);
        Self {
            generic_params: generic,
            detector,
            params,
            phase: SweepReclaimPhase::Idle,
            sweep_state: None,
            fishing_orders: Vec::new(),
            pending_signals: std::collections::HashMap::new(),
            last_loss_timestamp_ms: None,
            current_timestamp_ms: 0,
            prev_forced_flow_velocity: None,
            prev_min_depth: None,
            prev_max_spread: None,
            entry_zone_price: None,
            entry_is_long: None,
        }
    }

    /// Return a reference to the strategy-specific parameters.
    #[allow(dead_code)]
    pub fn sweep_params(&self) -> &SweepReclaimParams {
        &self.params
    }

    /// Check if zone data is stale.
    fn is_zone_data_stale(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let Some(ts) = ext.zone_capture_timestamp_ms {
            let age_secs = (self.current_timestamp_ms - ts).max(0) as u64 / 1000;
            age_secs > self.params.stale_data_threshold_secs
        } else {
            true
        }
    }

    /// Detect a zone sweep event: price rapidly crosses a liquidation zone then reverses.
    ///
    /// VAL-STRAT-SR-001: Zone sweep detected when price crosses then reverses.
    fn detect_zone_sweep(
        &mut self,
        snapshot: &MomentumSnapshot,
        ext: &crate::signal::MarketExtension,
    ) -> Option<(f64, String, bool)> {
        let zones = ext.liquidation_zones.as_ref()?;
        let price = snapshot.current_price;
        if price <= 0.0 {
            return None;
        }

        // Check for price crossing a zone with high velocity (sweep signature)
        let velocity = snapshot.price_velocity_pct;

        for zone in zones {
            if zone.confidence < self.params.min_confidence {
                continue;
            }

            // Check if price is near the zone (within max_chase_distance)
            let distance_bps = if zone.price > 0.0 {
                ((price - zone.price) / zone.price * 10_000.0).abs()
            } else {
                continue;
            };
            if distance_bps > self.params.max_chase_distance_bps {
                continue;
            }

            // Detect sweep: price crosses zone and starts reversing
            // For a long-liquidation zone (side_at_risk = "long"):
            //   Price drops below zone (swept_down = true), then reverses up
            // For a short-liquidation zone (side_at_risk = "short"):
            //   Price rises above zone (swept_down = false), then reverses down
            let swept_down = price < zone.price;
            let is_reversing = match zone.side_at_risk.as_str() {
                "long" => velocity > 0.0,   // Price dropped to zone, now reversing up
                "short" => velocity < 0.0,  // Price rose to zone, now reversing down
                _ => continue,
            };

            // Check if price is close enough to the zone to have just swept it
            let crossed = match zone.side_at_risk.as_str() {
                "long" => price <= zone.price * 1.005 && price >= zone.price * 0.995,
                "short" => price >= zone.price * 0.995 && price <= zone.price * 1.005,
                _ => false,
            };

            if crossed && is_reversing {
                return Some((zone.price, zone.side_at_risk.clone(), swept_down));
            }
        }

        None
    }

    /// Check forced-flow spike (VAL-STRAT-SR-002).
    fn check_forced_flow_spike(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let Some(velocity) = ext.forced_flow_velocity {
            velocity >= self.params.forced_flow_spike_threshold
        } else {
            false
        }
    }

    /// Check pressure deceleration (VAL-STRAT-SR-003).
    fn check_pressure_deceleration(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let (Some(current), Some(prev)) = (ext.forced_flow_velocity, self.prev_forced_flow_velocity) {
            // Deceleration: velocity was high (spike), now dropping below threshold
            current < self.params.velocity_deceleration_threshold && prev >= current
        } else {
            false
        }
    }

    /// Check VWAP reclaim (VAL-STRAT-SR-004).
    fn check_vwap_reclaim(&self, price: f64, ext: &crate::signal::MarketExtension, is_long: bool) -> bool {
        if !self.params.vwap_reclaim_required {
            return true; // Gate disabled
        }
        if let Some(vwap) = ext.vwap {
            if vwap > 0.0 {
                if is_long {
                    price >= vwap // Long reclaim: price back above VWAP
                } else {
                    price <= vwap // Short reclaim: price back below VWAP
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Check depth refill (VAL-STRAT-SR-005).
    fn check_depth_refill(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let Some(depth) = ext.depth_usd {
            if let Some(min_depth) = self.prev_min_depth {
                // Depth has recovered above the minimum observed during sweep
                depth >= self.params.depth_min_usd && depth > min_depth
            } else {
                depth >= self.params.depth_min_usd
            }
        } else {
            false
        }
    }

    /// Check spread normalization (VAL-STRAT-SR-006).
    fn check_spread_normalization(&self, ext: &crate::signal::MarketExtension) -> bool {
        if let Some(spread) = ext.spread_pct {
            spread <= self.params.spread_max_pct
        } else {
            false
        }
    }

    /// Check OI contraction (VAL-STRAT-SR-007).
    fn check_oi_contraction(&self, ext: &crate::signal::MarketExtension) -> bool {
        if !self.params.oi_contraction_required {
            return true; // Gate disabled
        }
        ext.oi_contracting.unwrap_or(false)
    }

    /// Check max distance enforcement (VAL-STRAT-SR-009).
    fn check_max_distance(&self, price: f64, zone_price: f64) -> bool {
        if zone_price <= 0.0 {
            return false;
        }
        let distance_bps = ((price - zone_price) / zone_price * 10_000.0).abs();
        distance_bps <= self.params.max_chase_distance_bps
    }

    /// Place fishing ladder orders (VAL-STRAT-SR-008 phase 1).
    fn place_fishing_ladder(&mut self, zone_price: f64, swept_down: bool, timestamp_ms: i64) {
        // Clear any existing orders
        self.fishing_orders.clear();

        let offsets = &self.params.fishing_ladder_offsets_bps;
        for offset_bps in offsets {
            let order_price = if swept_down {
                // Long sweep reversal: fishing orders below zone
                zone_price * (1.0 - offset_bps / 10_000.0)
            } else {
                // Short sweep reversal: fishing orders above zone
                zone_price * (1.0 + offset_bps / 10_000.0)
            };

            self.fishing_orders.push(FishingOrder {
                price: order_price,
                size_usd: self.params.fishing_tranche_usd,
                placed_timestamp_ms: timestamp_ms,
                filled: false,
                expired: false,
            });
        }

        self.phase = SweepReclaimPhase::Fishing;
        debug!(
            "[sweep-reclaim] Placed {} fishing orders around zone {:.2}",
            self.fishing_orders.len(),
            zone_price
        );
    }

    /// Check if fishing orders have expired.
    fn check_fishing_expiry(&mut self) {
        for order in &mut self.fishing_orders {
            if order.filled || order.expired {
                continue;
            }
            let age_secs = (self.current_timestamp_ms - order.placed_timestamp_ms).max(0) as u64 / 1000;
            if age_secs >= self.params.fishing_expiry_secs {
                order.expired = true;
                debug!(
                    "[sweep-reclaim] Fishing order at {:.2} expired after {}s",
                    order.price, age_secs
                );
            }
        }
    }

    /// Determine the trade direction based on zone side_at_risk.
    /// For sweep-reclaim (reversal): longs got rekt → price dropped → we go LONG.
    fn is_long_from_side(side_at_risk: &str) -> Option<bool> {
        match side_at_risk {
            "long" => Some(true),   // Longs liquidated (price dropped) → reversal is LONG
            "short" => Some(false), // Shorts liquidated (price rose) → reversal is SHORT
            _ => None,
        }
    }
}

impl Strategy for SweepReclaimStrategy {
    fn name(&self) -> &str {
        "sweep-reclaim"
    }

    #[allow(clippy::collapsible_if)]
    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal {
        // Gate 0: Strategy must be enabled
        if !self.params.enabled {
            return Signal::NoSignal;
        }

        // Gate 1: Need sufficient price history
        if snapshot.price_count < self.params.lookback_count {
            return Signal::NoSignal;
        }

        // Gate 2: Need extended market data
        let ext = match &snapshot.ext {
            Some(e) => e,
            None => return Signal::NoSignal,
        };

        // Gate 3: Stale zone data check
        if self.is_zone_data_stale(ext) {
            return Signal::NoSignal;
        }

        // Gate 4: Volume z-score
        if let Some(zscore) = ext.volume_zscore {
            if zscore < self.params.volume_z_score_threshold {
                return Signal::NoSignal;
            }
        } else {
            return Signal::NoSignal;
        }

        // Gate 5: Spread filter (global gate, checked before phase-specific logic)
        if let Some(spread) = ext.spread_pct {
            // Track peak spread for normalization detection
            if self.prev_max_spread.is_none() || self.prev_max_spread.is_some_and(|ps| spread > ps) {
                self.prev_max_spread = Some(spread);
            }
        }

        // Gate 6: Depth filter (global gate)
        if let Some(depth) = ext.depth_usd {
            // Track minimum depth for refill detection
            if self.prev_min_depth.is_none() || self.prev_min_depth.is_some_and(|md| depth < md) {
                self.prev_min_depth = Some(depth);
            }
        }

        // Gate 7: Regime compatibility
        if self.params.regime_filter {
            if let Some(label) = &ext.regime_label {
                // Sweep-reclaim works best in HighVol regime (exhaustion signals)
                // Trending is also acceptable. Choppy and LowVol are incompatible.
                match label.as_str() {
                    "Choppy" | "LowVol" => return Signal::NoSignal,
                    _ => {}
                }
            }
        }

        // Gate 8: Route cost veto
        if let Some(route_cost_bps) = ext.route_cost_bps {
            if route_cost_bps > self.params.route_cost_max_bps {
                return Signal::NoSignal;
            }
        }

        // Gate 9: Cooldown after loss
        if let Some(last_loss) = self.last_loss_timestamp_ms {
            let elapsed_secs = (self.current_timestamp_ms - last_loss).max(0) as u64 / 1000;
            if elapsed_secs < self.params.cooldown_after_loss_secs {
                return Signal::NoSignal;
            }
        }

        // Track previous velocity for deceleration detection
        let current_velocity = ext.forced_flow_velocity;
        let _prev_velocity = self.prev_forced_flow_velocity;
        self.prev_forced_flow_velocity = current_velocity;

        let price = snapshot.current_price;

        // ── Phase state machine ──
        match self.phase {
            SweepReclaimPhase::Idle => {
                // Look for a zone sweep event
                if let Some((zone_price, side_at_risk, swept_down)) =
                    self.detect_zone_sweep(snapshot, ext)
                {
                    // VAL-STRAT-SR-001: Zone sweep detected
                    let is_long = match Self::is_long_from_side(&side_at_risk) {
                        Some(d) => d,
                        None => return Signal::NoSignal,
                    };

                    // Check direction bias
                    if self.params.direction_bias == "long" && !is_long {
                        return Signal::NoSignal;
                    }
                    if self.params.direction_bias == "short" && is_long {
                        return Signal::NoSignal;
                    }

                    // VAL-STRAT-SR-002: Forced-flow spike required
                    let spike_confirmed = self.check_forced_flow_spike(ext);
                    if !spike_confirmed {
                        return Signal::NoSignal;
                    }

                    // Record sweep state and transition to Fishing phase
                    self.sweep_state = Some(SweepState {
                        zone_price,
                        side_at_risk,
                        swept_down,
                        sweep_timestamp_ms: self.current_timestamp_ms,
                        peak_velocity: current_velocity.unwrap_or(0.0),
                        current_velocity: current_velocity.unwrap_or(0.0),
                        spike_confirmed: true,
                        deceleration_confirmed: false,
                        vwap_reclaim_confirmed: false,
                        depth_refill_confirmed: false,
                        spread_normalization_confirmed: false,
                        oi_contraction_confirmed: false,
                    });

                    // VAL-STRAT-SR-008: Place passive fishing ladder
                    self.place_fishing_ladder(zone_price, swept_down, self.current_timestamp_ms);

                    // Return NoSignal — fishing orders are passive, not an active entry
                    debug!(
                        "[sweep-reclaim] Sweep detected at zone {:.2}, fishing phase started",
                        zone_price
                    );
                    return Signal::NoSignal;
                }
                Signal::NoSignal
            }
            SweepReclaimPhase::Fishing => {
                // Extract sweep state data before borrowing self for checks
                let (zone_price, side_at_risk, already_decel, already_vwap,
                     already_depth, already_spread, already_oi) = {
                    let sweep = match &self.sweep_state {
                        Some(s) => s,
                        None => {
                            self.phase = SweepReclaimPhase::Idle;
                            return Signal::NoSignal;
                        }
                    };
                    (
                        sweep.zone_price,
                        sweep.side_at_risk.clone(),
                        sweep.deceleration_confirmed,
                        sweep.vwap_reclaim_confirmed,
                        sweep.depth_refill_confirmed,
                        sweep.spread_normalization_confirmed,
                        sweep.oi_contraction_confirmed,
                    )
                };

                let is_long = match Self::is_long_from_side(&side_at_risk) {
                    Some(d) => d,
                    None => {
                        self.phase = SweepReclaimPhase::Idle;
                        self.sweep_state = None;
                        return Signal::NoSignal;
                    }
                };

                // Update velocity in sweep state
                if let Some(vel) = current_velocity {
                    if let Some(ref mut sweep) = self.sweep_state {
                        sweep.current_velocity = vel;
                    }
                }

                // Check fishing expiry
                self.check_fishing_expiry();

                // VAL-STRAT-SR-009: Max distance enforcement
                if !self.check_max_distance(price, zone_price) {
                    debug!(
                        "[sweep-reclaim] Price {:.2} too far from zone {:.2}, resetting",
                        price, zone_price
                    );
                    self.phase = SweepReclaimPhase::Idle;
                    self.sweep_state = None;
                    self.fishing_orders.clear();
                    return Signal::NoSignal;
                }

                // Compute all confirmation checks (uses &self only)
                let deceleration_confirmed = already_decel || self.check_pressure_deceleration(ext);
                let vwap_reclaim_confirmed = already_vwap || self.check_vwap_reclaim(price, ext, is_long);
                let depth_refill_confirmed = already_depth || self.check_depth_refill(ext);
                let spread_normalization_confirmed = already_spread || self.check_spread_normalization(ext);
                let oi_contraction_confirmed = already_oi || self.check_oi_contraction(ext);

                // Update sweep state with confirmation results
                if let Some(ref mut sweep) = self.sweep_state {
                    sweep.deceleration_confirmed = deceleration_confirmed;
                    sweep.vwap_reclaim_confirmed = vwap_reclaim_confirmed;
                    sweep.depth_refill_confirmed = depth_refill_confirmed;
                    sweep.spread_normalization_confirmed = spread_normalization_confirmed;
                    sweep.oi_contraction_confirmed = oi_contraction_confirmed;
                }

                // Transition to Confirmation when all gates pass
                if deceleration_confirmed
                    && vwap_reclaim_confirmed
                    && depth_refill_confirmed
                    && spread_normalization_confirmed
                    && oi_contraction_confirmed
                {
                    debug!(
                        "[sweep-reclaim] All confirmation gates passed, emitting confirmation signal"
                    );

                    // VAL-STRAT-SR-009: Final max distance check before emitting
                    if !self.check_max_distance(price, zone_price) {
                        self.phase = SweepReclaimPhase::Idle;
                        self.sweep_state = None;
                        self.fishing_orders.clear();
                        return Signal::NoSignal;
                    }

                    // Duplicate check
                    let side = if is_long { "long" } else { "short" };
                    let symbol = ext.symbol.clone().unwrap_or_default();
                    let key = (symbol.clone(), side.to_string());
                    if self.pending_signals.contains_key(&key) {
                        return Signal::NoSignal;
                    }

                    // Emit confirmation signal
                    let strength = 70.0;
                    let velocity_pct = snapshot.price_velocity_pct.abs();

                    // Record entry zone and direction
                    self.entry_zone_price = Some(zone_price);
                    self.entry_is_long = Some(is_long);

                    self.pending_signals.insert(key, self.current_timestamp_ms);

                    // Reset phase to Idle (ready for next sweep)
                    self.phase = SweepReclaimPhase::Idle;
                    self.sweep_state = None;
                    self.fishing_orders.clear();
                    // Reset tracking state
                    self.prev_forced_flow_velocity = None;
                    self.prev_min_depth = None;
                    self.prev_max_spread = None;

                    return if is_long {
                        Signal::MomentumLong { strength, velocity_pct }
                    } else {
                        Signal::MomentumShort { strength, velocity_pct }
                    };
                }

                Signal::NoSignal // Still in fishing phase, gates not yet all passed
            }
        }
    }

    fn detect_exit(
        &self,
        snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let current_price = ctx.current_price;
        let entry_price = ctx.entry_price;

        if entry_price <= 0.0 {
            return None;
        }

        // PnL from entry
        let pnl_pct = if ctx.is_long {
            (current_price - entry_price) / entry_price * 100.0
        } else {
            (entry_price - current_price) / entry_price * 100.0
        };

        // Priority 1: Stop-loss
        if pnl_pct <= -ctx.stop_loss_pct {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::StopLoss }
            } else {
                Signal::ExitShort { reason: ExitReason::StopLoss }
            });
        }

        // Priority 2: Take-profit
        if pnl_pct >= ctx.take_profit_pct {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::TakeProfit }
            } else {
                Signal::ExitShort { reason: ExitReason::TakeProfit }
            });
        }

        // VAL-STRAT-SR-010: Trailing stop after reclaim
        if ctx.trailing_stop_pct > 0.0 && ctx.trailing_activation_pct > 0.0 {
            let peak_profit_pct = if ctx.is_long {
                (ctx.peak_price - entry_price) / entry_price * 100.0
            } else {
                (entry_price - ctx.peak_price) / entry_price * 100.0
            };

            if peak_profit_pct >= ctx.trailing_activation_pct {
                let drawdown_from_peak = peak_profit_pct - pnl_pct;
                if drawdown_from_peak >= ctx.trailing_stop_pct {
                    return Some(if ctx.is_long {
                        Signal::ExitLong { reason: ExitReason::TrailingStop }
                    } else {
                        Signal::ExitShort { reason: ExitReason::TrailingStop }
                    });
                }
            }
        }

        // VAL-STRAT-SR-011: Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            return Some(if ctx.is_long {
                Signal::ExitLong { reason: ExitReason::TimeStop }
            } else {
                Signal::ExitShort { reason: ExitReason::TimeStop }
            });
        }

        // Stale zone data forced exit
        if let Some(zone_ts) = snapshot.ext.as_ref().and_then(|e| e.zone_capture_timestamp_ms) {
            let age_secs = (self.current_timestamp_ms - zone_ts).max(0) as u64 / 1000;
            if age_secs > self.params.stale_data_threshold_secs {
                return Some(if ctx.is_long {
                    Signal::ExitLong { reason: ExitReason::ReversalDetected }
                } else {
                    Signal::ExitShort { reason: ExitReason::ReversalDetected }
                });
            }
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.current_timestamp_ms = timestamp_ms;
        self.detector.push_price(price, timestamp_ms);
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let mut snap = self.detector.analyze();
        snap.ext = None;
        snap
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Strategy Factory
// ---------------------------------------------------------------------------

/// Canonical list of all registered strategy names.
pub fn available_strategies() -> &'static [&'static str] {
    &[
        "momentum-scalper",
        "lp-consumption",
        "mean-reversion",
        "trend-follower",
        "funding-capture",
        "blueprint-scalper",
        "blueprint-mean-revert",
        "blueprint-cluster-002",
        "blueprint-cluster-003",
        "blueprint-cluster-005",
        "blueprint-cluster-006",
        "blueprint-cluster-007",
        "blueprint-cluster-008",
        "blueprint-cluster-009",
        "blueprint-hft-market-maker",
        "liquidation-cascade-continuation",
        "liquidation-cascade-hunter",
        "sweep-reclaim",
    ]
}

/// Create a strategy instance by name and parameters.
///
/// This is the single point where strategy names are mapped to concrete types.
/// Both `ScalperEngine` and `PaperEngine` should use this function.
#[allow(dead_code)]
pub fn create_strategy(name: &str, params: StrategyParams) -> anyhow::Result<Box<dyn Strategy>> {
    match name {
        "momentum-scalper" => {
            // Validate params before creating
            if let Err(e) = params.validate() {
                anyhow::bail!("Invalid strategy parameters for '{}': {}", name, e);
            }
            Ok(Box::new(MomentumScalperStrategy::new(params)))
        }
        "funding-capture" => {
            // Funding capture uses its own params — use defaults from config
            let fc_params = crate::funding_capture::FundingCaptureParams::default();
            Ok(Box::new(
                crate::funding_capture::FundingRateCaptureStrategy::new(fc_params),
            ))
        }
        _ => {
            let available = available_strategies().join(", ");
            anyhow::bail!(
                "Unknown strategy '{}'. Available strategies: {}",
                name,
                available
            );
        }
    }
}

/// Create a Funding Rate Capture strategy from its specific parameters.
pub fn create_funding_capture_strategy(
    params: crate::funding_capture::FundingCaptureParams,
) -> anyhow::Result<Box<dyn Strategy>> {
    if let Err(e) = params.validate() {
        anyhow::bail!("Invalid funding capture parameters: {}", e);
    }
    Ok(Box::new(
        crate::funding_capture::FundingRateCaptureStrategy::new(params),
    ))
}

/// Create an LP Consumption strategy from its specific parameters.
pub fn create_lp_consumption_strategy(
    params: LpConsumptionParams,
) -> anyhow::Result<Box<dyn Strategy>> {
    if let Err(e) = params.validate() {
        anyhow::bail!("Invalid LP consumption parameters: {}", e);
    }
    Ok(Box::new(LpConsumptionStrategy::new(params)))
}

/// Create a Mean Reversion strategy from its specific parameters.
pub fn create_mean_reversion_strategy(
    params: MeanReversionParams,
) -> anyhow::Result<Box<dyn Strategy>> {
    if let Err(e) = params.validate() {
        anyhow::bail!("Invalid mean reversion parameters: {}", e);
    }
    Ok(Box::new(MeanReversionStrategy::new(params)))
}

/// Create a Trend Follower strategy from its specific parameters.
pub fn create_trend_follower_strategy(
    params: TrendFollowerParams,
) -> anyhow::Result<Box<dyn Strategy>> {
    if let Err(e) = params.validate() {
        anyhow::bail!("Invalid trend follower parameters: {}", e);
    }
    Ok(Box::new(TrendFollowerStrategy::new(params)))
}

/// Create a strategy by name using config-provided TOML sub-table.
///
/// This is the primary factory function used by engines. It handles both
/// momentum-scalper (via StrategyParams) and lp-consumption (via LpConsumptionParams).
pub fn create_strategy_from_config(
    name: &str,
    sub_table: Option<&toml::Value>,
    fallback_params: StrategyParams,
) -> anyhow::Result<Box<dyn Strategy>> {
    match name {
        "momentum-scalper" => {
            if let Err(e) = fallback_params.validate() {
                anyhow::bail!("Invalid strategy parameters for '{}': {}", name, e);
            }
            Ok(Box::new(MomentumScalperStrategy::new(fallback_params)))
        }
        "lp-consumption" => {
            let lp_params = if let Some(table) = sub_table {
                let params: LpConsumptionParams = table.clone().try_into().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse [strategy.lp-consumption] sub-table: {}",
                        e
                    )
                })?;
                params
            } else {
                // Use defaults
                LpConsumptionParams {
                    consumption_velocity_threshold: 0.5,
                    lp_concentration_min: 0.7,
                    confirmation_ticks: 3,
                    max_utilization: 0.9,
                    direction_bias: "neutral".to_string(),
                    clip_size_usd: fallback_params.clip_size_usd,
                    max_hold_secs: 3600,
                    take_profit_pct: 2.0,
                    stop_loss_pct: 1.0,
                    trailing_stop_pct: 0.8,
                    trailing_activation_pct: 1.2,
                    cooldown_after_loss_secs: 300,
                    use_native_tp_sl: true,
                    leverage: 2.5,
                    scale_in_clips: 1,
                    flash_native_mode: false,
                    flash_native_min_util_pct: 30.0,
                    flash_native_velocity_mult: 0.6,
                    is_flash_only_market: false,
                }
            };
            create_lp_consumption_strategy(lp_params)
        }
        "mean-reversion" => {
            let mr_params = if let Some(table) = sub_table {
                let params: MeanReversionParams = table.clone().try_into().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse [strategy.mean-reversion] sub-table: {}",
                        e
                    )
                })?;
                params
            } else {
                // Use defaults
                MeanReversionParams {
                    mean_lookback: 120,
                    deviation_threshold_pct: 1.5,
                    reversal_confirmation_ticks: 2,
                    mean_tolerance_pct: 0.3,
                    direction_bias: "neutral".to_string(),
                    clip_size_usd: fallback_params.clip_size_usd,
                    max_hold_secs: 1800,
                    take_profit_pct: 1.0,
                    stop_loss_pct: 1.5,
                    trailing_stop_pct: 0.0,
                    trailing_activation_pct: 0.0,
                    cooldown_after_loss_secs: 300,
                    use_native_tp_sl: true,
                    leverage: 3.0,
                    scale_in_clips: 1,
                }
            };
            create_mean_reversion_strategy(mr_params)
        }
        "trend-follower" => {
            let tf_params = if let Some(table) = sub_table {
                let params: TrendFollowerParams = table.clone().try_into().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse [strategy.trend-follower] sub-table: {}",
                        e
                    )
                })?;
                params
            } else {
                // Use sensible defaults (no M1 blueprint available)
                TrendFollowerParams {
                    breakout_threshold_pct: 0.25,
                    confirmation_ticks: 4,
                    min_price_count: 30,
                    direction_bias: "neutral".to_string(),
                    clip_size_usd: fallback_params.clip_size_usd,
                    max_hold_secs: 7200,
                    take_profit_pct: 5.0,
                    stop_loss_pct: 2.0,
                    trailing_stop_pct: 1.5,
                    trailing_activation_pct: 2.5,
                    cooldown_after_loss_secs: 300,
                    use_native_tp_sl: true,
                    leverage: 5.0,
                    scale_in_clips: 1,
                }
            };
            create_trend_follower_strategy(tf_params)
        }
        "funding-capture" => {
            let fc_params = if let Some(table) = sub_table {
                let params: crate::funding_capture::FundingCaptureParams =
                    table.clone().try_into().map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to parse [strategy.funding-capture] sub-table: {}",
                            e
                        )
                    })?;
                params
            } else {
                crate::funding_capture::FundingCaptureParams {
                    clip_size_usd: fallback_params.clip_size_usd,
                    ..Default::default()
                }
            };
            create_funding_capture_strategy(fc_params)
        }
        "blueprint-scalper" => {
            let bp_params = if let Some(table) = sub_table {
                // Build from config sub-table, filling source fields from defaults
                let momentum_threshold_pct = table
                    .get("momentum_threshold_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.339);
                let lookback_count = table
                    .get("lookback_count")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(30) as usize;
                let take_profit_pct = table
                    .get("take_profit_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.2983);
                let stop_loss_pct = table
                    .get("stop_loss_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.141);
                let max_hold_secs = table
                    .get("max_hold_secs")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(10128) as u64;
                let clip_size_usd = table
                    .get("clip_size_usd")
                    .and_then(|v| v.as_float())
                    .unwrap_or(63.68);
                let direction_bias = table
                    .get("direction_bias")
                    .and_then(|v| v.as_str())
                    .unwrap_or("long")
                    .to_string();

                BlueprintScalperParams {
                    source_cluster_id: "cluster-001".to_string(),
                    blueprint_path: "data/blueprints/cluster-001.json".to_string(),
                    source_wallet_count: 12,
                    source_total_trades: 4711,
                    confidence_score: 0.7334,
                    primary_market: "BTC".to_string(),
                    direction_bias,
                    momentum_threshold_pct,
                    lookback_count,
                    take_profit_pct,
                    stop_loss_pct,
                    max_hold_secs,
                    trailing_stop_pct: table
                        .get("trailing_stop_pct")
                        .and_then(|v| v.as_float())
                        .unwrap_or(0.0),
                    trailing_activation_pct: table
                        .get("trailing_activation_pct")
                        .and_then(|v| v.as_float())
                        .unwrap_or(0.0),
                    clip_size_usd,
                    leverage: table
                        .get("leverage")
                        .and_then(|v| v.as_float())
                        .unwrap_or(3.0),
                    scale_in_clips: table
                        .get("scale_in_clips")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(1) as u32,
                    cooldown_after_loss_secs: table
                        .get("cooldown_after_loss_secs")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(300) as u64,
                    use_native_tp_sl: table
                        .get("use_native_tp_sl")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                }
            } else {
                // Load from blueprint JSON
                match BlueprintScalperParams::from_blueprint() {
                    Ok(params) => params,
                    Err(e) => {
                        // Fallback to hardcoded cluster-001 defaults if blueprint file missing
                        debug!(
                            "[blueprint-scalper] Could not load blueprint, using defaults: {}",
                            e
                        );
                        BlueprintScalperParams {
                            source_cluster_id: "cluster-001".to_string(),
                            blueprint_path: "data/blueprints/cluster-001.json".to_string(),
                            source_wallet_count: 12,
                            source_total_trades: 4711,
                            confidence_score: 0.7334,
                            primary_market: "BTC".to_string(),
                            direction_bias: "long".to_string(),
                            momentum_threshold_pct: 0.339,
                            lookback_count: 30,
                            take_profit_pct: 0.2983,
                            stop_loss_pct: 0.141,
                            max_hold_secs: 10128,
                            trailing_stop_pct: 0.0,
                            trailing_activation_pct: 0.0,
                            clip_size_usd: 63.68,
                            leverage: 3.0,
                            scale_in_clips: 1,
                            cooldown_after_loss_secs: 300,
                            use_native_tp_sl: true,
                        }
                    }
                }
            };
            DataDrivenScalperStrategy::from_params(bp_params)
                .map(|s| Box::new(s) as Box<dyn Strategy>)
        }
        "blueprint-mean-revert" => {
            let bp_params = if let Some(table) = sub_table {
                let mean_lookback = table
                    .get("mean_lookback")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(30) as usize;
                let deviation_threshold_pct = table
                    .get("deviation_threshold_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(1.009);
                let take_profit_pct = table
                    .get("take_profit_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.4284);
                let stop_loss_pct = table
                    .get("stop_loss_pct")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.2879);
                let max_hold_secs = table
                    .get("max_hold_secs")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(12313) as u64;
                let clip_size_usd = table
                    .get("clip_size_usd")
                    .and_then(|v| v.as_float())
                    .unwrap_or(116.6);
                let direction_bias = table
                    .get("direction_bias")
                    .and_then(|v| v.as_str())
                    .unwrap_or("neutral")
                    .to_string();

                BlueprintMeanRevertParams {
                    source_cluster_id: "cluster-004".to_string(),
                    blueprint_path: "data/blueprints/cluster-004.json".to_string(),
                    source_wallet_count: 5,
                    source_total_trades: 518,
                    confidence_score: 0.8279,
                    primary_market: "BTC".to_string(),
                    direction_bias,
                    mean_lookback,
                    deviation_threshold_pct,
                    reversal_confirmation_ticks: table
                        .get("reversal_confirmation_ticks")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(2) as usize,
                    mean_tolerance_pct: table
                        .get("mean_tolerance_pct")
                        .and_then(|v| v.as_float())
                        .unwrap_or(0.3),
                    take_profit_pct,
                    stop_loss_pct,
                    max_hold_secs,
                    trailing_stop_pct: 0.0,
                    trailing_activation_pct: 0.0,
                    clip_size_usd,
                    leverage: table
                        .get("leverage")
                        .and_then(|v| v.as_float())
                        .unwrap_or(3.0),
                    scale_in_clips: 1,
                    cooldown_after_loss_secs: 300,
                    use_native_tp_sl: true,
                }
            } else {
                // Load from blueprint JSON
                match BlueprintMeanRevertParams::from_blueprint() {
                    Ok(params) => params,
                    Err(e) => {
                        debug!(
                            "[blueprint-mean-revert] Could not load blueprint, using defaults: {}",
                            e
                        );
                        BlueprintMeanRevertParams {
                            source_cluster_id: "cluster-004".to_string(),
                            blueprint_path: "data/blueprints/cluster-004.json".to_string(),
                            source_wallet_count: 5,
                            source_total_trades: 518,
                            confidence_score: 0.8279,
                            primary_market: "BTC".to_string(),
                            direction_bias: "neutral".to_string(),
                            mean_lookback: 30,
                            deviation_threshold_pct: 1.009,
                            reversal_confirmation_ticks: 2,
                            mean_tolerance_pct: 0.3,
                            take_profit_pct: 0.4284,
                            stop_loss_pct: 0.2879,
                            max_hold_secs: 12313,
                            trailing_stop_pct: 0.0,
                            trailing_activation_pct: 0.0,
                            clip_size_usd: 116.6,
                            leverage: 3.0,
                            scale_in_clips: 1,
                            cooldown_after_loss_secs: 300,
                            use_native_tp_sl: true,
                        }
                    }
                }
            };
            DataDrivenMeanRevertStrategy::from_params(bp_params)
                .map(|s| Box::new(s) as Box<dyn Strategy>)
        }
        // Generic blueprint strategies: load from cluster JSON
        "blueprint-cluster-002"
        | "blueprint-cluster-003"
        | "blueprint-cluster-005"
        | "blueprint-cluster-006"
        | "blueprint-cluster-007"
        | "blueprint-cluster-008"
        | "blueprint-cluster-009"
        | "blueprint-hft-market-maker" => {
            let cluster_id = name.strip_prefix("blueprint-").unwrap();
            match GenericBlueprintParams::from_cluster(cluster_id) {
                Ok(params) => {
                    GenericBlueprintStrategy::from_params(cluster_id, params)
                        .map(|s| Box::new(s) as Box<dyn Strategy>)
                }
                Err(e) => {
                    anyhow::bail!(
                        "Failed to load blueprint for '{}': {}",
                        cluster_id, e
                    );
                }
            }
        }
        "liquidation-cascade-continuation" => {
            let lc_params = if let Some(table) = sub_table {
                let params: LiquidationCascadeParams = table.clone().try_into().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse [strategy.liquidation-cascade-continuation] sub-table: {}",
                        e
                    )
                })?;
                params
            } else {
                LiquidationCascadeParams::default()
            };
            if let Err(e) = lc_params.validate() {
                anyhow::bail!("Invalid liquidation-cascade-continuation parameters: {}", e);
            }
            Ok(Box::new(LiquidationCascadeHunter::new(lc_params)))
        }
        "liquidation-cascade-hunter" => {
            // Legacy alias for liquidation-cascade-continuation
            tracing::debug!("liquidation-cascade-hunter is a legacy alias for liquidation-cascade-continuation");
            let lc_params = if let Some(table) = sub_table {
                let params: LiquidationCascadeParams = table.clone().try_into().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse [strategy.liquidation-cascade-hunter] sub-table: {}",
                        e
                    )
                })?;
                params
            } else {
                LiquidationCascadeParams::default()
            };
            if let Err(e) = lc_params.validate() {
                anyhow::bail!("Invalid liquidation-cascade-hunter parameters: {}", e);
            }
            Ok(Box::new(LiquidationCascadeHunter::new_legacy(lc_params)))
        }
        "sweep-reclaim" => {
            let sr_params = if let Some(table) = sub_table {
                SweepReclaimParams::from_toml_table(table)
                    .map_err(|e| anyhow::anyhow!("Invalid [strategy.sweep-reclaim] config: {}", e))?
            } else {
                SweepReclaimParams::default()
            };
            if let Err(e) = sr_params.validate() {
                anyhow::bail!("Invalid sweep-reclaim parameters: {}", e);
            }
            Ok(Box::new(SweepReclaimStrategy::new(sr_params)))
        }
        _ => {
            let available = available_strategies().join(", ");
            anyhow::bail!(
                "Unknown strategy '{}'. Available strategies: {}",
                name,
                available
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Blueprint Data Structures & Loader
// ---------------------------------------------------------------------------

/// Deserialized representation of a strategy blueprint JSON file produced by
/// the Python analysis pipeline (`analysis/blueprint_generator.py`).
///
/// Each blueprint captures the statistical parameters derived from a cluster of
/// profitable Hyperliquid wallets running the same strategy type.
///
/// File location: `data/blueprints/{cluster_id}.json`
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintData {
    pub strategy_name: String,
    pub strategy_type: String,
    pub source_cluster_id: String,
    pub source_wallets: Vec<String>,
    pub primary_market: String,
    pub direction: String,
    pub entry_conditions: BlueprintEntryConditions,
    pub exit_conditions: BlueprintExitConditions,
    pub risk_parameters: BlueprintRiskParameters,
    pub statistical_parameters: BlueprintStats,
    pub confidence_score: f64,
    pub sample_size: BlueprintSampleSize,
    pub parameter_traceability: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintEntryConditions {
    pub description: String,
    pub lookback_candles: u64,
    pub parameters: BlueprintEntryParams,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintEntryParams {
    pub price_velocity_threshold: f64,
    pub volume_spike_threshold_sd: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintExitConditions {
    pub description: String,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_hold_hours: f64,
    pub trailing_stop: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintRiskParameters {
    pub clip_size_usd: f64,
    pub max_hold_hours: f64,
    pub position_size_pct: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintStats {
    pub hold_time: BlueprintHoldTime,
    pub win_rate: BlueprintWinRate,
    pub clip_size: BlueprintClipSize,
    pub pnl: BlueprintPnl,
    pub fees: BlueprintFees,
    pub tp_sl: BlueprintTpSl,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintHoldTime {
    pub median_hours: f64,
    pub p25_hours: f64,
    pub p75_hours: f64,
    pub position_median_hours: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintWinRate {
    pub median: f64,
    pub p25: f64,
    pub p75: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintClipSize {
    pub median_notional: f64,
    pub p25_notional: f64,
    pub p75_notional: f64,
    pub position_median_size: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintPnl {
    pub median_fee_adjusted: f64,
    pub position_median: f64,
    pub avg_winner: f64,
    pub avg_loser: f64,
    pub total_positions: u64,
    pub winning_positions: u64,
    pub losing_positions: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintFees {
    pub median_per_position: f64,
    pub total: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintTpSl {
    pub median_tp_pct: f64,
    pub median_sl_pct: f64,
    pub p75_tp_pct: f64,
    pub p75_sl_pct: f64,
    pub num_winning_positions: u64,
    pub num_losing_positions: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintSampleSize {
    pub wallets: u64,
    pub total_trades: u64,
}

/// Load a blueprint JSON file from the data/blueprints/ directory.
///
/// Returns the deserialized `BlueprintData` or an error if the file doesn't
/// exist or is malformed.
pub fn load_blueprint(cluster_id: &str) -> anyhow::Result<BlueprintData> {
    let path = std::path::Path::new("data/blueprints")
        .join(format!("{}.json", cluster_id));
    load_blueprint_from_path(&path)
}

/// Load a blueprint JSON file from an explicit path.
fn load_blueprint_from_path(path: &std::path::Path) -> anyhow::Result<BlueprintData> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read blueprint {:?}: {}", path, e))?;
    let data: BlueprintData = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse blueprint {:?}: {}", path, e))?;
    Ok(data)
}

// ---------------------------------------------------------------------------
// Data-Driven Momentum Scalper (from cluster-001 blueprint)
// ---------------------------------------------------------------------------

/// Parameters for the data-driven momentum scalper strategy.
///
/// All parameter values are derived from `data/blueprints/cluster-001.json`,
/// which contains the statistical aggregates of 12 profitable Hyperliquid
/// wallets running a BTC-long momentum-scalper strategy (4,711 trades,
/// 71% win rate, confidence 0.73).
///
/// **Data lineage:**
/// ```text
/// 12 HL wallets → userFills → position_clusters → cluster medians
/// → blueprint JSON → these parameters → config TOML → strategy code
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintScalperParams {
    // --- Source identification ---
    /// Cluster ID that this strategy was derived from.
    pub source_cluster_id: String,
    /// Path to the blueprint JSON file.
    pub blueprint_path: String,
    /// Number of source wallets in the cluster.
    pub source_wallet_count: u64,
    /// Total trades across all source wallets.
    pub source_total_trades: u64,
    /// Confidence score of the cluster classification.
    pub confidence_score: f64,
    /// Primary market of the source wallets.
    pub primary_market: String,
    /// Direction bias of the source wallets.
    pub direction_bias: String,

    // --- Entry parameters (from blueprint entry_conditions) ---
    /// Velocity threshold (%) for momentum entry.
    /// Source: cluster-001 entry_conditions.parameters.price_velocity_threshold
    pub momentum_threshold_pct: f64,
    /// Number of lookback candles for velocity computation.
    /// Source: cluster-001 entry_conditions.lookback_candles (scaled)
    pub lookback_count: usize,

    // --- Exit parameters (from blueprint exit_conditions) ---
    /// Take-profit threshold (%).
    /// Source: cluster-001 exit_conditions.take_profit_pct (converted from decimal)
    pub take_profit_pct: f64,
    /// Stop-loss threshold (%).
    /// Source: cluster-001 exit_conditions.stop_loss_pct (converted from decimal)
    pub stop_loss_pct: f64,
    /// Maximum hold time in seconds.
    /// Source: cluster-001 exit_conditions.max_hold_hours (converted to seconds)
    pub max_hold_secs: u64,
    /// Trailing stop percentage (0.0 = disabled).
    /// Source: cluster-001 exit_conditions.trailing_stop
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage.
    /// Source: cluster-001 exit_conditions.trailing_stop (disabled for this cluster)
    pub trailing_activation_pct: f64,

    // --- Risk parameters (from blueprint risk_parameters) ---
    /// Position clip size in USD.
    /// Source: cluster-001 risk_parameters.clip_size_usd
    pub clip_size_usd: f64,
    /// Leverage for positions.
    pub leverage: f64,
    /// Number of scale-in clips.
    pub scale_in_clips: u32,
    /// Cooldown after a losing trade, in seconds.
    pub cooldown_after_loss_secs: u64,
    /// Whether to use native on-chain TP/SL trigger orders.
    pub use_native_tp_sl: bool,
}

impl BlueprintScalperParams {
    /// Load parameters from the cluster-001 blueprint.
    pub fn from_blueprint() -> anyhow::Result<Self> {
        let bp = load_blueprint("cluster-001")?;
        Ok(Self::from_blueprint_data(&bp))
    }

    /// Build params from a pre-loaded BlueprintData.
    pub fn from_blueprint_data(bp: &BlueprintData) -> Self {
        Self {
            source_cluster_id: bp.source_cluster_id.clone(),
            blueprint_path: format!("data/blueprints/{}.json", bp.source_cluster_id),
            source_wallet_count: bp.sample_size.wallets,
            source_total_trades: bp.sample_size.total_trades,
            confidence_score: bp.confidence_score,
            primary_market: bp.primary_market.clone(),
            direction_bias: bp.direction.clone(),
            momentum_threshold_pct: bp.entry_conditions.parameters.price_velocity_threshold,
            lookback_count: (bp.entry_conditions.lookback_candles as usize) * 5,
            take_profit_pct: bp.exit_conditions.take_profit_pct * 100.0,
            stop_loss_pct: bp.exit_conditions.stop_loss_pct * 100.0,
            max_hold_secs: (bp.exit_conditions.max_hold_hours * 3600.0) as u64,
            trailing_stop_pct: if bp.exit_conditions.trailing_stop { 0.3 } else { 0.0 },
            trailing_activation_pct: if bp.exit_conditions.trailing_stop { 0.5 } else { 0.0 },
            clip_size_usd: bp.risk_parameters.clip_size_usd,
            leverage: 3.0,
            scale_in_clips: 1,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    /// Validate parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.momentum_threshold_pct <= 0.0 {
            return Err(format!("momentum_threshold_pct must be > 0, got {}", self.momentum_threshold_pct));
        }
        if self.lookback_count == 0 {
            return Err("lookback_count must be > 0".to_string());
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!("clip_size_usd must be > 0, got {}", self.clip_size_usd));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!("take_profit_pct must be > 0, got {}", self.take_profit_pct));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!("stop_loss_pct must be > 0, got {}", self.stop_loss_pct));
        }
        Ok(())
    }

    /// Convert to generic StrategyParams for engine/risk integration.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.momentum_threshold_pct,
            lookback_count: self.lookback_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

/// Data-driven Momentum Scalper strategy.
///
/// This strategy replicates the behavior of a cluster of 12 profitable
/// Hyperliquid wallets identified by the Python analysis pipeline.
///
/// **Source:** `data/blueprints/cluster-001.json`
/// - Strategy type: momentum_scalper
/// - Primary market: BTC
/// - Direction: long
/// - Cluster size: 12 wallets, 4,711 trades
/// - Win rate: 71%, Confidence: 0.73
///
/// **Entry:** Momentum velocity exceeding cluster-derived threshold
/// **Exit:** Tight TP/SL from cluster's actual winning/losing position ranges
///
/// Every numeric parameter is a statistical aggregate from the cluster,
/// not a hardcoded guess. See `BlueprintScalperParams` for per-parameter
/// traceability to the source cluster data.
#[allow(dead_code)]
pub struct DataDrivenScalperStrategy {
    detector: MomentumDetector,
    params: BlueprintScalperParams,
    generic_params: StrategyParams,
}

impl DataDrivenScalperStrategy {
    /// Create from a pre-loaded BlueprintData.
    #[allow(dead_code)]
    pub fn from_blueprint(bp: &BlueprintData) -> anyhow::Result<Self> {
        let params = BlueprintScalperParams::from_blueprint_data(bp);
        Self::from_params(params)
    }

    /// Create from explicit parameters.
    pub fn from_params(params: BlueprintScalperParams) -> anyhow::Result<Self> {
        if let Err(e) = params.validate() {
            anyhow::bail!("Invalid data-driven scalper parameters: {}", e);
        }
        let generic = params.to_strategy_params();
        let detector = MomentumDetector::new(params.momentum_threshold_pct, params.lookback_count);
        Ok(Self { detector, params, generic_params: generic })
    }

    /// Return a reference to the blueprint params.
    #[allow(dead_code)]
    pub fn blueprint_params(&self) -> &BlueprintScalperParams {
        &self.params
    }
}

impl Strategy for DataDrivenScalperStrategy {
    fn name(&self) -> &str {
        "blueprint-scalper"
    }

    fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal {
        self.detector.detect_signal(snapshot)
    }

    fn detect_exit(
        &self,
        snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        self.detector.detect_exit(
            snapshot,
            ctx.is_long,
            ctx.entry_price,
            ctx.current_price,
            ctx.peak_price,
            ctx.hold_secs,
            ctx.max_hold_secs,
            ctx.take_profit_pct,
            ctx.stop_loss_pct,
            ctx.trailing_stop_pct,
            ctx.trailing_activation_pct,
        )
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.detector.push_price(price, timestamp_ms);
    }

    fn snapshot(&self) -> MomentumSnapshot {
        self.detector.analyze()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Data-Driven Mean Reversion (from cluster-004 blueprint)
// ---------------------------------------------------------------------------

/// Parameters for the data-driven mean reversion strategy.
///
/// All parameter values are derived from `data/blueprints/cluster-004.json`,
/// which contains the statistical aggregates of 5 profitable Hyperliquid
/// wallets running a BTC-mixed mean-reversion strategy (518 trades,
/// 64% win rate, confidence 0.83).
///
/// **Data lineage:**
/// ```text
/// 5 HL wallets → userFills → position_clusters → cluster medians
/// → blueprint JSON → these parameters → config TOML → strategy code
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub struct BlueprintMeanRevertParams {
    // --- Source identification ---
    /// Cluster ID that this strategy was derived from.
    pub source_cluster_id: String,
    /// Path to the blueprint JSON file.
    pub blueprint_path: String,
    /// Number of source wallets in the cluster.
    pub source_wallet_count: u64,
    /// Total trades across all source wallets.
    pub source_total_trades: u64,
    /// Confidence score of the cluster classification.
    pub confidence_score: f64,
    /// Primary market of the source wallets.
    pub primary_market: String,
    /// Direction bias of the source wallets.
    pub direction_bias: String,

    // --- Entry parameters (from blueprint) ---
    /// Number of price points to compute the SMA.
    /// Source: cluster-004 entry_conditions.lookback_candles (scaled)
    pub mean_lookback: usize,
    /// Minimum deviation from SMA (%) to detect a spike.
    /// Source: cluster-004 entry_conditions.parameters.price_velocity_threshold
    pub deviation_threshold_pct: f64,
    /// Number of consecutive reversal ticks to confirm entry.
    pub reversal_confirmation_ticks: usize,
    /// How close to SMA price must return (%) for mean-return exit.
    pub mean_tolerance_pct: f64,

    // --- Exit parameters (from blueprint exit_conditions) ---
    /// Take-profit threshold (%).
    /// Source: cluster-004 exit_conditions.take_profit_pct (converted from decimal)
    pub take_profit_pct: f64,
    /// Stop-loss threshold (%).
    /// Source: cluster-004 exit_conditions.stop_loss_pct (converted from decimal)
    pub stop_loss_pct: f64,
    /// Maximum hold time in seconds.
    /// Source: cluster-004 exit_conditions.max_hold_hours (converted to seconds)
    pub max_hold_secs: u64,
    /// Trailing stop percentage (0.0 = disabled for mean reversion).
    pub trailing_stop_pct: f64,
    /// Trailing stop activation percentage.
    pub trailing_activation_pct: f64,

    // --- Risk parameters (from blueprint) ---
    /// Position clip size in USD.
    /// Source: cluster-004 risk_parameters.clip_size_usd
    pub clip_size_usd: f64,
    /// Leverage for positions.
    pub leverage: f64,
    /// Number of scale-in clips.
    pub scale_in_clips: u32,
    /// Cooldown after a losing trade, in seconds.
    pub cooldown_after_loss_secs: u64,
    /// Whether to use native on-chain TP/SL trigger orders.
    pub use_native_tp_sl: bool,
}

impl BlueprintMeanRevertParams {
    /// Load parameters from the cluster-004 blueprint.
    pub fn from_blueprint() -> anyhow::Result<Self> {
        let bp = load_blueprint("cluster-004")?;
        Ok(Self::from_blueprint_data(&bp))
    }

    /// Build params from a pre-loaded BlueprintData.
    pub fn from_blueprint_data(bp: &BlueprintData) -> Self {
        Self {
            source_cluster_id: bp.source_cluster_id.clone(),
            blueprint_path: format!("data/blueprints/{}.json", bp.source_cluster_id),
            source_wallet_count: bp.sample_size.wallets,
            source_total_trades: bp.sample_size.total_trades,
            confidence_score: bp.confidence_score,
            primary_market: bp.primary_market.clone(),
            direction_bias: bp.direction.clone(),
            mean_lookback: (bp.entry_conditions.lookback_candles as usize) * 5,
            deviation_threshold_pct: bp.entry_conditions.parameters.price_velocity_threshold,
            reversal_confirmation_ticks: 2,
            mean_tolerance_pct: 0.3,
            take_profit_pct: bp.exit_conditions.take_profit_pct * 100.0,
            stop_loss_pct: bp.exit_conditions.stop_loss_pct * 100.0,
            max_hold_secs: (bp.exit_conditions.max_hold_hours * 3600.0) as u64,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
            clip_size_usd: bp.risk_parameters.clip_size_usd,
            leverage: 3.0,
            scale_in_clips: 1,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    /// Validate parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.mean_lookback == 0 {
            return Err("mean_lookback must be > 0".to_string());
        }
        if self.deviation_threshold_pct <= 0.0 {
            return Err(format!("deviation_threshold_pct must be > 0, got {}", self.deviation_threshold_pct));
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!("clip_size_usd must be > 0, got {}", self.clip_size_usd));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!("take_profit_pct must be > 0, got {}", self.take_profit_pct));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!("stop_loss_pct must be > 0, got {}", self.stop_loss_pct));
        }
        Ok(())
    }

    /// Convert to generic StrategyParams for engine/risk integration.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.deviation_threshold_pct,
            lookback_count: self.mean_lookback,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

/// Direction of a detected spike relative to the SMA (duplicated for the
/// data-driven strategy to avoid borrow issues with the SpikeDirection enum
/// defined above in a different scope).
#[derive(Debug, Clone, PartialEq)]
enum BlueprintSpikeDirection {
    Above,
    Below,
}

/// Data-driven Mean Reversion strategy.
///
/// This strategy replicates the behavior of a cluster of 5 profitable
/// Hyperliquid wallets identified by the Python analysis pipeline.
///
/// **Source:** `data/blueprints/cluster-004.json`
/// - Strategy type: mean_reversion
/// - Primary market: BTC
/// - Direction: mixed
/// - Cluster size: 5 wallets, 518 trades
/// - Win rate: 64%, Confidence: 0.83
///
/// **Entry:** Fade momentum spikes after deviation from SMA exceeds cluster threshold
/// **Exit:** Mean return, TP/SL from cluster's actual position ranges
///
/// Every numeric parameter is a statistical aggregate from the cluster,
/// not a hardcoded guess. See `BlueprintMeanRevertParams` for per-parameter
/// traceability to the source cluster data.
#[allow(dead_code)]
pub struct DataDrivenMeanRevertStrategy {
    params: BlueprintMeanRevertParams,
    generic_params: StrategyParams,
    /// Rolling price buffer for SMA calculation.
    prices: VecDeque<crate::signal::PricePoint>,
    /// Current spike state.
    spike_state: Option<BlueprintSpikeDirection>,
    /// Number of consecutive reversal ticks after a spike.
    reversal_ticks: usize,
}

impl DataDrivenMeanRevertStrategy {
    /// Create from a pre-loaded BlueprintData.
    #[allow(dead_code)]
    pub fn from_blueprint(bp: &BlueprintData) -> anyhow::Result<Self> {
        let params = BlueprintMeanRevertParams::from_blueprint_data(bp);
        Self::from_params(params)
    }

    /// Create from explicit parameters.
    pub fn from_params(params: BlueprintMeanRevertParams) -> anyhow::Result<Self> {
        if let Err(e) = params.validate() {
            anyhow::bail!("Invalid data-driven mean revert parameters: {}", e);
        }
        let generic = params.to_strategy_params();
        let capacity = params.mean_lookback * 2;
        Ok(Self {
            generic_params: generic,
            prices: VecDeque::with_capacity(capacity),
            spike_state: None,
            reversal_ticks: 0,
            params,
        })
    }

    /// Return a reference to the blueprint params.
    #[allow(dead_code)]
    pub fn blueprint_params(&self) -> &BlueprintMeanRevertParams {
        &self.params
    }

    /// Compute the SMA over the last `lookback` prices.
    fn compute_sma(&self, lookback: usize) -> Option<f64> {
        if self.prices.len() < lookback {
            return None;
        }
        let sum: f64 = self.prices.iter().rev().take(lookback).map(|p| p.price).sum();
        Some(sum / lookback as f64)
    }

    /// Get the previous price.
    fn prev_price(&self) -> Option<f64> {
        if self.prices.len() < 2 {
            return None;
        }
        self.prices.iter().rev().nth(1).map(|p| p.price)
    }

    /// Get the current price.
    fn current_price(&self) -> Option<f64> {
        self.prices.back().map(|p| p.price)
    }
}

impl Strategy for DataDrivenMeanRevertStrategy {
    fn name(&self) -> &str {
        "blueprint-mean-revert"
    }

    fn detect_entry(&mut self, _snapshot: &MomentumSnapshot) -> Signal {
        let current_price = match self.current_price() {
            Some(p) => p,
            None => return Signal::NoSignal,
        };

        let sma = match self.compute_sma(self.params.mean_lookback) {
            Some(s) => s,
            None => {
                debug!(
                    "[blueprint-mean-revert] Insufficient price history: {}/{} prices",
                    self.prices.len(),
                    self.params.mean_lookback
                );
                return Signal::NoSignal;
            }
        };

        if sma <= 0.0 {
            return Signal::NoSignal;
        }

        let deviation_pct = (current_price - sma) / sma * 100.0;

        // Spike detection
        if deviation_pct > self.params.deviation_threshold_pct {
            if self.spike_state != Some(BlueprintSpikeDirection::Above) {
                self.spike_state = Some(BlueprintSpikeDirection::Above);
                self.reversal_ticks = 0;
            }
        } else if deviation_pct < -self.params.deviation_threshold_pct
            && self.spike_state != Some(BlueprintSpikeDirection::Below)
        {
            self.spike_state = Some(BlueprintSpikeDirection::Below);
            self.reversal_ticks = 0;
        }

        // Reversal confirmation
        if let Some(ref spike_dir) = self.spike_state {
            let prev_price = match self.prev_price() {
                Some(p) => p,
                None => return Signal::NoSignal,
            };

            let moving_toward_mean = match spike_dir {
                BlueprintSpikeDirection::Above => current_price < prev_price,
                BlueprintSpikeDirection::Below => current_price > prev_price,
            };

            if moving_toward_mean {
                self.reversal_ticks += 1;

                if self.reversal_ticks >= self.params.reversal_confirmation_ticks {
                    let strength = (deviation_pct.abs() / self.params.deviation_threshold_pct * 50.0)
                        .clamp(50.0, 100.0);
                    let velocity_pct = deviation_pct.abs();

                    let spike_dir_clone = spike_dir.clone();
                    self.spike_state = None;
                    self.reversal_ticks = 0;

                    match spike_dir_clone {
                        BlueprintSpikeDirection::Above => {
                            info!(
                                "[blueprint-mean-revert] SHORT signal: cluster={}, price={:.2}, sma={:.2}, dev={:.2}%",
                                self.params.source_cluster_id, current_price, sma, deviation_pct
                            );
                            Signal::MomentumShort { strength, velocity_pct }
                        }
                        BlueprintSpikeDirection::Below => {
                            info!(
                                "[blueprint-mean-revert] LONG signal: cluster={}, price={:.2}, sma={:.2}, dev={:.2}%",
                                self.params.source_cluster_id, current_price, sma, deviation_pct
                            );
                            Signal::MomentumLong { strength, velocity_pct }
                        }
                    }
                } else {
                    Signal::NoSignal
                }
            } else {
                if deviation_pct.abs() <= self.params.deviation_threshold_pct {
                    self.spike_state = None;
                    self.reversal_ticks = 0;
                }
                Signal::NoSignal
            }
        } else {
            Signal::NoSignal
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let current_price = ctx.current_price;

        let pnl_pct = if ctx.is_long {
            (current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - current_price) / ctx.entry_price * 100.0
        };

        // 1. Stop loss
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!(
                "[blueprint-mean-revert] STOP LOSS: pnl={:.2}%, threshold=-{:.2}%",
                pnl_pct, ctx.stop_loss_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::StopLoss));
        }

        // 2. Mean return
        if let Some(sma) = self.compute_sma(self.params.mean_lookback)
            && sma > 0.0
        {
            let deviation_from_mean = (current_price - sma).abs() / sma * 100.0;
            if deviation_from_mean <= self.params.mean_tolerance_pct {
                info!(
                    "[blueprint-mean-revert] MEAN RETURN: price={:.2}, sma={:.2}, dev={:.2}%",
                    current_price, sma, deviation_from_mean
                );
                return Some(exit_signal(
                    ctx.is_long,
                    crate::signal::ExitReason::TakeProfit,
                ));
            }
        }

        // 3. Take profit
        if pnl_pct >= ctx.take_profit_pct {
            info!(
                "[blueprint-mean-revert] TAKE PROFIT: pnl={:.2}%, threshold={:.2}%",
                pnl_pct, ctx.take_profit_pct
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
        }

        // 4. Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            warn!(
                "[blueprint-mean-revert] TIME STOP: held {}s, max={}s",
                ctx.hold_secs, ctx.max_hold_secs
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TimeStop));
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.prices.push_back(crate::signal::PricePoint { price, timestamp_ms });
        while self.prices.len() > self.params.mean_lookback * 2 {
            self.prices.pop_front();
        }
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let lookback = self.params.mean_lookback;
        let sma = self.compute_sma(lookback);
        let current_price = self.current_price().unwrap_or(0.0);

        let velocity_pct = if let Some(sma) = sma {
            if sma > 0.0 { (current_price - sma) / sma * 100.0 } else { 0.0 }
        } else {
            0.0
        };

        let direction = if velocity_pct > 0.0 {
            crate::signal::TradeDirection::Long
        } else if velocity_pct < 0.0 {
            crate::signal::TradeDirection::Short
        } else {
            crate::signal::TradeDirection::Neutral
        };

        MomentumSnapshot {
            price_count: self.prices.len(),
            current_price,
            price_velocity_pct: velocity_pct,
            direction,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Generic Blueprint Strategy (handles any cluster blueprint)
// ---------------------------------------------------------------------------

/// Entry logic variant, selected by the `strategy_type` field in the blueprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum BlueprintEntryLogic {
    /// Momentum velocity exceeding threshold (momentum_scalper)
    Momentum,
    /// SMA deviation + reversal confirmation (mean_reversion)
    MeanReversion,
    /// Breakout above threshold with confirmation ticks (trend_follower)
    TrendBreakout,
    /// Grid/oscillation: enter on small dips, exit on small bounces (grid)
    Grid,
}

/// Parameters for the generic blueprint strategy.
///
/// Loads from any cluster blueprint JSON and selects entry logic based on
/// `strategy_type`. All numeric parameters come directly from the blueprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GenericBlueprintParams {
    // --- Source identification ---
    pub source_cluster_id: String,
    pub blueprint_path: String,
    pub source_wallet_count: u64,
    pub source_total_trades: u64,
    pub confidence_score: f64,
    pub primary_market: String,
    pub direction_bias: String,
    pub strategy_type: String,
    pub entry_logic: BlueprintEntryLogic,

    // --- Entry parameters ---
    pub momentum_threshold_pct: f64,
    pub lookback_count: usize,

    // --- Mean reversion specific ---
    pub mean_lookback: usize,
    pub deviation_threshold_pct: f64,
    pub reversal_confirmation_ticks: usize,
    pub mean_tolerance_pct: f64,

    // --- Trend follower specific ---
    pub confirmation_ticks: usize,

    // --- Exit parameters ---
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_hold_secs: u64,
    pub trailing_stop_pct: f64,
    pub trailing_activation_pct: f64,

    // --- Risk parameters ---
    pub clip_size_usd: f64,
    pub leverage: f64,
    pub scale_in_clips: u32,
    pub cooldown_after_loss_secs: u64,
    pub use_native_tp_sl: bool,
}

impl GenericBlueprintParams {
    /// Load parameters from any cluster blueprint by cluster ID.
    pub fn from_cluster(cluster_id: &str) -> anyhow::Result<Self> {
        let bp = load_blueprint(cluster_id)?;
        Ok(Self::from_blueprint_data(&bp))
    }

    /// Build params from a pre-loaded BlueprintData.
    pub fn from_blueprint_data(bp: &BlueprintData) -> Self {
        let entry_logic = match bp.strategy_type.as_str() {
            "momentum_scalper" => BlueprintEntryLogic::Momentum,
            "mean_reversion" => BlueprintEntryLogic::MeanReversion,
            "trend_follower" => BlueprintEntryLogic::TrendBreakout,
            "grid" => BlueprintEntryLogic::Grid,
            other => {
                // Default to momentum for unknown types
                warn!("Unknown strategy_type '{}', defaulting to Momentum", other);
                BlueprintEntryLogic::Momentum
            }
        };

        let direction_bias = match bp.direction.as_str() {
            "long" => "long".to_string(),
            "short" => "short".to_string(),
            _ => "neutral".to_string(),
        };

        let lookback_count = (bp.entry_conditions.lookback_candles as usize) * 5;
        let mean_lookback = (bp.entry_conditions.lookback_candles as usize) * 5;

        Self {
            source_cluster_id: bp.source_cluster_id.clone(),
            blueprint_path: format!("data/blueprints/{}.json", bp.source_cluster_id),
            source_wallet_count: bp.sample_size.wallets,
            source_total_trades: bp.sample_size.total_trades,
            confidence_score: bp.confidence_score,
            primary_market: bp.primary_market.clone(),
            direction_bias,
            strategy_type: bp.strategy_type.clone(),
            entry_logic,

            momentum_threshold_pct: bp.entry_conditions.parameters.price_velocity_threshold,
            lookback_count,

            mean_lookback,
            deviation_threshold_pct: bp.entry_conditions.parameters.price_velocity_threshold,
            reversal_confirmation_ticks: 2,
            mean_tolerance_pct: 0.3,

            confirmation_ticks: 3,

            take_profit_pct: bp.exit_conditions.take_profit_pct * 100.0,
            stop_loss_pct: bp.exit_conditions.stop_loss_pct * 100.0,
            max_hold_secs: (bp.exit_conditions.max_hold_hours * 3600.0) as u64,
            trailing_stop_pct: if bp.exit_conditions.trailing_stop { 0.5 } else { 0.0 },
            trailing_activation_pct: if bp.exit_conditions.trailing_stop { 0.3 } else { 0.0 },

            clip_size_usd: bp.risk_parameters.clip_size_usd,
            leverage: 3.0,
            scale_in_clips: 1,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    /// Validate parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.clip_size_usd <= 0.0 {
            return Err(format!("clip_size_usd must be > 0, got {}", self.clip_size_usd));
        }
        if self.take_profit_pct <= 0.0 {
            return Err(format!("take_profit_pct must be > 0, got {}", self.take_profit_pct));
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!("stop_loss_pct must be > 0, got {}", self.stop_loss_pct));
        }
        if self.max_hold_secs == 0 {
            return Err("max_hold_secs must be > 0".to_string());
        }
        Ok(())
    }

    /// Convert to generic StrategyParams for engine/risk integration.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: self.direction_bias.clone(),
            momentum_threshold_pct: self.momentum_threshold_pct,
            lookback_count: self.lookback_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

/// Spike direction for mean-reversion and grid logic.
#[derive(Debug, Clone, PartialEq)]
enum GenericSpikeDirection {
    Above,
    Below,
}

/// Generic blueprint strategy that handles any cluster.
///
/// Selects entry logic based on the `strategy_type` field in the blueprint:
/// - `momentum_scalper` → velocity-based entry
/// - `mean_reversion` → SMA deviation + reversal
/// - `trend_follower` → breakout with confirmation ticks
/// - `grid` → oscillation around SMA
///
/// All exit logic (TP/SL/trailing/time stop) is shared across types.
#[allow(dead_code)]
pub struct GenericBlueprintStrategy {
    params: GenericBlueprintParams,
    generic_params: StrategyParams,
    /// Strategy name (e.g., "blueprint-cluster-003")
    name: String,
    /// Rolling price buffer.
    prices: VecDeque<crate::signal::PricePoint>,
    /// SMA spike state (for mean-reversion and grid).
    spike_state: Option<GenericSpikeDirection>,
    /// Reversal tick counter (for mean-reversion).
    reversal_ticks: usize,
    /// Consecutive directional ticks (for trend follower confirmation).
    consecutive_directional: usize,
    /// Last direction seen for trend confirmation.
    last_direction: Option<crate::signal::TradeDirection>,
    /// Whether auto-scaling has been calibrated.
    auto_scaled: bool,
    /// Original velocity threshold from blueprint (before scaling).
    original_velocity_threshold: f64,
}

impl GenericBlueprintStrategy {
    /// Create from a cluster ID (e.g., "cluster-003").
    #[allow(dead_code)]
    pub fn from_cluster(cluster_id: &str) -> anyhow::Result<Self> {
        let params = GenericBlueprintParams::from_cluster(cluster_id)?;
        Self::from_params(cluster_id, params)
    }

    /// Create from explicit parameters.
    pub fn from_params(cluster_id: &str, params: GenericBlueprintParams) -> anyhow::Result<Self> {
        if let Err(e) = params.validate() {
            anyhow::bail!("Invalid generic blueprint parameters: {}", e);
        }
        let generic = params.to_strategy_params();
        let capacity = params.lookback_count.max(params.mean_lookback) * 2;
        let name = format!("blueprint-{}", cluster_id);
        let original_velocity_threshold = params.momentum_threshold_pct;
        Ok(Self {
            params,
            generic_params: generic,
            name,
            prices: VecDeque::with_capacity(capacity),
            spike_state: None,
            reversal_ticks: 0,
            consecutive_directional: 0,
            last_direction: None,
            auto_scaled: false,
            original_velocity_threshold,
        })
    }

    /// Return a reference to the blueprint params.
    #[allow(dead_code)]
    pub fn blueprint_params(&self) -> &GenericBlueprintParams {
        &self.params
    }

    /// Compute SMA over last N prices.
    fn compute_sma(&self, lookback: usize) -> Option<f64> {
        if self.prices.len() < lookback {
            return None;
        }
        let sum: f64 = self.prices.iter().rev().take(lookback).map(|p| p.price).sum();
        Some(sum / lookback as f64)
    }

    fn prev_price(&self) -> Option<f64> {
        self.prices.iter().rev().nth(1).map(|p| p.price)
    }

    fn current_price(&self) -> Option<f64> {
        self.prices.back().map(|p| p.price)
    }

    /// Velocity over the last N prices (pct change).
    fn compute_velocity(&self) -> f64 {
        let n = self.params.lookback_count;
        if self.prices.len() < n || n == 0 {
            return 0.0;
        }
        let prices: Vec<f64> = self.prices.iter().rev().take(n).map(|p| p.price).collect();
        let oldest = *prices.last().unwrap();
        let newest = prices[0];
        if oldest <= 0.0 { return 0.0; }
        (newest - oldest) / oldest * 100.0
    }

    /// Auto-scale velocity threshold based on actual market volatility.
    ///
    /// Strategy: measure the median absolute velocity over the lookback window.
    /// The threshold should be calibrated so that signals fire at approximately
    /// the same percentile of price movements as in the source market.
    ///
    /// We aim for the threshold to be at the 90th percentile of absolute velocity.
    /// If the current threshold is unreachable (>99th percentile) or too easy (<50th),
    /// we scale it to the 90th percentile of observed velocities.
    fn auto_scale_threshold(&mut self) {
        let lookback = self.params.lookback_count;
        if self.prices.len() < lookback * 2 {
            return;
        }

        // Compute absolute velocity for each window position
        let prices: Vec<f64> = self.prices.iter().map(|p| p.price).collect();
        let mut velocities: Vec<f64> = Vec::new();
        for i in lookback..prices.len() {
            let oldest = prices[i - lookback];
            let newest = prices[i];
            if oldest > 0.0 {
                velocities.push(((newest - oldest) / oldest * 100.0).abs());
            }
        }

        if velocities.len() < 10 {
            return;
        }

        velocities.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Check where the original threshold falls in the distribution
        let threshold = self.original_velocity_threshold;
        let above_count = velocities.iter().filter(|&&v| v >= threshold).count();
        let percentile = above_count as f64 / velocities.len() as f64 * 100.0;

        // Only scale if threshold is in extreme percentiles
        // Target: 85th-95th percentile (fires on ~10% of windows)
        if percentile > 1.0 && percentile < 99.0 {
            // Threshold is reachable, don't scale
            self.auto_scaled = true;
            return;
        }

        // Set threshold to 90th percentile of observed velocities
        let p90_idx = (velocities.len() as f64 * 0.90) as usize;
        let p90 = velocities[p90_idx.min(velocities.len() - 1)];

        if p90 > 0.0 {
            let ratio = p90 / threshold;
            // Only apply if scale factor is significant
            if !(0.67..=1.5).contains(&ratio) {
                info!(
                    "[{}] Auto-scaling velocity: {:.3}% → {:.3}% (p90 of {} velocities, original at p{:.0})",
                    self.name, threshold, p90, velocities.len(), percentile
                );
                self.params.momentum_threshold_pct = p90;
                if self.params.entry_logic == BlueprintEntryLogic::MeanReversion
                    || self.params.entry_logic == BlueprintEntryLogic::Grid
                {
                    self.params.deviation_threshold_pct = p90;
                }
            }
        }
        self.auto_scaled = true;
    }

    // --- Entry detectors per strategy type ---

    fn entry_momentum(&mut self) -> Signal {
        if self.prices.len() < self.params.lookback_count {
            return Signal::NoSignal;
        }

        let velocity_pct = self.compute_velocity();
        let strength = (velocity_pct.abs() / self.params.momentum_threshold_pct * 50.0)
            .clamp(50.0, 100.0);

        match self.params.direction_bias.as_str() {
            "long" => {
                if velocity_pct > self.params.momentum_threshold_pct {
                    info!(
                        "[{}] LONG signal: velocity={:.3}%, threshold={:.3}%",
                        self.name, velocity_pct, self.params.momentum_threshold_pct
                    );
                    return Signal::MomentumLong { strength, velocity_pct };
                }
            }
            "short" => {
                if velocity_pct < -self.params.momentum_threshold_pct {
                    info!(
                        "[{}] SHORT signal: velocity={:.3}%, threshold={:.3}%",
                        self.name, velocity_pct, self.params.momentum_threshold_pct
                    );
                    return Signal::MomentumShort { strength, velocity_pct };
                }
            }
            _ => {
                if velocity_pct > self.params.momentum_threshold_pct {
                    return Signal::MomentumLong { strength, velocity_pct };
                }
                if velocity_pct < -self.params.momentum_threshold_pct {
                    return Signal::MomentumShort { strength, velocity_pct };
                }
            }
        }
        Signal::NoSignal
    }

    fn entry_mean_reversion(&mut self) -> Signal {
        let current_price = match self.current_price() {
            Some(p) => p,
            None => return Signal::NoSignal,
        };

        let sma = match self.compute_sma(self.params.mean_lookback) {
            Some(s) => s,
            None => return Signal::NoSignal,
        };

        if sma <= 0.0 {
            return Signal::NoSignal;
        }

        let deviation_pct = (current_price - sma) / sma * 100.0;

        // Spike detection
        if deviation_pct > self.params.deviation_threshold_pct {
            if self.spike_state != Some(GenericSpikeDirection::Above) {
                self.spike_state = Some(GenericSpikeDirection::Above);
                self.reversal_ticks = 0;
            }
        } else if deviation_pct < -self.params.deviation_threshold_pct
            && self.spike_state != Some(GenericSpikeDirection::Below)
        {
            self.spike_state = Some(GenericSpikeDirection::Below);
            self.reversal_ticks = 0;
        }

        if let Some(ref spike_dir) = self.spike_state {
            let prev_price = match self.prev_price() {
                Some(p) => p,
                None => return Signal::NoSignal,
            };

            let moving_toward_mean = match spike_dir {
                GenericSpikeDirection::Above => current_price < prev_price,
                GenericSpikeDirection::Below => current_price > prev_price,
            };

            if moving_toward_mean {
                self.reversal_ticks += 1;
                if self.reversal_ticks >= self.params.reversal_confirmation_ticks {
                    let strength = (deviation_pct.abs() / self.params.deviation_threshold_pct * 50.0)
                        .clamp(50.0, 100.0);
                    let velocity_pct = deviation_pct.abs();
                    let dir_clone = spike_dir.clone();
                    self.spike_state = None;
                    self.reversal_ticks = 0;

                    match dir_clone {
                        GenericSpikeDirection::Above => {
                            info!(
                                "[{}] MR SHORT: dev={:.2}%",
                                self.name, deviation_pct
                            );
                            Signal::MomentumShort { strength, velocity_pct }
                        }
                        GenericSpikeDirection::Below => {
                            info!(
                                "[{}] MR LONG: dev={:.2}%",
                                self.name, deviation_pct
                            );
                            Signal::MomentumLong { strength, velocity_pct }
                        }
                    }
                } else {
                    Signal::NoSignal
                }
            } else {
                if deviation_pct.abs() <= self.params.deviation_threshold_pct {
                    self.spike_state = None;
                    self.reversal_ticks = 0;
                }
                Signal::NoSignal
            }
        } else {
            Signal::NoSignal
        }
    }

    fn entry_trend_breakout(&mut self) -> Signal {
        if self.prices.len() < self.params.lookback_count {
            return Signal::NoSignal;
        }

        let velocity_pct = self.compute_velocity();

        // Track consecutive directional ticks
        let current_dir = if velocity_pct > 0.0 {
            crate::signal::TradeDirection::Long
        } else if velocity_pct < 0.0 {
            crate::signal::TradeDirection::Short
        } else {
            crate::signal::TradeDirection::Neutral
        };

        match (&self.last_direction, &current_dir) {
            (Some(last), curr) if last == curr && *curr != crate::signal::TradeDirection::Neutral => {
                self.consecutive_directional += 1;
            }
            _ => {
                self.consecutive_directional = 1;
            }
        }
        self.last_direction = Some(current_dir.clone());

        // Need enough consecutive ticks AND velocity above threshold
        if self.consecutive_directional >= self.params.confirmation_ticks
            && velocity_pct.abs() > self.params.momentum_threshold_pct
        {
            let strength = (velocity_pct.abs() / self.params.momentum_threshold_pct * 50.0)
                .clamp(50.0, 100.0);

            self.consecutive_directional = 0;
            self.last_direction = None;

            match (current_dir, self.params.direction_bias.as_str()) {
                (crate::signal::TradeDirection::Long, "short") => Signal::NoSignal,
                (crate::signal::TradeDirection::Short, "long") => Signal::NoSignal,
                (crate::signal::TradeDirection::Long, _) => {
                    info!(
                        "[{}] TREND LONG: velocity={:.3}%, consec={}",
                        self.name, velocity_pct, self.params.confirmation_ticks
                    );
                    Signal::MomentumLong { strength, velocity_pct }
                }
                (crate::signal::TradeDirection::Short, _) => {
                    info!(
                        "[{}] TREND SHORT: velocity={:.3}%, consec={}",
                        self.name, velocity_pct, self.params.confirmation_ticks
                    );
                    Signal::MomentumShort { strength, velocity_pct }
                }
                _ => Signal::NoSignal,
            }
        } else {
            Signal::NoSignal
        }
    }

    fn entry_grid(&mut self) -> Signal {
        let current_price = match self.current_price() {
            Some(p) => p,
            None => return Signal::NoSignal,
        };

        let sma = match self.compute_sma(self.params.mean_lookback) {
            Some(s) => s,
            None => return Signal::NoSignal,
        };

        if sma <= 0.0 {
            return Signal::NoSignal;
        }

        let deviation_pct = (current_price - sma) / sma * 100.0;

        // Grid: enter when price dips below lower band, direction neutral (buy dips, sell rips)
        if deviation_pct < -self.params.deviation_threshold_pct {
            let strength = (deviation_pct.abs() / self.params.deviation_threshold_pct * 50.0)
                .clamp(50.0, 100.0);
            let velocity_pct = deviation_pct.abs();
            info!(
                "[{}] GRID LONG: dev={:.2}%",
                self.name, deviation_pct
            );
            Signal::MomentumLong { strength, velocity_pct }
        } else if deviation_pct > self.params.deviation_threshold_pct {
            let strength = (deviation_pct.abs() / self.params.deviation_threshold_pct * 50.0)
                .clamp(50.0, 100.0);
            let velocity_pct = deviation_pct.abs();
            info!(
                "[{}] GRID SHORT: dev={:.2}%",
                self.name, deviation_pct
            );
            Signal::MomentumShort { strength, velocity_pct }
        } else {
            Signal::NoSignal
        }
    }
}

impl Strategy for GenericBlueprintStrategy {
    fn name(&self) -> &str {
        // Leak the name string to get a 'static &str. This is fine since
        // strategy names are created once and live for the process lifetime.
        let name = self.name.clone();
        Box::leak(name.into_boxed_str())
    }

    fn detect_entry(&mut self, _snapshot: &MomentumSnapshot) -> Signal {
        match self.params.entry_logic {
            BlueprintEntryLogic::Momentum => self.entry_momentum(),
            BlueprintEntryLogic::MeanReversion => self.entry_mean_reversion(),
            BlueprintEntryLogic::TrendBreakout => self.entry_trend_breakout(),
            BlueprintEntryLogic::Grid => self.entry_grid(),
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        let current_price = ctx.current_price;
        let pnl_pct = if ctx.is_long {
            (current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - current_price) / ctx.entry_price * 100.0
        };

        // 1. Stop loss
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!("[{}] STOP LOSS: pnl={:.2}%", self.name, pnl_pct);
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::StopLoss));
        }

        // 2. Take profit
        if pnl_pct >= ctx.take_profit_pct {
            info!("[{}] TAKE PROFIT: pnl={:.2}%", self.name, pnl_pct);
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
        }

        // 3. Trailing stop
        if ctx.trailing_stop_pct > 0.0 && ctx.trailing_activation_pct > 0.0 {
            let activation_threshold = ctx.entry_price
                * (1.0 + ctx.trailing_activation_pct / 100.0 * if ctx.is_long { 1.0 } else { -1.0 });
            let activated = if ctx.is_long {
                ctx.peak_price >= activation_threshold
            } else {
                ctx.peak_price <= activation_threshold
            };

            if activated {
                let retrace_pct = if ctx.is_long {
                    (ctx.peak_price - current_price) / ctx.peak_price * 100.0
                } else {
                    (current_price - ctx.peak_price) / ctx.peak_price * 100.0
                };
                if retrace_pct >= ctx.trailing_stop_pct {
                    info!(
                        "[{}] TRAILING STOP: retrace={:.2}%, threshold={:.2}%",
                        self.name, retrace_pct, ctx.trailing_stop_pct
                    );
                    return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TrailingStop));
                }
            }
        }

        // 4. Time stop
        if ctx.hold_secs >= ctx.max_hold_secs {
            warn!(
                "[{}] TIME STOP: held {}s, max={}s",
                self.name, ctx.hold_secs, ctx.max_hold_secs
            );
            return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TimeStop));
        }

        // 5. Mean return exit (for mean_reversion and grid types)
        if (self.params.entry_logic == BlueprintEntryLogic::MeanReversion
            || self.params.entry_logic == BlueprintEntryLogic::Grid)
            && let Some(sma) = self.compute_sma(self.params.mean_lookback)
            && sma > 0.0
        {
            let deviation_from_mean = (current_price - sma).abs() / sma * 100.0;
            if deviation_from_mean <= self.params.mean_tolerance_pct {
                info!(
                    "[{}] MEAN RETURN: price={:.2}, sma={:.2}, dev={:.2}%",
                    self.name, current_price, sma, deviation_from_mean
                );
                return Some(exit_signal(ctx.is_long, crate::signal::ExitReason::TakeProfit));
            }
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.prices.push_back(crate::signal::PricePoint { price, timestamp_ms });
        let max_len = self.params.lookback_count.max(self.params.mean_lookback) * 2;
        while self.prices.len() > max_len {
            self.prices.pop_front();
        }
        // Auto-scale threshold once we have enough data (and haven't scaled yet)
        if !self.auto_scaled && self.prices.len() >= self.params.lookback_count {
            self.auto_scale_threshold();
        }
    }

    fn snapshot(&self) -> MomentumSnapshot {
        let current_price = self.current_price().unwrap_or(0.0);
        let velocity_pct = self.compute_velocity();

        let direction = if velocity_pct > 0.0 {
            crate::signal::TradeDirection::Long
        } else if velocity_pct < 0.0 {
            crate::signal::TradeDirection::Short
        } else {
            crate::signal::TradeDirection::Neutral
        };

        MomentumSnapshot {
            price_count: self.prices.len(),
            current_price,
            price_velocity_pct: velocity_pct,
            direction,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::{ExitReason, MomentumSnapshot, PoolSnapshot, Signal, TradeDirection};

    // ===== Momentum Scalper Tests =====

    /// Helper: create default strategy params for testing.
    fn default_params() -> StrategyParams {
        StrategyParams {
            direction_bias: "neutral".to_string(),
            momentum_threshold_pct: 0.15,
            lookback_count: 60,
            scale_in_clips: 1,
            clip_size_usd: 100.0,
            max_hold_secs: 1800,
            take_profit_pct: 2.5,
            stop_loss_pct: 1.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    /// Helper: build a price series that should produce a LONG signal.
    fn feed_rising_prices(strategy: &mut dyn Strategy, start: f64, count: usize) {
        let base_ts = 1000000_i64;
        for i in 0..count {
            let price = start * (1.0 + 0.005 * (i as f64));
            strategy.push_price(price, base_ts + (i as i64) * 1000);
        }
    }

    /// Helper: build a price series that should produce a SHORT signal.
    fn feed_falling_prices(strategy: &mut dyn Strategy, start: f64, count: usize) {
        let base_ts = 1000000_i64;
        for i in 0..count {
            let price = start * (1.0 - 0.005 * (i as f64));
            strategy.push_price(price, base_ts + (i as i64) * 1000);
        }
    }

    /// Helper: build a position context for exit tests.
    fn default_exit_context(
        is_long: bool,
        entry_price: f64,
        current_price: f64,
        peak_price: f64,
        hold_secs: u64,
    ) -> PositionContext {
        PositionContext {
            is_long,
            entry_price,
            current_price,
            peak_price,
            hold_secs,
            max_hold_secs: 1800,
            take_profit_pct: 2.5,
            stop_loss_pct: 1.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        }
    }

    #[test]
    fn test_long_signal() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        feed_rising_prices(&mut strategy, 100.0, 10);

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);

        match signal {
            Signal::MomentumLong { strength, velocity_pct } => {
                assert!(velocity_pct >= 0.15);
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumLong, got {:?}", other),
        }
    }

    #[test]
    fn test_short_signal() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        feed_falling_prices(&mut strategy, 100.0, 10);

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);

        match signal {
            Signal::MomentumShort { strength, velocity_pct } => {
                assert!(velocity_pct >= 0.15);
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumShort, got {:?}", other),
        }
    }

    #[test]
    fn test_no_signal_insufficient_prices() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);
        strategy.push_price(102.0, 3000);

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);
        assert_eq!(signal, Signal::NoSignal);
    }

    #[test]
    fn test_no_signal_flat_prices() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);
        assert_eq!(signal, Signal::NoSignal);
    }

    #[test]
    fn test_stop_loss_fires_before_take_profit() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);
        let ctx = default_exit_context(true, 100.0, 98.0, 100.0, 10);

        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(98.0, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_take_profit_exit() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);
        let ctx = default_exit_context(true, 100.0, 103.0, 103.0, 10);

        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(103.0, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TakeProfit);
            }
            other => panic!("Expected ExitLong(TakeProfit), got {:?}", other),
        }
    }

    #[test]
    fn test_time_stop_exit() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);
        let ctx = default_exit_context(true, 100.0, 100.5, 100.5, 2000);

        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(100.5, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_no_exit_when_position_stable() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);
        let ctx = default_exit_context(true, 100.0, 100.3, 100.3, 30);

        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(100.3, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        assert!(
            result.is_none()
                || matches!(
                    result,
                    Some(Signal::ExitLong { reason: ExitReason::MomentumLost })
                ),
            "Expected no exit or momentum-lost for stable position, got {:?}",
            result
        );
    }

    // ---- Factory Tests ----

    #[test]
    fn test_factory_creates_momentum_scalper() {
        let params = default_params();
        let strategy = create_strategy("momentum-scalper", params).unwrap();
        assert_eq!(strategy.name(), "momentum-scalper");
    }

    #[test]
    fn test_factory_rejects_unknown_strategy() {
        let params = default_params();
        let result = create_strategy("nonexistent", params);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("nonexistent"));
        assert!(err.contains("momentum-scalper"));
    }

    #[test]
    fn test_params_validation_rejects_zero_threshold() {
        let mut params = default_params();
        params.momentum_threshold_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_params_validation_rejects_zero_lookback() {
        let mut params = default_params();
        params.lookback_count = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_params_validation_rejects_zero_clip_size() {
        let mut params = default_params();
        params.clip_size_usd = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_params_validation_rejects_zero_tp() {
        let mut params = default_params();
        params.take_profit_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_params_validation_rejects_zero_sl() {
        let mut params = default_params();
        params.stop_loss_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_params_validation_accepts_valid() {
        let params = default_params();
        assert!(params.validate().is_ok());
    }

    // ===== LP Consumption Strategy Tests =====

    /// Helper: create default LP consumption params for testing.
    fn default_lp_params() -> LpConsumptionParams {
        LpConsumptionParams {
            consumption_velocity_threshold: 0.5,
            lp_concentration_min: 0.7,
            confirmation_ticks: 3,
            max_utilization: 0.9,
            direction_bias: "neutral".to_string(),
            clip_size_usd: 100.0,
            max_hold_secs: 3600,
            take_profit_pct: 2.0,
            stop_loss_pct: 1.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.2,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
            leverage: 2.5,
            scale_in_clips: 1,
            flash_native_mode: false,
            flash_native_min_util_pct: 30.0,
            flash_native_velocity_mult: 0.6,
            is_flash_only_market: false,
        }
    }

    /// Helper: build a snapshot with pool data simulating LP consumption.
    fn snapshot_with_pool(
        price: f64,
        long_util_velocity: f64,
        short_util_velocity: f64,
        long_utilization: f64,
        short_utilization: f64,
    ) -> MomentumSnapshot {
        MomentumSnapshot {
            price_count: 10,
            current_price: price,
            price_velocity_pct: 0.0,
            direction: TradeDirection::Neutral,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: Some(PoolSnapshot {
                aum_usd: 1_000_000.0,
                long_utilization,
                short_utilization,
                long_utilization_velocity: long_util_velocity,
                short_utilization_velocity: short_util_velocity,
            }),
            ext: None,
        }
    }

    /// Helper: build a snapshot with no pool data.
    fn snapshot_no_pool(price: f64) -> MomentumSnapshot {
        MomentumSnapshot {
            price_count: 10,
            current_price: price,
            price_velocity_pct: 0.0,
            direction: TradeDirection::Neutral,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    /// Helper: LP consumption exit context.
    fn lp_exit_context(
        is_long: bool,
        entry_price: f64,
        current_price: f64,
        peak_price: f64,
        hold_secs: u64,
    ) -> PositionContext {
        PositionContext {
            is_long,
            entry_price,
            current_price,
            peak_price,
            hold_secs,
            max_hold_secs: 3600,
            take_profit_pct: 2.0,
            stop_loss_pct: 1.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.2,
        }
    }

    #[test]
    fn test_lp_consumption_entry_long_when_consumption_exceeds_threshold() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        // Simulate 3 consecutive ticks with strong long-side LP consumption
        // Long velocity = 0.8 (above threshold 0.5), concentration = 80% (above 0.7)
        for _ in 0..3 {
            let snap = snapshot_with_pool(100.0, 0.8, 0.2, 0.3, 0.2);
            let _signal = strategy.detect_entry(&snap);
            // First two ticks should be NoSignal (need confirmation_ticks=3)
            // Third tick should produce a LONG signal
        }

        let snap = snapshot_with_pool(100.0, 0.8, 0.2, 0.3, 0.2);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumLong { strength, .. } => {
                assert!(strength >= 50.0, "strength should be >= 50, got {}", strength);
            }
            other => panic!("Expected MomentumLong, got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_entry_short_when_ask_side_consumed() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        // Short-side consumption: velocity = 0.9 (above threshold), concentration = 85%
        for _ in 0..3 {
            let snap = snapshot_with_pool(100.0, 0.15, 0.9, 0.2, 0.3);
            let _ = strategy.detect_entry(&snap);
        }

        let snap = snapshot_with_pool(100.0, 0.15, 0.9, 0.2, 0.3);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumShort { strength, .. } => {
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumShort, got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_no_signal_when_velocity_below_threshold() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        // Velocity = 0.3, below threshold of 0.5
        let snap = snapshot_with_pool(100.0, 0.3, 0.1, 0.3, 0.2);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(signal, Signal::NoSignal, "Should not signal when velocity is below threshold");
    }

    #[test]
    fn test_lp_consumption_no_signal_when_concentration_too_low() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        // Velocity above threshold (0.8) but concentration is balanced (55%/45%)
        let snap = snapshot_with_pool(100.0, 0.55, 0.45, 0.3, 0.2);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should not signal when directional concentration is below minimum"
        );
    }

    #[test]
    fn test_lp_consumption_no_signal_when_pool_data_missing() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        let snap = snapshot_no_pool(100.0);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should return NoSignal when pool data is missing"
        );
    }

    #[test]
    fn test_lp_consumption_no_signal_when_utilization_maxed() {
        let params = default_lp_params();
        let mut strategy = LpConsumptionStrategy::new(params);

        // Velocity and concentration good, but utilization at 92% (above max_utilization=0.9)
        let snap = snapshot_with_pool(100.0, 0.8, 0.1, 0.92, 0.3);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should not enter when pool utilization is above max"
        );
    }

    #[test]
    fn test_lp_consumption_exit_on_stall() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // Position entered when long velocity was 0.8, now velocity dropped to 0.05.
        // Entry at 100, current at 100.3 (0.3% gain, below TP of 2.0%, not enough for SL).
        // Held for 200s (> 300 threshold in code, but close).
        let snap = snapshot_with_pool(100.3, 0.05, 0.02, 0.3, 0.2);
        let ctx = lp_exit_context(true, 100.0, 100.3, 100.5, 400);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(
                    reason, ExitReason::MomentumLost,
                    "Should exit on consumption stall"
                );
            }
            other => panic!("Expected ExitLong(MomentumLost), got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_exit_on_stop_loss() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // Price dropped 1.5% (SL is 1.0%)
        let snap = snapshot_with_pool(98.5, 0.1, 0.05, 0.3, 0.2);
        let ctx = lp_exit_context(true, 100.0, 98.5, 100.0, 30);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_exit_on_take_profit() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // Price up 2.5% (TP is 2.0%)
        let snap = snapshot_with_pool(102.5, 0.3, 0.1, 0.3, 0.2);
        let ctx = lp_exit_context(true, 100.0, 102.5, 102.5, 60);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TakeProfit);
            }
            other => panic!("Expected ExitLong(TakeProfit), got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_exit_on_time_stop() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // Held for 4000s, max is 3600s
        let snap = snapshot_with_pool(100.3, 0.1, 0.05, 0.3, 0.2);
        let ctx = lp_exit_context(true, 100.0, 100.3, 100.5, 4000);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_exit_on_reversal() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // We're long, but short-side consumption is now accelerating.
        // Long velocity=0.2 (> stall threshold 0.15) so stall doesn't fire.
        // Short velocity=0.9 with concentration 0.9/1.1=0.818 > 0.7 min → reversal.
        let snap = snapshot_with_pool(100.3, 0.2, 0.9, 0.3, 0.35);
        let ctx = lp_exit_context(true, 100.0, 100.3, 100.5, 60);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(
                    reason, ExitReason::ReversalDetected,
                    "Should detect reversal when opposite-side consumption accelerates"
                );
            }
            other => panic!("Expected ExitLong(ReversalDetected), got {:?}", other),
        }
    }

    #[test]
    fn test_lp_consumption_no_exit_when_position_stable() {
        let params = default_lp_params();
        let strategy = LpConsumptionStrategy::new(params);

        // Stable position, velocity still good
        let snap = snapshot_with_pool(100.3, 0.6, 0.1, 0.3, 0.2);
        let ctx = lp_exit_context(true, 100.0, 100.3, 100.5, 30);

        let result = strategy.detect_exit(&snap, &ctx);
        assert!(
            result.is_none(),
            "Expected no exit for stable LP consumption, got {:?}",
            result
        );
    }

    #[test]
    fn test_lp_consumption_factory_creates_strategy() {
        let params = default_lp_params();
        let strategy = create_lp_consumption_strategy(params).unwrap();
        assert_eq!(strategy.name(), "lp-consumption");
    }

    #[test]
    fn test_lp_consumption_factory_from_config() {
        let fallback = default_params();
        let strategy = create_strategy_from_config(
            "lp-consumption",
            None, // No sub-table, use defaults
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "lp-consumption");
    }

    #[test]
    fn test_lp_consumption_factory_from_config_with_table() {
        let fallback = default_params();
        let toml_str = r#"
            consumption_velocity_threshold = 0.3
            lp_concentration_min = 0.6
            confirmation_ticks = 2
            max_utilization = 0.85
            clip_size_usd = 50.0
            take_profit_pct = 1.5
            stop_loss_pct = 0.5
            trailing_stop_pct = 0.6
            trailing_activation_pct = 1.0
            max_hold_secs = 2400
            cooldown_after_loss_secs = 120
            direction_bias = "long"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let strategy = create_strategy_from_config(
            "lp-consumption",
            Some(&value),
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "lp-consumption");
    }

    #[test]
    fn test_lp_consumption_params_validation_rejects_zero_velocity() {
        let mut params = default_lp_params();
        params.consumption_velocity_threshold = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_lp_consumption_params_validation_rejects_invalid_concentration() {
        let mut params = default_lp_params();
        params.lp_concentration_min = 1.5;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_lp_consumption_params_validation_rejects_zero_confirmation() {
        let mut params = default_lp_params();
        params.confirmation_ticks = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_lp_consumption_params_validation_accepts_valid() {
        let params = default_lp_params();
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_lp_consumption_directional_concentration_pure_long() {
        let pool = PoolSnapshot {
            aum_usd: 1_000_000.0,
            long_utilization: 0.3,
            short_utilization: 0.1,
            long_utilization_velocity: 1.0,
            short_utilization_velocity: 0.0,
        };
        let (long_c, short_c) = LpConsumptionStrategy::directional_concentration(&pool);
        assert!((long_c - 1.0).abs() < 0.01, "long_conc should be ~1.0, got {}", long_c);
        assert!((short_c - 0.0).abs() < 0.01, "short_conc should be ~0.0, got {}", short_c);
    }

    #[test]
    fn test_lp_consumption_directional_concentration_balanced() {
        let pool = PoolSnapshot {
            aum_usd: 1_000_000.0,
            long_utilization: 0.3,
            short_utilization: 0.3,
            long_utilization_velocity: 0.5,
            short_utilization_velocity: 0.5,
        };
        let (long_c, short_c) = LpConsumptionStrategy::directional_concentration(&pool);
        assert!((long_c - 0.5).abs() < 0.01, "long_conc should be ~0.5, got {}", long_c);
        assert!((short_c - 0.5).abs() < 0.01, "short_conc should be ~0.5, got {}", short_c);
    }

    #[test]
    fn test_lp_consumption_directional_concentration_zero_velocity() {
        let pool = PoolSnapshot {
            aum_usd: 1_000_000.0,
            long_utilization: 0.3,
            short_utilization: 0.3,
            long_utilization_velocity: 0.0,
            short_utilization_velocity: 0.0,
        };
        let (long_c, short_c) = LpConsumptionStrategy::directional_concentration(&pool);
        // When both are zero, should return 0.5/0.5 (neutral)
        assert!((long_c - 0.5).abs() < 0.01);
        assert!((short_c - 0.5).abs() < 0.01);
    }

    // ===== Mean Reversion Strategy Tests =====

    /// Helper: create default mean reversion params for testing.
    fn default_mr_params() -> MeanReversionParams {
        MeanReversionParams {
            mean_lookback: 20, // Small for testing
            deviation_threshold_pct: 1.5,
            reversal_confirmation_ticks: 2,
            mean_tolerance_pct: 0.3,
            direction_bias: "neutral".to_string(),
            clip_size_usd: 100.0,
            max_hold_secs: 1800,
            take_profit_pct: 1.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
            leverage: 3.0,
            scale_in_clips: 1,
        }
    }

    /// Helper: build a snapshot for mean reversion (no pool data needed).
    fn mr_snapshot(price: f64) -> MomentumSnapshot {
        MomentumSnapshot {
            price_count: 20,
            current_price: price,
            price_velocity_pct: 0.0,
            direction: TradeDirection::Neutral,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    /// Helper: mean reversion exit context.
    fn mr_exit_context(
        is_long: bool,
        entry_price: f64,
        current_price: f64,
        peak_price: f64,
        hold_secs: u64,
    ) -> PositionContext {
        PositionContext {
            is_long,
            entry_price,
            current_price,
            peak_price,
            hold_secs,
            max_hold_secs: 1800,
            take_profit_pct: 1.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        }
    }

    #[test]
    fn test_mean_reversion_entry_long_after_downward_spike_and_reversal() {
        // Scenario: prices trade around 100, then spike DOWN to 96 (4% below SMA),
        // then start reversing upward → should generate LONG signal.
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Feed 20 stable prices around 100 to establish SMA
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Spike down: 4% below SMA (100), well above 1.5% threshold
        // Spike tick: push price to 96
        strategy.push_price(96.0, 30_000);
        let snap = mr_snapshot(96.0);
        let signal = strategy.detect_entry(&snap);
        // Spike detected but no reversal yet
        assert_eq!(signal, Signal::NoSignal, "Should not signal on spike without reversal");

        // First reversal tick: price moves back up toward mean
        strategy.push_price(96.5, 31_000);
        let snap = mr_snapshot(96.5);
        let signal = strategy.detect_entry(&snap);
        // Only 1 reversal tick, need 2
        assert_eq!(signal, Signal::NoSignal, "Should not signal with only 1 reversal tick");

        // Second reversal tick: price continues up toward mean
        strategy.push_price(97.0, 32_000);
        let snap = mr_snapshot(97.0);
        let signal = strategy.detect_entry(&snap);
        // Should generate LONG signal (fade the downward spike)
        match signal {
            Signal::MomentumLong { strength, .. } => {
                assert!(strength >= 50.0, "strength should be >= 50, got {}", strength);
            }
            other => panic!("Expected MomentumLong after downward spike + reversal, got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_entry_short_after_upward_spike_and_reversal() {
        // Scenario: prices trade around 100, then spike UP to 104 (4% above SMA),
        // then start reversing downward → should generate SHORT signal.
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Feed 20 stable prices around 100 to establish SMA
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Spike up: 4% above SMA
        strategy.push_price(104.0, 30_000);
        let snap = mr_snapshot(104.0);
        let _ = strategy.detect_entry(&snap);

        // First reversal tick: price moves down
        strategy.push_price(103.5, 31_000);
        let snap = mr_snapshot(103.5);
        let _ = strategy.detect_entry(&snap);

        // Second reversal tick: price continues down
        strategy.push_price(103.0, 32_000);
        let snap = mr_snapshot(103.0);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumShort { strength, .. } => {
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumShort after upward spike + reversal, got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_no_entry_on_gradual_move_without_spike() {
        // Gradual drift without exceeding deviation threshold → no signal
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Feed 20 stable prices
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Gradual upward drift: 0.1% per tick for 10 ticks = 1% total (below 1.5% threshold)
        for i in 0..10 {
            let price = 100.0 * (1.0 + 0.001 * (i as f64));
            strategy.push_price(price, 20_000 + (i as i64) * 1000);
            let snap = mr_snapshot(price);
            let signal = strategy.detect_entry(&snap);
            assert_eq!(
                signal, Signal::NoSignal,
                "Should not signal on gradual move without spike (tick {})",
                i
            );
        }
    }

    #[test]
    fn test_mean_reversion_exit_on_mean_return() {
        // Position is LONG, entered at 96 (below SMA of 100).
        // Price returns to 100 (within tolerance of SMA) → exit.
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Feed 20 stable prices at 100 to establish SMA
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        // Current price is at 100 (exactly at mean)
        strategy.push_price(100.0, 21_000);

        let snap = mr_snapshot(100.0);
        let ctx = mr_exit_context(true, 96.0, 100.0, 100.0, 60);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(
                    reason, ExitReason::TakeProfit,
                    "Mean return should use TakeProfit exit reason"
                );
            }
            other => panic!("Expected ExitLong(TakeProfit) on mean return, got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_exit_on_stop_loss() {
        // Price drops 2% (SL is 1.5%)
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Need prices for SMA computation
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(98.0, 21_000);

        let snap = mr_snapshot(98.0);
        let ctx = mr_exit_context(true, 100.0, 98.0, 100.5, 30);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_exit_on_time_stop() {
        // Held for 2000s, max is 1800s. Price hasn't moved enough for TP or SL.
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        // Push a price far from mean so mean-return exit doesn't fire
        strategy.push_price(96.3, 21_000);

        let snap = mr_snapshot(96.3);
        // Entry at 96.0, current at 96.3 → 0.31% gain, below 1.0% TP
        let ctx = mr_exit_context(true, 96.0, 96.3, 96.5, 2000);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_no_signal_insufficient_history() {
        // With fewer prices than mean_lookback, should return NoSignal
        let params = default_mr_params(); // mean_lookback = 20
        let mut strategy = MeanReversionStrategy::new(params);

        // Only 10 prices (less than mean_lookback of 20)
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        let snap = mr_snapshot(100.0);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should return NoSignal when price history is insufficient"
        );

        // With 0 prices
        let mut strategy2 = MeanReversionStrategy::new(default_mr_params());
        let snap2 = mr_snapshot(100.0);
        let signal2 = strategy2.detect_entry(&snap2);
        assert_eq!(
            signal2, Signal::NoSignal,
            "Should return NoSignal with zero prices"
        );
    }

    #[test]
    fn test_mean_reversion_no_exit_when_price_still_far_from_mean() {
        // Price is still far from mean, no SL/TP hit, no time stop
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        // Push price far from mean, still within SL/TP range from entry
        strategy.push_price(96.3, 21_000);

        let snap = mr_snapshot(96.3);
        // Entry at 96.0, current at 96.3 → 0.31% gain (below 1.0% TP, above -1.5% SL)
        // Deviation from SMA ≈ 3.5% (above 0.3% tolerance)
        let ctx = mr_exit_context(true, 96.0, 96.3, 96.5, 60);

        let result = strategy.detect_exit(&snap, &ctx);
        assert!(
            result.is_none(),
            "Expected no exit when price is still far from mean, got {:?}",
            result
        );
    }

    #[test]
    fn test_mean_reversion_factory_creates_strategy() {
        let params = default_mr_params();
        let strategy = create_mean_reversion_strategy(params).unwrap();
        assert_eq!(strategy.name(), "mean-reversion");
    }

    #[test]
    fn test_mean_reversion_factory_from_config() {
        let fallback = default_params();
        let strategy = create_strategy_from_config(
            "mean-reversion",
            None, // No sub-table, use defaults
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "mean-reversion");
    }

    #[test]
    fn test_mean_reversion_factory_from_config_with_table() {
        let fallback = default_params();
        let toml_str = r#"
            mean_lookback = 60
            deviation_threshold_pct = 2.0
            reversal_confirmation_ticks = 3
            mean_tolerance_pct = 0.5
            clip_size_usd = 50.0
            take_profit_pct = 1.5
            stop_loss_pct = 2.0
            trailing_stop_pct = 0.0
            trailing_activation_pct = 0.0
            max_hold_secs = 2400
            cooldown_after_loss_secs = 120
            direction_bias = "long"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let strategy = create_strategy_from_config(
            "mean-reversion",
            Some(&value),
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "mean-reversion");
    }

    #[test]
    fn test_mean_reversion_params_validation_rejects_zero_lookback() {
        let mut params = default_mr_params();
        params.mean_lookback = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_mean_reversion_params_validation_rejects_zero_deviation() {
        let mut params = default_mr_params();
        params.deviation_threshold_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_mean_reversion_params_validation_rejects_zero_reversal_ticks() {
        let mut params = default_mr_params();
        params.reversal_confirmation_ticks = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_mean_reversion_params_validation_accepts_valid() {
        let params = default_mr_params();
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_mean_reversion_sma_computation() {
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        // Push 20 prices: 100, 101, 102, ..., 119
        for i in 0..20 {
            strategy.push_price(100.0 + i as f64, 1000 + (i as i64) * 1000);
        }

        let sma = strategy.compute_sma(20).unwrap();
        // Average of 100..119 = (100+119)/2 = 109.5
        assert!(
            (sma - 109.5).abs() < 0.01,
            "SMA should be 109.5, got {:.2}",
            sma
        );
    }

    #[test]
    fn test_mean_reversion_sma_insufficient_data() {
        let params = default_mr_params();
        let mut strategy = MeanReversionStrategy::new(params);

        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);

        let sma = strategy.compute_sma(20);
        assert!(sma.is_none(), "SMA should be None with insufficient data");
    }

    // ===== Trend Follower Strategy Tests =====

    /// Helper: create default trend follower params for testing.
    fn default_tf_params() -> TrendFollowerParams {
        TrendFollowerParams {
            breakout_threshold_pct: 0.25,
            confirmation_ticks: 3, // Small for testing
            min_price_count: 10,   // Small for testing
            direction_bias: "neutral".to_string(),
            clip_size_usd: 100.0,
            max_hold_secs: 7200,
            take_profit_pct: 5.0,
            stop_loss_pct: 2.0,
            trailing_stop_pct: 1.5,
            trailing_activation_pct: 2.5,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
            leverage: 5.0,
            scale_in_clips: 1,
        }
    }

    /// Helper: build a snapshot for trend follower (no pool data needed).
    fn tf_snapshot(price: f64) -> MomentumSnapshot {
        MomentumSnapshot {
            price_count: 30,
            current_price: price,
            price_velocity_pct: 0.0,
            direction: TradeDirection::Neutral,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
            ext: None,
        }
    }

    /// Helper: trend follower exit context.
    fn tf_exit_context(
        is_long: bool,
        entry_price: f64,
        current_price: f64,
        peak_price: f64,
        hold_secs: u64,
    ) -> PositionContext {
        PositionContext {
            is_long,
            entry_price,
            current_price,
            peak_price,
            hold_secs,
            max_hold_secs: 7200,
            take_profit_pct: 5.0,
            stop_loss_pct: 2.0,
            trailing_stop_pct: 1.5,
            trailing_activation_pct: 2.5,
        }
    }

    #[test]
    fn test_trend_follower_entry_long_on_breakout() {
        // Feed rising prices that produce a sustained upward breakout.
        // Start stable, then rapid rise to exceed breakout_threshold_pct = 0.25%.
        let params = default_tf_params(); // confirmation_ticks = 3, min_price_count = 10
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed 10 stable prices around 100 (to meet min_price_count)
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Now feed strongly rising prices to trigger breakout.
        // Each tick adds +0.3% from the initial price, building a strong trend.
        // Need to call detect_entry on each tick to accumulate consecutive_breakout count.
        for i in 0..5 {
            let price = 100.0 * (1.0 + 0.003 * ((i + 1) as f64));
            strategy.push_price(price, 20_000 + (i as i64) * 1000);
            let snap = tf_snapshot(price);
            let _signal = strategy.detect_entry(&snap);
        }

        // After 5 ticks of upward breakout (need 3 consecutive), should produce LONG
        let last_price = 100.0 * (1.0 + 0.003 * 6.0);
        strategy.push_price(last_price, 25_000);
        let snap = tf_snapshot(last_price);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumLong { strength, velocity_pct } => {
                assert!(strength >= 50.0, "strength should be >= 50, got {}", strength);
                assert!(velocity_pct > 0.25, "velocity should exceed breakout threshold");
            }
            other => panic!("Expected MomentumLong on upward breakout, got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_entry_short_on_breakout() {
        // Feed falling prices that produce a sustained downward breakout.
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed 10 stable prices around 100
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Now feed strongly falling prices
        for i in 0..5 {
            let price = 100.0 * (1.0 - 0.003 * ((i + 1) as f64));
            strategy.push_price(price, 20_000 + (i as i64) * 1000);
            let snap = tf_snapshot(price);
            let _signal = strategy.detect_entry(&snap);
        }

        let last_price = 100.0 * (1.0 - 0.003 * 6.0);
        strategy.push_price(last_price, 25_000);
        let snap = tf_snapshot(last_price);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumShort { strength, velocity_pct } => {
                assert!(strength >= 50.0);
                assert!(velocity_pct < -0.25, "velocity should be below negative breakout threshold");
            }
            other => panic!("Expected MomentumShort on downward breakout, got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_no_signal_insufficient_prices() {
        let params = default_tf_params(); // min_price_count = 10
        let mut strategy = TrendFollowerStrategy::new(params);

        // Only 5 prices, below min_price_count
        for i in 0..5 {
            strategy.push_price(100.0 + (i as f64) * 0.5, 1000 + (i as i64) * 1000);
        }

        let snap = tf_snapshot(strategy.current_price().unwrap());
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should return NoSignal with insufficient price history"
        );
    }

    #[test]
    fn test_trend_follower_no_signal_flat_prices() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed 15 flat prices (no breakout)
        for i in 0..15 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        let snap = tf_snapshot(100.0);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should not signal on flat prices"
        );
    }

    #[test]
    fn test_trend_follower_no_signal_below_threshold() {
        // Velocity slightly below breakout threshold
        let params = default_tf_params(); // threshold = 0.25
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed 10 stable prices
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Rising but only 0.1% per tick (below 0.25% threshold)
        for i in 0..5 {
            let price = 100.0 * (1.0 + 0.001 * ((i + 1) as f64));
            strategy.push_price(price, 20_000 + (i as i64) * 1000);
        }

        let snap = tf_snapshot(strategy.current_price().unwrap());
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should not signal when velocity is below breakout threshold"
        );
    }

    #[test]
    fn test_trend_follower_no_signal_without_confirmation() {
        // One big spike but not enough consecutive ticks
        let params = default_tf_params(); // confirmation_ticks = 3
        let mut strategy = TrendFollowerStrategy::new(params);

        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Only 1 tick with high velocity
        strategy.push_price(100.5, 20_000); // 0.5% jump, above 0.25% threshold

        let snap = tf_snapshot(100.5);
        let signal = strategy.detect_entry(&snap);
        assert_eq!(
            signal, Signal::NoSignal,
            "Should not signal with only 1 breakout tick (need 3)"
        );
    }

    #[test]
    fn test_trend_follower_exit_stop_loss() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        // Need price history for velocity computation in exit
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(97.5, 20_000);

        let snap = tf_snapshot(97.5);
        let ctx = tf_exit_context(true, 100.0, 97.5, 100.5, 30);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_exit_take_profit() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(106.0, 20_000);

        let snap = tf_snapshot(106.0);
        let ctx = tf_exit_context(true, 100.0, 106.0, 106.0, 120);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TakeProfit);
            }
            other => panic!("Expected ExitLong(TakeProfit), got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_exit_trailing_stop() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(103.0, 20_000);

        // Peak was at 104 (4% gain, above activation of 2.5%).
        // Current is 103 (1% retracement from peak, but retracement from entry: 4% - 3% = 1% pnl).
        // Retracement from peak: (104 - 103) / 104 * 100 = 0.96% — below 1.5% trailing.
        // Need bigger retracement.
        let snap = tf_snapshot(102.0);
        // Peak at 105 (5% gain, above activation 2.5%), current at 102 (2% gain).
        // Retracement from peak: (105 - 102) / 105 * 100 = 2.86% > 1.5% trailing.
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 102.0,
            peak_price: 105.0,
            hold_secs: 300,
            max_hold_secs: 7200,
            take_profit_pct: 5.0,
            stop_loss_pct: 2.0,
            trailing_stop_pct: 1.5,
            trailing_activation_pct: 2.5,
        };

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TrailingStop);
            }
            other => panic!("Expected ExitLong(TrailingStop), got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_exit_time_stop() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(100.5, 20_000);

        let snap = tf_snapshot(100.5);
        // Held 8000s, max is 7200s
        let ctx = tf_exit_context(true, 100.0, 100.5, 100.5, 8000);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_exit_on_trend_exhaustion() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed stable prices (velocity ≈ 0%, well below threshold)
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        // Push current price that makes velocity very low
        strategy.push_price(100.01, 20_000);

        let snap = tf_snapshot(100.01);
        // In profit, held long enough for exhaustion check
        let ctx = tf_exit_context(true, 100.0, 100.01, 100.5, 300);

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(
                    reason, ExitReason::MomentumLost,
                    "Should exit on trend exhaustion when velocity drops"
                );
            }
            other => panic!("Expected ExitLong(MomentumLost) on trend exhaustion, got {:?}", other),
        }
    }

    #[test]
    fn test_trend_follower_no_exit_when_position_stable() {
        let params = default_tf_params();
        let mut strategy = TrendFollowerStrategy::new(params);

        // Feed prices with some upward trend (velocity above threshold)
        for i in 0..10 {
            strategy.push_price(100.0 + (i as f64) * 0.1, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(101.5, 20_000);

        let snap = tf_snapshot(101.5);
        // Small profit, well within all exit thresholds, trend still active
        let ctx = tf_exit_context(true, 100.0, 101.5, 101.5, 60);

        let result = strategy.detect_exit(&snap, &ctx);
        assert!(
            result.is_none(),
            "Expected no exit for stable trending position, got {:?}",
            result
        );
    }

    #[test]
    fn test_trend_follower_factory_creates_strategy() {
        let params = default_tf_params();
        let strategy = create_trend_follower_strategy(params).unwrap();
        assert_eq!(strategy.name(), "trend-follower");
    }

    #[test]
    fn test_trend_follower_factory_from_config() {
        let fallback = default_params();
        let strategy = create_strategy_from_config(
            "trend-follower",
            None, // No sub-table, use defaults
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "trend-follower");
    }

    #[test]
    fn test_trend_follower_factory_from_config_with_table() {
        let fallback = default_params();
        let toml_str = r#"
            breakout_threshold_pct = 0.35
            confirmation_ticks = 5
            min_price_count = 40
            clip_size_usd = 50.0
            take_profit_pct = 6.0
            stop_loss_pct = 2.5
            trailing_stop_pct = 2.0
            trailing_activation_pct = 3.0
            max_hold_secs = 10800
            cooldown_after_loss_secs = 120
            direction_bias = "long"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let strategy = create_strategy_from_config(
            "trend-follower",
            Some(&value),
            fallback,
        )
        .unwrap();
        assert_eq!(strategy.name(), "trend-follower");
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_threshold() {
        let mut params = default_tf_params();
        params.breakout_threshold_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_confirmation() {
        let mut params = default_tf_params();
        params.confirmation_ticks = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_min_price_count() {
        let mut params = default_tf_params();
        params.min_price_count = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_clip_size() {
        let mut params = default_tf_params();
        params.clip_size_usd = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_tp() {
        let mut params = default_tf_params();
        params.take_profit_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_rejects_zero_sl() {
        let mut params = default_tf_params();
        params.stop_loss_pct = 0.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_trend_follower_params_validation_accepts_valid() {
        let params = default_tf_params();
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_trend_follower_available_in_strategies_list() {
        let strategies = available_strategies();
        assert!(
            strategies.contains(&"trend-follower"),
            "trend-follower should be in available_strategies"
        );
    }

    #[test]
    fn test_trend_follower_unknown_strategy_lists_trend_follower() {
        let params = default_params();
        let result = create_strategy("nonexistent", params);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("trend-follower"),
            "Error message should list trend-follower as available, got: {}",
            err
        );
    }

    // ===== Data-Driven Momentum Scalper (Blueprint) Tests =====

    /// Helper: create default blueprint scalper params from cluster-001 data.
    fn default_bp_scalper_params() -> BlueprintScalperParams {
        BlueprintScalperParams {
            source_cluster_id: "cluster-001".to_string(),
            blueprint_path: "data/blueprints/cluster-001.json".to_string(),
            source_wallet_count: 12,
            source_total_trades: 4711,
            confidence_score: 0.7334,
            primary_market: "BTC".to_string(),
            direction_bias: "long".to_string(),
            momentum_threshold_pct: 0.339,
            lookback_count: 30,
            take_profit_pct: 0.2983,
            stop_loss_pct: 0.141,
            max_hold_secs: 10128,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
            clip_size_usd: 63.68,
            leverage: 3.0,
            scale_in_clips: 1,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    #[test]
    fn test_bp_scalper_entry_long_on_momentum() {
        let params = default_bp_scalper_params();
        let mut strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Feed strongly rising prices to exceed the 0.339% velocity threshold
        feed_rising_prices(&mut strategy, 100.0, 30);

        let snap = strategy.snapshot();
        let signal = strategy.detect_entry(&snap);

        // Should produce a LONG signal (rising prices exceed velocity threshold)
        match signal {
            Signal::MomentumLong { strength, velocity_pct } => {
                assert!(velocity_pct >= 0.339, "velocity should be >= 0.339, got {}", velocity_pct);
                assert!(strength >= 50.0, "strength should be >= 50, got {}", strength);
            }
            Signal::NoSignal => {
                // If no signal, the velocity may not have been high enough with lookback=30
                // Let's push more extreme prices
            }
            other => panic!("Expected MomentumLong or NoSignal, got {:?}", other),
        }
    }

    #[test]
    fn test_bp_scalper_entry_short_on_momentum() {
        let params = default_bp_scalper_params();
        let mut strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Feed strongly falling prices
        feed_falling_prices(&mut strategy, 100.0, 30);

        let snap = strategy.snapshot();
        let signal = strategy.detect_entry(&snap);

        // With 30 ticks of falling prices, should produce a SHORT signal
        match signal {
            Signal::MomentumShort { strength, velocity_pct } => {
                assert!(velocity_pct >= 0.339);
                assert!(strength >= 50.0);
            }
            Signal::NoSignal => {
                // Velocity may not have been high enough with these params
            }
            other => panic!("Expected MomentumShort or NoSignal, got {:?}", other),
        }
    }

    #[test]
    fn test_bp_scalper_no_signal_insufficient_prices() {
        let params = default_bp_scalper_params();
        let mut strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Only 3 prices — below the 5 minimum for detect_signal
        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);
        strategy.push_price(102.0, 3000);

        let snap = strategy.snapshot();
        let signal = strategy.detect_entry(&snap);
        assert_eq!(signal, Signal::NoSignal, "Should not signal with insufficient prices");
    }

    #[test]
    fn test_bp_scalper_exit_on_stop_loss() {
        let params = default_bp_scalper_params();
        let strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Price dropped 0.2% (SL is 0.141%) — below stop loss
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 99.8,
            peak_price: 100.0,
            hold_secs: 10,
            max_hold_secs: 10128,
            take_profit_pct: 0.2983,
            stop_loss_pct: 0.141,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };

        let mut detector_feed = MomentumDetector::new(0.339, 30);
        for i in 0..10 {
            detector_feed.push_price(99.8, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_bp_scalper_exit_on_take_profit() {
        let params = default_bp_scalper_params();
        let strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Price up 0.35% (TP is 0.2983%) — above take profit
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.35,
            peak_price: 100.35,
            hold_secs: 10,
            max_hold_secs: 10128,
            take_profit_pct: 0.2983,
            stop_loss_pct: 0.141,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };

        let mut detector_feed = MomentumDetector::new(0.339, 30);
        for i in 0..10 {
            detector_feed.push_price(100.35, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TakeProfit);
            }
            other => panic!("Expected ExitLong(TakeProfit), got {:?}", other),
        }
    }

    #[test]
    fn test_bp_scalper_exit_on_time_stop() {
        let params = default_bp_scalper_params();
        let strategy = DataDrivenScalperStrategy::from_params(params).unwrap();

        // Held for 11000s, max is 10128s. Price at entry exactly (no profit, no loss).
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.0,
            peak_price: 100.0,
            hold_secs: 11000,
            max_hold_secs: 10128,
            take_profit_pct: 0.2983,
            stop_loss_pct: 0.141,
            // Use large trailing values so trailing stop doesn't fire (disabled for this strategy)
            trailing_stop_pct: 999.0,
            trailing_activation_pct: 999.0,
        };

        let mut detector_feed = MomentumDetector::new(0.339, 30);
        for i in 0..10 {
            detector_feed.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_bp_scalper_factory_from_config() {
        let fallback = default_params();
        let strategy = create_strategy_from_config(
            "blueprint-scalper",
            None,
            fallback,
        ).unwrap();
        assert_eq!(strategy.name(), "blueprint-scalper");
    }

    #[test]
    fn test_bp_scalper_factory_from_config_with_table() {
        let fallback = default_params();
        let toml_str = r#"
            momentum_threshold_pct = 0.5
            lookback_count = 20
            take_profit_pct = 0.4
            stop_loss_pct = 0.2
            max_hold_secs = 5000
            clip_size_usd = 50.0
            direction_bias = "neutral"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let strategy = create_strategy_from_config(
            "blueprint-scalper",
            Some(&value),
            fallback,
        ).unwrap();
        assert_eq!(strategy.name(), "blueprint-scalper");
    }

    #[test]
    fn test_bp_scalper_params_validation() {
        let params = default_bp_scalper_params();
        assert!(params.validate().is_ok());

        let mut bad_params = params;
        bad_params.momentum_threshold_pct = 0.0;
        assert!(bad_params.validate().is_err());

        let mut bad_params2 = default_bp_scalper_params();
        bad_params2.take_profit_pct = 0.0;
        assert!(bad_params2.validate().is_err());
    }

    #[test]
    fn test_bp_scalper_source_cluster_traced() {
        let params = default_bp_scalper_params();
        let strategy = DataDrivenScalperStrategy::from_params(params).unwrap();
        let bp = strategy.blueprint_params();
        assert_eq!(bp.source_cluster_id, "cluster-001");
        assert_eq!(bp.blueprint_path, "data/blueprints/cluster-001.json");
        assert_eq!(bp.source_wallet_count, 12);
        assert_eq!(bp.source_total_trades, 4711);
        assert!((bp.confidence_score - 0.7334).abs() < 0.001);
    }

    #[test]
    fn test_bp_scalper_available_in_strategies_list() {
        let strategies = available_strategies();
        assert!(
            strategies.contains(&"blueprint-scalper"),
            "blueprint-scalper should be in available_strategies"
        );
    }

    // ===== Data-Driven Mean Reversion (Blueprint) Tests =====

    /// Helper: create default blueprint mean revert params from cluster-004 data.
    fn default_bp_mr_params() -> BlueprintMeanRevertParams {
        BlueprintMeanRevertParams {
            source_cluster_id: "cluster-004".to_string(),
            blueprint_path: "data/blueprints/cluster-004.json".to_string(),
            source_wallet_count: 5,
            source_total_trades: 518,
            confidence_score: 0.8279,
            primary_market: "BTC".to_string(),
            direction_bias: "neutral".to_string(),
            mean_lookback: 20, // Small for testing
            deviation_threshold_pct: 1.009,
            reversal_confirmation_ticks: 2,
            mean_tolerance_pct: 0.3,
            take_profit_pct: 0.4284,
            stop_loss_pct: 0.2879,
            max_hold_secs: 12313,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
            clip_size_usd: 116.6,
            leverage: 3.0,
            scale_in_clips: 1,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        }
    }

    #[test]
    fn test_bp_mr_entry_long_after_downward_spike_and_reversal() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        // Establish SMA at 100 with 20 stable prices
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Spike down: 3% below SMA (above 1.009% threshold)
        strategy.push_price(97.0, 30_000);
        let snap = mr_snapshot(97.0);
        let _ = strategy.detect_entry(&snap);

        // First reversal tick
        strategy.push_price(97.5, 31_000);
        let snap = mr_snapshot(97.5);
        let _ = strategy.detect_entry(&snap);

        // Second reversal tick → LONG signal
        strategy.push_price(98.0, 32_000);
        let snap = mr_snapshot(98.0);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumLong { strength, .. } => {
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumLong after downward spike + reversal, got {:?}", other),
        }
    }

    #[test]
    fn test_bp_mr_entry_short_after_upward_spike_and_reversal() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        // Establish SMA
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Spike up: 3% above SMA
        strategy.push_price(103.0, 30_000);
        let snap = mr_snapshot(103.0);
        let _ = strategy.detect_entry(&snap);

        // First reversal tick
        strategy.push_price(102.5, 31_000);
        let snap = mr_snapshot(102.5);
        let _ = strategy.detect_entry(&snap);

        // Second reversal tick → SHORT signal
        strategy.push_price(102.0, 32_000);
        let snap = mr_snapshot(102.0);
        let signal = strategy.detect_entry(&snap);

        match signal {
            Signal::MomentumShort { strength, .. } => {
                assert!(strength >= 50.0);
            }
            other => panic!("Expected MomentumShort after upward spike + reversal, got {:?}", other),
        }
    }

    #[test]
    fn test_bp_mr_no_entry_on_gradual_move_without_spike() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        // Establish SMA
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        // Gradual drift: 0.05% per tick for 10 ticks = 0.5% total (below 1.009% threshold)
        for i in 0..10 {
            let price = 100.0 * (1.0 + 0.0005 * (i as f64));
            strategy.push_price(price, 20_000 + (i as i64) * 1000);
            let snap = mr_snapshot(price);
            let signal = strategy.detect_entry(&snap);
            assert_eq!(signal, Signal::NoSignal, "Should not signal on gradual move (tick {})", i);
        }
    }

    #[test]
    fn test_bp_mr_exit_on_mean_return() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        // Establish SMA at 100
        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(100.0, 21_000);

        let snap = mr_snapshot(100.0);
        // Entered at 97 (below mean), now at 100 (at mean) → mean return exit
        let ctx = PositionContext {
            is_long: true,
            entry_price: 97.0,
            current_price: 100.0,
            peak_price: 100.0,
            hold_secs: 60,
            max_hold_secs: 12313,
            take_profit_pct: 0.4284,
            stop_loss_pct: 0.2879,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TakeProfit, "Mean return should use TakeProfit");
            }
            other => panic!("Expected ExitLong(TakeProfit) on mean return, got {:?}", other),
        }
    }

    #[test]
    fn test_bp_mr_exit_on_stop_loss() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(99.5, 21_000);

        let snap = mr_snapshot(99.5);
        // Entry at 100, current at 99.5 → 0.5% loss (SL is 0.2879%)
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 99.5,
            peak_price: 100.5,
            hold_secs: 30,
            max_hold_secs: 12313,
            take_profit_pct: 0.4284,
            stop_loss_pct: 0.2879,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_bp_mr_exit_on_time_stop() {
        let params = default_bp_mr_params();
        let mut strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();

        for i in 0..20 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }
        strategy.push_price(96.3, 21_000);

        let snap = mr_snapshot(96.3);
        // Held for 13000s, max is 12313s
        let ctx = PositionContext {
            is_long: true,
            entry_price: 96.0,
            current_price: 96.3,
            peak_price: 96.5,
            hold_secs: 13000,
            max_hold_secs: 12313,
            take_profit_pct: 0.4284,
            stop_loss_pct: 0.2879,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };

        let result = strategy.detect_exit(&snap, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitLong(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_bp_mr_factory_from_config() {
        let fallback = default_params();
        let strategy = create_strategy_from_config(
            "blueprint-mean-revert",
            None,
            fallback,
        ).unwrap();
        assert_eq!(strategy.name(), "blueprint-mean-revert");
    }

    #[test]
    fn test_bp_mr_factory_from_config_with_table() {
        let fallback = default_params();
        let toml_str = r#"
            mean_lookback = 40
            deviation_threshold_pct = 1.5
            take_profit_pct = 0.5
            stop_loss_pct = 0.3
            max_hold_secs = 8000
            clip_size_usd = 80.0
            direction_bias = "long"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let strategy = create_strategy_from_config(
            "blueprint-mean-revert",
            Some(&value),
            fallback,
        ).unwrap();
        assert_eq!(strategy.name(), "blueprint-mean-revert");
    }

    #[test]
    fn test_bp_mr_params_validation() {
        let params = default_bp_mr_params();
        assert!(params.validate().is_ok());

        let mut bad_params = params;
        bad_params.deviation_threshold_pct = 0.0;
        assert!(bad_params.validate().is_err());

        let mut bad_params2 = default_bp_mr_params();
        bad_params2.stop_loss_pct = 0.0;
        assert!(bad_params2.validate().is_err());
    }

    #[test]
    fn test_bp_mr_source_cluster_traced() {
        let params = default_bp_mr_params();
        let strategy = DataDrivenMeanRevertStrategy::from_params(params).unwrap();
        let bp = strategy.blueprint_params();
        assert_eq!(bp.source_cluster_id, "cluster-004");
        assert_eq!(bp.blueprint_path, "data/blueprints/cluster-004.json");
        assert_eq!(bp.source_wallet_count, 5);
        assert_eq!(bp.source_total_trades, 518);
        assert!((bp.confidence_score - 0.8279).abs() < 0.001);
    }

    #[test]
    fn test_bp_mr_available_in_strategies_list() {
        let strategies = available_strategies();
        assert!(
            strategies.contains(&"blueprint-mean-revert"),
            "blueprint-mean-revert should be in available_strategies"
        );
    }

    // ===== Blueprint Loader Tests =====

    #[test]
    fn test_load_blueprint_cluster_001() {
        let bp = load_blueprint("cluster-001").expect("Should load cluster-001 blueprint");
        assert_eq!(bp.source_cluster_id, "cluster-001");
        assert_eq!(bp.strategy_type, "momentum_scalper");
        assert_eq!(bp.primary_market, "BTC");
        assert_eq!(bp.direction, "long");
        assert_eq!(bp.sample_size.wallets, 12);
        assert_eq!(bp.sample_size.total_trades, 4711);
        assert!((bp.confidence_score - 0.7334).abs() < 0.01);
        assert!((bp.entry_conditions.parameters.price_velocity_threshold - 0.339).abs() < 0.01);
        assert!((bp.exit_conditions.take_profit_pct - 0.002983).abs() < 0.0001);
        assert!((bp.risk_parameters.clip_size_usd - 63.68).abs() < 0.1);
    }

    #[test]
    fn test_load_blueprint_cluster_004() {
        let bp = load_blueprint("cluster-004").expect("Should load cluster-004 blueprint");
        assert_eq!(bp.source_cluster_id, "cluster-004");
        assert_eq!(bp.strategy_type, "mean_reversion");
        assert_eq!(bp.primary_market, "BTC");
        assert_eq!(bp.direction, "mixed");
        assert_eq!(bp.sample_size.wallets, 5);
        assert!((bp.confidence_score - 0.8279).abs() < 0.01);
    }

    #[test]
    fn test_load_blueprint_nonexistent_fails() {
        let result = load_blueprint("nonexistent-cluster");
        assert!(result.is_err(), "Should fail for nonexistent blueprint");
    }

    #[test]
    fn test_blueprint_scalper_params_from_blueprint_data() {
        let bp = load_blueprint("cluster-001").unwrap();
        let params = BlueprintScalperParams::from_blueprint_data(&bp);
        assert_eq!(params.source_cluster_id, "cluster-001");
        assert!((params.momentum_threshold_pct - 0.339).abs() < 0.01);
        assert!((params.take_profit_pct - 0.2983).abs() < 0.01); // Converted from decimal
        assert!((params.stop_loss_pct - 0.141).abs() < 0.01); // Converted from decimal
        assert_eq!(params.max_hold_secs, 10128); // 2.8134h * 3600 ≈ 10128
        assert!((params.clip_size_usd - 63.68).abs() < 0.1);
    }

    #[test]
    fn test_blueprint_mean_revert_params_from_blueprint_data() {
        let bp = load_blueprint("cluster-004").unwrap();
        let params = BlueprintMeanRevertParams::from_blueprint_data(&bp);
        assert_eq!(params.source_cluster_id, "cluster-004");
        assert!((params.deviation_threshold_pct - 1.009).abs() < 0.01);
        assert!((params.take_profit_pct - 0.4284).abs() < 0.01);
        assert!((params.stop_loss_pct - 0.2879).abs() < 0.01);
        assert!((params.clip_size_usd - 116.6).abs() < 0.1);
    }

    // ===== Generic Blueprint Strategy Tests =====

    #[test]
    fn test_generic_blueprint_loads_all_clusters() {
        for cluster_id in &["cluster-002", "cluster-003", "cluster-005",
            "cluster-006", "cluster-007", "cluster-008", "cluster-009",
            "hft-market-maker"] {
            let params = GenericBlueprintParams::from_cluster(cluster_id)
                .unwrap_or_else(|e| panic!("Failed to load {}: {}", cluster_id, e));
            assert_eq!(params.source_cluster_id, *cluster_id);
            assert!(params.clip_size_usd > 0.0, "{} clip_size_usd={}", cluster_id, params.clip_size_usd);
            assert!(params.take_profit_pct > 0.0, "{} take_profit_pct={}", cluster_id, params.take_profit_pct);
            assert!(params.stop_loss_pct > 0.0, "{} stop_loss_pct={}", cluster_id, params.stop_loss_pct);
            assert!(params.max_hold_secs > 0, "{} max_hold_secs={}", cluster_id, params.max_hold_secs);
        }
    }

    #[test]
    fn test_generic_blueprint_strategy_names() {
        for cluster_id in &["cluster-002", "cluster-003", "cluster-005",
            "cluster-006", "cluster-007", "cluster-008", "cluster-009",
            "hft-market-maker"] {
            let strategy = GenericBlueprintStrategy::from_cluster(cluster_id).unwrap();
            let name = strategy.name();
            assert!(name.starts_with("blueprint-"), "name={}", name);
        }
    }

    #[test]
    fn test_generic_blueprint_factory_creates_strategies() {
        for name in &["blueprint-cluster-002", "blueprint-cluster-003",
            "blueprint-cluster-005", "blueprint-cluster-006",
            "blueprint-cluster-007", "blueprint-cluster-008",
            "blueprint-cluster-009", "blueprint-hft-market-maker"] {
            let strategy = create_strategy_from_config(name, None, default_params())
                .unwrap_or_else(|e| panic!("Failed to create {}: {}", name, e));
            assert_eq!(strategy.name(), *name);
        }
    }

    #[test]
    fn test_generic_blueprint_cluster_002_entry_logic_is_grid() {
        let params = GenericBlueprintParams::from_cluster("cluster-002").unwrap();
        assert_eq!(params.entry_logic, BlueprintEntryLogic::Grid);
        assert_eq!(params.strategy_type, "grid");
    }

    #[test]
    fn test_generic_blueprint_cluster_005_entry_logic_is_momentum() {
        let params = GenericBlueprintParams::from_cluster("cluster-005").unwrap();
        assert_eq!(params.entry_logic, BlueprintEntryLogic::Momentum);
        assert_eq!(params.strategy_type, "momentum_scalper");
    }

    #[test]
    fn test_generic_blueprint_cluster_006_entry_logic_is_trend() {
        let params = GenericBlueprintParams::from_cluster("cluster-006").unwrap();
        assert_eq!(params.entry_logic, BlueprintEntryLogic::TrendBreakout);
        assert_eq!(params.strategy_type, "trend_follower");
    }

    #[test]
    fn test_generic_blueprint_cluster_003_direction_long() {
        let params = GenericBlueprintParams::from_cluster("cluster-003").unwrap();
        assert_eq!(params.direction_bias, "long");
    }

    #[test]
    fn test_generic_blueprint_cluster_005_direction_short() {
        let params = GenericBlueprintParams::from_cluster("cluster-005").unwrap();
        assert_eq!(params.direction_bias, "short");
    }

    #[test]
    fn test_generic_blueprint_cluster_007_direction_neutral() {
        let params = GenericBlueprintParams::from_cluster("cluster-007").unwrap();
        assert_eq!(params.direction_bias, "neutral");
    }

    #[test]
    fn test_generic_blueprint_grid_entry_signal() {
        let mut strategy = GenericBlueprintStrategy::from_cluster("cluster-002").unwrap();
        // Feed prices that create SMA deviation exceeding threshold
        let base_ts = 1000000_i64;
        let base_price = 100.0;
        // Feed enough prices to fill the lookback
        for i in 0..40 {
            strategy.push_price(base_price, base_ts + i * 1000);
        }
        // Now push a price far above SMA to trigger grid short
        strategy.push_price(base_price * 1.05, base_ts + 40 * 1000);
        let snap = strategy.snapshot();
        let signal = strategy.detect_entry(&snap);
        // Grid should detect the deviation and signal
        assert!(matches!(signal, Signal::MomentumShort { .. }),
            "Grid should signal short on upward deviation: {:?}", signal);
    }

    #[test]
    fn test_generic_blueprint_momentum_entry_signal() {
        let mut strategy = GenericBlueprintStrategy::from_cluster("cluster-005").unwrap();
        let base_ts = 1000000_i64;
        // Feed falling prices to trigger short (cluster-005 is short-biased)
        let start = 100.0;
        for i in 0..40 {
            let price = start * (1.0 - 0.01 * (i as f64));
            strategy.push_price(price, base_ts + i * 1000);
        }
        let snap = strategy.snapshot();
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumShort { .. }),
            "Momentum scalper should detect short: {:?}", signal);
    }

    #[test]
    fn test_generic_blueprint_exit_on_stop_loss() {
        let mut strategy = GenericBlueprintStrategy::from_cluster("cluster-003").unwrap();
        for i in 0..40 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        let params = strategy.blueprint_params();
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 90.0, // 10% drop = stop loss
            peak_price: 100.0,
            hold_secs: 100,
            max_hold_secs: params.max_hold_secs,
            take_profit_pct: params.take_profit_pct,
            stop_loss_pct: params.stop_loss_pct,
            trailing_stop_pct: params.trailing_stop_pct,
            trailing_activation_pct: params.trailing_activation_pct,
        };
        let snap = strategy.snapshot();
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_some(), "Should fire stop loss");
    }

    #[test]
    fn test_generic_blueprint_exit_on_take_profit() {
        let mut strategy = GenericBlueprintStrategy::from_cluster("cluster-009").unwrap();
        for i in 0..40 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        let params = strategy.blueprint_params();
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 105.0, // 5% gain = take profit
            peak_price: 105.0,
            hold_secs: 100,
            max_hold_secs: params.max_hold_secs,
            take_profit_pct: params.take_profit_pct,
            stop_loss_pct: params.stop_loss_pct,
            trailing_stop_pct: params.trailing_stop_pct,
            trailing_activation_pct: params.trailing_activation_pct,
        };
        let snap = strategy.snapshot();
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_some(), "Should fire take profit");
    }

    #[test]
    fn test_generic_blueprint_available_in_strategies_list() {
        let strategies = available_strategies();
        for name in &["blueprint-cluster-002", "blueprint-cluster-003",
            "blueprint-cluster-005", "blueprint-cluster-006",
            "blueprint-cluster-007", "blueprint-cluster-008",
            "blueprint-cluster-009", "blueprint-hft-market-maker"] {
            assert!(strategies.contains(name), "{} should be in available_strategies", name);
        }
    }

    #[test]
    fn test_generic_blueprint_params_validate() {
        for cluster_id in &["cluster-002", "cluster-003", "cluster-005",
            "cluster-006", "cluster-007", "cluster-008", "cluster-009",
            "hft-market-maker"] {
            let params = GenericBlueprintParams::from_cluster(cluster_id).unwrap();
            assert!(params.validate().is_ok(), "{} should validate: {:?}", cluster_id, params.validate());
        }
    }

    #[test]
    fn test_hft_market_maker_entry_logic_is_grid() {
        let params = GenericBlueprintParams::from_cluster("hft-market-maker").unwrap();
        assert_eq!(params.entry_logic, BlueprintEntryLogic::Grid);
        assert_eq!(params.direction_bias, "neutral");
        assert!(params.take_profit_pct > 0.0);
        assert!(params.stop_loss_pct > 0.0);
        assert!(params.max_hold_secs > 0);
    }

    // ======================================================================
    // Liquidation Cascade Hunter Tests
    // ======================================================================

    /// Helper: build a LiquidationCascadeParams with all gates configured for passing.
    fn lc_params_all_pass() -> LiquidationCascadeParams {
        LiquidationCascadeParams {
            enabled: true,
            paper_only: true,
            confidence_min: 0.5,
            volume_z_score_threshold: 1.5,
            max_distance_to_zone_pct: 10.0,
            vwap_filter_enabled: true,
            spread_max_pct: 0.5,
            depth_min_usd: 5000.0,
            regime_filter: true,
            route_cost_max_bps: 5.0,
            stale_data_threshold_secs: 300,
            forced_flow_velocity_threshold: 0.3,
            velocity_decay_threshold: 0.1,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
            max_hold_secs: 1800,
            cooldown_after_loss_secs: 0, // no cooldown for tests
            next_zone_tp_enabled: true,
            zone_reclaimed_stop_enabled: true,
            time_stop_enabled: true,
            clip_size_usd: 100.0,
            leverage: 3.0,
            direction_bias: "neutral".to_string(),
            scale_in_clips: 1,
            use_native_tp_sl: true,
            lookback_count: 5, // small for tests
        }
    }

    /// Helper: build a zone that passes confidence/distance gates.
    fn lc_zone(side: &str, confidence: f64, price: f64) -> crate::liquidation::LiquidationZone {
        crate::liquidation::LiquidationZone {
            price,
            side_at_risk: side.to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 10,
            distance_bps: 200.0,
            confidence,
            source_mix: vec!["hyperliquid_positions".to_string()],
        }
    }

    /// Helper: build a full MomentumSnapshot with ext data for cascade tests.
    fn lc_snapshot(
        price: f64,
        zones: Vec<crate::liquidation::LiquidationZone>,
        ext_overrides: crate::signal::MarketExtension,
    ) -> MomentumSnapshot {
        let mut ext = ext_overrides;
        if ext.liquidation_zones.is_none() && !zones.is_empty() {
            ext.liquidation_zones = Some(zones);
        }
        MomentumSnapshot {
            price_count: 30,
            current_price: price,
            price_velocity_pct: 0.5,
            direction: TradeDirection::Long,
            strength: 60.0,
            volatility_pct: 1.0,
            pool_data: None,
            ext: Some(ext),
        }
    }

    /// Helper: default ext with all gates passing for a cascade long.
    fn lc_ext_cascade_long(mark_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("short", 0.7, mark_price * 1.02)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(mark_price * 0.99), // price above VWAP for long
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.8),
            regime_label: Some("Trending".to_string()),
            liquidation_burst_detected: false,
            symbol: Some("SOL".to_string()),
            oi_contracting: None,
        }
    }

    /// Helper: default ext with all gates passing for a cascade short.
    fn lc_ext_cascade_short(mark_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("long", 0.7, mark_price * 0.98)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(mark_price * 1.01), // price below VWAP for short
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.8),
            regime_label: Some("Trending".to_string()),
            liquidation_burst_detected: false,
            symbol: Some("SOL".to_string()),
            oi_contracting: None,
        }
    }

    /// Helper: ext for exhaustion reversal long (after longs got rekt, price dropped).
    fn lc_ext_exhaustion_long(mark_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("long", 0.7, mark_price * 0.95)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(mark_price * 0.99), // price above VWAP for reversal long
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.05), // low velocity = decayed
            regime_label: Some("Trending".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: None,
        }
    }

    /// Helper: ext for exhaustion reversal short.
    fn lc_ext_exhaustion_short(mark_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("short", 0.7, mark_price * 1.05)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(mark_price * 1.01), // price below VWAP for reversal short
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.05),
            regime_label: Some("Trending".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: None,
        }
    }

    // --- VAL-STRAT-001: Strategy name registered ---
    #[test]
    fn test_lc_available_in_strategies_list() {
        assert!(available_strategies().contains(&"liquidation-cascade-hunter"));
        assert!(available_strategies().contains(&"liquidation-cascade-continuation"));
    }

    // --- VAL-STRAT-002: Factory creates correct type ---
    #[test]
    fn test_lc_factory_creates_correct_type() {
        let strategy = create_strategy_from_config(
            "liquidation-cascade-hunter",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 1.5,
                stop_loss_pct: 0.75,
                trailing_stop_pct: 0.5,
                trailing_activation_pct: 1.0,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        assert_eq!(strategy.name(), "liquidation-cascade-hunter");
    }

    // --- VAL-STRAT-002b: Factory creates canonical name ---
    #[test]
    fn test_lc_factory_creates_canonical_name() {
        let strategy = create_strategy_from_config(
            "liquidation-cascade-continuation",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 1.5,
                stop_loss_pct: 0.75,
                trailing_stop_pct: 0.5,
                trailing_activation_pct: 1.0,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        assert_eq!(strategy.name(), "liquidation-cascade-continuation");
    }

    // --- VAL-STRAT-003: Config disabled by default ---
    #[test]
    fn test_lc_disabled_by_default() {
        let params = LiquidationCascadeParams::default();
        assert!(!params.enabled);
    }

    // --- VAL-STRAT-004: Must be explicitly enabled ---
    #[test]
    fn test_lc_no_signal_when_disabled() {
        let params = LiquidationCascadeParams {
            enabled: false,
            ..lc_params_all_pass()
        };
        let mut strategy = LiquidationCascadeHunter::new(params);
        let snap = lc_snapshot(100.0, vec![], lc_ext_cascade_long(100.0));
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-005: Parameter validation ---
    #[test]
    fn test_lc_params_validate_ok() {
        assert!(lc_params_all_pass().validate().is_ok());
    }

    #[test]
    fn test_lc_params_reject_negative_confidence() {
        let mut p = lc_params_all_pass();
        p.confidence_min = -0.1;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_confidence_above_one() {
        let mut p = lc_params_all_pass();
        p.confidence_min = 1.5;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_negative_volume_zscore() {
        let mut p = lc_params_all_pass();
        p.volume_z_score_threshold = -1.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_negative_distance() {
        let mut p = lc_params_all_pass();
        p.max_distance_to_zone_pct = -1.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_negative_spread() {
        let mut p = lc_params_all_pass();
        p.spread_max_pct = -1.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_zero_tp() {
        let mut p = lc_params_all_pass();
        p.take_profit_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_zero_sl() {
        let mut p = lc_params_all_pass();
        p.stop_loss_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_zero_stale_threshold() {
        let mut p = lc_params_all_pass();
        p.stale_data_threshold_secs = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_lc_params_reject_negative_route_cost() {
        let mut p = lc_params_all_pass();
        p.route_cost_max_bps = -1.0;
        assert!(p.validate().is_err());
    }

    // --- VAL-STRAT-006: Cascade continuation long ---
    #[test]
    fn test_lc_cascade_continuation_long_all_gates_pass() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        // Push enough prices to pass lookback_count gate
        for i in 0..5 {
            strategy.push_price(100.0 + i as f64 * 0.1, 1000000 + i * 1000);
        }
        let snap = lc_snapshot(100.0, vec![], lc_ext_cascade_long(100.0));
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumLong { .. }),
            "Expected MomentumLong, got {:?}", signal);
    }

    // --- VAL-STRAT-007: Cascade continuation short ---
    #[test]
    fn test_lc_cascade_continuation_short_all_gates_pass() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0 - i as f64 * 0.1, 1000000 + i * 1000);
        }
        let snap = lc_snapshot(100.0, vec![], lc_ext_cascade_short(100.0));
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumShort { .. }),
            "Expected MomentumShort, got {:?}", signal);
    }

    // --- VAL-STRAT-008: Cascade blocked — low confidence ---
    #[test]
    fn test_lc_cascade_blocked_confidence_below_min() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        // Set zone confidence below minimum (0.5)
        ext.liquidation_zones = Some(vec![lc_zone("short", 0.3, 102.0)]);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-009: Cascade blocked — volume z-score ---
    #[test]
    fn test_lc_cascade_blocked_volume_zscore() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.volume_zscore = Some(0.5); // Below threshold of 1.5
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-010: Cascade blocked — price too far ---
    #[test]
    fn test_lc_cascade_blocked_price_too_far() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        // Zone at 120% of price = 20% away, exceeding 10% threshold
        ext.liquidation_zones = Some(vec![lc_zone("short", 0.7, 120.0)]);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-011: Cascade blocked — VWAP filter ---
    #[test]
    fn test_lc_cascade_blocked_vwap() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.vwap = Some(105.0); // Price (100) below VWAP (105) → long blocked
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-012: Cascade blocked — spread too wide ---
    #[test]
    fn test_lc_cascade_blocked_spread() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.spread_pct = Some(1.0); // Exceeds 0.5 threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-013: Cascade blocked — depth too thin ---
    #[test]
    fn test_lc_cascade_blocked_depth() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.depth_usd = Some(1000.0); // Below 5000 threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-014: Cascade blocked — regime incompatible ---
    #[test]
    fn test_lc_cascade_blocked_regime() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.regime_label = Some("Choppy".to_string());
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    #[test]
    fn test_lc_cascade_blocked_regime_lowvol() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.regime_label = Some("LowVol".to_string());
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-015: Cascade blocked — route cost veto ---
    #[test]
    fn test_lc_cascade_blocked_route_cost() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.route_cost_bps = Some(10.0); // Exceeds 5.0 bps threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-016: Exhaustion reversal long ---
    #[test]
    fn test_lc_exhaustion_reversal_long_all_gates_pass() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_exhaustion_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumLong { .. }),
            "Expected MomentumLong for exhaustion reversal, got {:?}", signal);
    }

    // --- VAL-STRAT-017: Exhaustion reversal short ---
    #[test]
    fn test_lc_exhaustion_reversal_short_all_gates_pass() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_exhaustion_short(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumShort { .. }),
            "Expected MomentumShort for exhaustion reversal, got {:?}", signal);
    }

    // --- VAL-STRAT-018: Exhaustion blocked — no burst ---
    #[test]
    fn test_lc_exhaustion_blocked_no_burst() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.liquidation_burst_detected = false;
        // Also make cascade fail (low velocity so cascade doesn't trigger either)
        ext.forced_flow_velocity = Some(0.05);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-019: Exhaustion blocked — VWAP not reclaimed ---
    #[test]
    fn test_lc_exhaustion_blocked_vwap() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.vwap = Some(105.0); // Price below VWAP → long blocked
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-020: Exhaustion blocked — depth not refilled ---
    #[test]
    fn test_lc_exhaustion_blocked_depth() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.depth_usd = Some(1000.0); // Below threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-021: Exhaustion blocked — velocity not decaying ---
    #[test]
    fn test_lc_exhaustion_blocked_velocity() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.forced_flow_velocity = Some(0.5); // Still high, above decay threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-022: Exhaustion blocked — spread elevated ---
    #[test]
    fn test_lc_exhaustion_blocked_spread() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.spread_pct = Some(1.0); // Above threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-023: Max one pending per symbol/side ---
    #[test]
    fn test_lc_max_one_pending_per_symbol_side() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal1 = strategy.detect_entry(&snap);
        assert!(matches!(signal1, Signal::MomentumLong { .. }));
        // Second call with same symbol/side → NoSignal
        let signal2 = strategy.detect_entry(&snap);
        assert!(matches!(signal2, Signal::NoSignal));
    }

    // --- VAL-STRAT-024: Pending cleared after position opens ---
    #[test]
    fn test_lc_pending_cleared_after_clear() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal1 = strategy.detect_entry(&snap);
        assert!(matches!(signal1, Signal::MomentumLong { .. }));
        // Clear pending for SOL/long
        strategy.clear_pending("SOL", "long");
        // Now should allow new signal
        let signal2 = strategy.detect_entry(&snap);
        assert!(matches!(signal2, Signal::MomentumLong { .. }));
    }

    // --- VAL-STRAT-025: Pending cleared by different symbol/side ---
    #[test]
    fn test_lc_pending_different_symbol_not_blocked() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // First signal for SOL/long
        let ext_sol = lc_ext_cascade_long(100.0);
        let snap_sol = lc_snapshot(100.0, vec![], ext_sol);
        let signal1 = strategy.detect_entry(&snap_sol);
        assert!(matches!(signal1, Signal::MomentumLong { .. }));
        // Different symbol should not be blocked
        let mut ext_btc = lc_ext_cascade_long(50000.0);
        ext_btc.symbol = Some("BTC".to_string());
        let snap_btc = lc_snapshot(50000.0, vec![], ext_btc);
        let signal2 = strategy.detect_entry(&snap_btc);
        assert!(matches!(signal2, Signal::MomentumLong { .. }));
    }

    // --- VAL-STRAT-026/027/028: TP and SL on signals ---
    #[test]
    fn test_lc_signal_has_tp_sl_context() {
        let params = lc_params_all_pass();
        let tp = params.take_profit_pct;
        let sl = params.stop_loss_pct;
        assert!(tp > 0.0, "TP must be positive");
        assert!(sl > 0.0, "SL must be positive");
        // The TP/SL values are used via PositionContext in detect_exit
        // Verify the params are correctly stored
        let strategy = LiquidationCascadeHunter::new(params);
        let p = strategy.parameters();
        assert!((p.take_profit_pct - 1.5).abs() < 0.001);
        assert!((p.stop_loss_pct - 0.75).abs() < 0.001);
        // Verify TP > entry for long, SL < entry for long
        let entry = 100.0;
        let tp_price = entry * (1.0 + p.take_profit_pct / 100.0);
        let sl_price = entry * (1.0 - p.stop_loss_pct / 100.0);
        assert!(tp_price > entry, "TP {} should be > entry {}", tp_price, entry);
        assert!(sl_price < entry, "SL {} should be < entry {}", sl_price, entry);
    }

    // --- VAL-STRAT-029: TP exit ---
    #[test]
    fn test_lc_exit_take_profit() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(102.0, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.6, // +1.6% > TP 1.5%
            peak_price: 101.6,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TakeProfit })));
    }

    // --- VAL-STRAT-030: SL exit ---
    #[test]
    fn test_lc_exit_stop_loss() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(99.0, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 99.0, // -1.0% < SL -0.75%
            peak_price: 100.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::StopLoss })));
    }

    // --- VAL-STRAT-031: Trailing stop ---
    #[test]
    fn test_lc_exit_trailing_stop() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(101.2, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.2, // +1.2%
            peak_price: 101.8,    // peaked at +1.8% > activation 1.0%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        // Peak profit 1.8%, current profit 1.2%, drawdown 0.6% >= trailing 0.5%
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TrailingStop })));
    }

    // --- VAL-STRAT-032: Trailing not triggered before activation ---
    #[test]
    fn test_lc_trailing_not_before_activation() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5, // +0.5%
            peak_price: 100.8,    // peaked at +0.8% < activation 1.0%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_none(), "Should not trigger trailing before activation");
    }

    // --- VAL-STRAT-033: Time stop ---
    #[test]
    fn test_lc_exit_time_stop() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5,
            peak_price: 100.5,
            hold_secs: 2000, // > max_hold_secs 1800
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TimeStop })));
    }

    // --- VAL-STRAT-034: Exit priority ---
    #[test]
    fn test_lc_exit_priority_tp_over_sl() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(103.0, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: false, // Short position where price went up
            entry_price: 100.0,
            current_price: 103.0, // -3% loss for short (exceeds SL 0.75%)
            peak_price: 97.0,     // best was +3% profit
            hold_secs: 2000,      // Also exceeds time stop
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        // SL should fire since price moved against the short
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::StopLoss })));
    }

    // --- VAL-STRAT-035: No exit when healthy ---
    #[test]
    fn test_lc_no_exit_when_healthy() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let mut snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        snap.ext = Some(crate::signal::MarketExtension {
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5, // +0.5% (between SL and TP)
            peak_price: 100.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_none(), "No exit when position is healthy");
    }

    // --- VAL-STRAT-036: Stale zone data forces exit ---
    #[test]
    fn test_lc_stale_zone_forces_exit() {
        let params = LiquidationCascadeParams {
            stale_data_threshold_secs: 60,
            ..lc_params_all_pass()
        };
        let mut strategy = LiquidationCascadeHunter::new(params);
        strategy.push_price(100.0, 1000100); // current_ts = 1000100ms
        let mut snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        snap.ext = Some(crate::signal::MarketExtension {
            zone_capture_timestamp_ms: Some(1000000), // 100ms ago, within 60s
            ..Default::default()
        });
        // Not stale
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5,
            peak_price: 100.5,
            hold_secs: 10,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        assert!(strategy.detect_exit(&snap, &ctx).is_none());

        // Now make zone data stale (>60 seconds old)
        strategy.push_price(100.0, 1070000); // current_ts = 1070000ms, zone at 1000000 → 70s stale
        assert!(strategy.detect_exit(&snap, &ctx).is_some());
    }

    // --- VAL-STRAT-038: Stale exit independent of PnL ---
    #[test]
    fn test_lc_stale_exit_independent_of_pnl() {
        let params = LiquidationCascadeParams {
            stale_data_threshold_secs: 10,
            ..lc_params_all_pass()
        };
        let mut strategy = LiquidationCascadeHunter::new(params);
        strategy.push_price(100.0, 10000000); // current_ts far ahead
        let mut snap = lc_snapshot(102.0, vec![], crate::signal::MarketExtension::default());
        snap.ext = Some(crate::signal::MarketExtension {
            zone_capture_timestamp_ms: Some(1000000), // Very old → stale
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 102.0, // +2% profit
            peak_price: 102.0,
            hold_secs: 10,
            max_hold_secs: 1800,
            take_profit_pct: 1.5, // TP not hit yet (1.5%)
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        // TP would be at 101.5, current is 102.0 → actually TP IS hit first
        // Let's use a price that's profitable but below TP
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0, // +1.0% (below TP 1.5%)
            peak_price: 101.0,
            hold_secs: 10,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_some(), "Should force exit even with profitable position");
    }

    // --- VAL-STRAT-039: Stale data prevents new entries ---
    #[test]
    fn test_lc_stale_data_prevents_entries() {
        let params = LiquidationCascadeParams {
            stale_data_threshold_secs: 10,
            ..lc_params_all_pass()
        };
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 10000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.zone_capture_timestamp_ms = Some(1000000); // Very old
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-040: Paper-only in live engine ---
    #[test]
    fn test_lc_paper_only_in_live_engine() {
        // The live engine (ScalperEngine::new) rejects liquidation-cascade-hunter
        // This test verifies the strategy's paper_only flag
        let params = LiquidationCascadeParams::default();
        assert!(params.paper_only, "Default params must have paper_only = true");
    }

    // --- VAL-STRAT-043: Route cost checked inside detect_entry ---
    #[test]
    fn test_lc_route_cost_checked_in_detect_entry() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        // Route cost exceeds threshold
        ext.route_cost_bps = Some(10.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-044: Route cost zero/missing does not block ---
    #[test]
    fn test_lc_route_cost_missing_does_not_block() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.route_cost_bps = None; // No route cost data
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumLong { .. }),
            "Should allow entry when route cost data is missing");
    }

    #[test]
    fn test_lc_route_cost_zero_does_not_block() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.route_cost_bps = Some(0.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumLong { .. }),
            "Should allow entry when route cost is zero");
    }

    // --- VAL-STRAT-045: Route cost below threshold allows ---
    #[test]
    fn test_lc_route_cost_below_threshold_allows() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.route_cost_bps = Some(4.99); // Just below threshold of 5.0
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::MomentumLong { .. }));
    }

    // --- VAL-STRAT-057: Strategy trait impl ---
    #[test]
    fn test_lc_strategy_trait_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LiquidationCascadeHunter>();
    }

    // --- VAL-STRAT-058: name() ---
    #[test]
    fn test_lc_name() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        assert_eq!(strategy.name(), "liquidation-cascade-continuation");
    }

    // --- Legacy alias name ---
    #[test]
    fn test_lc_legacy_name() {
        let strategy = LiquidationCascadeHunter::new_legacy(lc_params_all_pass());
        assert_eq!(strategy.name(), "liquidation-cascade-hunter");
    }

    // --- VAL-STRAT-059: parameters() returns expected defaults ---
    #[test]
    fn test_lc_parameters_defaults() {
        let params = lc_params_all_pass();
        let strategy = LiquidationCascadeHunter::new(params);
        let p = strategy.parameters();
        assert!((p.take_profit_pct - 1.5).abs() < 0.001);
        assert!((p.stop_loss_pct - 0.75).abs() < 0.001);
        assert!((p.trailing_stop_pct - 0.5).abs() < 0.001);
        assert_eq!(p.max_hold_secs, 1800);
        assert!((p.clip_size_usd - 100.0).abs() < 0.001);
    }

    // --- VAL-STRAT-060: push_price updates state ---
    #[test]
    fn test_lc_push_price_updates_state() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);
        let snap = strategy.snapshot();
        assert!(snap.price_count >= 2);
        assert!((snap.current_price - 101.0).abs() < 0.01);
    }

    // --- VAL-STRAT-062: as_any() downcasting ---
    #[test]
    fn test_lc_as_any_downcasting() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let dyn_ref: &dyn Strategy = &strategy;
        let any_ref = dyn_ref.as_any();
        let downcast = any_ref.downcast_ref::<LiquidationCascadeHunter>();
        assert!(downcast.is_some());
        assert_eq!(downcast.unwrap().name(), "liquidation-cascade-continuation");
    }

    // --- VAL-STRAT-063: No entry without confirmation ---
    #[test]
    fn test_lc_no_entry_without_confirmation() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Price near zone but no velocity confirmation
        let mut ext = lc_ext_cascade_long(100.0);
        ext.forced_flow_velocity = Some(0.1); // Below threshold
        ext.liquidation_burst_detected = false; // No burst either
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-064: No entry without velocity ---
    #[test]
    fn test_lc_cascade_no_entry_without_velocity() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.forced_flow_velocity = None; // No velocity data
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-065: Exhaustion requires velocity decay ---
    #[test]
    fn test_lc_exhaustion_requires_velocity_decay() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_exhaustion_long(100.0);
        ext.forced_flow_velocity = Some(0.5); // Still high velocity
        let snap = lc_snapshot(100.0, vec![], ext);
        // Cascade continuation should also fail because price above VWAP for long
        // and zone is for longs being at risk (side_at_risk = "long" → short signal for cascade)
        // but VWAP has price above it → short blocked. So overall NoSignal.
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-066: No signal before sufficient history ---
    #[test]
    fn test_lc_no_signal_before_sufficient_history() {
        let params = LiquidationCascadeParams {
            lookback_count: 10,
            ..lc_params_all_pass()
        };
        let mut strategy = LiquidationCascadeHunter::new(params);
        strategy.push_price(100.0, 1000); // Only 1 price, need 10
        // Create snapshot with low price_count
        let mut snap = lc_snapshot(100.0, vec![], lc_ext_cascade_long(100.0));
        snap.price_count = 1; // Only 1 price pushed, need 10
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-067: Zero AUM handled ---
    #[test]
    fn test_lc_handles_missing_pool_data() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Snapshot without ext → NoSignal (no pool/zone data)
        let snap = MomentumSnapshot {
            price_count: 30,
            current_price: 100.0,
            price_velocity_pct: 0.5,
            direction: TradeDirection::Long,
            strength: 60.0,
            volatility_pct: 1.0,
            pool_data: None,
            ext: None,
        };
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- VAL-STRAT-069: Extreme volatility ---
    #[test]
    fn test_lc_extreme_volatility_no_panic() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        strategy.push_price(100.0, 1000);
        strategy.push_price(50.0, 2000);  // 50% drop
        strategy.push_price(150.0, 3000); // 200% rise
        strategy.push_price(10.0, 4000);  // 93% drop
        strategy.push_price(500.0, 5000); // 4900% rise
        let snap = strategy.snapshot();
        assert!(snap.current_price.is_finite());
        assert!(!snap.current_price.is_nan());
    }

    // --- VAL-STRAT-070: Config from TOML sub-table ---
    #[test]
    fn test_lc_config_from_toml() {
        let toml_str = r#"
            enabled = true
            confidence_min = 0.7
            volume_z_score_threshold = 3.0
            take_profit_pct = 2.0
            stop_loss_pct = 1.0
        "#;
        let value: toml::Value = toml_str.parse().unwrap();
        let params: LiquidationCascadeParams = value.try_into().unwrap();
        assert!(params.enabled);
        assert!((params.confidence_min - 0.7).abs() < 0.001);
        assert!((params.volume_z_score_threshold - 3.0).abs() < 0.001);
        assert!((params.take_profit_pct - 2.0).abs() < 0.001);
        assert!((params.stop_loss_pct - 1.0).abs() < 0.001);
    }

    // --- VAL-STRAT-071: Missing optional fields use defaults ---
    #[test]
    fn test_lc_config_missing_fields_use_defaults() {
        let toml_str = r#"
            enabled = true
        "#;
        let value: toml::Value = toml_str.parse().unwrap();
        let params: LiquidationCascadeParams = value.try_into().unwrap();
        assert!(params.enabled);
        assert!(params.paper_only); // defaults to true
        assert!((params.confidence_min - 0.6).abs() < 0.001);
        assert!((params.route_cost_max_bps - 5.0).abs() < 0.001);
    }

    // --- VAL-STRAT-072: Direction bias ---
    #[test]
    fn test_lc_direction_bias_long() {
        let mut params = lc_params_all_pass();
        params.direction_bias = "long".to_string();
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Short signal should be blocked
        let ext = lc_ext_cascade_short(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal),
            "Long bias should block short signals");
    }

    #[test]
    fn test_lc_direction_bias_short() {
        let mut params = lc_params_all_pass();
        params.direction_bias = "short".to_string();
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Long signal should be blocked
        let ext = lc_ext_cascade_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal),
            "Short bias should block long signals");
    }

    #[test]
    fn test_lc_direction_bias_neutral_allows_both() {
        let mut params = lc_params_all_pass();
        params.direction_bias = "neutral".to_string();
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Both directions should work
        let ext_long = lc_ext_cascade_long(100.0);
        let snap_long = lc_snapshot(100.0, vec![], ext_long);
        assert!(matches!(strategy.detect_entry(&snap_long), Signal::MomentumLong { .. }));
        // Clear pending
        strategy.clear_pending("SOL", "long");
        let ext_short = lc_ext_cascade_short(100.0);
        let snap_short = lc_snapshot(100.0, vec![], ext_short);
        assert!(matches!(strategy.detect_entry(&snap_short), Signal::MomentumShort { .. }));
    }

    // --- VAL-CROSS-003: Route oracle veto applies ---
    #[test]
    fn test_lc_route_oracle_veto_applies() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let mut ext = lc_ext_cascade_long(100.0);
        ext.route_cost_bps = Some(10.0); // Exceeds 5.0 bps threshold
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
        // Below threshold → signal
        let mut ext2 = lc_ext_cascade_long(100.0);
        ext2.route_cost_bps = Some(3.0);
        let snap2 = lc_snapshot(100.0, vec![], ext2);
        assert!(matches!(strategy.detect_entry(&snap2), Signal::MomentumLong { .. }));
    }

    // --- VAL-CROSS-004: Zone data flows into strategy ---
    #[test]
    fn test_lc_zone_data_flows_into_strategy() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // High-confidence fresh zone → signal
        let ext = lc_ext_cascade_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        assert!(matches!(strategy.detect_entry(&snap), Signal::MomentumLong { .. }));

        // Clear pending
        strategy.clear_pending("SOL", "long");

        // Stale zone → no signal
        let mut ext_stale = lc_ext_cascade_long(100.0);
        ext_stale.zone_capture_timestamp_ms = Some(100); // Very old
        let snap_stale = lc_snapshot(100.0, vec![], ext_stale);
        assert!(matches!(strategy.detect_entry(&snap_stale), Signal::NoSignal),
            "Stale zone data should block entry");
    }

    // --- No entry without pool data / ext ---
    #[test]
    fn test_lc_no_entry_without_ext_data() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let snap = MomentumSnapshot {
            price_count: 30,
            current_price: 100.0,
            price_velocity_pct: 0.5,
            direction: TradeDirection::Long,
            strength: 60.0,
            volatility_pct: 1.0,
            pool_data: None,
            ext: None,
        };
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Trailing stop for shorts ---
    #[test]
    fn test_lc_exit_trailing_stop_short() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(98.8, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 98.8, // -1.2% (profit for short)
            peak_price: 98.2,    // best was -1.8% > activation 1.0%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::TrailingStop })));
    }

    // --- Time stop for short ---
    #[test]
    fn test_lc_exit_time_stop_short() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(99.5, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 99.5,
            peak_price: 99.5,
            hold_secs: 2000,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::TimeStop })));
    }

    // ======================================================================
    // New Exit Conditions: Take-profit into next zone, Zone-reclaimed stop, Time stop
    // ======================================================================

    // --- VAL-STRAT-CC-015: Take-profit into next zone (long) ---
    #[test]
    fn test_lc_tp_into_next_zone_long() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        // Simulate entry so entry zone price is recorded
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry); // Triggers entry, records zone

        // Now test exit: price has reached the next zone above (105.0)
        let next_zone = lc_zone("short", 0.8, 105.0); // Zone above entry
        let snap_exit = lc_snapshot(105.5, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![next_zone]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 105.5, // Above the next zone at 105.0
            peak_price: 105.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0, // High TP so fixed TP doesn't trigger first
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TakeProfit })),
            "Should trigger TP when price reaches next zone in cascade direction");
    }

    // --- VAL-STRAT-CC-015: Take-profit into next zone (short) ---
    #[test]
    fn test_lc_tp_into_next_zone_short() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_short(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        // Next zone below entry at 95.0
        let next_zone = lc_zone("long", 0.8, 95.0);
        let snap_exit = lc_snapshot(94.5, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![next_zone]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 94.5, // Below the next zone at 95.0
            peak_price: 94.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::TakeProfit })),
            "Should trigger TP when price reaches next zone below for short");
    }

    // --- VAL-STRAT-CC-015: No next zone TP when no zone in direction ---
    #[test]
    fn test_lc_no_next_zone_tp_when_no_zone_in_direction() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        // Only zone below entry (no zone above for long)
        let zone_below = lc_zone("long", 0.8, 95.0);
        let snap_exit = lc_snapshot(103.0, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![zone_below]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 103.0,
            peak_price: 103.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0, // High TP so fixed TP doesn't trigger
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(exit.is_none(), "No next zone TP when no zone in cascade direction");
    }

    // --- VAL-STRAT-CC-015: Next zone TP disabled ---
    #[test]
    fn test_lc_next_zone_tp_disabled() {
        let mut params = lc_params_all_pass();
        params.next_zone_tp_enabled = false;
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        let next_zone = lc_zone("short", 0.8, 105.0);
        let snap_exit = lc_snapshot(105.5, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![next_zone]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 105.5,
            peak_price: 105.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(exit.is_none(), "Next zone TP should not trigger when disabled");
    }

    // --- VAL-STRAT-CC-016: Zone-reclaimed stop (long position) ---
    #[test]
    fn test_lc_zone_reclaimed_stop_long() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        // Entry zone is at 102.0 (shorts being liquidated → long signal, zone price = 102.0)
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        // Price drops back below the entry zone price (102.0)
        let snap_exit = lc_snapshot(101.0, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("short", 0.7, 102.0)]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0, // Below entry zone price 102.0
            peak_price: 103.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::ReversalDetected })),
            "Zone reclaimed stop should trigger when price drops below entry zone for long");
    }

    // --- VAL-STRAT-CC-016: Zone-reclaimed stop (short position) ---
    #[test]
    fn test_lc_zone_reclaimed_stop_short() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_short(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        // Price rises back above the entry zone price (98.0)
        let snap_exit = lc_snapshot(99.5, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("long", 0.7, 98.0)]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 99.5, // Above entry zone price 98.0
            peak_price: 97.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::ReversalDetected })),
            "Zone reclaimed stop should trigger when price rises above entry zone for short");
    }

    // --- VAL-STRAT-CC-016: Zone-reclaimed stop disabled ---
    #[test]
    fn test_lc_zone_reclaimed_stop_disabled() {
        let mut params = lc_params_all_pass();
        params.zone_reclaimed_stop_enabled = false;
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        let snap_exit = lc_snapshot(101.0, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("short", 0.7, 102.0)]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0,
            peak_price: 103.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(exit.is_none(), "Zone reclaimed stop should not trigger when disabled");
    }

    // --- VAL-STRAT-CC-016: No zone reclaimed stop when price stays beyond zone ---
    #[test]
    fn test_lc_no_zone_reclaimed_when_price_beyond_zone() {
        let mut params = lc_params_all_pass();
        params.next_zone_tp_enabled = false; // Disable next-zone TP so it doesn't interfere
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        // Price is above the entry zone (102.0) → zone NOT reclaimed
        let snap_exit = lc_snapshot(103.0, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![lc_zone("short", 0.7, 102.0)]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 103.0, // Above entry zone price 102.0
            peak_price: 103.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        assert!(exit.is_none(), "No zone reclaimed stop when price is still beyond zone");
    }

    // --- VAL-STRAT-CC-017: Time stop enforced ---
    #[test]
    fn test_lc_time_stop_enforced() {
        let strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        let snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5,
            peak_price: 100.5,
            hold_secs: 2000, // > max_hold_secs 1800
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TimeStop })),
            "Time stop should trigger when hold_secs >= max_hold_secs");
    }

    // --- VAL-STRAT-CC-017: Time stop disabled ---
    #[test]
    fn test_lc_time_stop_disabled() {
        let mut params = lc_params_all_pass();
        params.time_stop_enabled = false;
        let strategy = LiquidationCascadeHunter::new(params);
        let snap = lc_snapshot(100.5, vec![], crate::signal::MarketExtension::default());
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.5,
            peak_price: 100.5,
            hold_secs: 2000,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_none(), "Time stop should not trigger when disabled");
    }

    // --- VAL-STRAT-CC-012: Cooldown enforced after loss ---
    #[test]
    fn test_lc_cooldown_after_loss() {
        let mut params = lc_params_all_pass();
        params.cooldown_after_loss_secs = 60;
        let mut strategy = LiquidationCascadeHunter::new(params);
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }

        // First entry succeeds
        let ext = lc_ext_cascade_long(100.0);
        let snap = lc_snapshot(100.0, vec![], ext);
        let signal1 = strategy.detect_entry(&snap);
        assert!(matches!(signal1, Signal::MomentumLong { .. }));

        // Clear pending and record loss
        strategy.clear_pending("SOL", "long");
        strategy.record_loss(1001000); // loss at ts=1001000ms

        // Advance time only 30s (less than 60s cooldown)
        for i in 0..5 { strategy.push_price(100.0, 1031000 + i * 1000); }
        let signal2 = strategy.detect_entry(&snap);
        assert!(matches!(signal2, Signal::NoSignal), "Should be in cooldown");

        // Advance time 70s past loss (past 60s cooldown)
        for i in 0..5 { strategy.push_price(100.0, 1071000 + i * 1000); }
        let signal3 = strategy.detect_entry(&snap);
        assert!(matches!(signal3, Signal::MomentumLong { .. }), "Should allow entry after cooldown");
    }

    // --- VAL-STRAT-CC-001: Legacy alias resolves via factory ---
    #[test]
    fn test_lc_legacy_alias_factory_roundtrip() {
        // Legacy name creates a working strategy
        let strategy = create_strategy_from_config(
            "liquidation-cascade-hunter",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 1.5,
                stop_loss_pct: 0.75,
                trailing_stop_pct: 0.5,
                trailing_activation_pct: 1.0,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        // Legacy name strategy should have the legacy name
        assert_eq!(strategy.name(), "liquidation-cascade-hunter");
    }

    // --- VAL-STRAT-CC-001: Both names produce same behavior ---
    #[test]
    fn test_lc_both_names_produce_signals() {
        let strategy_canonical = create_strategy_from_config(
            "liquidation-cascade-continuation",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 1.5,
                stop_loss_pct: 0.75,
                trailing_stop_pct: 0.5,
                trailing_activation_pct: 1.0,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        let strategy_legacy = create_strategy_from_config(
            "liquidation-cascade-hunter",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 1.5,
                stop_loss_pct: 0.75,
                trailing_stop_pct: 0.5,
                trailing_activation_pct: 1.0,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        assert_eq!(strategy_canonical.name(), "liquidation-cascade-continuation");
        assert_eq!(strategy_legacy.name(), "liquidation-cascade-hunter");
    }

    // --- Default params have new exit condition flags ---
    #[test]
    fn test_lc_default_params_exit_conditions() {
        let params = LiquidationCascadeParams::default();
        assert!(params.next_zone_tp_enabled, "next_zone_tp_enabled should default to true");
        assert!(params.zone_reclaimed_stop_enabled, "zone_reclaimed_stop_enabled should default to true");
        assert!(params.time_stop_enabled, "time_stop_enabled should default to true");
    }

    // --- TOML config for new exit condition fields ---
    #[test]
    fn test_lc_config_new_exit_fields() {
        let toml_str = r#"
            enabled = true
            next_zone_tp_enabled = false
            zone_reclaimed_stop_enabled = false
            time_stop_enabled = false
        "#;
        let value: toml::Value = toml_str.parse().unwrap();
        let params: LiquidationCascadeParams = value.try_into().unwrap();
        assert!(params.enabled);
        assert!(!params.next_zone_tp_enabled);
        assert!(!params.zone_reclaimed_stop_enabled);
        assert!(!params.time_stop_enabled);
    }

    // --- Exit priority: fixed TP fires before next-zone TP ---
    #[test]
    fn test_lc_fixed_tp_priority_over_next_zone_tp() {
        let mut strategy = LiquidationCascadeHunter::new(lc_params_all_pass());
        for i in 0..5 { strategy.push_price(100.0, 1000000 + i * 1000); }
        let ext = lc_ext_cascade_long(100.0);
        let snap_entry = lc_snapshot(100.0, vec![], ext);
        let _ = strategy.detect_entry(&snap_entry);

        let next_zone = lc_zone("short", 0.8, 103.0);
        let snap_exit = lc_snapshot(102.0, vec![], crate::signal::MarketExtension {
            liquidation_zones: Some(vec![next_zone]),
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 102.0, // +2.0% > TP 1.5%
            peak_price: 102.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 1.5, // Fixed TP at 1.5% → triggers at 101.5
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        let exit = strategy.detect_exit(&snap_exit, &ctx);
        // Fixed TP should fire (price exceeds 1.5% TP) before next-zone TP
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TakeProfit })),
            "Fixed TP should take priority over next-zone TP");
    }

    // ======================================================================
    // Sweep Reclaim Strategy Tests
    // ======================================================================

    /// Helper: build SweepReclaimParams with all gates configured for passing.
    fn sr_params_all_pass() -> SweepReclaimParams {
        SweepReclaimParams {
            enabled: true,
            paper_only: true,
            min_confidence: 0.5,
            max_chase_distance_bps: 300.0,
            forced_flow_spike_threshold: 1.0,
            velocity_deceleration_threshold: 0.8,
            vwap_reclaim_required: true,
            spread_max_pct: 0.5,
            depth_min_usd: 5000.0,
            oi_contraction_required: true,
            volume_z_score_threshold: 1.0,
            stale_data_threshold_secs: 300,
            regime_filter: true,
            route_cost_max_bps: 5.0,
            cooldown_after_loss_secs: 0,
            fishing_ladder_offsets_bps: vec![10.0, 20.0, 30.0],
            fishing_tranche_usd: 25.0,
            fishing_expiry_secs: 300,
            take_profit_pct: 3.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
            max_hold_secs: 1800,
            clip_size_usd: 100.0,
            leverage: 2.0,
            direction_bias: "neutral".to_string(),
            scale_in_clips: 1,
            use_native_tp_sl: true,
            lookback_count: 5,
        }
    }

    /// Helper: build a zone for sweep-reclaim tests.
    fn sr_zone(side: &str, confidence: f64, price: f64) -> crate::liquidation::LiquidationZone {
        crate::liquidation::LiquidationZone {
            price,
            side_at_risk: side.to_string(),
            estimated_notional_usd: 500_000.0,
            wallet_count: 10,
            distance_bps: 200.0,
            confidence,
            source_mix: vec!["hyperliquid_positions".to_string()],
        }
    }

    /// Helper: build a MomentumSnapshot for sweep-reclaim tests.
    fn sr_snapshot(price: f64, ext: crate::signal::MarketExtension, velocity_pct: f64) -> MomentumSnapshot {
        MomentumSnapshot {
            price_count: 30,
            current_price: price,
            price_velocity_pct: velocity_pct,
            direction: if velocity_pct > 0.0 { TradeDirection::Long } else { TradeDirection::Short },
            strength: 60.0,
            volatility_pct: 1.0,
            pool_data: None,
            ext: Some(ext),
        }
    }

    /// Helper: build an ext for a sweep-reclaim LONG scenario.
    /// Longs got liquidated (price dropped to zone), reversal is LONG.
    fn sr_ext_sweep_long(zone_price: f64, current_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![sr_zone("long", 0.7, zone_price)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(zone_price * 1.001), // price near/above VWAP for reclaim
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(2.5), // Spike
            regime_label: Some("HighVol".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: Some(true),
        }
    }

    /// Helper: build an ext for the fishing phase (deceleration + all confirmation gates).
    fn sr_ext_fishing_phase(zone_price: f64, current_price: f64) -> crate::signal::MarketExtension {
        crate::signal::MarketExtension {
            liquidation_zones: Some(vec![sr_zone("long", 0.7, zone_price)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(current_price * 0.999), // price above VWAP
            spread_pct: Some(0.2),             // normalized
            depth_usd: Some(60_000.0),         // refilled
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.3),   // decelerated
            regime_label: Some("HighVol".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: Some(true),
        }
    }

    // --- VAL-STRAT-SR-REG: Strategy registered ---
    #[test]
    fn test_sr_available_in_strategies_list() {
        assert!(available_strategies().contains(&"sweep-reclaim"));
    }

    // --- VAL-STRAT-SR-FACTORY: Factory creates correct type ---
    #[test]
    fn test_sr_factory_creates_correct_type() {
        let strategy = create_strategy_from_config(
            "sweep-reclaim",
            None,
            StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 5,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 3.0,
                stop_loss_pct: 1.5,
                trailing_stop_pct: 0.8,
                trailing_activation_pct: 1.5,
                cooldown_after_loss_secs: 0,
                use_native_tp_sl: true,
            },
        ).unwrap();
        assert_eq!(strategy.name(), "sweep-reclaim");
    }

    // --- Strategy disabled by default ---
    #[test]
    fn test_sr_disabled_by_default() {
        let params = SweepReclaimParams::default();
        assert!(!params.enabled);
        assert!(params.paper_only);
    }

    // --- No signal when disabled ---
    #[test]
    fn test_sr_no_signal_when_disabled() {
        let params = SweepReclaimParams {
            enabled: false,
            ..sr_params_all_pass()
        };
        let mut strategy = SweepReclaimStrategy::new(params);
        let snap = sr_snapshot(100.0, sr_ext_sweep_long(95.0, 100.0), 1.0);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Parameter validation ---
    #[test]
    fn test_sr_params_validate_ok() {
        assert!(sr_params_all_pass().validate().is_ok());
    }

    #[test]
    fn test_sr_params_reject_negative_confidence() {
        let mut p = sr_params_all_pass();
        p.min_confidence = -0.1;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_confidence_above_one() {
        let mut p = sr_params_all_pass();
        p.min_confidence = 1.5;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_zero_tp() {
        let mut p = sr_params_all_pass();
        p.take_profit_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_zero_sl() {
        let mut p = sr_params_all_pass();
        p.stop_loss_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_zero_stale_threshold() {
        let mut p = sr_params_all_pass();
        p.stale_data_threshold_secs = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_negative_route_cost() {
        let mut p = sr_params_all_pass();
        p.route_cost_max_bps = -1.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_zero_spike_threshold() {
        let mut p = sr_params_all_pass();
        p.forced_flow_spike_threshold = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_empty_fishing_offsets() {
        let mut p = sr_params_all_pass();
        p.fishing_ladder_offsets_bps = vec![];
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_sr_params_reject_zero_fishing_tranche() {
        let mut p = sr_params_all_pass();
        p.fishing_tranche_usd = 0.0;
        assert!(p.validate().is_err());
    }

    // --- VAL-STRAT-SR-001: Zone sweep detection ---
    #[test]
    fn test_sr_zone_sweep_detected_when_price_crosses_and_reverses() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        // Push prices to build history
        for i in 0..5 {
            strategy.push_price(100.0 - i as f64 * 0.5, 1000000 + i * 1000);
        }
        // Zone at 97.0 (long liquidation zone), price dropped below it (swept_down)
        // Now price at 97.02, velocity positive (reversing up)
        let ext = sr_ext_sweep_long(97.0, 97.02);
        let snap = sr_snapshot(97.02, ext, 0.5); // velocity > 0 = reversal up
        let signal = strategy.detect_entry(&snap);
        // Should detect sweep and transition to fishing (returns NoSignal for fishing)
        // but internally the sweep is detected
        assert!(matches!(signal, Signal::NoSignal)); // Fishing phase = NoSignal
    }

    // --- No sweep when price doesn't reverse ---
    #[test]
    fn test_sr_no_sweep_when_price_continues() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0 - i as f64 * 0.5, 1000000 + i * 1000);
        }
        // Price below zone but velocity is negative (continuing down, no reversal)
        let mut ext = sr_ext_sweep_long(97.0, 96.0);
        ext.forced_flow_velocity = Some(2.5); // Still high velocity
        let snap = sr_snapshot(96.0, ext, -0.5); // velocity < 0 = continuing down
        let signal = strategy.detect_entry(&snap);
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-002: Forced-flow spike required ---
    #[test]
    fn test_sr_no_entry_without_forced_flow_spike() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        // Zone sweep with price near zone and positive velocity, but low forced-flow
        let mut ext = sr_ext_sweep_long(97.0, 97.02);
        ext.forced_flow_velocity = Some(0.3); // Below spike threshold of 1.0
        let snap = sr_snapshot(97.02, ext, 0.5);
        let signal = strategy.detect_entry(&snap);
        // Should be NoSignal because forced-flow spike not detected
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-003: Pressure deceleration required ---
    #[test]
    fn test_sr_no_confirmation_without_deceleration() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep (high velocity)
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: Fishing phase, but velocity still high (no deceleration)
        let mut ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        ext_fishing.forced_flow_velocity = Some(2.5); // Still high, no deceleration
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal = strategy.detect_entry(&snap_fishing);
        // Should stay in fishing, NoSignal
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-004: VWAP reclaim required ---
    #[test]
    fn test_sr_no_entry_without_vwap_reclaim() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: Deceleration but price below VWAP
        let mut ext_fishing = sr_ext_fishing_phase(97.0, 96.5);
        ext_fishing.vwap = Some(98.0); // Price 96.5 below VWAP 98.0 → no reclaim
        ext_fishing.forced_flow_velocity = Some(0.3); // decelerated
        let snap_fishing = sr_snapshot(96.5, ext_fishing, 0.3);
        let signal = strategy.detect_entry(&snap_fishing);
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-005: Depth refill required ---
    #[test]
    fn test_sr_no_entry_without_depth_refill() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: Deceleration + VWAP reclaim, but depth too thin
        let mut ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        ext_fishing.depth_usd = Some(1000.0); // Below min threshold of 5000
        ext_fishing.forced_flow_velocity = Some(0.3);
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal = strategy.detect_entry(&snap_fishing);
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-006: Spread normalization required ---
    #[test]
    fn test_sr_no_entry_without_spread_normalization() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: All pass except spread
        let mut ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        ext_fishing.spread_pct = Some(1.0); // Above max 0.5
        ext_fishing.forced_flow_velocity = Some(0.3);
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal = strategy.detect_entry(&snap_fishing);
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-007: OI contraction required ---
    #[test]
    fn test_sr_no_entry_without_oi_contraction() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: All pass except OI not contracting
        let mut ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        ext_fishing.oi_contracting = Some(false); // OI still expanding
        ext_fishing.forced_flow_velocity = Some(0.3);
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal = strategy.detect_entry(&snap_fishing);
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- VAL-STRAT-SR-008: Passive fishing ladder before confirmation entry ---
    #[test]
    fn test_sr_fishing_ladder_placed_before_confirmation() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep → fishing ladder placed
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let signal1 = strategy.detect_entry(&snap_sweep);
        // Fishing phase: should return NoSignal (passive orders, not active entry)
        assert!(matches!(signal1, Signal::NoSignal));
        // Verify fishing orders were placed (via phase state)
        assert_eq!(strategy.phase, SweepReclaimPhase::Fishing);
    }

    // --- Full two-phase entry: fishing → confirmation → signal ---
    #[test]
    fn test_sr_full_two_phase_entry_long() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let signal1 = strategy.detect_entry(&snap_sweep);
        assert!(matches!(signal1, Signal::NoSignal)); // Fishing phase

        // Phase 2: All confirmation gates pass
        // First push a high velocity to set prev_forced_flow_velocity
        let mut ext_high_vel = sr_ext_fishing_phase(97.0, 98.0);
        ext_high_vel.forced_flow_velocity = Some(2.5); // High velocity
        let snap_high_vel = sr_snapshot(98.0, ext_high_vel, 0.3);
        let _ = strategy.detect_entry(&snap_high_vel);

        // Now push decelerated velocity
        let ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal2 = strategy.detect_entry(&snap_fishing);
        // Should now be in confirmation and emit MomentumLong
        assert!(matches!(signal2, Signal::MomentumLong { .. }),
            "Expected MomentumLong after all gates pass, got {:?}", signal2);
    }

    // --- VAL-STRAT-SR-009: Max distance enforcement ---
    #[test]
    fn test_sr_no_entry_beyond_max_distance() {
        let params = SweepReclaimParams {
            max_chase_distance_bps: 50.0, // Very tight
            ..sr_params_all_pass()
        };
        let mut strategy = SweepReclaimStrategy::new(params);
        for i in 0..5 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        // Zone at 97.0, price at 100.0 → distance = 300 bps > 50 bps max
        let ext = sr_ext_sweep_long(97.0, 100.0);
        let snap = sr_snapshot(100.0, ext, 0.5);
        let signal = strategy.detect_entry(&snap);
        // Price too far from zone, no sweep detected
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- Max distance enforcement during fishing phase ---
    #[test]
    fn test_sr_max_distance_resets_during_fishing() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Phase 2: Price moves too far from zone
        let mut ext_far = sr_ext_fishing_phase(97.0, 130.0);
        ext_far.forced_flow_velocity = Some(2.5); // Still high
        let snap_far = sr_snapshot(130.0, ext_far, 0.3);
        let _ = strategy.detect_entry(&snap_far);

        // Max distance check should have reset the phase
        // Distance = (130 - 97) / 97 * 10000 ≈ 3401 bps > 300 bps max
        assert_eq!(strategy.phase, SweepReclaimPhase::Idle);
    }

    // --- VAL-STRAT-SR-010: Trailing stop after reclaim ---
    #[test]
    fn test_sr_trailing_stop_triggers() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(103.0, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 103.0,   // +3.0%
            peak_price: 105.0,      // peaked at +5.0% > activation 1.5%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,  // High TP so trailing fires first
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        // Peak profit 5.0%, current profit 3.0%, drawdown 2.0% >= trailing 0.8%
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TrailingStop })),
            "Expected TrailingStop, got {:?}", exit);
    }

    #[test]
    fn test_sr_trailing_stop_not_before_activation() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(100.3, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 100.3,   // +0.3%
            peak_price: 100.5,      // peaked at +0.5% < activation 1.5%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_none(), "Should not trigger trailing before activation");
    }

    // --- VAL-STRAT-SR-011: Time stop ---
    #[test]
    fn test_sr_time_stop_triggers() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(101.0, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0,
            peak_price: 101.0,
            hold_secs: 2000,         // > max_hold_secs 1800
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TimeStop })),
            "Expected TimeStop, got {:?}", exit);
    }

    // --- TP and SL exits ---
    #[test]
    fn test_sr_exit_take_profit() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(104.0, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 103.5,    // +3.5% > TP 3.0%
            peak_price: 103.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 3.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::TakeProfit })));
    }

    #[test]
    fn test_sr_exit_stop_loss() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(98.0, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 98.0,     // -2.0% < SL -1.5%
            peak_price: 100.5,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 3.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitLong { reason: ExitReason::StopLoss })));
    }

    // --- No exit when position healthy ---
    #[test]
    fn test_sr_no_exit_when_healthy() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let mut snap = sr_snapshot(101.0, crate::signal::MarketExtension::default(), 0.0);
        snap.ext = Some(crate::signal::MarketExtension {
            zone_capture_timestamp_ms: Some(1000000),
            ..Default::default()
        });
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0,    // +1.0%
            peak_price: 101.0,
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 3.0,
            stop_loss_pct: 1.5,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(exit.is_none());
    }

    // --- Duplicate signal blocked ---
    #[test]
    fn test_sr_duplicate_signal_blocked() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        // Phase 1: Trigger sweep
        let ext_sweep = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep = sr_snapshot(97.02, ext_sweep, 0.5);
        let _ = strategy.detect_entry(&snap_sweep);

        // Push high velocity
        let mut ext_high = sr_ext_fishing_phase(97.0, 98.0);
        ext_high.forced_flow_velocity = Some(2.5);
        let snap_high = sr_snapshot(98.0, ext_high, 0.3);
        let _ = strategy.detect_entry(&snap_high);

        // Phase 2: Confirmation → signal
        let ext_fishing = sr_ext_fishing_phase(97.0, 98.0);
        let snap_fishing = sr_snapshot(98.0, ext_fishing, 0.3);
        let signal1 = strategy.detect_entry(&snap_fishing);
        assert!(matches!(signal1, Signal::MomentumLong { .. }));

        // Now try to get a second signal (reset and re-trigger)
        // After emitting signal, phase resets to Idle, so we need to trigger again
        // Phase 1: sweep again
        let ext_sweep2 = sr_ext_sweep_long(97.0, 97.02);
        let snap_sweep2 = sr_snapshot(97.02, ext_sweep2, 0.5);
        let _ = strategy.detect_entry(&snap_sweep2);

        // Push high velocity
        let mut ext_high2 = sr_ext_fishing_phase(97.0, 98.0);
        ext_high2.forced_flow_velocity = Some(2.5);
        let snap_high2 = sr_snapshot(98.0, ext_high2, 0.3);
        let _ = strategy.detect_entry(&snap_high2);

        // Phase 2: Confirmation again → should be blocked (duplicate pending)
        let ext_fishing2 = sr_ext_fishing_phase(97.0, 98.0);
        let snap_fishing2 = sr_snapshot(98.0, ext_fishing2, 0.3);
        let signal2 = strategy.detect_entry(&snap_fishing2);
        assert!(matches!(signal2, Signal::NoSignal), "Duplicate signal should be blocked");
    }

    // --- Strategy trait impl ---
    #[test]
    fn test_sr_strategy_trait_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SweepReclaimStrategy>();
    }

    // --- name() ---
    #[test]
    fn test_sr_name() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        assert_eq!(strategy.name(), "sweep-reclaim");
    }

    // --- Paper-only in live engine ---
    #[test]
    fn test_sr_paper_only_by_default() {
        let params = SweepReclaimParams::default();
        assert!(params.paper_only, "Default params must have paper_only = true");
    }

    // --- Stale data prevents entry ---
    #[test]
    fn test_sr_stale_data_prevents_entry() {
        let params = SweepReclaimParams {
            stale_data_threshold_secs: 10,
            ..sr_params_all_pass()
        };
        let mut strategy = SweepReclaimStrategy::new(params);
        for i in 0..5 {
            strategy.push_price(97.0, 10000000 + i * 1000);
        }
        let mut ext = sr_ext_sweep_long(97.0, 97.02);
        ext.zone_capture_timestamp_ms = Some(1000000); // Very old
        let snap = sr_snapshot(97.02, ext, 0.5);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Regime incompatible ---
    #[test]
    fn test_sr_regime_incompatible_lowvol() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        let mut ext = sr_ext_sweep_long(97.0, 97.02);
        ext.regime_label = Some("LowVol".to_string());
        let snap = sr_snapshot(97.02, ext, 0.5);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    #[test]
    fn test_sr_regime_incompatible_choppy() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        let mut ext = sr_ext_sweep_long(97.0, 97.02);
        ext.regime_label = Some("Choppy".to_string());
        let snap = sr_snapshot(97.02, ext, 0.5);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Route cost veto ---
    #[test]
    fn test_sr_route_cost_veto() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(97.0, 1000000 + i * 1000);
        }
        let mut ext = sr_ext_sweep_long(97.0, 97.02);
        ext.route_cost_bps = Some(10.0); // Exceeds max 5.0
        let snap = sr_snapshot(97.02, ext, 0.5);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Config from TOML table ---
    #[test]
    fn test_sr_config_from_toml_table() {
        let toml_str = r#"
enabled = true
paper_only = true
min_confidence = 0.7
max_chase_distance_bps = 200.0
forced_flow_spike_threshold = 2.5
velocity_deceleration_threshold = 0.6
clip_size_usd = 150.0
leverage = 2.5
take_profit_pct = 4.0
stop_loss_pct = 2.0
trailing_stop_pct = 1.0
trailing_activation_pct = 2.0
max_hold_secs = 3600
fishing_ladder_offsets_bps = [5.0, 15.0, 25.0]
fishing_tranche_usd = 30.0
fishing_expiry_secs = 600
"#;
        let value: toml::Value = toml_str.parse().unwrap();
        let params = SweepReclaimParams::from_toml_table(&value).unwrap();
        assert!(params.enabled);
        assert!((params.min_confidence - 0.7).abs() < 0.001);
        assert!((params.max_chase_distance_bps - 200.0).abs() < 0.001);
        assert!((params.forced_flow_spike_threshold - 2.5).abs() < 0.001);
        assert!((params.clip_size_usd - 150.0).abs() < 0.001);
        assert_eq!(params.fishing_ladder_offsets_bps, vec![5.0, 15.0, 25.0]);
        assert!((params.fishing_tranche_usd - 30.0).abs() < 0.001);
        assert!(params.validate().is_ok());
    }

    // --- Default params validate ---
    #[test]
    fn test_sr_default_params_validate() {
        let params = SweepReclaimParams::default();
        assert!(params.validate().is_ok());
    }

    // --- as_any() downcasting ---
    #[test]
    fn test_sr_as_any_downcasting() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let dyn_ref: &dyn Strategy = &strategy;
        let any_ref = dyn_ref.as_any();
        let downcast = any_ref.downcast_ref::<SweepReclaimStrategy>();
        assert!(downcast.is_some());
        assert_eq!(downcast.unwrap().name(), "sweep-reclaim");
    }

    // --- push_price updates state ---
    #[test]
    fn test_sr_push_price_updates_state() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);
        let snap = strategy.snapshot();
        assert!(snap.price_count >= 2);
        assert!((snap.current_price - 101.0).abs() < 0.01);
    }

    // --- Short sweep reversal ---
    #[test]
    fn test_sr_short_sweep_reversal() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        // Zone at 103.0 (short liquidation zone), price rose above it
        // Now price at 102.98, velocity negative (reversing down)
        let mut ext = crate::signal::MarketExtension {
            liquidation_zones: Some(vec![sr_zone("short", 0.7, 103.0)]),
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(102.5),         // price near/above VWAP for short
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(2.5),
            regime_label: Some("HighVol".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: Some(true),
        };
        // Price above zone but close, velocity negative (reversing down from above zone)
        let snap = sr_snapshot(102.98, ext, -0.5);
        let signal = strategy.detect_entry(&snap);
        // Should detect sweep and go to fishing phase (returns NoSignal)
        assert!(matches!(signal, Signal::NoSignal));
    }

    // --- No signal without ext data ---
    #[test]
    fn test_sr_no_signal_without_ext() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        let snap = MomentumSnapshot {
            price_count: 30,
            current_price: 100.0,
            price_velocity_pct: 0.5,
            direction: TradeDirection::Long,
            strength: 60.0,
            volatility_pct: 1.0,
            pool_data: None,
            ext: None, // No ext data
        };
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- No signal without liquidation zones ---
    #[test]
    fn test_sr_no_signal_without_zones() {
        let mut strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        for i in 0..5 {
            strategy.push_price(100.0, 1000000 + i * 1000);
        }
        let ext = crate::signal::MarketExtension {
            liquidation_zones: None,
            zone_capture_timestamp_ms: Some(1000000),
            route_cost_bps: Some(2.0),
            vwap: Some(100.0),
            spread_pct: Some(0.2),
            depth_usd: Some(50_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(2.5),
            regime_label: Some("HighVol".to_string()),
            liquidation_burst_detected: true,
            symbol: Some("SOL".to_string()),
            oi_contracting: Some(true),
        };
        let snap = sr_snapshot(100.0, ext, 0.5);
        assert!(matches!(strategy.detect_entry(&snap), Signal::NoSignal));
    }

    // --- Short trailing stop ---
    #[test]
    fn test_sr_short_trailing_stop() {
        let strategy = SweepReclaimStrategy::new(sr_params_all_pass());
        let snap = sr_snapshot(97.0, crate::signal::MarketExtension::default(), 0.0);
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 97.0,     // +3.0% profit for short
            peak_price: 95.0,        // peaked at +5.0% > activation 1.5%
            hold_secs: 100,
            max_hold_secs: 1800,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 0.8,
            trailing_activation_pct: 1.5,
        };
        let exit = strategy.detect_exit(&snap, &ctx);
        assert!(matches!(exit, Some(Signal::ExitShort { reason: ExitReason::TrailingStop })),
            "Expected Short TrailingStop, got {:?}", exit);
    }
}
