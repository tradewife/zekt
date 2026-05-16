//! Strategy trait and implementations for the Zekt trading system.
//!
//! This module defines the `Strategy` trait that all trading strategies must implement.
//! It also provides the `MomentumScalperStrategy` (extracted from the original `MomentumDetector`)
//! and a centralized factory function for strategy instantiation.

use crate::signal::{
    MomentumDetector, MomentumSnapshot, Signal,
};
use serde::{Deserialize, Serialize};

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
}

// ---------------------------------------------------------------------------
// Strategy Factory
// ---------------------------------------------------------------------------

/// Canonical list of all registered strategy names.
pub fn available_strategies() -> &'static [&'static str] {
    &["momentum-scalper"]
}

/// Create a strategy instance by name and parameters.
///
/// This is the single point where strategy names are mapped to concrete types.
/// Both `ScalperEngine` and `PaperEngine` should use this function.
pub fn create_strategy(name: &str, params: StrategyParams) -> anyhow::Result<Box<dyn Strategy>> {
    match name {
        "momentum-scalper" => {
            // Validate params before creating
            if let Err(e) = params.validate() {
                anyhow::bail!("Invalid strategy parameters for '{}': {}", name, e);
            }
            Ok(Box::new(MomentumScalperStrategy::new(params)))
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::{ExitReason, Signal};

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
    /// We need prices rising enough to exceed the threshold and strength >= 50.
    fn feed_rising_prices(strategy: &mut dyn Strategy, start: f64, count: usize) {
        let base_ts = 1000000_i64;
        for i in 0..count {
            let price = start * (1.0 + 0.005 * (i as f64)); // 0.5% per step, cumulative ~30%
            strategy.push_price(price, base_ts + (i as i64) * 1000);
        }
    }

    /// Helper: build a price series that should produce a SHORT signal.
    fn feed_falling_prices(strategy: &mut dyn Strategy, start: f64, count: usize) {
        let base_ts = 1000000_i64;
        for i in 0..count {
            let price = start * (1.0 - 0.005 * (i as f64)); // -0.5% per step
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

    // ---- Entry Signal Tests ----

    #[test]
    fn test_long_signal() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        feed_rising_prices(&mut strategy, 100.0, 10);

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);

        match signal {
            Signal::MomentumLong {
                strength,
                velocity_pct,
            } => {
                assert!(velocity_pct >= 0.15, "velocity should exceed threshold");
                assert!(strength >= 50.0, "strength should be >= 50");
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
            Signal::MomentumShort {
                strength,
                velocity_pct,
            } => {
                assert!(velocity_pct >= 0.15, "velocity should exceed threshold");
                assert!(strength >= 50.0, "strength should be >= 50");
            }
            other => panic!("Expected MomentumShort, got {:?}", other),
        }
    }

    #[test]
    fn test_no_signal_insufficient_prices() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        // Feed fewer than 5 prices
        strategy.push_price(100.0, 1000);
        strategy.push_price(101.0, 2000);
        strategy.push_price(102.0, 3000);

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);
        assert_eq!(signal, Signal::NoSignal, "Should return NoSignal with < 5 prices");
    }

    #[test]
    fn test_no_signal_flat_prices() {
        let params = default_params();
        let mut strategy = MomentumScalperStrategy::new(params);
        // Feed constant prices — no velocity
        for i in 0..10 {
            strategy.push_price(100.0, 1000 + (i as i64) * 1000);
        }

        let snapshot = strategy.snapshot();
        let signal = strategy.detect_entry(&snapshot);
        assert_eq!(signal, Signal::NoSignal, "Should return NoSignal with flat prices");
    }

    // ---- Exit Signal Tests ----

    #[test]
    fn test_stop_loss_fires_before_take_profit() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);

        // Create a context where both SL and TP could be relevant:
        // Entry at 100, current at 98 (2% loss, SL is 1%)
        let ctx = default_exit_context(true, 100.0, 98.0, 100.0, 10);

        // Feed flat prices so momentum is neutral
        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(98.0, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        match result {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(
                    reason,
                    ExitReason::StopLoss,
                    "SL should fire first: pnl=-2% > -1% threshold"
                );
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_take_profit_exit() {
        let params = default_params();
        let strategy = MomentumScalperStrategy::new(params);

        // Entry at 100, current at 103 (3% gain, TP is 2.5%)
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

        // Held for 2000s, max is 1800s, but no SL/TP/trailing triggered
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

        // Small profit, not enough for TP, no SL, held short time
        let ctx = default_exit_context(true, 100.0, 100.3, 100.3, 30);

        let mut detector_feed = MomentumDetector::new(0.15, 60);
        for i in 0..10 {
            detector_feed.push_price(100.3, 1000 + (i as i64) * 1000);
        }
        let snapshot = detector_feed.analyze();

        let result = strategy.detect_exit(&snapshot, &ctx);
        assert!(
            result.is_none() || matches!(result, Some(Signal::ExitLong { reason: ExitReason::MomentumLost })),
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
        assert!(
            err.contains("nonexistent"),
            "Error should mention the unknown name: {}",
            err
        );
        assert!(
            err.contains("momentum-scalper"),
            "Error should list available strategies: {}",
            err
        );
    }

    // ---- Params Validation Tests ----

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
}
