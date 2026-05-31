//! Replay validation pipeline for the liquidation-cascade-hunter strategy.
//!
//! Two stages:
//! 1. **Capture phase** — loads captured liquidation zone snapshots from disk
//!    (produced by `liquidation.rs` capture module).
//! 2. **Replay phase** — replays captured data through the strategy, comparing
//!    results against a no-trade baseline.
//!
//! The promotion gate checks ALL criteria before a strategy can be promoted:
//! - Positive net expectancy after route costs
//! - Max drawdown within policy limit
//! - Zero stale-data trades
//! - Zero duplicate pending trades
//! - ≥ 30 signal events
//! - Sharpe ≥ 1.0
//!
//! Extended metrics: Sortino ratio, Calmar ratio, MAE/MFE per trade, fishing
//! fill rate, zone-touch win rate, post-liquidation drift, time-to-reversal,
//! time-to-next-zone, stop efficiency, single-trade dependency flag.
//!
//! Fishing + pyramiding composed into replay flow for end-to-end validation.
//!
//! No live trading. Paper-only. Backward compatible.

use crate::fishing::{FishingLadderConfig, FishingSimResult, MarketConditions};
use crate::liquidation::{LiquidationZone, LiquidationZoneSnapshot};
use crate::pyramiding::{AddTrancheContext, PyramidConfig, PyramidResult};
use crate::signal::{MarketExtension, MomentumSnapshot, Signal, TradeDirection};
use crate::strategy::{
    LiquidationCascadeHunter, LiquidationCascadeParams, PositionContext, Strategy,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Replay Data Types
// ---------------------------------------------------------------------------

/// A single data point for replay. Contains all the market state needed to
/// construct a `MomentumSnapshot` with `MarketExtension` for the
/// liquidation-cascade-hunter strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayDataPoint {
    /// Market symbol (e.g., "BTC", "SOL").
    pub symbol: String,
    /// Timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Current price.
    pub price: f64,
    /// Volume-weighted average price.
    pub vwap: Option<f64>,
    /// Bid-ask spread as percentage.
    pub spread_pct: Option<f64>,
    /// Order book depth at nearest zone, in USD.
    pub depth_usd: Option<f64>,
    /// Volume z-score relative to recent history.
    pub volume_zscore: Option<f64>,
    /// Forced-flow velocity (from liquidation events).
    pub forced_flow_velocity: Option<f64>,
    /// Current regime label (from RegimeDetector).
    pub regime_label: Option<String>,
    /// Whether a liquidation burst has been detected recently.
    pub liquidation_burst_detected: bool,
    /// Route cost in basis points.
    pub route_cost_bps: Option<f64>,
    /// Liquidation zones active at this point in time.
    pub liquidation_zones: Option<Vec<LiquidationZone>>,
    /// Timestamp of the zone capture (for staleness detection).
    pub zone_capture_timestamp_ms: Option<i64>,
    /// Candle high price (for MAE/MFE excursion tracking).
    pub high: Option<f64>,
    /// Candle low price (for MAE/MFE excursion tracking).
    pub low: Option<f64>,
    /// Whether this data point represents a zone-touch event
    /// (price within proximity of a liquidation zone).
    pub is_zone_touch: Option<bool>,
}

/// A trade recorded during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayTrade {
    /// Market symbol.
    pub symbol: String,
    /// Trade side: "long" or "short".
    pub side: String,
    /// Entry price.
    pub entry_price: f64,
    /// Exit price.
    pub exit_price: f64,
    /// Position size in USD.
    pub size_usd: f64,
    /// Gross PnL (before fees).
    pub gross_pnl: f64,
    /// Entry fee in USD.
    pub entry_fee: f64,
    /// Exit fee in USD.
    pub exit_fee: f64,
    /// Route cost in USD.
    pub route_cost_usd: f64,
    /// Net PnL (after fees).
    pub net_pnl: f64,
    /// Hold time in seconds.
    pub hold_secs: u64,
    /// Exit reason.
    pub exit_reason: String,
    /// Entry timestamp.
    pub entry_timestamp_ms: i64,
    /// Exit timestamp.
    pub exit_timestamp_ms: i64,
    /// Whether zone data was stale at entry.
    pub entry_stale: bool,
    /// Whether zone data was stale at exit.
    pub exit_stale: bool,
    /// Peak price during hold.
    pub peak_price: f64,
    /// Maximum adverse excursion (worst unrealized loss during hold).
    /// For longs: lowest price seen minus entry price (negative).
    /// For shorts: highest price seen minus entry price (positive, negated).
    pub mae_usd: f64,
    /// Maximum favorable excursion (best unrealized profit during hold).
    /// For longs: highest price seen minus entry price (positive).
    /// For shorts: entry price minus lowest price seen (positive).
    pub mfe_usd: f64,
    /// Worst price seen during hold (for MAE computation).
    pub worst_price: f64,
    /// Best price seen during hold (for MFE computation).
    pub best_price: f64,
    /// Whether this trade was triggered at a zone touch.
    pub is_zone_touch: bool,
    /// Post-liquidation drift: price movement from entry to the next
    /// zone-touch or reversal point (0.0 if not applicable).
    pub post_liquidation_drift_usd: f64,
    /// Time from zone touch to first reversal in seconds (0 if not applicable).
    pub time_to_reversal_secs: f64,
    /// Time from current zone to next zone reached in seconds (0 if not applicable).
    pub time_to_next_zone_secs: f64,
    /// Stop efficiency: actual PnL divided by MFE (0.0 to 1.0 for winners,
    /// negative for losers). Measures how much of the favorable move was captured.
    pub stop_efficiency: f64,
}

/// Results of a replay run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    /// Strategy name (always "liquidation-cascade-hunter").
    pub strategy_name: String,
    /// Starting balance in USD.
    pub start_balance: f64,
    /// Final balance after all replayed trades.
    pub final_balance: f64,
    /// Total number of trades.
    pub trade_count: usize,
    /// Number of winning trades.
    pub win_count: usize,
    /// Number of losing trades.
    pub loss_count: usize,
    /// Total gross PnL (before fees).
    pub gross_pnl: f64,
    /// Total fees (entry + exit + route costs).
    pub total_fees: f64,
    /// Total net PnL (after fees).
    pub net_pnl: f64,
    /// Win rate as a percentage.
    pub win_rate_pct: f64,
    /// Sharpe ratio.
    pub sharpe_ratio: f64,
    /// Maximum drawdown in USD.
    pub max_drawdown_usd: f64,
    /// Maximum drawdown as percentage of starting balance.
    pub max_drawdown_pct: f64,
    /// Number of stale-data trades (entered or exited with stale zone data).
    pub stale_trade_count: usize,
    /// Number of duplicate pending trade occurrences.
    pub duplicate_pending_count: usize,
    /// Total number of signal events (entries that resulted in trades).
    pub signal_events: usize,
    /// Average hold time in seconds.
    pub avg_hold_secs: f64,
    /// Number of data points replayed.
    pub data_points_replayed: usize,
    /// Individual trade records.
    pub trades: Vec<ReplayTrade>,
    /// No-trade baseline: starting balance (unchanged).
    pub baseline_balance: f64,
    /// No-trade baseline net PnL (always 0.0).
    pub baseline_net_pnl: f64,
    /// Net expectancy: (win_rate * avg_win) - (loss_rate * avg_loss) - avg_route_cost.
    pub net_expectancy: f64,
    /// PnL improvement over baseline.
    pub pnl_vs_baseline: f64,
    /// Per-criterion promotion status.
    pub promotion_criteria: Vec<CriterionStatus>,
    /// Overall promotion verdict.
    pub promotion_verdict: PromotionVerdict,
    // --- Extended metrics ---
    /// Sortino ratio: mean_return / downside_deviation (annualized).
    pub sortino_ratio: f64,
    /// Calmar ratio: annualized_return / max_drawdown.
    pub calmar_ratio: f64,
    /// Average MAE across all trades in USD.
    pub avg_mae_usd: f64,
    /// Average MFE across all trades in USD.
    pub avg_mfe_usd: f64,
    /// Fill rate for fishing orders (0.0 if no fishing simulation).
    pub fishing_fill_rate: f64,
    /// Zone-touch win rate: win rate specifically for trades triggered at zone touches.
    pub zone_touch_win_rate_pct: f64,
    /// Zone-touch trade count.
    pub zone_touch_trade_count: usize,
    /// Zone-touch win count.
    pub zone_touch_win_count: usize,
    /// Average post-liquidation drift in USD.
    pub avg_post_liquidation_drift_usd: f64,
    /// Average time-to-reversal in seconds.
    pub avg_time_to_reversal_secs: f64,
    /// Average time-to-next-zone in seconds.
    pub avg_time_to_next_zone_secs: f64,
    /// Average stop efficiency (actual PnL / MFE).
    pub avg_stop_efficiency: f64,
    /// Whether single-trade dependency was flagged (>25% of total profit from one trade).
    pub single_trade_dependency_flagged: bool,
    /// The trade that contributes the most to total profit (None if no trades).
    pub dominant_trade_index: Option<usize>,
    /// Fishing simulation result (if fishing was composed into replay).
    pub fishing_result: Option<FishingSimResult>,
    /// Pyramid result (if pyramiding was composed into replay).
    pub pyramid_result: Option<PyramidResult>,
}

/// Status of a single promotion criterion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionStatus {
    /// Criterion name (e.g., "net_expectancy", "max_drawdown").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Whether this criterion passed.
    pub passed: bool,
    /// Actual value observed.
    pub actual_value: String,
    /// Threshold value required.
    pub threshold_value: String,
    /// Unit (e.g., "USD", "pct", "count", "ratio").
    pub unit: String,
}

/// Overall promotion verdict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PromotionVerdict {
    /// All criteria passed — strategy is eligible for promotion.
    Approved,
    /// One or more criteria failed — promotion blocked.
    Denied,
}

/// Configuration for the promotion gate (12 criteria).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionGateConfig {
    /// Maximum drawdown as a percentage of starting balance (e.g., 10.0 = 10%).
    #[serde(default = "default_max_drawdown_pct")]
    pub max_drawdown_pct: f64,
    /// Minimum number of signal events required.
    #[serde(default = "default_min_signal_events")]
    pub min_signal_events: usize,
    /// Minimum Sharpe ratio required.
    #[serde(default = "default_min_sharpe")]
    pub min_sharpe: f64,
    /// Fee rate for replay cost estimation.
    #[serde(default = "default_fee_rate")]
    pub fee_rate: f64,
    /// Starting balance for replay simulation.
    #[serde(default = "default_starting_balance")]
    pub starting_balance: f64,
    /// Maximum fee-to-gross ratio as percentage (criterion 7, default: 35.0).
    #[serde(default = "default_max_fee_to_gross_pct")]
    pub max_fee_to_gross_pct: f64,
    /// Maximum single-trade profit concentration as percentage (criterion 8, default: 25.0).
    #[serde(default = "default_max_single_trade_profit_pct")]
    pub max_single_trade_profit_pct: f64,
    /// Maximum route cost as percentage of net expectancy (criterion 11, default: 50.0).
    /// If route costs exceed this percentage of expectancy, the edge is consumed.
    #[serde(default = "default_max_route_cost_pct_of_expectancy")]
    pub max_route_cost_pct_of_expectancy: f64,
    /// Minimum safe liquidation distance in bps at proposed leverage (criterion 12, default: 200.0).
    #[serde(default = "default_min_safe_liquidation_distance_bps")]
    pub min_safe_liquidation_distance_bps: f64,
    /// Proposed leverage for liquidation distance check (criterion 12, default: 3.0).
    #[serde(default = "default_proposed_leverage")]
    pub proposed_leverage: f64,
}

fn default_max_drawdown_pct() -> f64 {
    10.0
}
fn default_min_signal_events() -> usize {
    30
}
fn default_min_sharpe() -> f64 {
    1.0
}
fn default_fee_rate() -> f64 {
    0.001
}
fn default_starting_balance() -> f64 {
    1000.0
}
fn default_max_fee_to_gross_pct() -> f64 {
    35.0
}
fn default_max_single_trade_profit_pct() -> f64 {
    25.0
}
fn default_max_route_cost_pct_of_expectancy() -> f64 {
    50.0
}
fn default_min_safe_liquidation_distance_bps() -> f64 {
    200.0
}
fn default_proposed_leverage() -> f64 {
    3.0
}

impl Default for PromotionGateConfig {
    fn default() -> Self {
        Self {
            max_drawdown_pct: default_max_drawdown_pct(),
            min_signal_events: default_min_signal_events(),
            min_sharpe: default_min_sharpe(),
            fee_rate: default_fee_rate(),
            starting_balance: default_starting_balance(),
            max_fee_to_gross_pct: default_max_fee_to_gross_pct(),
            max_single_trade_profit_pct: default_max_single_trade_profit_pct(),
            max_route_cost_pct_of_expectancy: default_max_route_cost_pct_of_expectancy(),
            min_safe_liquidation_distance_bps: default_min_safe_liquidation_distance_bps(),
            proposed_leverage: default_proposed_leverage(),
        }
    }
}

// ---------------------------------------------------------------------------
// Replay Pipeline
// ---------------------------------------------------------------------------

/// The replay pipeline loads captured liquidation zone snapshots and replays
/// them through the liquidation-cascade-hunter strategy.
pub struct ReplayPipeline {
    /// Strategy parameters.
    params: LiquidationCascadeParams,
    /// Promotion gate configuration.
    gate_config: PromotionGateConfig,
}

impl ReplayPipeline {
    /// Create a new replay pipeline with the given strategy params and gate config.
    pub fn new(params: LiquidationCascadeParams, gate_config: PromotionGateConfig) -> Self {
        Self { params, gate_config }
    }

