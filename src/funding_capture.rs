//! Funding rate capture strategy for passive yield generation.
//!
//! Monitors Hyperliquid perpetual funding rates and enters delta-neutral
//! short perp positions when annualized funding exceeds a threshold.
//! Exits when funding drops below the exit threshold or position exceeds
//! max hold duration.
//!
//! This is a yield strategy, not a momentum strategy — the signal comes
//! from the funding rate itself rather than price velocity.

use crate::signal::{ExitReason, MomentumSnapshot, Signal};
use crate::strategy::{PositionContext, Strategy, StrategyParams};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Parameters for the funding rate capture strategy.
///
/// Loaded from `[strategy.funding-capture]` in `config/perps.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingCaptureParams {
    /// Minimum annualized funding rate (%) to trigger a short entry.
    /// Default: 20.0% (enter when annualized funding > 20%).
    #[serde(default = "default_min_annualized_rate_pct")]
    pub min_annualized_rate_pct: f64,

    /// Exit when annualized funding rate drops below this (%).
    /// Default: 5.0%.
    #[serde(default = "default_exit_annualized_rate_pct")]
    pub exit_annualized_rate_pct: f64,

    /// Maximum position hold time in hours before forced exit.
    /// Default: 72 hours.
    #[serde(default = "default_max_position_hours")]
    pub max_position_hours: u64,

    /// Leverage for funding capture positions (typically 1.0 for delta-neutral).
    #[serde(default = "default_leverage")]
    pub leverage: f64,

    /// Position clip size in USD.
    #[serde(default = "default_clip_size_usd")]
    pub clip_size_usd: f64,

    /// Number of consecutive ticks funding must exceed threshold before entry.
    #[serde(default = "default_confirmation_ticks")]
    pub confirmation_ticks: usize,

    /// Stop-loss percentage for adverse price movement.
    #[serde(default = "default_stop_loss_pct")]
    pub stop_loss_pct: f64,

    /// Cooldown after a losing trade, in seconds.
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,

    /// Whether to use native on-chain TP/SL trigger orders.
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,

    /// Funding interval in hours (Hyperliquid uses 8h funding).
    #[serde(default = "default_funding_interval_hours")]
    pub funding_interval_hours: u64,
}

fn default_min_annualized_rate_pct() -> f64 {
    20.0
}
fn default_exit_annualized_rate_pct() -> f64 {
    5.0
}
fn default_max_position_hours() -> u64 {
    72
}
fn default_leverage() -> f64 {
    1.0
}
fn default_clip_size_usd() -> f64 {
    200.0
}
fn default_confirmation_ticks() -> usize {
    2
}
fn default_stop_loss_pct() -> f64 {
    3.0
}
fn default_cooldown_after_loss_secs() -> u64 {
    300
}
fn default_use_native_tp_sl() -> bool {
    true
}
fn default_funding_interval_hours() -> u64 {
    8
}

impl Default for FundingCaptureParams {
    fn default() -> Self {
        Self {
            min_annualized_rate_pct: default_min_annualized_rate_pct(),
            exit_annualized_rate_pct: default_exit_annualized_rate_pct(),
            max_position_hours: default_max_position_hours(),
            leverage: default_leverage(),
            clip_size_usd: default_clip_size_usd(),
            confirmation_ticks: default_confirmation_ticks(),
            stop_loss_pct: default_stop_loss_pct(),
            cooldown_after_loss_secs: default_cooldown_after_loss_secs(),
            use_native_tp_sl: default_use_native_tp_sl(),
            funding_interval_hours: default_funding_interval_hours(),
        }
    }
}