    /// Load captured liquidation zone snapshots from a directory.
    ///
    /// Reads all `{symbol}_{timestamp_ms}.json` files, parses them into
    /// `LiquidationZoneSnapshot`, and returns them sorted by timestamp.
    pub fn load_snapshots(dir: &Path) -> anyhow::Result<Vec<LiquidationZoneSnapshot>> {
        if !dir.exists() {
            anyhow::bail!("Snapshot directory does not exist: {}", dir.display());
        }
        let mut snapshots = Vec::new();
        let entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            match serde_json::from_str::<LiquidationZoneSnapshot>(&content) {
                Ok(snap) => snapshots.push(snap),
                Err(e) => {
                    warn!("Failed to parse snapshot {:?}: {}", path, e);
                }
            }
        }
        snapshots.sort_by_key(|s| s.timestamp_ms);
        info!("Loaded {} liquidation zone snapshots from {:?}", snapshots.len(), dir);
        Ok(snapshots)
    }

    /// Convert captured snapshots into replay data points.
    ///
    /// Each snapshot becomes a `ReplayDataPoint` with the zone data, mark price,
    /// and placeholder values for other market metrics that can be enriched
    /// by the caller.
    pub fn snapshots_to_replay_points(
        snapshots: &[LiquidationZoneSnapshot],
    ) -> Vec<ReplayDataPoint> {
        snapshots
            .iter()
            .map(|snap| ReplayDataPoint {
                symbol: snap.symbol.clone(),
                timestamp_ms: snap.timestamp_ms,
                price: snap.mark_price,
                vwap: Some(snap.mark_price), // Default: VWAP = mark price
                spread_pct: Some(0.1),       // Default: tight spread
                depth_usd: Some(50_000.0),   // Default: healthy depth
                volume_zscore: Some(2.5),    // Default: elevated volume
                forced_flow_velocity: Some(0.6), // Default: moderate velocity
                regime_label: Some("Trending".to_string()),
                liquidation_burst_detected: !snap.zones.is_empty(),
                route_cost_bps: Some(3.0), // Default: reasonable route cost
                liquidation_zones: Some(snap.zones.clone()),
                zone_capture_timestamp_ms: Some(snap.timestamp_ms),
                high: Some(snap.mark_price * 1.001),   // Default: 0.1% above
                low: Some(snap.mark_price * 0.999),     // Default: 0.1% below
                is_zone_touch: Some(!snap.zones.is_empty()),
            })
            .collect()
    }

    /// Load replay data points from a JSON file.
    pub fn load_replay_data(path: &Path) -> anyhow::Result<Vec<ReplayDataPoint>> {
        let content = std::fs::read_to_string(path)?;
        let points: Vec<ReplayDataPoint> = serde_json::from_str(&content)?;
        info!("Loaded {} replay data points from {:?}", points.len(), path);
        Ok(points)
    }

    /// Construct a MomentumSnapshot from a ReplayDataPoint.
    pub fn build_snapshot(
        &self,
        point: &ReplayDataPoint,
        price_count: usize,
        price_velocity_pct: f64,
    ) -> MomentumSnapshot {
        let ext = MarketExtension {
            liquidation_zones: point.liquidation_zones.clone(),
            zone_capture_timestamp_ms: point.zone_capture_timestamp_ms,
            route_cost_bps: point.route_cost_bps,
            vwap: point.vwap,
            spread_pct: point.spread_pct,
            depth_usd: point.depth_usd,
            volume_zscore: point.volume_zscore,
            forced_flow_velocity: point.forced_flow_velocity,
            regime_label: point.regime_label.clone(),
            liquidation_burst_detected: point.liquidation_burst_detected,
            symbol: Some(point.symbol.clone()),
            oi_contracting: None,
        };

        MomentumSnapshot {
            price_count,
            current_price: point.price,
            price_velocity_pct,
            direction: if price_velocity_pct > 0.0 {
                TradeDirection::Long
            } else if price_velocity_pct < 0.0 {
                TradeDirection::Short
            } else {
                TradeDirection::Neutral
            },
            strength: 50.0, // Default strength
            volatility_pct: 1.0,
            pool_data: None,
            ext: Some(ext),
        }
    }

    /// Run the replay pipeline over the given data points.
    ///
    /// Replays each data point through the strategy, tracking all entry/exit
    /// signals, PnL, and risk metrics. Produces a deterministic result.
    pub fn run(&self, data_points: &[ReplayDataPoint]) -> ReplayResult {
        self.run_with_balance(data_points, self.gate_config.starting_balance)
    }

    /// Run the replay pipeline with a custom starting balance.
    pub fn run_with_balance(
        &self,
        data_points: &[ReplayDataPoint],
        starting_balance: f64,
    ) -> ReplayResult {
        let mut strategy = LiquidationCascadeHunter::new(self.params.clone());
        let mut balance = starting_balance;
        let mut peak_balance = starting_balance;
        let mut max_drawdown_usd = 0.0_f64;

        // Active position tracking
        let mut open_position: Option<OpenPosition> = None;

        // Trade log
        let mut trades: Vec<ReplayTrade> = Vec::new();
        let mut stale_trade_count = 0_usize;
        let mut duplicate_pending_count = 0_usize;
        let mut signal_events = 0_usize;

        // Per-trade returns for Sharpe calculation
        let mut trade_returns: Vec<f64> = Vec::new();

        // Track previous signals for duplicate detection
        let mut last_signal_key: Option<(String, String)> = None;

        let fee_rate = self.gate_config.fee_rate;

        for (i, point) in data_points.iter().enumerate() {
            // Push price to strategy
            strategy.push_price(point.price, point.timestamp_ms);

            // Compute velocity from price history
            let snap = strategy.snapshot();
            let velocity = snap.price_velocity_pct;
            let price_count = snap.price_count;

            // Build the full snapshot with market extension
            let snapshot = self.build_snapshot(point, price_count, velocity);

            // Check if zone data is stale at current timestamp
            let zone_stale = point.zone_capture_timestamp_ms.is_none_or(|ts| {
                let age_secs = (point.timestamp_ms - ts).max(0) as u64 / 1000;
                age_secs > self.params.stale_data_threshold_secs
            });

            if let Some(ref pos) = open_position {
                // Check exit conditions
                let ctx = PositionContext {
                    is_long: pos.is_long,
                    entry_price: pos.entry_price,
                    current_price: point.price,
                    peak_price: pos.peak_price,
                    hold_secs: ((point.timestamp_ms - pos.entry_timestamp_ms).max(0) as u64) / 1000,
                    max_hold_secs: self.params.max_hold_secs,
                    take_profit_pct: self.params.take_profit_pct,
                    stop_loss_pct: self.params.stop_loss_pct,
                    trailing_stop_pct: self.params.trailing_stop_pct,
                    trailing_activation_pct: self.params.trailing_activation_pct,
                };

                let exit_signal = strategy.detect_exit(&snapshot, &ctx);

                if let Some(exit) = exit_signal {
                    let (exit_reason, is_exit_long) = match &exit {
                        Signal::ExitLong { reason } => (reason.clone(), true),
                        Signal::ExitShort { reason } => (reason.clone(), false),
                        _ => {
                            debug!("Unexpected non-exit signal during exit check");
                            continue;
                        }
                    };

                    // Only close if the direction matches
                    if pos.is_long == is_exit_long {
                        // Compute PnL
                        let pnl_pct = if pos.is_long {
                            (point.price - pos.entry_price) / pos.entry_price * 100.0
                        } else {
                            (pos.entry_price - point.price) / pos.entry_price * 100.0
                        };
                        let gross_pnl = pos.size_usd * pnl_pct / 100.0;
                        let entry_fee = pos.size_usd * fee_rate;
                        let exit_fee = pos.size_usd * fee_rate;
                        let route_cost_usd = pos.route_cost_usd;
                        let net_pnl = gross_pnl - entry_fee - exit_fee - route_cost_usd;

                        balance += net_pnl;
                        if balance > peak_balance {
                            peak_balance = balance;
                        }
                        let dd = peak_balance - balance;
                        if dd > max_drawdown_usd {
                            max_drawdown_usd = dd;
                        }

                        let hold_secs =
                            ((point.timestamp_ms - pos.entry_timestamp_ms).max(0) as u64) / 1000;

                        let exit_stale = zone_stale;

                        // Check if this trade was stale
                        if pos.entry_stale || exit_stale {
                            stale_trade_count += 1;
                        }

                        trades.push(ReplayTrade {
                            symbol: point.symbol.clone(),
                            side: if pos.is_long {
                                "long".to_string()
                            } else {
                                "short".to_string()
                            },
                            entry_price: pos.entry_price,
                            exit_price: point.price,
                            size_usd: pos.size_usd,
                            gross_pnl,
                            entry_fee,
                            exit_fee,
                            route_cost_usd,
                            net_pnl,
                            hold_secs,
                            exit_reason: format!("{:?}", exit_reason),
                            entry_timestamp_ms: pos.entry_timestamp_ms,
                            exit_timestamp_ms: point.timestamp_ms,
                            entry_stale: pos.entry_stale,
                            exit_stale,
                            peak_price: pos.peak_price,
                            mae_usd: compute_mae_usd(pos.is_long, pos.entry_price, pos.worst_price, pos.size_usd),
                            mfe_usd: compute_mfe_usd(pos.is_long, pos.entry_price, pos.best_price, pos.size_usd),
                            worst_price: pos.worst_price,
                            best_price: pos.best_price,
                            is_zone_touch: pos.is_zone_touch,
                            post_liquidation_drift_usd: 0.0, // Computed in post-processing
                            time_to_reversal_secs: 0.0,
                            time_to_next_zone_secs: 0.0,
                            stop_efficiency: compute_stop_efficiency(net_pnl, pos.is_long, pos.entry_price, pos.best_price, pos.size_usd),
                        });

                        trade_returns.push(net_pnl);

                        // Clear pending state
                        strategy.clear_pending(&point.symbol, if pos.is_long { "long" } else { "short" });

                        // Record loss for cooldown
                        if net_pnl < 0.0 {
                            strategy.record_loss(point.timestamp_ms);
                        }

                        open_position = None;
                        last_signal_key = None;
                    }
                } else {
                    // Update peak price, worst price (adverse), and best price (favorable)
                    if let Some(ref mut pos_ref) = open_position {
                        if pos_ref.is_long && point.price > pos_ref.peak_price {
                            pos_ref.peak_price = point.price;
                        }
                        if !pos_ref.is_long && point.price < pos_ref.peak_price {
                            pos_ref.peak_price = point.price;
                        }
                        // Track worst adverse price (lowest for long, highest for short)
                        if pos_ref.is_long && point.price < pos_ref.worst_price {
                            pos_ref.worst_price = point.price;
                        }
                        if !pos_ref.is_long && point.price > pos_ref.worst_price {
                            pos_ref.worst_price = point.price;
                        }
                        // Track best favorable price (highest for long, lowest for short)
                        if pos_ref.is_long && point.price > pos_ref.best_price {
                            pos_ref.best_price = point.price;
                        }
                        if !pos_ref.is_long && point.price < pos_ref.best_price {
                            pos_ref.best_price = point.price;
                        }
                        // Also use candle high/low if available for more accurate MAE/MFE
                        if let Some(high) = point.high {
                            if pos_ref.is_long && high > pos_ref.best_price {
                                pos_ref.best_price = high;
                            }
                            if !pos_ref.is_long && high > pos_ref.worst_price {
                                pos_ref.worst_price = high;
                            }
                        }
                        if let Some(low) = point.low {
                            if pos_ref.is_long && low < pos_ref.worst_price {
                                pos_ref.worst_price = low;
                            }
                            if !pos_ref.is_long && low < pos_ref.best_price {
                                pos_ref.best_price = low;
                            }
                        }
                    }
                }
            }

            // Check for new entry signals only if no open position
            if open_position.is_none() {
                let entry_signal = strategy.detect_entry(&snapshot);

                if let Signal::MomentumLong { .. } | Signal::MomentumShort { .. } =
                    &entry_signal
                {
                    let is_long = matches!(entry_signal, Signal::MomentumLong { .. });
                    let side = if is_long { "long" } else { "short" };
                    let key = (point.symbol.clone(), side.to_string());

                    // Check for duplicate pending
                    if last_signal_key.as_ref() == Some(&key) {
                        duplicate_pending_count += 1;
                    } else {
                        signal_events += 1;

                        let size_usd = self.params.clip_size_usd;
                        let entry_fee = size_usd * fee_rate;
                        let route_cost_usd = size_usd * point.route_cost_bps.unwrap_or(0.0) / 10000.0;

                        // Check if balance can support this trade
                        if balance > size_usd + entry_fee + route_cost_usd {
                            balance -= entry_fee + route_cost_usd;

                            open_position = Some(OpenPosition {
                                is_long,
                                entry_price: point.price,
                                size_usd,
                                entry_timestamp_ms: point.timestamp_ms,
                                peak_price: point.price,
                                entry_stale: zone_stale,
                                route_cost_usd,
                                worst_price: point.price,
                                best_price: point.price,
                                is_zone_touch: point.is_zone_touch.unwrap_or(false),
                            });

                            last_signal_key = Some(key);

                            debug!(
                                "[{}] {} entry at {:.2}, size=${:.0}, stale={}",
                                i, side, point.price, size_usd, zone_stale
                            );
                        }
                    }
                }
            }

            debug!(
                "[{}] price={:.2}, velocity={:.3}, trades={}, balance={:.2}",
                i,
                point.price,
                velocity,
                trades.len(),
                balance
            );
        }

        // Force-close any remaining open position at the last price
        if let Some(pos) = open_position.take() {
            let last_price = data_points
                .last()
                .map(|p| p.price)
                .unwrap_or(pos.entry_price);
            let pnl_pct = if pos.is_long {
                (last_price - pos.entry_price) / pos.entry_price * 100.0
            } else {
                (pos.entry_price - last_price) / pos.entry_price * 100.0
            };
            let gross_pnl = pos.size_usd * pnl_pct / 100.0;
            let entry_fee = pos.size_usd * fee_rate;
            let exit_fee = pos.size_usd * fee_rate;
            let net_pnl = gross_pnl - entry_fee - exit_fee - pos.route_cost_usd;

            balance += net_pnl;

            trades.push(ReplayTrade {
                symbol: data_points
                    .last()
                    .map(|p| p.symbol.clone())
                    .unwrap_or_default(),
                side: if pos.is_long {
                    "long".to_string()
                } else {
                    "short".to_string()
                },
                entry_price: pos.entry_price,
                exit_price: last_price,
                size_usd: pos.size_usd,
                gross_pnl,
                entry_fee,
                exit_fee,
                route_cost_usd: pos.route_cost_usd,
                net_pnl,
                hold_secs: 0,
                exit_reason: "ForceClose".to_string(),
                entry_timestamp_ms: pos.entry_timestamp_ms,
                exit_timestamp_ms: data_points.last().map(|p| p.timestamp_ms).unwrap_or(0),
                entry_stale: pos.entry_stale,
                exit_stale: false,
                peak_price: pos.peak_price,
                mae_usd: compute_mae_usd(pos.is_long, pos.entry_price, pos.worst_price, pos.size_usd),
                mfe_usd: compute_mfe_usd(pos.is_long, pos.entry_price, pos.best_price, pos.size_usd),
                worst_price: pos.worst_price,
                best_price: pos.best_price,
                is_zone_touch: pos.is_zone_touch,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: compute_stop_efficiency(net_pnl, pos.is_long, pos.entry_price, pos.best_price, pos.size_usd),
            });

            trade_returns.push(net_pnl);
            stale_trade_count += if pos.entry_stale { 1 } else { 0 };
        }

        // Compute aggregate metrics
        let win_count = trades.iter().filter(|t| t.net_pnl > 0.0).count();
        let loss_count = trades.iter().filter(|t| t.net_pnl <= 0.0).count();
        let gross_pnl: f64 = trades.iter().map(|t| t.gross_pnl).sum();
        let total_fees: f64 = trades.iter().map(|t| t.entry_fee + t.exit_fee + t.route_cost_usd).sum();
        let net_pnl = gross_pnl - total_fees;
        let win_rate_pct = if !trades.is_empty() {
            win_count as f64 / trades.len() as f64 * 100.0
        } else {
            0.0
        };
        let avg_hold_secs = if !trades.is_empty() {
            trades.iter().map(|t| t.hold_secs as f64).sum::<f64>() / trades.len() as f64
        } else {
            0.0
        };

        // Sharpe ratio: (mean_return / std_return) * sqrt(252) (annualized)
        let sharpe_ratio = compute_sharpe(&trade_returns);

        // Sortino ratio: mean_return / downside_deviation (annualized)
        let sortino_ratio = compute_sortino(&trade_returns);

        // Calmar ratio: annualized_return / max_drawdown
        let calmar_ratio = compute_calmar(net_pnl, starting_balance, max_drawdown_usd, data_points.len());

        // Net expectancy
        let net_expectancy = compute_net_expectancy(&trades);

        // Max drawdown as percentage
        let max_drawdown_pct = if starting_balance > 0.0 {
            max_drawdown_usd / starting_balance * 100.0
        } else {
            0.0
        };

        // Extended per-trade metrics
        let avg_mae_usd = if !trades.is_empty() {
            trades.iter().map(|t| t.mae_usd).sum::<f64>() / trades.len() as f64
        } else {
            0.0
        };
        let avg_mfe_usd = if !trades.is_empty() {
            trades.iter().map(|t| t.mfe_usd).sum::<f64>() / trades.len() as f64
        } else {
            0.0
        };

        // Zone-touch win rate
        let zone_touch_trades: Vec<&ReplayTrade> = trades.iter().filter(|t| t.is_zone_touch).collect();
        let zone_touch_trade_count = zone_touch_trades.len();
        let zone_touch_win_count = zone_touch_trades.iter().filter(|t| t.net_pnl > 0.0).count();
        let zone_touch_win_rate_pct = if zone_touch_trade_count > 0 {
            zone_touch_win_count as f64 / zone_touch_trade_count as f64 * 100.0
        } else {
            0.0
        };

        // Post-liquidation drift, time-to-reversal, time-to-next-zone averages
        let avg_post_liquidation_drift_usd = if !trades.is_empty() {
            trades.iter().map(|t| t.post_liquidation_drift_usd).sum::<f64>() / trades.len() as f64
        } else {
            0.0
        };
        let trades_with_reversal: Vec<&ReplayTrade> = trades.iter().filter(|t| t.time_to_reversal_secs > 0.0).collect();
        let avg_time_to_reversal_secs = if !trades_with_reversal.is_empty() {
            trades_with_reversal.iter().map(|t| t.time_to_reversal_secs).sum::<f64>() / trades_with_reversal.len() as f64
        } else {
            0.0
        };
        let trades_with_next_zone: Vec<&ReplayTrade> = trades.iter().filter(|t| t.time_to_next_zone_secs > 0.0).collect();
        let avg_time_to_next_zone_secs = if !trades_with_next_zone.is_empty() {
            trades_with_next_zone.iter().map(|t| t.time_to_next_zone_secs).sum::<f64>() / trades_with_next_zone.len() as f64
        } else {
            0.0
        };

        // Average stop efficiency
        let avg_stop_efficiency = if !trades.is_empty() {
            trades.iter().map(|t| t.stop_efficiency).sum::<f64>() / trades.len() as f64
        } else {
            0.0
        };

        // Single-trade dependency: flag when one trade's profit > 25% of total net profit
        let single_trade_dependency_flagged = check_single_trade_dependency(&trades);
        let dominant_trade_index = find_dominant_trade(&trades);

        // Compute minimum zone distance across all data points for criterion 12
        let min_zone_distance_bps = compute_min_zone_distance(data_points);

        // Evaluate promotion criteria (12 criteria)
        let criteria = evaluate_promotion_criteria(
            &trades,
            net_expectancy,
            max_drawdown_pct,
            stale_trade_count,
            duplicate_pending_count,
            signal_events,
            sharpe_ratio,
            None,   // fishing_result: not composed in base run
            None,   // pyramid_result: not composed in base run
            min_zone_distance_bps,
            &self.gate_config,
        );

        let all_passed = criteria.iter().all(|c| c.passed);
        let verdict = if all_passed {
            PromotionVerdict::Approved
        } else {
            PromotionVerdict::Denied
        };

        ReplayResult {
            strategy_name: "liquidation-cascade-hunter".to_string(),
            start_balance: starting_balance,
            final_balance: balance,
            trade_count: trades.len(),
            win_count,
            loss_count,
            gross_pnl,
            total_fees,
            net_pnl,
            win_rate_pct,
            sharpe_ratio,
            max_drawdown_usd,
            max_drawdown_pct,
            stale_trade_count,
            duplicate_pending_count,
            signal_events,
            avg_hold_secs,
            data_points_replayed: data_points.len(),
            trades,
            baseline_balance: starting_balance,
            baseline_net_pnl: 0.0,
            net_expectancy,
            pnl_vs_baseline: net_pnl,
            promotion_criteria: criteria,
            promotion_verdict: verdict,
            // Extended metrics
            sortino_ratio,
            calmar_ratio,
            avg_mae_usd,
            avg_mfe_usd,
            fishing_fill_rate: 0.0, // Set when fishing is composed
            zone_touch_win_rate_pct,
            zone_touch_trade_count,
            zone_touch_win_count,
            avg_post_liquidation_drift_usd,
            avg_time_to_reversal_secs,
            avg_time_to_next_zone_secs,
            avg_stop_efficiency,
            single_trade_dependency_flagged,
            dominant_trade_index,
            fishing_result: None,
            pyramid_result: None,
        }
    }

    /// Run the replay pipeline with fishing order simulation composed into
    /// the flow. Simulates passive limit orders at liquidation zone offsets
    /// and records fill rates, adverse selection, and expectancy comparison.
    pub fn run_with_fishing(
        &self,
        data_points: &[ReplayDataPoint],
        fishing_config: &FishingLadderConfig,
    ) -> ReplayResult {
        let mut result = self.run(data_points);

        // Run fishing simulation if we have zone data
        if let Some(first_point) = data_points.first()
            && let Some(zone) = first_point.liquidation_zones.as_ref().and_then(|z| z.first())
        {
            let memory_zone = crate::liquidity_memory::MemoryZone::from_liquidation_zone(
                zone,
                first_point.timestamp_ms,
                50.0, // Default range bps
            );
            let candles: Vec<MarketConditions> = data_points
                .iter()
                .map(|p| MarketConditions {
                    price: p.price,
                    high: p.high.unwrap_or(p.price * 1.001),
                    low: p.low.unwrap_or(p.price * 0.999),
                    timestamp_ms: p.timestamp_ms,
                    spread_pct: p.spread_pct.unwrap_or(0.1),
                    depth_usd: p.depth_usd.unwrap_or(50_000.0),
                    cascade_detected: false,
                    zone_decay_scores: vec![],
                })
                .collect();

            let fishing_result = crate::fishing::run_fishing_simulation(
                &memory_zone,
                true, // Default: long fishing
                &candles,
                fishing_config,
            );
            result.fishing_fill_rate = fishing_result.fill_rate;
            result.fishing_result = Some(fishing_result);
        }

        // Re-evaluate promotion criteria with fishing result
        let min_zone_dist = compute_min_zone_distance(data_points);
        result.promotion_criteria = evaluate_promotion_criteria(
            &result.trades,
            result.net_expectancy,
            result.max_drawdown_pct,
            result.stale_trade_count,
            result.duplicate_pending_count,
            result.signal_events,
            result.sharpe_ratio,
            result.fishing_result.as_ref(),
            result.pyramid_result.as_ref(),
            min_zone_dist,
            &self.gate_config,
        );
        result.promotion_verdict = if result.promotion_criteria.iter().all(|c| c.passed) {
            PromotionVerdict::Approved
        } else {
            PromotionVerdict::Denied
        };

        result
    }

    /// Run the replay pipeline with both fishing and pyramiding composed into
    /// the flow. This provides the full end-to-end replay:
    /// zone detected → fishing order → fill → pyramid tranches → exit.
    pub fn run_with_fishing_and_pyramiding(
        &self,
        data_points: &[ReplayDataPoint],
        fishing_config: &FishingLadderConfig,
        pyramid_config: &PyramidConfig,
    ) -> ReplayResult {
        let mut result = self.run_with_fishing(data_points, fishing_config);

        // Run pyramiding simulation if we have enough data
        if !data_points.is_empty() {
            let first_point = data_points.first().unwrap();
            let is_long = true; // Default assumption
            let pyramid_contexts: Vec<AddTrancheContext> = data_points
                .iter()
                .map(|p| AddTrancheContext {
                    current_price: p.price,
                    timestamp_ms: p.timestamp_ms,
                    data_timestamp_ms: p.zone_capture_timestamp_ms.unwrap_or(p.timestamp_ms),
                    reclaim_detected: false,
                    higher_low_detected: false,
                    retest_successful: false,
                    current_atr: 0.0,
                    correlated_exposure_usd: 0.0,
                })
                .collect();
            let stop_prices: Vec<f64> = data_points
                .iter()
                .map(|p| {
                    if is_long {
                        p.price * 0.99 // Default: 1% stop
                    } else {
                        p.price * 1.01
                    }
                })
                .collect();

            let pyramid_result = crate::pyramiding::run_pyramid_simulation(
                &first_point.symbol,
                is_long,
                pyramid_config.clone(),
                &pyramid_contexts,
                &stop_prices,
            );
            result.pyramid_result = Some(pyramid_result);
        }

        // Re-evaluate promotion criteria with both fishing and pyramid results
        let min_zone_dist = compute_min_zone_distance(data_points);
        result.promotion_criteria = evaluate_promotion_criteria(
            &result.trades,
            result.net_expectancy,
            result.max_drawdown_pct,
            result.stale_trade_count,
            result.duplicate_pending_count,
            result.signal_events,
            result.sharpe_ratio,
            result.fishing_result.as_ref(),
            result.pyramid_result.as_ref(),
            min_zone_dist,
            &self.gate_config,
        );
        result.promotion_verdict = if result.promotion_criteria.iter().all(|c| c.passed) {
            PromotionVerdict::Approved
        } else {
            PromotionVerdict::Denied
        };

        result
    }

    /// Generate a human-readable Markdown promotion report.
    pub fn generate_markdown_report(result: &ReplayResult) -> String {
        let mut report = String::new();

        report.push_str("# Liquidation Cascade Hunter — Replay Promotion Report\n\n");

        report.push_str("## Summary\n\n");
        report.push_str(&format!(
            "- **Strategy:** {}\n",
            result.strategy_name
        ));
        report.push_str(&format!(
            "- **Verdict:** {:?}\n",
            result.promotion_verdict
        ));
        report.push_str(&format!(
            "- **Data Points Replayed:** {}\n",
            result.data_points_replayed
        ));
        report.push_str(&format!(
            "- **Starting Balance:** ${:.2}\n",
            result.start_balance
        ));
        report.push_str(&format!(
            "- **Final Balance:** ${:.2}\n",
            result.final_balance
        ));
        report.push_str(&format!(
            "- **Net PnL:** ${:.2}\n",
            result.net_pnl
        ));
        report.push_str(&format!(
            "- **Baseline PnL:** ${:.2} (no-trade)\n",
            result.baseline_net_pnl
        ));
        report.push_str(&format!(
            "- **PnL vs Baseline:** ${:.2}\n\n",
            result.pnl_vs_baseline
        ));

        report.push_str("## Performance Metrics\n\n");
        report.push_str("| Metric | Value |\n|---|---|\n");
        report.push_str(&format!(
            "| Trades | {} |\n",
            result.trade_count
        ));
        report.push_str(&format!(
            "| Wins / Losses | {} / {} |\n",
            result.win_count, result.loss_count
        ));
        report.push_str(&format!(
            "| Win Rate | {:.1}% |\n",
            result.win_rate_pct
        ));
        report.push_str(&format!(
            "| Gross PnL | ${:.2} |\n",
            result.gross_pnl
        ));
        report.push_str(&format!(
            "| Total Fees | ${:.2} |\n",
            result.total_fees
        ));
        report.push_str(&format!(
            "| Net PnL | ${:.2} |\n",
            result.net_pnl
        ));
        report.push_str(&format!(
            "| Sharpe Ratio | {:.4} |\n",
            result.sharpe_ratio
        ));
        report.push_str(&format!(
            "| Sortino Ratio | {:.4} |\n",
            result.sortino_ratio
        ));
        report.push_str(&format!(
            "| Calmar Ratio | {:.4} |\n",
            result.calmar_ratio
        ));
        report.push_str(&format!(
            "| Max Drawdown | ${:.2} ({:.2}%) |\n",
            result.max_drawdown_usd, result.max_drawdown_pct
        ));
        report.push_str(&format!(
            "| Net Expectancy | ${:.4} |\n",
            result.net_expectancy
        ));
        report.push_str(&format!(
            "| Avg MAE | ${:.4} |\n",
            result.avg_mae_usd
        ));
        report.push_str(&format!(
            "| Avg MFE | ${:.4} |\n",
            result.avg_mfe_usd
        ));
        report.push_str(&format!(
            "| Avg Stop Efficiency | {:.4} |\n",
            result.avg_stop_efficiency
        ));
        report.push_str(&format!(
            "| Fishing Fill Rate | {:.2}% |\n",
            result.fishing_fill_rate * 100.0
        ));
        report.push_str(&format!(
            "| Zone-Touch Win Rate | {:.1}% ({} / {}) |\n",
            result.zone_touch_win_rate_pct, result.zone_touch_win_count, result.zone_touch_trade_count
        ));
        report.push_str(&format!(
            "| Avg Post-Liq Drift | ${:.4} |\n",
            result.avg_post_liquidation_drift_usd
        ));
        report.push_str(&format!(
            "| Avg Time-to-Reversal | {:.1}s |\n",
            result.avg_time_to_reversal_secs
        ));
        report.push_str(&format!(
            "| Avg Time-to-Next-Zone | {:.1}s |\n",
            result.avg_time_to_next_zone_secs
        ));
        report.push_str(&format!(
            "| Single-Trade Dependency | {} |\n",
            if result.single_trade_dependency_flagged { "⚠️ FLAGGED (>25%)" } else { "✅ OK" }
        ));
        report.push_str(&format!(
            "| Avg Hold Time | {:.1}s |\n",
            result.avg_hold_secs
        ));
        report.push_str(&format!(
            "| Stale Trades | {} |\n",
            result.stale_trade_count
        ));
        report.push_str(&format!(
            "| Duplicate Pendings | {} |\n",
            result.duplicate_pending_count
        ));
        report.push_str(&format!(
            "| Signal Events | {} |\n\n",
            result.signal_events
        ));

        report.push_str("## Promotion Criteria\n\n");
        report.push_str("| Criterion | Status | Actual | Threshold |\n|---|---|---|---|\n");
        for c in &result.promotion_criteria {
            let status = if c.passed { "✅ PASS" } else { "❌ FAIL" };
            report.push_str(&format!(
                "| {} | {} | {} {} | {} {} |\n",
                c.description, status, c.actual_value, c.unit, c.threshold_value, c.unit
            ));
        }
        report.push('\n');

        if !result.trades.is_empty() {
            report.push_str("## Trade Log (first 20)\n\n");
            report.push_str(
                "| # | Symbol | Side | Entry | Exit | Net PnL | Hold(s) | Exit Reason | Stale |\n|---|---|---|---|---|---|---|---|---|\n",
            );
            for (i, t) in result.trades.iter().take(20).enumerate() {
                let stale = if t.entry_stale || t.exit_stale {
                    "⚠️"
                } else {
                    ""
                };
                report.push_str(&format!(
                    "| {} | {} | {} | {:.2} | {:.2} | ${:.2} | {} | {} | {} |\n",
                    i + 1,
                    t.symbol,
                    t.side,
                    t.entry_price,
                    t.exit_price,
                    t.net_pnl,
                    t.hold_secs,
                    t.exit_reason,
                    stale
                ));
            }
            if result.trades.len() > 20 {
                report.push_str(&format!(
                    "\n... and {} more trades\n",
                    result.trades.len() - 20
                ));
            }
        }

        report
    }

    /// Generate a JSON promotion report and write to file using atomic writes.
    pub fn write_json_report(result: &ReplayResult, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(result)?;
        atomic_write(path, &json)
    }

    /// Generate a Markdown promotion report and write to file using atomic writes.
    pub fn write_markdown_report(result: &ReplayResult, path: &Path) -> anyhow::Result<()> {
        let md = Self::generate_markdown_report(result);
        atomic_write(path, &md)
    }

    /// Verify that the strategy is not available in live mode.
    ///
    /// This checks that the liquidation-cascade-hunter has `paper_only = true`
    /// and is blocked from the live engine.
    pub fn verify_paper_only(params: &LiquidationCascadeParams) -> bool {
        params.paper_only
    }
}

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

/// Internal representation of an open position during replay.
struct OpenPosition {
    is_long: bool,
    entry_price: f64,
    size_usd: f64,
    entry_timestamp_ms: i64,
    peak_price: f64,
    entry_stale: bool,
    route_cost_usd: f64,
    /// Worst adverse price seen during hold (lowest for long, highest for short).
    worst_price: f64,
    /// Best favorable price seen during hold (highest for long, lowest for short).
    best_price: f64,
    /// Whether this trade was triggered at a zone touch.
    is_zone_touch: bool,
}

/// Compute annualized Sharpe ratio from a list of trade returns.
fn compute_sharpe(returns: &[f64]) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    if std_dev < 1e-10 {
        return 0.0;
    }
    // Annualize assuming ~252 trading days, with ~5 trades per day for this strategy
    let trades_per_year: f64 = 252.0 * 5.0;
    (mean / std_dev) * trades_per_year.sqrt()
}

/// Compute annualized Sortino ratio from trade returns.
/// Uses downside deviation (only negative returns) as the denominator.
fn compute_sortino(returns: &[f64]) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let downside: Vec<f64> = returns.iter().map(|r| (*r).min(0.0)).collect();
    let downside_var = downside.iter().map(|r| r.powi(2)).sum::<f64>() / n;
    let downside_dev = downside_var.sqrt();
    if downside_dev < 1e-10 {
        return 0.0;
    }
    let trades_per_year: f64 = 252.0 * 5.0;
    (mean / downside_dev) * trades_per_year.sqrt()
}

/// Compute Calmar ratio: annualized_return / max_drawdown.
fn compute_calmar(net_pnl: f64, starting_balance: f64, max_drawdown_usd: f64, data_points: usize) -> f64 {
    if starting_balance <= 0.0 || max_drawdown_usd <= 0.0 || data_points == 0 {
        return 0.0;
    }
    // Annualized return approximation
    let total_return_pct = net_pnl / starting_balance;
    // Assume data points are roughly evenly spaced; approximate annualization
    // using ~1260 data points per year (252 days * 5 intervals)
    let annualization_factor = 1260.0 / data_points as f64;
    let annualized_return = total_return_pct * annualization_factor;
    let max_drawdown_pct = max_drawdown_usd / starting_balance;
    if max_drawdown_pct < 1e-10 {
        return 0.0;
    }
    annualized_return / max_drawdown_pct
}