impl FundingCaptureParams {
    /// Validate that all parameter values are within acceptable ranges.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_annualized_rate_pct <= 0.0 {
            return Err(format!(
                "min_annualized_rate_pct must be > 0, got {}",
                self.min_annualized_rate_pct
            ));
        }
        if self.exit_annualized_rate_pct < 0.0 {
            return Err(format!(
                "exit_annualized_rate_pct must be >= 0, got {}",
                self.exit_annualized_rate_pct
            ));
        }
        if self.exit_annualized_rate_pct >= self.min_annualized_rate_pct {
            return Err(format!(
                "exit_annualized_rate_pct ({}) must be < min_annualized_rate_pct ({})",
                self.exit_annualized_rate_pct, self.min_annualized_rate_pct
            ));
        }
        if self.clip_size_usd <= 0.0 {
            return Err(format!(
                "clip_size_usd must be > 0, got {}",
                self.clip_size_usd
            ));
        }
        if self.max_position_hours == 0 {
            return Err("max_position_hours must be > 0".to_string());
        }
        if self.stop_loss_pct <= 0.0 {
            return Err(format!(
                "stop_loss_pct must be > 0, got {}",
                self.stop_loss_pct
            ));
        }
        if self.confirmation_ticks == 0 {
            return Err("confirmation_ticks must be > 0".to_string());
        }
        Ok(())
    }

    /// Convert to the generic StrategyParams for use by engine/risk modules.
    pub fn to_strategy_params(&self) -> StrategyParams {
        StrategyParams {
            direction_bias: "short".to_string(),
            momentum_threshold_pct: self.min_annualized_rate_pct,
            lookback_count: self.confirmation_ticks,
            scale_in_clips: 1,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_position_hours * 3600,
            take_profit_pct: 100.0, // No TP — exit on funding drop or time stop
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

// ---------------------------------------------------------------------------
// Funding Rate Snapshot
// ---------------------------------------------------------------------------

/// A snapshot of funding rate data for a single market, used as strategy input.
///
/// Unlike momentum strategies that consume `MomentumSnapshot`, the funding capture
/// strategy reads funding rate data injected into the `pool_data` field (repurposed
/// as a generic data channel) or tracked internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingSnapshot {
    /// Market coin (e.g., "BTC").
    pub coin: String,
    /// Current annualized funding rate as a percentage (e.g., 36.5 means 36.5%).
    pub annualized_rate_pct: f64,
    /// Current raw funding rate (e.g., 0.0001 = 0.01% per 8h).
    pub raw_funding_rate: f64,
    /// Mark price.
    pub mark_px: f64,
    /// Open interest in USD.
    pub open_interest_usd: f64,
    /// Timestamp of this snapshot (ms since epoch).
    pub timestamp_ms: i64,
    /// Previous day's mark price for 24h volatility calculation (0.0 if unavailable).
    #[serde(default)]
    pub prev_day_px: f64,
}

// ---------------------------------------------------------------------------
// Strategy
// ---------------------------------------------------------------------------

/// Funding rate capture strategy.
///
/// Enters short perp positions when annualized funding exceeds the entry threshold
/// and exits when funding drops below the exit threshold or time stop is reached.
/// This captures the funding payment from longs (shorts receive funding when rate > 0).
///
/// The strategy tracks funding history internally via `push_funding()`. Entry
/// detection requires `confirmation_ticks` consecutive snapshots above the
/// entry threshold. Exit is triggered by:
/// 1. Stop-loss (adverse price movement)
/// 2. Time stop (position held > max_position_hours)
/// 3. Funding rate dropped below exit threshold
#[allow(dead_code)]
pub struct FundingRateCaptureStrategy {
    params: FundingCaptureParams,
    generic_params: StrategyParams,

    /// Rolling window of funding snapshots (one per poll tick).
    #[allow(dead_code)]
    funding_history: VecDeque<FundingSnapshot>,

    /// Number of consecutive ticks where funding exceeded the entry threshold.
    consecutive_above_threshold: usize,

    /// Last known funding snapshot for the currently-tracked market.
    last_snapshot: Option<FundingSnapshot>,

    /// Price buffer for internal snapshot generation.
    prices: VecDeque<(f64, i64)>,
}

impl FundingRateCaptureStrategy {
    pub fn new(params: FundingCaptureParams) -> Self {
        let generic = params.to_strategy_params();
        Self {
            params,
            generic_params: generic,
            funding_history: VecDeque::with_capacity(100),
            consecutive_above_threshold: 0,
            last_snapshot: None,
            prices: VecDeque::with_capacity(200),
        }
    }

    /// Push a funding rate snapshot into the strategy's internal buffer.
    ///
    /// Call this on each tick with the latest funding data for the target market.
    /// The strategy tracks how many consecutive ticks funding exceeds the threshold
    /// and uses this for entry confirmation.
    #[allow(dead_code)]
    pub fn push_funding(&mut self, snapshot: FundingSnapshot) {
        debug!(
            "[funding-capture] push_funding: coin={} rate={:.2}% mark={:.2} oi={:.0}",
            snapshot.coin,
            snapshot.annualized_rate_pct,
            snapshot.mark_px,
            snapshot.open_interest_usd,
        );

        // Track consecutive ticks above entry threshold
        if snapshot.annualized_rate_pct >= self.params.min_annualized_rate_pct {
            self.consecutive_above_threshold += 1;
        } else {
            if self.consecutive_above_threshold > 0 {
                debug!(
                    "[funding-capture] Consecutive above threshold reset: was {}, now below {:.2}% < {:.2}%",
                    self.consecutive_above_threshold,
                    snapshot.annualized_rate_pct,
                    self.params.min_annualized_rate_pct,
                );
            }
            self.consecutive_above_threshold = 0;
        }

        self.last_snapshot = Some(snapshot.clone());
        self.funding_history.push_back(snapshot);
        if self.funding_history.len() > 100 {
            self.funding_history.pop_front();
        }
    }

    /// Check whether the current funding rate has dropped below the exit threshold.
    /// Returns true when funding is below exit level.
    pub fn is_funding_below_exit(&self) -> bool {
        match &self.last_snapshot {
            Some(s) => s.annualized_rate_pct < self.params.exit_annualized_rate_pct,
            None => false,
        }
    }

    /// Get the current annualized funding rate (or 0.0 if no data).
    pub fn current_rate(&self) -> f64 {
        self.last_snapshot
            .as_ref()
            .map(|s| s.annualized_rate_pct)
            .unwrap_or(0.0)
    }
}

impl Strategy for FundingRateCaptureStrategy {
    fn name(&self) -> &str {
        "funding-capture"
    }

    fn detect_entry(&mut self, _snapshot: &MomentumSnapshot) -> Signal {
        // The funding capture strategy uses push_funding() for its primary signal.
        // detect_entry checks the internal consecutive counter.

        if self.consecutive_above_threshold < self.params.confirmation_ticks {
            if self.consecutive_above_threshold > 0 {
                debug!(
                    "[funding-capture] Accumulating confirmation: {}/{} (rate={:.2}%)",
                    self.consecutive_above_threshold,
                    self.params.confirmation_ticks,
                    self.current_rate(),
                );
            }
            return Signal::NoSignal;
        }

        // Entry signal — always SHORT (we earn funding from longs)
        let strength = if self.current_rate() > 50.0 {
            90.0
        } else if self.current_rate() > 30.0 {
            75.0
        } else {
            60.0
        };

        info!(
            "[funding-capture] ENTRY SHORT signal: annualized_rate={:.2}% (threshold={:.2}%), \
             consecutive={}, strength={:.0}",
            self.current_rate(),
            self.params.min_annualized_rate_pct,
            self.consecutive_above_threshold,
            strength,
        );

        // Reset counter after firing signal
        self.consecutive_above_threshold = 0;

        Signal::MomentumShort {
            strength,
            velocity_pct: self.current_rate(),
        }
    }

    fn detect_exit(
        &self,
        _snapshot: &MomentumSnapshot,
        ctx: &PositionContext,
    ) -> Option<Signal> {
        // PnL calculation (short position: profit when price drops)
        let pnl_pct = if ctx.is_long {
            (ctx.current_price - ctx.entry_price) / ctx.entry_price * 100.0
        } else {
            (ctx.entry_price - ctx.current_price) / ctx.entry_price * 100.0
        };

        let max_hold_secs = self.params.max_position_hours * 3600;

        // 1. Stop-loss (highest priority — adverse price movement)
        if pnl_pct <= -ctx.stop_loss_pct {
            warn!(
                "[funding-capture] STOP LOSS: pnl={:.2}%, threshold=-{:.2}%",
                pnl_pct, ctx.stop_loss_pct
            );
            return Some(exit_signal(ctx.is_long, ExitReason::StopLoss));
        }

        // 2. Time stop (position held too long)
        if ctx.hold_secs >= max_hold_secs {
            info!(
                "[funding-capture] TIME STOP: hold={}s >= max={}s ({:.1}h), pnl={:.2}%",
                ctx.hold_secs,
                max_hold_secs,
                self.params.max_position_hours,
                pnl_pct,
            );
            return Some(exit_signal(ctx.is_long, ExitReason::TimeStop));
        }

        // 3. Funding rate dropped below exit threshold
        if self.is_funding_below_exit() {
            info!(
                "[funding-capture] FUNDING EXIT: rate={:.2}% < exit_threshold={:.2}%, \
                 hold={:.1}h, pnl={:.2}%",
                self.current_rate(),
                self.params.exit_annualized_rate_pct,
                ctx.hold_secs as f64 / 3600.0,
                pnl_pct,
            );
            return Some(exit_signal(ctx.is_long, ExitReason::ReversalDetected));
        }

        None
    }

    fn parameters(&self) -> &StrategyParams {
        &self.generic_params
    }

    fn push_price(&mut self, price: f64, timestamp_ms: i64) {
        self.prices.push_back((price, timestamp_ms));
        if self.prices.len() > 200 {
            self.prices.pop_front();
        }
    }

    fn snapshot(&self) -> MomentumSnapshot {
        use crate::signal::TradeDirection;

        let (current_price, price_count) = if self.prices.is_empty() {
            (0.0, 0)
        } else {
            (
                self.prices.back().map(|(p, _)| *p).unwrap_or(0.0),
                self.prices.len(),
            )
        };

        // Compute velocity from price buffer
        let velocity_pct = if self.prices.len() >= 2 {
            let newest = self.prices.back().unwrap().0;
            let oldest = self.prices.front().unwrap().0;
            if oldest > 0.0 {
                (newest - oldest) / oldest * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };

        MomentumSnapshot {
            price_count,
            current_price,
            price_velocity_pct: velocity_pct,
            direction: TradeDirection::Neutral,
            strength: self.current_rate() / self.params.min_annualized_rate_pct * 50.0,
            volatility_pct: 0.0,
            pool_data: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Helper to create an exit signal matching the position direction.
fn exit_signal(is_long: bool, reason: ExitReason) -> Signal {
    if is_long {
        Signal::ExitLong { reason }
    } else {
        Signal::ExitShort { reason }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> FundingCaptureParams {
        FundingCaptureParams {
            min_annualized_rate_pct: 20.0,
            exit_annualized_rate_pct: 5.0,
            max_position_hours: 72,
            leverage: 1.0,
            clip_size_usd: 200.0,
            confirmation_ticks: 2,
            stop_loss_pct: 3.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
            funding_interval_hours: 8,
        }
    }

    fn make_snapshot(rate_pct: f64, mark_px: f64) -> FundingSnapshot {
        FundingSnapshot {
            coin: "BTC".to_string(),
            annualized_rate_pct: rate_pct,
            raw_funding_rate: rate_pct / 100.0 / (365.0 * 3.0), // approximate
            mark_px,
            open_interest_usd: 1_000_000.0,
            timestamp_ms: 1700000000000,
            prev_day_px: 0.0,
        }
    }

    fn make_momentum_snapshot(price: f64) -> MomentumSnapshot {
        use crate::signal::TradeDirection;
        MomentumSnapshot {
            price_count: 10,
            current_price: price,
            price_velocity_pct: 0.0,
            direction: TradeDirection::Neutral,
            strength: 0.0,
            volatility_pct: 0.0,
            pool_data: None,
        }
    }

    fn make_position_context(is_long: bool, entry: f64, current: f64, hold_secs: u64) -> PositionContext {
        PositionContext {
            is_long,
            entry_price: entry,
            current_price: current,
            peak_price: if is_long { current.max(entry) } else { current.min(entry) },
            hold_secs,
            max_hold_secs: 72 * 3600,
            take_profit_pct: 100.0,
            stop_loss_pct: 3.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        }
    }

    // --- Parameter validation ---

    #[test]
    fn test_params_valid() {
        assert!(default_params().validate().is_ok());
    }

    #[test]
    fn test_params_zero_min_rate_rejected() {
        let mut p = default_params();
        p.min_annualized_rate_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_negative_min_rate_rejected() {
        let mut p = default_params();
        p.min_annualized_rate_pct = -5.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_exit_above_entry_rejected() {
        let mut p = default_params();
        p.exit_annualized_rate_pct = 25.0; // > min of 20.0
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_zero_clip_size_rejected() {
        let mut p = default_params();
        p.clip_size_usd = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_zero_max_hours_rejected() {
        let mut p = default_params();
        p.max_position_hours = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_zero_stop_loss_rejected() {
        let mut p = default_params();
        p.stop_loss_pct = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_zero_confirmation_rejected() {
        let mut p = default_params();
        p.confirmation_ticks = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_params_to_strategy_params() {
        let p = default_params();
        let sp = p.to_strategy_params();
        assert_eq!(sp.direction_bias, "short");
        assert_eq!(sp.clip_size_usd, 200.0);
        assert_eq!(sp.max_hold_secs, 72 * 3600);
        assert_eq!(sp.stop_loss_pct, 3.0);
    }

    // --- push_funding and entry detection ---

    #[test]
    fn test_no_signal_without_funding_data() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        let snap = make_momentum_snapshot(100000.0);
        assert_eq!(s.detect_entry(&snap), Signal::NoSignal);
    }

    #[test]
    fn test_no_signal_below_threshold() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        // Push funding below threshold (15% < 20%)
        s.push_funding(make_snapshot(15.0, 100000.0));
        s.push_funding(make_snapshot(15.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        assert_eq!(s.detect_entry(&snap), Signal::NoSignal);
    }

    #[test]
    fn test_no_signal_insufficient_confirmation() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        // Only 1 tick above threshold, need 2
        s.push_funding(make_snapshot(25.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        assert_eq!(s.detect_entry(&snap), Signal::NoSignal);
    }

    #[test]
    fn test_entry_signal_after_confirmation() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        // 2 ticks above threshold
        s.push_funding(make_snapshot(25.0, 100000.0));
        s.push_funding(make_snapshot(25.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        let result = s.detect_entry(&snap);

        match result {
            Signal::MomentumShort { strength, .. } => {
                assert!(strength >= 60.0);
            }
            other => panic!("Expected MomentumShort, got {:?}", other),
        }
    }

    #[test]
    fn test_entry_signal_resets_consecutive_counter() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(25.0, 100000.0));
        s.push_funding(make_snapshot(25.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        let _ = s.detect_entry(&snap); // fires and resets

        // Should not fire again without new confirmations
        assert_eq!(s.detect_entry(&snap), Signal::NoSignal);
    }

    #[test]
    fn test_entry_strength_high_rate() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(55.0, 100000.0)); // > 50%
        s.push_funding(make_snapshot(55.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        match s.detect_entry(&snap) {
            Signal::MomentumShort { strength, .. } => {
                assert_eq!(strength, 90.0);
            }
            other => panic!("Expected MomentumShort, got {:?}", other),
        }
    }

    #[test]
    fn test_consecutive_resets_on_below_threshold() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(25.0, 100000.0)); // tick 1 above
        s.push_funding(make_snapshot(10.0, 100000.0)); // below → reset
        s.push_funding(make_snapshot(25.0, 100000.0)); // tick 1 above (restarted)

        let snap = make_momentum_snapshot(100000.0);
        assert_eq!(s.detect_entry(&snap), Signal::NoSignal); // need 2, only have 1
    }

    // --- Exit detection ---

    #[test]
    fn test_exit_stop_loss_long() {
        let s = FundingRateCaptureStrategy::new(default_params());
        let ctx = make_position_context(true, 100.0, 95.0, 1000); // -5% loss
        let snap = make_momentum_snapshot(95.0);

        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitLong { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitLong(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_exit_stop_loss_short() {
        let s = FundingRateCaptureStrategy::new(default_params());
        let ctx = make_position_context(false, 100.0, 105.0, 1000); // -5% loss for short
        let snap = make_momentum_snapshot(105.0);

        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss);
            }
            other => panic!("Expected ExitShort(StopLoss), got {:?}", other),
        }
    }

    #[test]
    fn test_exit_time_stop() {
        let s = FundingRateCaptureStrategy::new(default_params());
        let ctx = make_position_context(false, 100.0, 99.0, 72 * 3600); // exactly at max hold
        let snap = make_momentum_snapshot(99.0);

        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort { reason }) => {
                assert_eq!(reason, ExitReason::TimeStop);
            }
            other => panic!("Expected ExitShort(TimeStop), got {:?}", other),
        }
    }

    #[test]
    fn test_exit_time_stop_before_max() {
        let s = FundingRateCaptureStrategy::new(default_params());
        // Hold just under max (71h 59m 59s)
        let ctx = make_position_context(false, 100.0, 99.0, 72 * 3600 - 1);
        let snap = make_momentum_snapshot(99.0);
        assert_eq!(s.detect_exit(&snap, &ctx), None);
    }

    #[test]
    fn test_exit_funding_below_threshold() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        // Funding was high when entered, now dropped below exit threshold
        s.push_funding(make_snapshot(2.0, 100000.0)); // 2% < 5% exit threshold

        let ctx = make_position_context(false, 100.0, 99.0, 3600); // profitable short
        let snap = make_momentum_snapshot(99.0);

        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort { reason }) => {
                assert_eq!(reason, ExitReason::ReversalDetected);
            }
            other => panic!("Expected ExitShort(ReversalDetected), got {:?}", other),
        }
    }

    #[test]
    fn test_no_exit_when_funding_still_high() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        // Funding still above exit threshold (10% > 5%)
        s.push_funding(make_snapshot(10.0, 100000.0));

        let ctx = make_position_context(false, 100.0, 99.0, 3600);
        let snap = make_momentum_snapshot(99.0);
        assert_eq!(s.detect_exit(&snap, &ctx), None);
    }

    #[test]
    fn test_no_exit_no_stop_no_time() {
        let s = FundingRateCaptureStrategy::new(default_params());
        // Small profit, short hold, no funding data
        let ctx = make_position_context(false, 100.0, 99.5, 1000);
        let snap = make_momentum_snapshot(99.5);
        assert_eq!(s.detect_exit(&snap, &ctx), None);
    }

    // --- Exit priority: SL > Time > Funding ---

    #[test]
    fn test_exit_priority_stop_loss_over_time_stop() {
        let s = FundingRateCaptureStrategy::new(default_params());
        // Both stop-loss AND time-stop triggered
        let ctx = make_position_context(false, 100.0, 105.0, 72 * 3600); // -5% + max hold
        let snap = make_momentum_snapshot(105.0);

        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort { reason }) => {
                assert_eq!(reason, ExitReason::StopLoss); // SL takes priority
            }
            other => panic!("Expected ExitShort(StopLoss), got {:?}", other),
        }
    }

    // --- push_price and snapshot ---

    #[test]
    fn test_push_price_updates_snapshot() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_price(100.0, 1000);
        s.push_price(101.0, 2000);
        s.push_price(102.0, 3000);

        let snap = s.snapshot();
        assert_eq!(snap.price_count, 3);
        assert_eq!(snap.current_price, 102.0);
    }

    #[test]
    fn test_push_price_truncates_at_capacity() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        for i in 0..250 {
            s.push_price(i as f64, i * 1000);
        }
        let snap = s.snapshot();
        assert_eq!(snap.price_count, 200);
    }

    #[test]
    fn test_snapshot_empty() {
        let s = FundingRateCaptureStrategy::new(default_params());
        let snap = s.snapshot();
        assert_eq!(snap.price_count, 0);
        assert_eq!(snap.current_price, 0.0);
    }

    // --- Utility methods ---

    #[test]
    fn test_current_rate_no_data() {
        let s = FundingRateCaptureStrategy::new(default_params());
        assert_eq!(s.current_rate(), 0.0);
    }

    #[test]
    fn test_current_rate_with_data() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(35.0, 100000.0));
        assert_eq!(s.current_rate(), 35.0);
    }

    #[test]
    fn test_is_funding_below_exit_no_data() {
        let s = FundingRateCaptureStrategy::new(default_params());
        assert!(!s.is_funding_below_exit()); // no data = not below exit
    }

    #[test]
    fn test_is_funding_below_exit_above() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(10.0, 100000.0)); // 10% > 5% exit
        assert!(!s.is_funding_below_exit());
    }

    #[test]
    fn test_is_funding_below_exit_below() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        s.push_funding(make_snapshot(3.0, 100000.0)); // 3% < 5% exit
        assert!(s.is_funding_below_exit());
    }

    // --- Funding history truncation ---

    #[test]
    fn test_funding_history_truncates() {
        let mut s = FundingRateCaptureStrategy::new(default_params());
        for i in 0..105 {
            s.push_funding(FundingSnapshot {
                coin: "BTC".to_string(),
                annualized_rate_pct: i as f64,
                raw_funding_rate: 0.0,
                mark_px: 100000.0,
                open_interest_usd: 1_000_000.0,
                timestamp_ms: i as i64 * 1000,
                prev_day_px: 0.0,
            });
        }
        assert!(s.funding_history.len() <= 100);
    }

    // --- Strategy trait name ---

    #[test]
    fn test_strategy_name() {
        let s = FundingRateCaptureStrategy::new(default_params());
        assert_eq!(s.name(), "funding-capture");
    }

    // --- Full pipeline: entry -> hold -> exit via funding drop ---

    #[test]
    fn test_full_pipeline_funding_exit() {
        let mut s = FundingRateCaptureStrategy::new(default_params());

        // Funding rises above threshold
        s.push_funding(make_snapshot(25.0, 100000.0));
        s.push_funding(make_snapshot(30.0, 100000.0));

        // Entry signal fires
        let snap = make_momentum_snapshot(100000.0);
        match s.detect_entry(&snap) {
            Signal::MomentumShort { .. } => {}
            other => panic!("Expected MomentumShort, got {:?}", other),
        }

        // Funding drops below exit threshold
        s.push_funding(make_snapshot(3.0, 99500.0));

        // Exit fires
        let ctx = make_position_context(false, 100000.0, 99500.0, 3600);
        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort {
                reason: ExitReason::ReversalDetected,
            }) => {}
            other => panic!("Expected ExitShort(ReversalDetected), got {:?}", other),
        }
    }

    // --- Full pipeline: entry -> hold -> exit via time stop ---

    #[test]
    fn test_full_pipeline_time_exit() {
        let mut s = FundingRateCaptureStrategy::new(default_params());

        s.push_funding(make_snapshot(25.0, 100000.0));
        s.push_funding(make_snapshot(25.0, 100000.0));

        let snap = make_momentum_snapshot(100000.0);
        let _ = s.detect_entry(&snap); // fire entry

        // Still above exit threshold but held too long
        s.push_funding(make_snapshot(10.0, 99500.0)); // 10% > 5% exit
        let ctx = make_position_context(false, 100000.0, 99500.0, 72 * 3600);
        match s.detect_exit(&snap, &ctx) {
            Some(Signal::ExitShort {
                reason: ExitReason::TimeStop,
            }) => {}
            other => panic!("Expected ExitShort(TimeStop), got {:?}", other),
        }
    }

    // --- Serde roundtrip for params ---

    #[test]
    fn test_params_serde_roundtrip() {
        let params = default_params();
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: FundingCaptureParams = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.min_annualized_rate_pct, params.min_annualized_rate_pct);
        assert_eq!(deserialized.exit_annualized_rate_pct, params.exit_annualized_rate_pct);
        assert_eq!(deserialized.max_position_hours, params.max_position_hours);
        assert_eq!(deserialized.leverage, params.leverage);
        assert_eq!(deserialized.clip_size_usd, params.clip_size_usd);
    }

    // --- FundingSnapshot serde ---

    #[test]
    fn test_funding_snapshot_serde() {
        let snap = FundingSnapshot {
            coin: "ETH".to_string(),
            annualized_rate_pct: 42.5,
            raw_funding_rate: 0.0005,
            mark_px: 3500.0,
            open_interest_usd: 500_000.0,
            timestamp_ms: 1700000000000,
            prev_day_px: 3450.0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: FundingSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.coin, "ETH");
        assert_eq!(back.annualized_rate_pct, 42.5);
    }

    // --- Params from TOML-like structure ---

    #[test]
    fn test_params_from_toml_value() {
        let toml_str = r#"
min_annualized_rate_pct = 30.0
exit_annualized_rate_pct = 10.0
max_position_hours = 48
leverage = 2.0
clip_size_usd = 500.0
confirmation_ticks = 3
stop_loss_pct = 5.0
cooldown_after_loss_secs = 600
use_native_tp_sl = false
funding_interval_hours = 8
"#;
        let value: toml::Value = toml_str.parse().unwrap();
        let params: FundingCaptureParams = value.try_into().unwrap();
        assert_eq!(params.min_annualized_rate_pct, 30.0);
        assert_eq!(params.exit_annualized_rate_pct, 10.0);
        assert_eq!(params.max_position_hours, 48);
        assert_eq!(params.leverage, 2.0);
        assert_eq!(params.clip_size_usd, 500.0);
        assert_eq!(params.confirmation_ticks, 3);
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_params_defaults_from_empty_toml() {
        let value: toml::Value = toml::from_str("").unwrap();
        let params: FundingCaptureParams = value.try_into().unwrap();
        assert_eq!(params.min_annualized_rate_pct, 20.0);
        assert_eq!(params.exit_annualized_rate_pct, 5.0);
        assert_eq!(params.max_position_hours, 72);
        assert_eq!(params.leverage, 1.0);
        assert_eq!(params.clip_size_usd, 200.0);
        assert_eq!(params.confirmation_ticks, 2);
    }
}