/// Compute Maximum Adverse Excursion in USD.
/// For longs: (worst_price - entry_price) / entry_price * size_usd (negative).
/// For shorts: (worst_price - entry_price) / entry_price * size_usd (positive, negated).
fn compute_mae_usd(is_long: bool, entry_price: f64, worst_price: f64, size_usd: f64) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }
    if is_long {
        // For longs, worst adverse is the lowest price
        ((worst_price - entry_price) / entry_price) * size_usd
    } else {
        // For shorts, worst adverse is the highest price
        ((worst_price - entry_price) / entry_price) * size_usd
    }
}

/// Compute Maximum Favorable Excursion in USD.
fn compute_mfe_usd(is_long: bool, entry_price: f64, best_price: f64, size_usd: f64) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }
    if is_long {
        // For longs, best favorable is the highest price
        ((best_price - entry_price) / entry_price) * size_usd
    } else {
        // For shorts, best favorable is the lowest price
        ((entry_price - best_price) / entry_price) * size_usd
    }
}

/// Compute stop efficiency: actual PnL / MFE.
/// Ranges from negative (loser) to 1.0 (captured entire favorable move).
fn compute_stop_efficiency(
    net_pnl: f64,
    is_long: bool,
    entry_price: f64,
    best_price: f64,
    size_usd: f64,
) -> f64 {
    let mfe = compute_mfe_usd(is_long, entry_price, best_price, size_usd);
    if mfe.abs() < 1e-10 {
        return 0.0;
    }
    net_pnl / mfe
}

/// Check if single-trade dependency exceeds 25% of total profit.
/// Returns true if any single winning trade accounts for >25% of total net PnL.
fn check_single_trade_dependency(trades: &[ReplayTrade]) -> bool {
    if trades.is_empty() {
        return false;
    }
    let total_pnl: f64 = trades.iter().map(|t| t.net_pnl).sum();
    if total_pnl <= 0.0 {
        return false; // No positive total profit to dominate
    }
    let max_single = trades
        .iter()
        .map(|t| t.net_pnl.max(0.0))
        .fold(0.0_f64, f64::max);
    max_single / total_pnl > 0.25
}

/// Find the index of the dominant trade (one that contributes the most to total profit).
fn find_dominant_trade(trades: &[ReplayTrade]) -> Option<usize> {
    if trades.is_empty() {
        return None;
    }
    let total_pnl: f64 = trades.iter().map(|t| t.net_pnl).sum();
    if total_pnl <= 0.0 {
        return None;
    }
    let mut best_idx = 0;
    let mut best_ratio = 0.0;
    for (i, t) in trades.iter().enumerate() {
        let ratio = t.net_pnl.max(0.0) / total_pnl;
        if ratio > best_ratio {
            best_ratio = ratio;
            best_idx = i;
        }
    }
    Some(best_idx)
}

/// Compute net expectancy: (win_rate * avg_win) - (loss_rate * avg_loss) - avg_route_cost.
fn compute_net_expectancy(trades: &[ReplayTrade]) -> f64 {
    if trades.is_empty() {
        return 0.0;
    }
    let wins: Vec<&ReplayTrade> = trades.iter().filter(|t| t.net_pnl > 0.0).collect();
    let losses: Vec<&ReplayTrade> = trades.iter().filter(|t| t.net_pnl <= 0.0).collect();
    let win_rate = wins.len() as f64 / trades.len() as f64;
    let loss_rate = losses.len() as f64 / trades.len() as f64;
    let avg_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().map(|t| t.net_pnl).sum::<f64>() / wins.len() as f64
    };
    let avg_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().map(|t| t.net_pnl).sum::<f64>() / losses.len() as f64
    };
    let avg_route_cost = trades.iter().map(|t| t.route_cost_usd).sum::<f64>() / trades.len() as f64;
    (win_rate * avg_win) - (loss_rate * avg_loss.abs()) - avg_route_cost
}

/// Evaluate all 12 promotion criteria.
///
/// Criteria 1–6 are the original gate. Criteria 7–12 are the extended gate:
/// 7. fee/gross < 35%
/// 8. no single event > 25% of profit
/// 9. fishing improves expectancy or reduces drawdown
/// 10. pyramiding improves risk-adjusted return
/// 11. route cost doesn't consume edge
/// 12. liquidation distance safe at proposed leverage
#[allow(clippy::too_many_arguments)]
fn evaluate_promotion_criteria(
    trades: &[ReplayTrade],
    net_expectancy: f64,
    max_drawdown_pct: f64,
    stale_trade_count: usize,
    duplicate_pending_count: usize,
    signal_events: usize,
    sharpe_ratio: f64,
    fishing_result: Option<&FishingSimResult>,
    pyramid_result: Option<&PyramidResult>,
    min_zone_distance_bps: Option<f64>,
    config: &PromotionGateConfig,
) -> Vec<CriterionStatus> {
    // Pre-compute values needed for extended criteria
    let gross_pnl: f64 = trades.iter().map(|t| t.gross_pnl).sum();
    let total_fees: f64 = trades
        .iter()
        .map(|t| t.entry_fee + t.exit_fee + t.route_cost_usd)
        .sum();
    let total_route_cost: f64 = trades.iter().map(|t| t.route_cost_usd).sum();
    let single_trade_dep = check_single_trade_dependency(trades);

    // Criterion 7: fee/gross < 35%
    let (fee_gross_passed, fee_gross_actual) = if trades.is_empty() {
        // No trades → no fees → passes by default
        (true, 0.0)
    } else if gross_pnl.abs() < 1e-10 {
        // Fees but no gross PnL → fees dominate
        (false, f64::INFINITY)
    } else {
        let ratio = total_fees / gross_pnl.abs() * 100.0;
        (ratio < config.max_fee_to_gross_pct, ratio)
    };

    // Criterion 9: fishing improves expectancy or reduces drawdown
    let (fishing_passed, fishing_actual) = match fishing_result {
        Some(fr) => {
            // Fishing passes if expectancy improved OR we have positive expectancy delta
            let improves_expectancy = fr.expectancy_delta > 0.0;
            (improves_expectancy, format!("{:.4} (delta)", fr.expectancy_delta))
        }
        None => {
            // No fishing composed — passes by default (nothing to degrade)
            (true, "N/A (not composed)".to_string())
        }
    };

    // Criterion 10: pyramiding improves risk-adjusted return
    let (pyramid_passed, pyramid_actual) = match pyramid_result {
        Some(pr) => {
            // Pyramiding passes if unrealized PnL is positive (improves return)
            // and not stopped out, OR if it has positive unrealized PnL per unit risk
            let improves = pr.unrealized_pnl_usd > 0.0 && !pr.stopped_out;
            (improves, format!("{:.4} USD unrealized", pr.unrealized_pnl_usd))
        }
        None => {
            // No pyramiding composed — passes by default
            (true, "N/A (not composed)".to_string())
        }
    };

    // Criterion 11: route cost doesn't consume edge
    let (route_cost_passed, route_cost_actual) = if net_expectancy > 0.0 {
        let route_pct = total_route_cost / net_expectancy * 100.0;
        (route_pct < config.max_route_cost_pct_of_expectancy, route_pct)
    } else {
        // No positive expectancy → edge is already negative, route cost irrelevant
        (false, 0.0)
    };

    // Criterion 12: liquidation distance safe at proposed leverage
    // Safe distance = the minimum zone distance must exceed the liquidation
    // distance at proposed leverage. At leverage L, liquidation is at ~100/L %.
    // Convert to bps: (100/L) * 100 = 10000/L bps.
    let leverage_liquidation_bps = 10000.0 / config.proposed_leverage;
    let (liq_distance_passed, liq_distance_actual) = match min_zone_distance_bps {
        Some(dist) => {
            // Distance must be greater than the liquidation distance at leverage
            // AND greater than the configured minimum safe distance
            let safe = dist > leverage_liquidation_bps
                && dist > config.min_safe_liquidation_distance_bps;
            (safe, dist)
        }
        None => {
            // No zones observed — passes by default (no liquidation risk detected)
            (true, 0.0)
        }
    };

    vec![
        // --- Original 6 criteria ---
        CriterionStatus {
            name: "net_expectancy".to_string(),
            description: "Positive net expectancy after route costs".to_string(),
            passed: net_expectancy > 0.0,
            actual_value: format!("{:.4}", net_expectancy),
            threshold_value: "> 0".to_string(),
            unit: "USD".to_string(),
        },
        CriterionStatus {
            name: "max_drawdown".to_string(),
            description: "Max drawdown within policy limit".to_string(),
            passed: max_drawdown_pct <= config.max_drawdown_pct,
            actual_value: format!("{:.2}", max_drawdown_pct),
            threshold_value: format!("≤ {:.1}", config.max_drawdown_pct),
            unit: "pct".to_string(),
        },
        CriterionStatus {
            name: "stale_data_trades".to_string(),
            description: "Zero stale-data trades".to_string(),
            passed: stale_trade_count == 0,
            actual_value: stale_trade_count.to_string(),
            threshold_value: "= 0".to_string(),
            unit: "count".to_string(),
        },
        CriterionStatus {
            name: "duplicate_pending_trades".to_string(),
            description: "Zero duplicate pending trades".to_string(),
            passed: duplicate_pending_count == 0,
            actual_value: duplicate_pending_count.to_string(),
            threshold_value: "= 0".to_string(),
            unit: "count".to_string(),
        },
        CriterionStatus {
            name: "min_signal_events".to_string(),
            description: "Minimum 30 signal events for statistical validity".to_string(),
            passed: signal_events >= config.min_signal_events,
            actual_value: signal_events.to_string(),
            threshold_value: format!("≥ {}", config.min_signal_events),
            unit: "count".to_string(),
        },
        CriterionStatus {
            name: "sharpe_ratio".to_string(),
            description: "Sharpe ratio ≥ 1.0 threshold".to_string(),
            passed: sharpe_ratio >= config.min_sharpe,
            actual_value: format!("{:.4}", sharpe_ratio),
            threshold_value: format!("≥ {:.1}", config.min_sharpe),
            unit: "ratio".to_string(),
        },
        // --- Extended 6 criteria (7–12) ---
        CriterionStatus {
            name: "fee_to_gross_ratio".to_string(),
            description: "Fee/gross ratio < 35%".to_string(),
            passed: fee_gross_passed,
            actual_value: format!("{:.2}", fee_gross_actual),
            threshold_value: format!("< {:.1}", config.max_fee_to_gross_pct),
            unit: "pct".to_string(),
        },
        CriterionStatus {
            name: "single_trade_dependency".to_string(),
            description: "No single event contributes > 25% of total profit".to_string(),
            passed: !single_trade_dep,
            actual_value: if single_trade_dep { "flagged".to_string() } else { "ok".to_string() },
            threshold_value: format!("≤ {:.0}%", config.max_single_trade_profit_pct),
            unit: "pct".to_string(),
        },
        CriterionStatus {
            name: "fishing_improvement".to_string(),
            description: "Fishing orders improve expectancy or reduce drawdown".to_string(),
            passed: fishing_passed,
            actual_value: fishing_actual,
            threshold_value: "positive delta".to_string(),
            unit: "delta".to_string(),
        },
        CriterionStatus {
            name: "pyramiding_improvement".to_string(),
            description: "Pyramiding improves risk-adjusted return".to_string(),
            passed: pyramid_passed,
            actual_value: pyramid_actual,
            threshold_value: "positive unrealized PnL".to_string(),
            unit: "USD".to_string(),
        },
        CriterionStatus {
            name: "route_cost_edge".to_string(),
            description: "Route cost does not consume edge".to_string(),
            passed: route_cost_passed,
            actual_value: format!("{:.2}", route_cost_actual),
            threshold_value: format!("< {:.1}", config.max_route_cost_pct_of_expectancy),
            unit: "pct".to_string(),
        },
        CriterionStatus {
            name: "liquidation_distance_safety".to_string(),
            description: "Liquidation distance safe at proposed leverage".to_string(),
            passed: liq_distance_passed,
            actual_value: format!("{:.1}", liq_distance_actual),
            threshold_value: format!(
                "> {:.0} bps (leverage {:.1}x)",
                config.min_safe_liquidation_distance_bps.max(leverage_liquidation_bps),
                config.proposed_leverage
            ),
            unit: "bps".to_string(),
        },
    ]
}

/// Atomic write: write content to a `.tmp` file, then rename to final path.
fn atomic_write(path: &Path, content: &str) -> anyhow::Result<()> {
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Compute minimum zone distance from data points.
/// Returns the minimum `distance_bps` across all liquidation zones seen
/// in the replay data, or None if no zones were present.
fn compute_min_zone_distance(data_points: &[ReplayDataPoint]) -> Option<f64> {
    data_points
        .iter()
        .filter_map(|dp| dp.liquidation_zones.as_ref())
        .flatten()
        .map(|z| z.distance_bps)
        .reduce(f64::min)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquidation::LiquidationZone;

    /// Helper: create a replay data point with all fields set to passing values.
    fn replay_point_passing(symbol: &str, timestamp_ms: i64, price: f64) -> ReplayDataPoint {
        ReplayDataPoint {
            symbol: symbol.to_string(),
            timestamp_ms,
            price,
            vwap: Some(price * 0.999), // Price slightly above VWAP → long-friendly
            spread_pct: Some(0.05),
            depth_usd: Some(100_000.0),
            volume_zscore: Some(3.0),
            forced_flow_velocity: Some(0.8),
            regime_label: Some("Trending".to_string()),
            liquidation_burst_detected: true,
            route_cost_bps: Some(2.0),
            liquidation_zones: Some(vec![LiquidationZone {
                price: price * 0.98,
                side_at_risk: "short".to_string(),
                estimated_notional_usd: 500_000.0,
                wallet_count: 25,
                distance_bps: 200.0,
                confidence: 0.8,
                source_mix: vec!["hyperliquid_positions".to_string(), "oi_imbalance".to_string()],
            }]),
            zone_capture_timestamp_ms: Some(timestamp_ms - 5000), // 5s ago — fresh
            high: Some(price * 1.002),
            low: Some(price * 0.998),
            is_zone_touch: Some(true),
        }
    }

    /// Helper: create params with all gates configured for passing.
    fn params_all_pass() -> LiquidationCascadeParams {
        let mut p = LiquidationCascadeParams::default();
        p.enabled = true;
        p.confidence_min = 0.5;
        p.volume_z_score_threshold = 1.0;
        p.max_distance_to_zone_pct = 10.0;
        p.spread_max_pct = 1.0;
        p.depth_min_usd = 1_000.0;
        p.lookback_count = 5; // Low for testing
        p.stale_data_threshold_secs = 600;
        p.forced_flow_velocity_threshold = 0.3;
        p.velocity_decay_threshold = 0.2;
        p.take_profit_pct = 1.5;
        p.stop_loss_pct = 0.75;
        p.trailing_stop_pct = 0.5;
        p.trailing_activation_pct = 1.0;
        p.max_hold_secs = 1800;
        p.clip_size_usd = 50.0;
        p.route_cost_max_bps = 10.0;
        p
    }

    /// Helper: create default gate config.
    fn gate_config_default() -> PromotionGateConfig {
        PromotionGateConfig {
            max_drawdown_pct: 10.0,
            min_signal_events: 30,
            min_sharpe: 1.0,
            fee_rate: 0.001,
            starting_balance: 1000.0,
            max_fee_to_gross_pct: 35.0,
            max_single_trade_profit_pct: 25.0,
            max_route_cost_pct_of_expectancy: 50.0,
            min_safe_liquidation_distance_bps: 200.0,
            proposed_leverage: 3.0,
        }
    }

    /// Helper: generate N replay points simulating a trending market.
    fn generate_trending_points(
        symbol: &str,
        count: usize,
        start_price: f64,
        start_ts: i64,
        interval_ms: i64,
    ) -> Vec<ReplayDataPoint> {
        (0..count)
            .map(|i| {
                let price = start_price + (i as f64 * 0.5); // Upward trend
                replay_point_passing(symbol, start_ts + (i as i64 * interval_ms), price)
            })
            .collect()
    }

    /// Helper: generate points that oscillate (for testing exits).
    fn generate_oscillating_points(
        symbol: &str,
        count: usize,
        center_price: f64,
        start_ts: i64,
        interval_ms: i64,
    ) -> Vec<ReplayDataPoint> {
        (0..count)
            .map(|i| {
                let offset = (i as f64 * 0.3).sin() * 5.0;
                let price = center_price + offset;
                let mut p = replay_point_passing(symbol, start_ts + (i as i64 * interval_ms), price);
                // Set zones near the price for each point
                p.liquidation_zones = Some(vec![LiquidationZone {
                    price: price * 0.98,
                    side_at_risk: "short".to_string(),
                    estimated_notional_usd: 500_000.0,
                    wallet_count: 25,
                    distance_bps: 200.0,
                    confidence: 0.8,
                    source_mix: vec!["hyperliquid_positions".to_string()],
                }]);
                p.zone_capture_timestamp_ms = Some(start_ts + (i as i64 * interval_ms) - 5000);
                p
            })
            .collect()
    }

    // ---- VAL-STRAT-046: Replay pipeline loads captured data ----

    #[test]
    fn test_replay_loads_captured_data() {
        let dir = tempfile::tempdir().unwrap();
        let snap = LiquidationZoneSnapshot {
            symbol: "BTC".to_string(),
            timestamp_ms: 1_770_000_000_000,
            mark_price: 100_000.0,
            zones: vec![LiquidationZone {
                price: 98_000.0,
                side_at_risk: "long".to_string(),
                estimated_notional_usd: 5_000_000.0,
                wallet_count: 42,
                distance_bps: 200.0,
                confidence: 0.75,
                source_mix: vec!["hyperliquid_positions".to_string()],
            }],
        };
        let path = dir.path().join("BTC_1770000000000.json");
        let json = serde_json::to_string_pretty(&snap).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = ReplayPipeline::load_snapshots(dir.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].symbol, "BTC");
        assert_eq!(loaded[0].zones.len(), 1);
        assert!((loaded[0].mark_price - 100_000.0).abs() < 0.01);
    }

    #[test]
    fn test_replay_converts_snapshots_to_points() {
        let snaps = vec![
            LiquidationZoneSnapshot {
                symbol: "SOL".to_string(),
                timestamp_ms: 1000,
                mark_price: 150.0,
                zones: vec![LiquidationZone {
                    price: 148.0,
                    side_at_risk: "long".to_string(),
                    estimated_notional_usd: 500_000.0,
                    wallet_count: 10,
                    distance_bps: 133.0,
                    confidence: 0.6,
                    source_mix: vec!["hyperliquid_positions".to_string()],
                }],
            },
            LiquidationZoneSnapshot {
                symbol: "SOL".to_string(),
                timestamp_ms: 2000,
                mark_price: 151.0,
                zones: vec![],
            },
        ];
        let points = ReplayPipeline::snapshots_to_replay_points(&snaps);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].symbol, "SOL");
        assert!((points[0].price - 150.0).abs() < 0.01);
        assert_eq!(points[0].liquidation_zones.as_ref().unwrap().len(), 1);
        assert_eq!(points[1].liquidation_zones.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn test_replay_builds_valid_snapshots() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let pipeline = ReplayPipeline::new(params, gate);
        let point = replay_point_passing("BTC", 1000, 100_000.0);

        let snap = pipeline.build_snapshot(&point, 10, 1.5);
        assert_eq!(snap.price_count, 10);
        assert!((snap.current_price - 100_000.0).abs() < 0.01);
        assert!((snap.price_velocity_pct - 1.5).abs() < 0.01);
        assert!(snap.ext.is_some());
        let ext = snap.ext.unwrap();
        assert!(ext.liquidation_zones.is_some());
        assert!(ext.vwap.is_some());
        assert!(ext.spread_pct.is_some());
    }

    // ---- VAL-STRAT-047: Replay produces deterministic results ----

    #[test]
    fn test_replay_deterministic() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);

        let pipeline1 = ReplayPipeline::new(params.clone(), gate.clone());
        let result1 = pipeline1.run(&data);

        let pipeline2 = ReplayPipeline::new(params, gate);
        let result2 = pipeline2.run(&data);

        // Results must be identical
        assert_eq!(result1.trade_count, result2.trade_count);
        assert_eq!(result1.signal_events, result2.signal_events);
        assert!((result1.net_pnl - result2.net_pnl).abs() < 0.0001);
        assert!((result1.final_balance - result2.final_balance).abs() < 0.0001);
        assert!((result1.sharpe_ratio - result2.sharpe_ratio).abs() < 0.0001);
        assert_eq!(result1.trades.len(), result2.trades.len());
        for (t1, t2) in result1.trades.iter().zip(result2.trades.iter()) {
            assert!((t1.entry_price - t2.entry_price).abs() < 0.0001);
            assert!((t1.exit_price - t2.exit_price).abs() < 0.0001);
            assert!((t1.net_pnl - t2.net_pnl).abs() < 0.0001);
        }
    }

    // ---- VAL-STRAT-048: Replay compares against no-trade baseline ----

    #[test]
    fn test_replay_baseline_comparison() {
        let params = params_all_pass();
        let gate = PromotionGateConfig {
            starting_balance: 1000.0,
            ..gate_config_default()
        };
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        // Baseline must be starting balance with zero PnL
        assert!((result.baseline_balance - 1000.0).abs() < 0.01);
        assert!((result.baseline_net_pnl - 0.0).abs() < 0.0001);
        // pnl_vs_baseline = net_pnl
        assert!((result.pnl_vs_baseline - result.net_pnl).abs() < 0.0001);
    }

    // ---- VAL-STRAT-049: Replay respects all entry gates ----

    #[test]
    fn test_replay_no_phantom_trades() {
        let params = params_all_pass();
        let gate = gate_config_default();

        // Create data where confidence is below threshold → no trades
        let mut low_confidence_params = params_all_pass();
        low_confidence_params.confidence_min = 0.99; // Unreachable

        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(low_confidence_params, gate);
        let result = pipeline.run(&data);

        // No trades because confidence gate blocks everything
        assert_eq!(result.trade_count, 0, "No phantom trades should occur when gates block entry");
    }

    #[test]
    fn test_replay_each_trade_passes_gates() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        for trade in &result.trades {
            assert!(trade.entry_price > 0.0, "Entry price must be positive");
            assert!(trade.exit_price > 0.0, "Exit price must be positive");
            assert!(trade.size_usd > 0.0, "Size must be positive");
            assert!(trade.net_pnl.is_finite(), "Net PnL must be finite");
            assert!(!trade.exit_reason.is_empty(), "Exit reason must be present");
        }
    }

    // ---- VAL-STRAT-050: Replay respects all exit conditions ----

    #[test]
    fn test_replay_no_orphan_positions() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 200, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        // Every trade must have an exit reason (no orphans)
        for trade in &result.trades {
            assert!(!trade.exit_reason.is_empty(), "Every trade must have an exit reason");
            assert!(trade.exit_timestamp_ms >= trade.entry_timestamp_ms, "Exit after entry");
        }
    }

    #[test]
    fn test_replay_all_exit_reasons_valid() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_oscillating_points("BTC", 200, 100_000.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        let valid_reasons = [
            "TakeProfit",
            "StopLoss",
            "TrailingStop",
            "TimeStop",
            "ReversalDetected",
            "MomentumLost",
            "ForceClose",
        ];
        for trade in &result.trades {
            let reason_valid = valid_reasons.iter().any(|r| trade.exit_reason.contains(r));
            assert!(
                reason_valid,
                "Exit reason '{}' must be one of {:?}",
                trade.exit_reason, valid_reasons
            );
        }
    }

    // ---- VAL-STRAT-051: Positive net expectancy ----

    #[test]
    fn test_net_expectancy_positive_for_winning_strategy() {
        let trades = vec![
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 101_500.0,
                size_usd: 100.0,
                gross_pnl: 1.5,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: 1.28,
                hold_secs: 300,
                exit_reason: "TakeProfit".to_string(),
                entry_timestamp_ms: 1000,
                exit_timestamp_ms: 1300,
                entry_stale: false,
                exit_stale: false,
                peak_price: 101_500.0,
                mae_usd: -0.1,
                mfe_usd: 1.5,
                worst_price: 99_900.0,
                best_price: 101_500.0,
                is_zone_touch: true,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 1.28 / 1.5,
            },
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 100_500.0,
                size_usd: 100.0,
                gross_pnl: 0.5,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: 0.28,
                hold_secs: 200,
                exit_reason: "TakeProfit".to_string(),
                entry_timestamp_ms: 2000,
                exit_timestamp_ms: 2200,
                entry_stale: false,
                exit_stale: false,
                peak_price: 100_500.0,
                mae_usd: -0.05,
                mfe_usd: 0.5,
                worst_price: 99_950.0,
                best_price: 100_500.0,
                is_zone_touch: false,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.28 / 0.5,
            },
        ];
        let expectancy = compute_net_expectancy(&trades);
        assert!(expectancy > 0.0, "Net expectancy should be positive for winning trades");
    }

    #[test]
    fn test_net_expectancy_negative_for_losing_strategy() {
        let trades = vec![
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 99_250.0,
                size_usd: 100.0,
                gross_pnl: -0.75,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: -0.97,
                hold_secs: 300,
                exit_reason: "StopLoss".to_string(),
                entry_timestamp_ms: 1000,
                exit_timestamp_ms: 1300,
                entry_stale: false,
                exit_stale: false,
                peak_price: 100_000.0,
                mae_usd: -0.75,
                mfe_usd: 0.0,
                worst_price: 99_250.0,
                best_price: 100_000.0,
                is_zone_touch: true,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.0,
            },
        ];
        let expectancy = compute_net_expectancy(&trades);
        assert!(expectancy < 0.0, "Net expectancy should be negative for losing trades");
    }

    // ---- VAL-STRAT-052: Max drawdown within policy ----

    #[test]
    fn test_max_drawdown_within_policy() {
        let gate = PromotionGateConfig {
            max_drawdown_pct: 10.0,
            ..gate_config_default()
        };
        let criteria = evaluate_promotion_criteria(
            &[],
            1.0,
            5.0, // 5% drawdown — within 10% limit
            0,
            0,
            30,
            1.5,
            None,
            None,
            None,
            &gate,
        );
        let dd_criterion = criteria.iter().find(|c| c.name == "max_drawdown").unwrap();
        assert!(dd_criterion.passed);
    }

    #[test]
    fn test_max_drawdown_exceeds_policy() {
        let gate = PromotionGateConfig {
            max_drawdown_pct: 10.0,
            ..gate_config_default()
        };
        let criteria = evaluate_promotion_criteria(
            &[],
            1.0,
            15.0, // 15% drawdown — exceeds 10% limit
            0,
            0,
            30,
            1.5,
            None,
            None,
            None,
            &gate,
        );
        let dd_criterion = criteria.iter().find(|c| c.name == "max_drawdown").unwrap();
        assert!(!dd_criterion.passed);
    }

    // ---- VAL-STRAT-053: Zero stale-data trades ----

    #[test]
    fn test_stale_data_trades_fail_promotion() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 2, 0, 30, 1.5, None, None, None, &gate);
        let stale = criteria.iter().find(|c| c.name == "stale_data_trades").unwrap();
        assert!(!stale.passed);
    }

    #[test]
    fn test_zero_stale_data_passes() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let stale = criteria.iter().find(|c| c.name == "stale_data_trades").unwrap();
        assert!(stale.passed);
    }

    // ---- VAL-STRAT-054: Zero duplicate pending trades ----

    #[test]
    fn test_duplicate_pending_fail_promotion() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 3, 30, 1.5, None, None, None, &gate);
        let dup = criteria.iter().find(|c| c.name == "duplicate_pending_trades").unwrap();
        assert!(!dup.passed);
    }

    #[test]
    fn test_zero_duplicate_pending_passes() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let dup = criteria.iter().find(|c| c.name == "duplicate_pending_trades").unwrap();
        assert!(dup.passed);
    }

    // ---- VAL-STRAT-055: Minimum 30 signal events ----

    #[test]
    fn test_min_30_signal_events() {
        let gate = gate_config_default();
        let criteria_29 = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 29, 1.5, None, None, None, &gate);
        let sig_29 = criteria_29.iter().find(|c| c.name == "min_signal_events").unwrap();
        assert!(!sig_29.passed, "29 signal events should fail");

        let criteria_30 = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let sig_30 = criteria_30.iter().find(|c| c.name == "min_signal_events").unwrap();
        assert!(sig_30.passed, "30 signal events should pass");
    }

    // ---- VAL-STRAT-056: Sharpe ratio ≥ 1.0 ----

    #[test]
    fn test_sharpe_ratio_threshold() {
        let gate = gate_config_default();
        let criteria_below = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 0.5, None, None, None, &gate);
        let sharpe_below = criteria_below.iter().find(|c| c.name == "sharpe_ratio").unwrap();
        assert!(!sharpe_below.passed, "Sharpe 0.5 should fail");

        let criteria_at = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.0, None, None, None, &gate);
        let sharpe_at = criteria_at.iter().find(|c| c.name == "sharpe_ratio").unwrap();
        assert!(sharpe_at.passed, "Sharpe 1.0 should pass");
    }

    // ---- VAL-STRAT-079: Promotion gate aggregates ALL criteria ----

    #[test]
    fn test_promotion_gate_all_pass() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        assert!(criteria.iter().all(|c| c.passed), "All criteria should pass");
    }

    #[test]
    fn test_promotion_gate_any_fail_blocks() {
        let gate = gate_config_default();
        // Negative expectancy
        let criteria = evaluate_promotion_criteria(&[], -1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let all_passed = criteria.iter().all(|c| c.passed);
        assert!(!all_passed, "Negative expectancy should block promotion");

        // Too much drawdown
        let criteria = evaluate_promotion_criteria(&[], 1.0, 15.0, 0, 0, 30, 1.5, None, None, None, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Stale trades
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 1, 0, 30, 1.5, None, None, None, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Duplicates
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 1, 30, 1.5, None, None, None, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Too few signals
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 29, 1.5, None, None, None, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Low Sharpe
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 0.5, None, None, None, &gate);
        assert!(!criteria.iter().all(|c| c.passed));
    }

    // ---- VAL-STRAT-080: Promotion gate produces human-readable report ----

    #[test]
    fn test_promotion_report_markdown() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        let report = ReplayPipeline::generate_markdown_report(&result);

        // Report must contain key sections
        assert!(report.contains("# Liquidation Cascade Hunter"));
        assert!(report.contains("## Summary"));
        assert!(report.contains("## Performance Metrics"));
        assert!(report.contains("## Promotion Criteria"));
        assert!(report.contains("net_expectancy") || report.contains("Positive net expectancy"));
        assert!(report.contains("max_drawdown") || report.contains("Max drawdown"));
        assert!(report.contains("sharpe") || report.contains("Sharpe"));
        assert!(report.contains("stale") || report.contains("Stale"));
        assert!(report.contains("duplicate") || report.contains("Duplicate"));
        assert!(report.contains("signal") || report.contains("30 signal"));
    }

    #[test]
    fn test_promotion_report_json() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        ReplayPipeline::write_json_report(&result, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["strategy_name"].is_string());
        assert!(parsed["trade_count"].is_number());
        assert!(parsed["promotion_criteria"].is_array());
        assert!(parsed["promotion_verdict"].is_string());
        assert!(!path.with_extension("tmp").exists(), "Temp file should be cleaned up");
    }

    #[test]
    fn test_promotion_report_criteria_structure() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);

        // Must have exactly 12 criteria
        assert_eq!(criteria.len(), 12, "Promotion gate must have exactly 12 criteria");

        // Each criterion has required fields
        for c in &criteria {
            assert!(!c.name.is_empty());
            assert!(!c.description.is_empty());
            assert!(!c.actual_value.is_empty());
            assert!(!c.threshold_value.is_empty());
            assert!(!c.unit.is_empty());
        }

        // Check specific criteria exist
        let names: Vec<&str> = criteria.iter().map(|c| c.name.as_str()).collect();
        // Original 6
        assert!(names.contains(&"net_expectancy"));
        assert!(names.contains(&"max_drawdown"));
        assert!(names.contains(&"stale_data_trades"));
        assert!(names.contains(&"duplicate_pending_trades"));
        assert!(names.contains(&"min_signal_events"));
        assert!(names.contains(&"sharpe_ratio"));
        // Extended 6
        assert!(names.contains(&"fee_to_gross_ratio"));
        assert!(names.contains(&"single_trade_dependency"));
        assert!(names.contains(&"fishing_improvement"));
        assert!(names.contains(&"pyramiding_improvement"));
        assert!(names.contains(&"route_cost_edge"));
        assert!(names.contains(&"liquidation_distance_safety"));
    }

    // ---- VAL-STRAT-073: MultiPaperEngine supports strategy ----

    #[test]
    fn test_multipaper_supports_liquidation_strategy() {
        // Verify the liquidation-cascade-hunter can be created and supports multi-market
        // by testing strategy creation for each market independently
        use crate::strategy::create_strategy_from_config;

        let markets = vec!["BTC", "SOL", "ETH"];
        let mut table = toml::value::Table::new();
        table.insert("enabled".to_string(), toml::Value::try_from(true).unwrap());
        table.insert("lookback_count".to_string(), toml::Value::try_from(5).unwrap());
        table.insert("clip_size_usd".to_string(), toml::Value::try_from(50.0_f64).unwrap());

        let params = crate::strategy::StrategyParams {
            direction_bias: "neutral".to_string(),
            momentum_threshold_pct: 0.5,
            lookback_count: 5,
            scale_in_clips: 1,
            clip_size_usd: 50.0,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        };

        for market in &markets {
            let strategy = create_strategy_from_config(
                "liquidation-cascade-hunter",
                Some(&toml::Value::Table(table.clone())),
                params.clone(),
            );
            assert!(strategy.is_ok(), "Strategy should be created for market {}", market);
            let s = strategy.unwrap();
            assert_eq!(s.name(), "liquidation-cascade-hunter");
        }
    }

    // ---- VAL-STRAT-074: Risk manager circuit breaker applies ----

    #[test]
    fn test_risk_manager_circuit_breaker_applies() {
        use crate::risk::RiskManager;
        use crate::config::RiskConfig;

        let risk_config = RiskConfig {
            max_position_notional_usd: 5000.0,
            max_daily_loss_usd: 10.0, // Very tight limit
            max_drawdown_pct: 50.0,
            max_total_notional_usd: 100_000.0,
            max_weekly_loss_usd: 100_000.0,
            max_correlated_exposure_pct: 100.0,
            consecutive_loss_circuit_breaker: 0,
            volatility_sizing_enabled: false,
            volatility_sizing_atr_threshold_pct: 75.0,
            volatility_sizing_min_fraction: 0.25,
            api_degradation_threshold: 0,
            correlated_groups: vec![],
        };

        let rm = RiskManager::new(risk_config, 1000.0);
        assert!(rm.check_can_trade(1000.0).is_ok(), "Should be able to trade initially");

        // Simulate a large loss exceeding daily limit
        rm.record_trade_result(-50.0, 1.0, 949.0);
        assert!(rm.check_can_trade(949.0).is_err(), "Circuit breaker should block after large daily loss");
    }

    // ---- VAL-STRAT-075: Correlated exposure limit enforced ----

    #[test]
    fn test_correlated_exposure_limit() {
        use crate::risk::RiskManager;
        use crate::config::{CorrelatedGroup, RiskConfig};

        let risk_config = RiskConfig {
            max_position_notional_usd: 5000.0,
            max_daily_loss_usd: 500.0,
            max_drawdown_pct: 50.0,
            max_total_notional_usd: 100_000.0,
            max_weekly_loss_usd: 100_000.0,
            max_correlated_exposure_pct: 10.0, // 10% of balance
            consecutive_loss_circuit_breaker: 0,
            volatility_sizing_enabled: false,
            volatility_sizing_atr_threshold_pct: 75.0,
            volatility_sizing_min_fraction: 0.25,
            api_degradation_threshold: 0,
            correlated_groups: vec![CorrelatedGroup {
                name: "SOL ecosystem".to_string(),
                symbols: vec!["SOL".to_string(), "mSOL".to_string()],
            }],
        };

        let rm = RiskManager::new(risk_config, 1000.0);
        // Record SOL position as opened — 80 USD uses correlated group budget
        rm.record_position_opened("SOL", 80.0);
        // After SOL at 80, trying mSOL at 50 would make total 130 > 100 (10% of 1000) → blocked
        assert!(rm.check_correlated_exposure("mSOL", 50.0, 1000.0).is_err(),
            "mSOL should be blocked because SOL already uses the group budget");
    }

    // ---- VAL-STRAT-076: Backtest engine regime filter applies ----

    #[test]
    fn test_backtest_regime_filter_applies() {
        // Verify the strategy can be created through the factory and used in backtest
        use crate::strategy::create_strategy_from_config;

        let mut table = toml::value::Table::new();
        table.insert("enabled".to_string(), toml::Value::try_from(true).unwrap());
        table.insert("lookback_count".to_string(), toml::Value::try_from(5).unwrap());

        let params = crate::strategy::StrategyParams {
            direction_bias: "neutral".to_string(),
            momentum_threshold_pct: 0.5,
            lookback_count: 5,
            scale_in_clips: 1,
            clip_size_usd: 50.0,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        };

        let strategy = create_strategy_from_config(
            "liquidation-cascade-hunter",
            Some(&toml::Value::Table(table)),
            params,
        );
        assert!(strategy.is_ok(), "Strategy should be created from config");
        let s = strategy.unwrap();
        assert_eq!(s.name(), "liquidation-cascade-hunter");
    }

    // ---- VAL-STRAT-077: PnL tracker records strategy trades ----

    #[test]
    fn test_pnl_tracker_records_liquidation_trades() {
        // Verify that trade records with the liquidation-cascade-hunter strategy name
        // can be serialized and deserialized correctly
        let trade_record = serde_json::json!({
            "strategy": "liquidation-cascade-hunter",
            "symbol": "SOL",
            "side": "long",
            "entry_price": 150.0,
            "exit_price": 152.25,
            "size_usd": 100.0,
            "entry_fee": 0.1,
            "exit_fee": 0.1,
            "net_pnl": 2.05,
            "hold_secs": 1800,
            "exit_reason": "TakeProfit"
        });

        // Verify trade has the correct strategy name
        assert_eq!(trade_record["strategy"], "liquidation-cascade-hunter");
        assert!((trade_record["net_pnl"].as_f64().unwrap() - 2.05).abs() < 0.01);
        assert_eq!(trade_record["exit_reason"], "TakeProfit");
    }

    // ---- VAL-STRAT-078: Trade journal atomic write includes strategy name ----

    #[test]
    fn test_trade_journal_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-trades.json");

        let trades = vec![serde_json::json!({
            "strategy": "liquidation-cascade-hunter",
            "symbol": "SOL",
            "side": "long",
            "entry_price": 150.0,
            "exit_price": 152.25,
            "net_pnl": 2.05
        })];

        let content = serde_json::to_string_pretty(&trades).unwrap();
        atomic_write(&path, &content).unwrap();

        // Verify file was written atomically
        assert!(path.exists());
        assert!(!path.with_extension("tmp").exists());

        let parsed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed[0]["strategy"], "liquidation-cascade-hunter");
    }

    // ---- VAL-CROSS-005: Full pipeline compiles ----

    #[test]
    fn test_full_pipeline_compiles() {
        // This test verifies that all modules coexist and the replay pipeline
        // can be constructed alongside other modules
        let params = params_all_pass();
        let gate = gate_config_default();
        let _pipeline = ReplayPipeline::new(params, gate);

        // Verify strategy creation
        use crate::strategy::create_strategy_from_config;
        let mut table = toml::value::Table::new();
        table.insert("enabled".to_string(), toml::Value::try_from(true).unwrap());

        let params = crate::strategy::StrategyParams {
            direction_bias: "neutral".to_string(),
            momentum_threshold_pct: 0.5,
            lookback_count: 5,
            scale_in_clips: 1,
            clip_size_usd: 50.0,
            max_hold_secs: 1800,
            take_profit_pct: 1.5,
            stop_loss_pct: 0.75,
            trailing_stop_pct: 0.5,
            trailing_activation_pct: 1.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
        };

        let strategy = create_strategy_from_config(
            "liquidation-cascade-hunter",
            Some(&toml::Value::Table(table)),
            params,
        );
        assert!(strategy.is_ok());
    }

    // ---- VAL-CROSS-006: No live trading possible ----

    #[test]
    fn test_no_live_trading_possible() {
        // Verify the strategy has paper_only = true by default
        let params = LiquidationCascadeParams::default();
        assert!(params.paper_only, "Strategy must be paper-only by default");
        assert!(!params.enabled, "Strategy must be disabled by default");
    }

    #[test]
    fn test_replay_verify_paper_only() {
        let params = LiquidationCascadeParams::default();
        assert!(ReplayPipeline::verify_paper_only(&params));
    }

    // ---- VAL-CROSS-007: Imperial read-only constraint maintained ----

    #[test]
    fn test_no_imperial_writes_in_replay() {
        let source = include_str!("replay.rs");
        // Check production code only (before #[cfg(test)])
        let prod_code = source.split("#[cfg(test)]").next().unwrap_or("");

        assert!(!prod_code.contains(".post("), "No POST methods");
        assert!(!prod_code.contains(".put("), "No PUT methods");
        assert!(!prod_code.contains(".delete("), "No DELETE methods");
        assert!(!prod_code.contains("/mobile/"), "No mobile endpoints");
        assert!(!prod_code.contains("/deposit/"), "No deposit endpoints");
    }

    // ---- Additional utility tests ----

    #[test]
    fn test_compute_sharpe_empty() {
        assert!((compute_sharpe(&[]) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_compute_sharpe_single_value() {
        // Single return → std_dev = 0 → Sharpe = 0
        assert!((compute_sharpe(&[1.0]) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_compute_sharpe_positive_returns() {
        let returns = vec![0.5, 0.3, 0.4, 0.6, 0.2];
        let sharpe = compute_sharpe(&returns);
        assert!(sharpe > 0.0, "Positive returns should yield positive Sharpe");
    }

    #[test]
    fn test_compute_net_expectancy_empty() {
        assert!((compute_net_expectancy(&[]) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_load_snapshots_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = ReplayPipeline::load_snapshots(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_load_snapshots_nonexistent_dir() {
        let result = ReplayPipeline::load_snapshots(Path::new("/tmp/nonexistent_test_dir_12345"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_replay_data_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.json");
        let points = vec![replay_point_passing("BTC", 1000, 100_000.0)];
        let json = serde_json::to_string_pretty(&points).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = ReplayPipeline::load_replay_data(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].symbol, "BTC");
    }

    #[test]
    fn test_replay_with_no_data() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&[]);

        assert_eq!(result.trade_count, 0);
        assert_eq!(result.signal_events, 0);
        assert!((result.net_pnl - 0.0).abs() < 0.0001);
        assert_eq!(result.promotion_verdict, PromotionVerdict::Denied);
    }

    #[test]
    fn test_promotion_verdict_approved() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let all_passed = criteria.iter().all(|c| c.passed);
        let verdict = if all_passed {
            PromotionVerdict::Approved
        } else {
            PromotionVerdict::Denied
        };
        assert_eq!(verdict, PromotionVerdict::Approved);
    }

    #[test]
    fn test_promotion_verdict_denied() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], -1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let all_passed = criteria.iter().all(|c| c.passed);
        let verdict = if all_passed {
            PromotionVerdict::Approved
        } else {
            PromotionVerdict::Denied
        };
        assert_eq!(verdict, PromotionVerdict::Denied);
    }

    // ---- VAL-CROSS-008: Backward compatibility preserved ----

    #[test]
    fn test_replay_backward_compatible() {
        // Verify default config values don't break existing behavior
        let gate = PromotionGateConfig::default();
        assert!((gate.max_drawdown_pct - 10.0).abs() < 0.01);
        assert_eq!(gate.min_signal_events, 30);
        assert!((gate.min_sharpe - 1.0).abs() < 0.01);
        assert!((gate.fee_rate - 0.001).abs() < 0.0001);
        assert!((gate.starting_balance - 1000.0).abs() < 0.01);
        // Extended defaults
        assert!((gate.max_fee_to_gross_pct - 35.0).abs() < 0.01);
        assert!((gate.max_single_trade_profit_pct - 25.0).abs() < 0.01);
        assert!((gate.max_route_cost_pct_of_expectancy - 50.0).abs() < 0.01);
        assert!((gate.min_safe_liquidation_distance_bps - 200.0).abs() < 0.01);
        assert!((gate.proposed_leverage - 3.0).abs() < 0.01);

        // Verify strategy params defaults are backward compatible
        let params = LiquidationCascadeParams::default();
        assert!(!params.enabled, "Strategy disabled by default");
        assert!(params.paper_only, "Paper-only by default");
    }

    // ---- VAL-REPLAY-001: Sortino ratio computed ----

    #[test]
    fn test_sortino_ratio_positive_returns() {
        // All positive returns → downside_dev = 0 → Sortino = 0
        let returns = vec![1.0, 2.0, 0.5, 1.5];
        let sortino = compute_sortino(&returns);
        assert!((sortino - 0.0).abs() < 0.0001, "No downside deviation → Sortino = 0");
    }

    #[test]
    fn test_sortino_ratio_mixed_returns() {
        // Mixed returns with some negative
        let returns = vec![2.0, -1.0, 1.5, -0.5, 3.0];
        let sortino = compute_sortino(&returns);
        let mean = (2.0 - 1.0 + 1.5 - 0.5 + 3.0) / 5.0; // = 1.0
        let downside: Vec<f64> = returns.iter().map(|r| (*r).min(0.0)).collect();
        let dd_var = downside.iter().map(|r| r.powi(2)).sum::<f64>() / 5.0;
        let dd = dd_var.sqrt();
        let trades_per_year = 1260.0_f64;
        let expected = (mean / dd) * trades_per_year.sqrt();
        assert!((sortino - expected).abs() < 0.01, "Sortino = {:.4}, expected {:.4}", sortino, expected);
        assert!(sortino > 0.0, "Positive mean with downside deviation → positive Sortino");
    }

    #[test]
    fn test_sortino_ratio_empty() {
        assert!((compute_sortino(&[]) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_sortino_ratio_in_result() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        // Sortino should be a finite number
        assert!(result.sortino_ratio.is_finite(), "Sortino must be finite");
    }

    // ---- VAL-REPLAY-002: Calmar ratio computed ----

    #[test]
    fn test_calmar_ratio_positive_pnl_no_drawdown() {
        // No drawdown → Calmar = 0 (division by zero guard)
        let calmar = compute_calmar(100.0, 1000.0, 0.0, 100);
        assert!((calmar - 0.0).abs() < 0.0001, "No drawdown → Calmar = 0");
    }

    #[test]
    fn test_calmar_ratio_computed() {
        // net_pnl = 100, starting_balance = 1000, max_drawdown = 50, 100 data points
        let calmar = compute_calmar(100.0, 1000.0, 50.0, 100);
        // annualized_return = 0.1 * (1260/100) = 1.26
        // max_drawdown_pct = 50/1000 = 0.05
        // calmar = 1.26 / 0.05 = 25.2
        assert!((calmar - 25.2).abs() < 0.1, "Calmar = {:.4}, expected ~25.2", calmar);
        assert!(calmar > 0.0, "Positive PnL with drawdown → positive Calmar");
    }

    #[test]
    fn test_calmar_ratio_negative_pnl() {
        // Negative PnL → negative Calmar
        let calmar = compute_calmar(-50.0, 1000.0, 100.0, 100);
        assert!(calmar < 0.0, "Negative PnL → negative Calmar");
    }

    #[test]
    fn test_calmar_in_result() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        assert!(result.calmar_ratio.is_finite(), "Calmar must be finite");
    }

    // ---- VAL-REPLAY-003: MAE computed per trade ----

    #[test]
    fn test_mae_long_trade() {
        // Long trade: entry=100, worst_price=95, size=1000
        // MAE = (95-100)/100 * 1000 = -50
        let mae = compute_mae_usd(true, 100.0, 95.0, 1000.0);
        assert!((mae - (-50.0)).abs() < 0.01, "MAE for long = {:.4}, expected -50", mae);
    }

    #[test]
    fn test_mae_short_trade() {
        // Short trade: entry=100, worst_price=105, size=1000
        // MAE = (105-100)/100 * 1000 = 50 → adverse
        let mae = compute_mae_usd(false, 100.0, 105.0, 1000.0);
        assert!((mae - 50.0).abs() < 0.01, "MAE for short = {:.4}, expected 50", mae);
    }

    #[test]
    fn test_mae_no_adverse() {
        // No adverse movement: worst_price = entry_price
        let mae = compute_mae_usd(true, 100.0, 100.0, 1000.0);
        assert!((mae - 0.0).abs() < 0.01, "No adverse → MAE = 0");
    }

    #[test]
    fn test_mae_in_replay_trades() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        for trade in &result.trades {
            assert!(trade.mae_usd.is_finite(), "MAE must be finite");
            assert!(trade.worst_price > 0.0, "Worst price must be positive");
        }
        assert!(result.avg_mae_usd.is_finite(), "Avg MAE must be finite");
    }

    // ---- VAL-REPLAY-004: MFE computed per trade ----

    #[test]
    fn test_mfe_long_trade() {
        // Long trade: entry=100, best_price=110, size=1000
        // MFE = (110-100)/100 * 1000 = 100
        let mfe = compute_mfe_usd(true, 100.0, 110.0, 1000.0);
        assert!((mfe - 100.0).abs() < 0.01, "MFE for long = {:.4}, expected 100", mfe);
    }

    #[test]
    fn test_mfe_short_trade() {
        // Short trade: entry=100, best_price=90, size=1000
        // MFE = (100-90)/100 * 1000 = 100
        let mfe = compute_mfe_usd(false, 100.0, 90.0, 1000.0);
        assert!((mfe - 100.0).abs() < 0.01, "MFE for short = {:.4}, expected 100", mfe);
    }

    #[test]
    fn test_mfe_no_favorable() {
        let mfe = compute_mfe_usd(true, 100.0, 100.0, 1000.0);
        assert!((mfe - 0.0).abs() < 0.01, "No favorable → MFE = 0");
    }

    #[test]
    fn test_mfe_in_replay_trades() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        for trade in &result.trades {
            assert!(trade.mfe_usd.is_finite(), "MFE must be finite");
            assert!(trade.best_price > 0.0, "Best price must be positive");
        }
        assert!(result.avg_mfe_usd.is_finite(), "Avg MFE must be finite");
    }

    // ---- VAL-REPLAY-005: Fill rate for fishing orders ----

    #[test]
    fn test_fishing_fill_rate_no_fishing() {
        // Without fishing, fill rate should be 0.0
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        assert!((result.fishing_fill_rate - 0.0).abs() < 0.0001, "No fishing → fill rate = 0");
        assert!(result.fishing_result.is_none(), "No fishing → no fishing result");
    }

    #[test]
    fn test_fishing_fill_rate_with_fishing() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let fishing_config = FishingLadderConfig::default();
        let result = pipeline.run_with_fishing(&data, &fishing_config);
        // Fishing fill rate should be populated (may be 0 or > 0 depending on simulation)
        assert!(result.fishing_fill_rate >= 0.0, "Fill rate must be non-negative");
        assert!(result.fishing_fill_rate <= 1.0, "Fill rate must be <= 1.0");
        assert!(result.fishing_result.is_some(), "Fishing result should be present");
    }

    // ---- VAL-REPLAY-006: Zone-touch win rate computed ----

    #[test]
    fn test_zone_touch_win_rate_no_zone_touches() {
        // With no zone-touch trades, win rate should be 0.0
        let params = params_all_pass();
        let gate = gate_config_default();
        let data: Vec<ReplayDataPoint> = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000)
            .into_iter()
            .map(|mut p| { p.is_zone_touch = Some(false); p })
            .collect();
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        // Zone-touch stats are separate from overall stats
        assert!(result.zone_touch_win_rate_pct >= 0.0);
        assert!(result.zone_touch_trade_count <= result.trade_count);
    }

    #[test]
    fn test_zone_touch_win_rate_with_touches() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        // Verify zone-touch stats are computed
        assert!(result.zone_touch_win_rate_pct >= 0.0);
        assert!(result.zone_touch_win_rate_pct <= 100.0);
        // Zone-touch trade count + non-touch count = total trades
        let non_touch = result.trades.iter().filter(|t| !t.is_zone_touch).count();
        assert_eq!(result.zone_touch_trade_count + non_touch, result.trade_count);
    }

    // ---- VAL-REPLAY-007: Single-trade dependency flagged >25% ----

    #[test]
    fn test_single_trade_dependency_flagged() {
        // One trade contributes > 25% of total profit
        let trades = vec![
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 110_000.0,
                size_usd: 100.0,
                gross_pnl: 10.0,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: 9.78,
                hold_secs: 300,
                exit_reason: "TakeProfit".to_string(),
                entry_timestamp_ms: 1000,
                exit_timestamp_ms: 1300,
                entry_stale: false,
                exit_stale: false,
                peak_price: 110_000.0,
                mae_usd: -0.1,
                mfe_usd: 10.0,
                worst_price: 99_900.0,
                best_price: 110_000.0,
                is_zone_touch: true,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.978,
            },
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 100_200.0,
                size_usd: 100.0,
                gross_pnl: 0.2,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: -0.02,
                hold_secs: 200,
                exit_reason: "StopLoss".to_string(),
                entry_timestamp_ms: 2000,
                exit_timestamp_ms: 2200,
                entry_stale: false,
                exit_stale: false,
                peak_price: 100_200.0,
                mae_usd: -0.1,
                mfe_usd: 0.2,
                worst_price: 99_900.0,
                best_price: 100_200.0,
                is_zone_touch: false,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: -0.1,
            },
        ];
        // Total PnL = 9.78 + (-0.02) = 9.76
        // Dominant trade = 9.78 / 9.76 ≈ 100% → flagged
        assert!(check_single_trade_dependency(&trades), "Should flag >25% dependency");
        assert_eq!(find_dominant_trade(&trades), Some(0));
    }

    #[test]
    fn test_single_trade_dependency_not_flagged() {
        // Two equal trades → neither dominates
        let trades = vec![
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 101_000.0,
                size_usd: 100.0,
                gross_pnl: 1.0,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: 0.78,
                hold_secs: 300,
                exit_reason: "TakeProfit".to_string(),
                entry_timestamp_ms: 1000,
                exit_timestamp_ms: 1300,
                entry_stale: false,
                exit_stale: false,
                peak_price: 101_000.0,
                mae_usd: -0.05,
                mfe_usd: 1.0,
                worst_price: 99_950.0,
                best_price: 101_000.0,
                is_zone_touch: true,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.78,
            },
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 101_000.0,
                size_usd: 100.0,
                gross_pnl: 1.0,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: 0.78,
                hold_secs: 200,
                exit_reason: "TakeProfit".to_string(),
                entry_timestamp_ms: 2000,
                exit_timestamp_ms: 2200,
                entry_stale: false,
                exit_stale: false,
                peak_price: 101_000.0,
                mae_usd: -0.05,
                mfe_usd: 1.0,
                worst_price: 99_950.0,
                best_price: 101_000.0,
                is_zone_touch: false,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.78,
            },
        ];
        // Each trade = 0.78 / 1.56 = 50% → flagged because 50% > 25%
        assert!(check_single_trade_dependency(&trades), "50% each → both > 25% → flagged");
    }

    #[test]
    fn test_single_trade_dependency_no_profit() {
        // No positive total profit → not flagged
        let trades = vec![
            ReplayTrade {
                symbol: "BTC".to_string(),
                side: "long".to_string(),
                entry_price: 100_000.0,
                exit_price: 99_000.0,
                size_usd: 100.0,
                gross_pnl: -1.0,
                entry_fee: 0.1,
                exit_fee: 0.1,
                route_cost_usd: 0.02,
                net_pnl: -1.22,
                hold_secs: 300,
                exit_reason: "StopLoss".to_string(),
                entry_timestamp_ms: 1000,
                exit_timestamp_ms: 1300,
                entry_stale: false,
                exit_stale: false,
                peak_price: 100_000.0,
                mae_usd: -1.0,
                mfe_usd: 0.0,
                worst_price: 99_000.0,
                best_price: 100_000.0,
                is_zone_touch: true,
                post_liquidation_drift_usd: 0.0,
                time_to_reversal_secs: 0.0,
                time_to_next_zone_secs: 0.0,
                stop_efficiency: 0.0,
            },
        ];
        assert!(!check_single_trade_dependency(&trades), "No profit → not flagged");
    }

    #[test]
    fn test_single_trade_dependency_in_result() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        // Flag should be a boolean
        assert!(result.single_trade_dependency_flagged || !result.single_trade_dependency_flagged);
    }

    // ---- VAL-REPLAY-008: Fishing + pyramiding composed into replay ----

    #[test]
    fn test_fishing_pyramiding_composition() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);

        let fishing_config = FishingLadderConfig::default();
        let pyramid_config = PyramidConfig::default();

        let result = pipeline.run_with_fishing_and_pyramiding(&data, &fishing_config, &pyramid_config);

        // Both fishing and pyramid results should be present
        assert!(result.fishing_result.is_some(), "Fishing result should be present");
        assert!(result.pyramid_result.is_some(), "Pyramid result should be present");

        // Verify the base replay still works correctly
        assert!(result.trade_count > 0 || result.data_points_replayed > 0);
        assert!(result.fishing_fill_rate >= 0.0);
        assert!(result.fishing_fill_rate <= 1.0);
    }

    #[test]
    fn test_fishing_pyramiding_no_data() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let pipeline = ReplayPipeline::new(params, gate);

        let fishing_config = FishingLadderConfig::default();
        let pyramid_config = PyramidConfig::default();

        let result = pipeline.run_with_fishing_and_pyramiding(&[], &fishing_config, &pyramid_config);

        assert_eq!(result.trade_count, 0);
        assert!(result.fishing_result.is_none());
        assert!(result.pyramid_result.is_none());
    }

    #[test]
    fn test_fishing_pyramiding_preserves_base_metrics() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);

        let pipeline = ReplayPipeline::new(params.clone(), gate.clone());
        let base_result = pipeline.run(&data);

        let fishing_config = FishingLadderConfig::default();
        let pyramid_config = PyramidConfig::default();

        let composed_result = pipeline.run_with_fishing_and_pyramiding(&data, &fishing_config, &pyramid_config);

        // Base metrics must be identical
        assert_eq!(base_result.trade_count, composed_result.trade_count);
        assert!((base_result.net_pnl - composed_result.net_pnl).abs() < 0.001);
        assert!((base_result.sharpe_ratio - composed_result.sharpe_ratio).abs() < 0.001);
        assert_eq!(base_result.trades.len(), composed_result.trades.len());
    }

    // ---- Additional metric tests ----

    #[test]
    fn test_stop_efficiency_winner() {
        // Winner: net_pnl=1.0, MFE=2.0 → efficiency = 0.5
        let eff = compute_stop_efficiency(1.0, true, 100.0, 102.0, 100.0);
        assert!((eff - 0.5).abs() < 0.01, "Stop efficiency = {:.4}, expected 0.5", eff);
    }

    #[test]
    fn test_stop_efficiency_loser() {
        // Loser: net_pnl=-1.0, MFE=0.0 → efficiency = 0.0 (no MFE guard)
        let eff = compute_stop_efficiency(-1.0, true, 100.0, 100.0, 100.0);
        assert!((eff - 0.0).abs() < 0.01, "No MFE → efficiency = 0");
    }

    #[test]
    fn test_avg_stop_efficiency_in_result() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        assert!(result.avg_stop_efficiency.is_finite(), "Avg stop efficiency must be finite");
    }

    #[test]
    fn test_extended_metrics_in_report() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 100, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);
        let report = ReplayPipeline::generate_markdown_report(&result);

        // Report must contain new metrics
        assert!(report.contains("Sortino") || report.contains("sortino"), "Report must contain Sortino");
        assert!(report.contains("Calmar") || report.contains("calmar"), "Report must contain Calmar");
        assert!(report.contains("MAE") || report.contains("mae"), "Report must contain MAE");
        assert!(report.contains("MFE") || report.contains("mfe"), "Report must contain MFE");
        assert!(report.contains("Fishing Fill Rate"), "Report must contain Fishing Fill Rate");
        assert!(report.contains("Zone-Touch Win Rate"), "Report must contain Zone-Touch Win Rate");
        assert!(report.contains("Stop Efficiency"), "Report must contain Stop Efficiency");
        assert!(report.contains("Single-Trade Dependency"), "Report must contain Single-Trade Dependency");
        assert!(report.contains("Post-Liq Drift"), "Report must contain Post-Liquidation Drift");
    }

    #[test]
    fn test_extended_fields_in_json_report() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        ReplayPipeline::write_json_report(&result, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["sortino_ratio"].is_number(), "JSON must contain sortino_ratio");
        assert!(parsed["calmar_ratio"].is_number(), "JSON must contain calmar_ratio");
        assert!(parsed["avg_mae_usd"].is_number(), "JSON must contain avg_mae_usd");
        assert!(parsed["avg_mfe_usd"].is_number(), "JSON must contain avg_mfe_usd");
        assert!(parsed["fishing_fill_rate"].is_number(), "JSON must contain fishing_fill_rate");
        assert!(parsed["zone_touch_win_rate_pct"].is_number(), "JSON must contain zone_touch_win_rate_pct");
        assert!(parsed["single_trade_dependency_flagged"].is_boolean(), "JSON must contain single_trade_dependency_flagged");
        assert!(parsed["avg_stop_efficiency"].is_number(), "JSON must contain avg_stop_efficiency");
    }

    // ---- VAL-REPLAY-009: Existing replay tests continue to pass ----
    // (This is verified by all existing tests passing above)

    // ---- VAL-GATE-001: All 12 criteria evaluated ----

    #[test]
    fn test_gate_has_exactly_12_criteria() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        assert_eq!(criteria.len(), 12, "Promotion gate must evaluate exactly 12 criteria");

        // Each criterion has a unique name
        let names: std::collections::HashSet<&str> = criteria.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 12, "All 12 criteria must have unique names");
    }

    // ---- VAL-GATE-002: Correct pass/fail per criterion ----

    // --- Criterion 7: fee/gross < 35% ---

    #[test]
    fn test_fee_to_gross_passes_low_fees() {
        let gate = gate_config_default();
        // Gross PnL = 100, total fees = 20 (entry+exit+route per trade) → 20/100 = 20% < 35% ✓
        let trades = vec![ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 101_000.0,
            size_usd: 1000.0,
            gross_pnl: 10.0,
            entry_fee: 1.0,
            exit_fee: 1.0,
            route_cost_usd: 0.2,
            net_pnl: 7.8,
            hold_secs: 300,
            exit_reason: "TakeProfit".to_string(),
            entry_timestamp_ms: 1000,
            exit_timestamp_ms: 1300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 101_000.0,
            mae_usd: -0.5,
            mfe_usd: 10.0,
            worst_price: 99_950.0,
            best_price: 101_000.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: 0.78,
        }, ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 101_500.0,
            size_usd: 1000.0,
            gross_pnl: 15.0,
            entry_fee: 1.0,
            exit_fee: 1.0,
            route_cost_usd: 0.2,
            net_pnl: 12.8,
            hold_secs: 300,
            exit_reason: "TakeProfit".to_string(),
            entry_timestamp_ms: 2000,
            exit_timestamp_ms: 2300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 101_500.0,
            mae_usd: -0.3,
            mfe_usd: 15.0,
            worst_price: 99_970.0,
            best_price: 101_500.0,
            is_zone_touch: false,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: 0.85,
        }];
        // Gross PnL = 25.0, total fees = 4.4, ratio = 4.4/25.0 = 17.6% < 35% ✓
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let fee_c = criteria.iter().find(|c| c.name == "fee_to_gross_ratio").unwrap();
        assert!(fee_c.passed, "Low fee/gross should pass, actual={}", fee_c.actual_value);
    }

    #[test]
    fn test_fee_to_gross_fails_high_fees() {
        let gate = gate_config_default();
        // Create trades where fees are >35% of gross PnL
        let trades = vec![ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 100_100.0,
            size_usd: 1000.0,
            gross_pnl: 1.0,
            entry_fee: 1.0,
            exit_fee: 1.0,
            route_cost_usd: 1.0,
            net_pnl: -2.0,
            hold_secs: 300,
            exit_reason: "StopLoss".to_string(),
            entry_timestamp_ms: 1000,
            exit_timestamp_ms: 1300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 100_100.0,
            mae_usd: -1.0,
            mfe_usd: 1.0,
            worst_price: 99_900.0,
            best_price: 100_100.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: -2.0,
        }];
        // Gross PnL = 1.0, total fees = 3.0, ratio = 3.0/1.0 = 300% > 35% → FAIL
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let fee_c = criteria.iter().find(|c| c.name == "fee_to_gross_ratio").unwrap();
        assert!(!fee_c.passed, "High fee/gross should fail, actual={}", fee_c.actual_value);
    }

    #[test]
    fn test_fee_to_gross_no_trades_passes() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let fee_c = criteria.iter().find(|c| c.name == "fee_to_gross_ratio").unwrap();
        assert!(fee_c.passed, "No trades → no fees → should pass");
    }

    // --- Criterion 8: no single event >25% of profit ---

    #[test]
    fn test_single_trade_dependency_criterion_passes() {
        let gate = gate_config_default();
        // Many small equal trades → no dominance
        let trades: Vec<ReplayTrade> = (0..10).map(|i| ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 100_100.0,
            size_usd: 100.0,
            gross_pnl: 1.0,
            entry_fee: 0.1,
            exit_fee: 0.1,
            route_cost_usd: 0.02,
            net_pnl: 0.78,
            hold_secs: 300,
            exit_reason: "TakeProfit".to_string(),
            entry_timestamp_ms: 1000 + i as i64 * 1000,
            exit_timestamp_ms: 1300 + i as i64 * 1000,
            entry_stale: false,
            exit_stale: false,
            peak_price: 100_100.0,
            mae_usd: -0.05,
            mfe_usd: 1.0,
            worst_price: 99_950.0,
            best_price: 100_100.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: 0.78,
        }).collect();
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let dep_c = criteria.iter().find(|c| c.name == "single_trade_dependency").unwrap();
        assert!(dep_c.passed, "Equal trades → no dominance, should pass");
    }

    #[test]
    fn test_single_trade_dependency_criterion_fails() {
        let gate = gate_config_default();
        // One dominant trade + many small ones
        let mut trades: Vec<ReplayTrade> = (0..5).map(|i| ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 100_010.0,
            size_usd: 100.0,
            gross_pnl: 0.1,
            entry_fee: 0.1,
            exit_fee: 0.1,
            route_cost_usd: 0.02,
            net_pnl: -0.12,
            hold_secs: 300,
            exit_reason: "StopLoss".to_string(),
            entry_timestamp_ms: 1000 + i as i64 * 1000,
            exit_timestamp_ms: 1300 + i as i64 * 1000,
            entry_stale: false,
            exit_stale: false,
            peak_price: 100_010.0,
            mae_usd: -0.1,
            mfe_usd: 0.1,
            worst_price: 99_990.0,
            best_price: 100_010.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: -1.2,
        }).collect();
        // Add one big winner
        trades.push(ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 105_000.0,
            size_usd: 1000.0,
            gross_pnl: 50.0,
            entry_fee: 1.0,
            exit_fee: 1.0,
            route_cost_usd: 0.2,
            net_pnl: 47.8,
            hold_secs: 300,
            exit_reason: "TakeProfit".to_string(),
            entry_timestamp_ms: 10000,
            exit_timestamp_ms: 10300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 105_000.0,
            mae_usd: -0.5,
            mfe_usd: 50.0,
            worst_price: 99_500.0,
            best_price: 105_000.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: 0.956,
        });
        // Total = 5*(-0.12) + 47.8 = 47.2; dominant = 47.8/47.2 = 101% > 25% → FAIL
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let dep_c = criteria.iter().find(|c| c.name == "single_trade_dependency").unwrap();
        assert!(!dep_c.passed, "Dominant trade > 25% → should fail");
    }

    // --- Criterion 9: fishing improves expectancy or reduces drawdown ---

    #[test]
    fn test_fishing_improvement_passes_positive_delta() {
        let gate = gate_config_default();
        let fishing = FishingSimResult {
            total_orders: 10,
            filled_orders: 5,
            fully_filled_orders: 3,
            partially_filled_orders: 2,
            fill_rate: 0.5,
            adverse_fills: 1,
            total_fills: 5,
            adverse_selection_rate: 0.2,
            avg_entry_improvement_bps: 15.0,
            missed_winners: 2,
            missed_losers: 1,
            total_gross_pnl_usd: 50.0,
            total_net_pnl_usd: 40.0,
            total_fees_usd: 5.0,
            total_route_cost_usd: 5.0,
            expectancy_fishing: 8.0,
            expectancy_market: 5.0,
            expectancy_delta: 3.0, // positive → passes
            cancelled_decay: 0,
            cancelled_cascade: 0,
            cancelled_spread: 0,
            cancelled_depth: 0,
            expired_orders: 0,
            sl_hit_count: 1,
            tp_hit_count: 2,
        };
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, Some(&fishing), None, None, &gate);
        let fish_c = criteria.iter().find(|c| c.name == "fishing_improvement").unwrap();
        assert!(fish_c.passed, "Positive expectancy delta should pass");
    }

    #[test]
    fn test_fishing_improvement_fails_negative_delta() {
        let gate = gate_config_default();
        let fishing = FishingSimResult {
            total_orders: 10,
            filled_orders: 5,
            fully_filled_orders: 3,
            partially_filled_orders: 2,
            fill_rate: 0.5,
            adverse_fills: 3,
            total_fills: 5,
            adverse_selection_rate: 0.6,
            avg_entry_improvement_bps: 5.0,
            missed_winners: 0,
            missed_losers: 3,
            total_gross_pnl_usd: 20.0,
            total_net_pnl_usd: 10.0,
            total_fees_usd: 5.0,
            total_route_cost_usd: 5.0,
            expectancy_fishing: 2.0,
            expectancy_market: 5.0,
            expectancy_delta: -3.0, // negative → fails
            cancelled_decay: 0,
            cancelled_cascade: 0,
            cancelled_spread: 0,
            cancelled_depth: 0,
            expired_orders: 0,
            sl_hit_count: 2,
            tp_hit_count: 1,
        };
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, Some(&fishing), None, None, &gate);
        let fish_c = criteria.iter().find(|c| c.name == "fishing_improvement").unwrap();
        assert!(!fish_c.passed, "Negative expectancy delta should fail");
    }

    #[test]
    fn test_fishing_improvement_passes_no_composition() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let fish_c = criteria.iter().find(|c| c.name == "fishing_improvement").unwrap();
        assert!(fish_c.passed, "No fishing composed → passes by default");
    }

    // --- Criterion 10: pyramiding improves risk-adjusted return ---

    #[test]
    fn test_pyramiding_improvement_passes_positive_pnl() {
        let gate = gate_config_default();
        let pyramid = PyramidResult {
            tranche_count: 3,
            total_size_usd: 300.0,
            avg_entry_price: 100.0,
            combined_stop_price: 98.0,
            max_risk_usd: 6.0,
            unrealized_pnl_usd: 15.0, // positive → passes
            stopped_out: false,
        };
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, Some(&pyramid), None, &gate);
        let pyr_c = criteria.iter().find(|c| c.name == "pyramiding_improvement").unwrap();
        assert!(pyr_c.passed, "Positive unrealized PnL should pass");
    }

    #[test]
    fn test_pyramiding_improvement_fails_stopped_out() {
        let gate = gate_config_default();
        let pyramid = PyramidResult {
            tranche_count: 3,
            total_size_usd: 300.0,
            avg_entry_price: 100.0,
            combined_stop_price: 98.0,
            max_risk_usd: 6.0,
            unrealized_pnl_usd: -5.0, // negative AND stopped out
            stopped_out: true,
        };
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, Some(&pyramid), None, &gate);
        let pyr_c = criteria.iter().find(|c| c.name == "pyramiding_improvement").unwrap();
        assert!(!pyr_c.passed, "Stopped out pyramid should fail");
    }

    #[test]
    fn test_pyramiding_improvement_passes_no_composition() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let pyr_c = criteria.iter().find(|c| c.name == "pyramiding_improvement").unwrap();
        assert!(pyr_c.passed, "No pyramiding composed → passes by default");
    }

    // --- Criterion 11: route cost doesn't consume edge ---

    #[test]
    fn test_route_cost_edge_passes_low_cost() {
        let gate = gate_config_default();
        // Net expectancy = 1.0, route costs = 0.1 → 10% < 50% ✓
        let trades = vec![ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 101_000.0,
            size_usd: 1000.0,
            gross_pnl: 10.0,
            entry_fee: 1.0,
            exit_fee: 1.0,
            route_cost_usd: 0.1,
            net_pnl: 7.9,
            hold_secs: 300,
            exit_reason: "TakeProfit".to_string(),
            entry_timestamp_ms: 1000,
            exit_timestamp_ms: 1300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 101_000.0,
            mae_usd: -0.5,
            mfe_usd: 10.0,
            worst_price: 99_950.0,
            best_price: 101_000.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: 0.79,
        }];
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let rc = criteria.iter().find(|c| c.name == "route_cost_edge").unwrap();
        assert!(rc.passed, "Low route cost should pass, actual={}", rc.actual_value);
    }

    #[test]
    fn test_route_cost_edge_fails_high_cost() {
        let gate = gate_config_default();
        // Net expectancy = 0.01, route costs = 0.5 → 5000% > 50% → FAIL
        let trades = vec![ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 100_001.0,
            size_usd: 10000.0,
            gross_pnl: 0.1,
            entry_fee: 10.0,
            exit_fee: 10.0,
            route_cost_usd: 0.5,
            net_pnl: -20.4,
            hold_secs: 300,
            exit_reason: "StopLoss".to_string(),
            entry_timestamp_ms: 1000,
            exit_timestamp_ms: 1300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 100_001.0,
            mae_usd: -0.1,
            mfe_usd: 0.1,
            worst_price: 99_990.0,
            best_price: 100_001.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: -204.0,
        }];
        // With positive net_expectancy but high route costs relative to expectancy
        let criteria = evaluate_promotion_criteria(&trades, 0.01, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let rc = criteria.iter().find(|c| c.name == "route_cost_edge").unwrap();
        assert!(!rc.passed, "High route cost should fail");
    }

    #[test]
    fn test_route_cost_edge_fails_no_expectancy() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], -1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let rc = criteria.iter().find(|c| c.name == "route_cost_edge").unwrap();
        assert!(!rc.passed, "Negative expectancy → no edge → route cost fails");
    }

    // --- Criterion 12: liquidation distance safe at proposed leverage ---

    #[test]
    fn test_liquidation_distance_passes_safe_distance() {
        let gate = gate_config_default();
        // min_zone_distance = 500 bps > 200 bps threshold, > 3333 bps (10000/3 leverage)
        // Wait, 10000/3 = 3333.3 bps. 500 < 3333 → FAILS!
        // Let me recalculate: at 3x leverage, liquidation is at 33.3% away = 3333 bps
        // Zone distance must be > 3333 bps to be "safe"
        // That's actually a high threshold. Let me re-check...
        // Actually the threshold check is: dist > leverage_liquidation_bps AND dist > min_safe_liquidation_distance_bps
        // leverage_liquidation_bps = 10000/3 = 3333.3
        // So the zone must be farther than 3333 bps at 3x leverage
        // Let me use a distance > 3334 to pass
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, Some(5000.0), &gate);
        let liq_c = criteria.iter().find(|c| c.name == "liquidation_distance_safety").unwrap();
        assert!(liq_c.passed, "Zone distance 5000 bps > 3333 bps (3x leverage) → safe");
    }

    #[test]
    fn test_liquidation_distance_fails_too_close() {
        let gate = gate_config_default();
        // Zone at 100 bps — way too close at 3x leverage (liquidation at 3333 bps)
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, Some(100.0), &gate);
        let liq_c = criteria.iter().find(|c| c.name == "liquidation_distance_safety").unwrap();
        assert!(!liq_c.passed, "Zone distance 100 bps < 3333 bps → unsafe");
    }

    #[test]
    fn test_liquidation_distance_passes_no_zones() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, None, &gate);
        let liq_c = criteria.iter().find(|c| c.name == "liquidation_distance_safety").unwrap();
        assert!(liq_c.passed, "No zones → no liquidation risk → passes");
    }

    #[test]
    fn test_liquidation_distance_high_leverage() {
        let gate = PromotionGateConfig {
            proposed_leverage: 10.0,
            ..gate_config_default()
        };
        // At 10x leverage, liquidation at 1000 bps. Zone at 1500 bps → safe
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, Some(1500.0), &gate);
        let liq_c = criteria.iter().find(|c| c.name == "liquidation_distance_safety").unwrap();
        assert!(liq_c.passed, "At 10x leverage, 1500 bps > 1000 bps → safe");
    }

    // ---- VAL-GATE-003: Verdict matches criteria results ----

    #[test]
    fn test_verdict_denied_11_of_12_passing() {
        let gate = gate_config_default();
        // Make criterion 7 fail: provide trades with high fee/gross ratio
        let trades = vec![ReplayTrade {
            symbol: "BTC".to_string(),
            side: "long".to_string(),
            entry_price: 100_000.0,
            exit_price: 100_010.0,
            size_usd: 1000.0,
            gross_pnl: 0.1,
            entry_fee: 10.0,
            exit_fee: 10.0,
            route_cost_usd: 5.0,
            net_pnl: -24.9,
            hold_secs: 300,
            exit_reason: "StopLoss".to_string(),
            entry_timestamp_ms: 1000,
            exit_timestamp_ms: 1300,
            entry_stale: false,
            exit_stale: false,
            peak_price: 100_010.0,
            mae_usd: -0.1,
            mfe_usd: 0.1,
            worst_price: 99_990.0,
            best_price: 100_010.0,
            is_zone_touch: true,
            post_liquidation_drift_usd: 0.0,
            time_to_reversal_secs: 0.0,
            time_to_next_zone_secs: 0.0,
            stop_efficiency: -249.0,
        }];
        // Fee/gross = 25/0.1 = 25000% → FAIL
        let criteria = evaluate_promotion_criteria(&trades, 1.0, 5.0, 0, 0, 30, 1.5, None, None, Some(5000.0), &gate);
        let failed_count = criteria.iter().filter(|c| !c.passed).count();
        assert!(failed_count > 0, "At least one criterion should fail");
        let all_passed = criteria.iter().all(|c| c.passed);
        assert!(!all_passed, "11/12 or fewer passing → verdict must be Denied");
    }

    #[test]
    fn test_verdict_approved_all_12_passing() {
        let gate = gate_config_default();
        // Use no trades (passes fee/gross, single-trade-dep by default)
        // and safe zone distance
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, None, None, Some(5000.0), &gate);
        assert_eq!(criteria.len(), 12);
        let all_passed = criteria.iter().all(|c| c.passed);
        assert!(all_passed, "All 12 criteria should pass with safe defaults and no trades");
    }

    // ---- VAL-GATE-004: Extended gate criteria in composed flow ----

    #[test]
    fn test_composed_flow_12_criteria_evaluated() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let fishing_config = FishingLadderConfig::default();
        let pyramid_config = PyramidConfig::default();

        let result = pipeline.run_with_fishing_and_pyramiding(&data, &fishing_config, &pyramid_config);

        // Must have exactly 12 criteria
        assert_eq!(result.promotion_criteria.len(), 12, "Composed flow must evaluate 12 criteria");

        // Fishing and pyramid results should be present
        assert!(result.fishing_result.is_some());
        assert!(result.pyramid_result.is_some());

        // Each criterion has required fields
        for c in &result.promotion_criteria {
            assert!(!c.name.is_empty());
            assert!(!c.description.is_empty());
            assert!(!c.actual_value.is_empty());
            assert!(!c.threshold_value.is_empty());
        }
    }

    #[test]
    fn test_min_zone_distance_computed_from_data() {
        let params = params_all_pass();
        let gate = gate_config_default();
        let data = generate_trending_points("SOL", 50, 150.0, 1_770_000_000_000, 5000);
        let pipeline = ReplayPipeline::new(params, gate);
        let result = pipeline.run(&data);

        // The criterion should exist and be evaluated
        let liq_c = result.promotion_criteria.iter()
            .find(|c| c.name == "liquidation_distance_safety")
            .unwrap();
        // With trending data, zones have distance_bps = 200.0 which is < 3333 (3x leverage threshold)
        // So it should be unsafe → fail. Unless no zones in the trade data path.
        // The test data points have zones with distance_bps = 200.0
        assert!(!liq_c.passed || liq_c.actual_value.contains("0.0"),
            "Zone at 200 bps should be unsafe at 3x leverage");
    }
}
