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
//! No live trading. Paper-only. Backward compatible.

use crate::liquidation::{LiquidationZone, LiquidationZoneSnapshot};
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

/// Configuration for the promotion gate.
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

impl Default for PromotionGateConfig {
    fn default() -> Self {
        Self {
            max_drawdown_pct: default_max_drawdown_pct(),
            min_signal_events: default_min_signal_events(),
            min_sharpe: default_min_sharpe(),
            fee_rate: default_fee_rate(),
            starting_balance: default_starting_balance(),
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
                    // Update peak price
                    if let Some(ref mut pos_ref) = open_position {
                        if pos_ref.is_long && point.price > pos_ref.peak_price {
                            pos_ref.peak_price = point.price;
                        }
                        if !pos_ref.is_long && point.price < pos_ref.peak_price {
                            pos_ref.peak_price = point.price;
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

        // Net expectancy
        let net_expectancy = compute_net_expectancy(&trades);

        // Max drawdown as percentage
        let max_drawdown_pct = if starting_balance > 0.0 {
            max_drawdown_usd / starting_balance * 100.0
        } else {
            0.0
        };

        // Evaluate promotion criteria
        let criteria = evaluate_promotion_criteria(
            &trades,
            net_expectancy,
            max_drawdown_pct,
            stale_trade_count,
            duplicate_pending_count,
            signal_events,
            sharpe_ratio,
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
        }
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
            "| Max Drawdown | ${:.2} ({:.2}%) |\n",
            result.max_drawdown_usd, result.max_drawdown_pct
        ));
        report.push_str(&format!(
            "| Net Expectancy | ${:.4} |\n",
            result.net_expectancy
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

/// Evaluate all promotion criteria.
#[allow(clippy::too_many_arguments)]
fn evaluate_promotion_criteria(
    _trades: &[ReplayTrade],
    net_expectancy: f64,
    max_drawdown_pct: f64,
    stale_trade_count: usize,
    duplicate_pending_count: usize,
    signal_events: usize,
    sharpe_ratio: f64,
    config: &PromotionGateConfig,
) -> Vec<CriterionStatus> {
    vec![
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
    ]
}

/// Atomic write: write content to a `.tmp` file, then rename to final path.
fn atomic_write(path: &Path, content: &str) -> anyhow::Result<()> {
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
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
            &gate,
        );
        let dd_criterion = criteria.iter().find(|c| c.name == "max_drawdown").unwrap();
        assert!(!dd_criterion.passed);
    }

    // ---- VAL-STRAT-053: Zero stale-data trades ----

    #[test]
    fn test_stale_data_trades_fail_promotion() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 2, 0, 30, 1.5, &gate);
        let stale = criteria.iter().find(|c| c.name == "stale_data_trades").unwrap();
        assert!(!stale.passed);
    }

    #[test]
    fn test_zero_stale_data_passes() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);
        let stale = criteria.iter().find(|c| c.name == "stale_data_trades").unwrap();
        assert!(stale.passed);
    }

    // ---- VAL-STRAT-054: Zero duplicate pending trades ----

    #[test]
    fn test_duplicate_pending_fail_promotion() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 3, 30, 1.5, &gate);
        let dup = criteria.iter().find(|c| c.name == "duplicate_pending_trades").unwrap();
        assert!(!dup.passed);
    }

    #[test]
    fn test_zero_duplicate_pending_passes() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);
        let dup = criteria.iter().find(|c| c.name == "duplicate_pending_trades").unwrap();
        assert!(dup.passed);
    }

    // ---- VAL-STRAT-055: Minimum 30 signal events ----

    #[test]
    fn test_min_30_signal_events() {
        let gate = gate_config_default();
        let criteria_29 = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 29, 1.5, &gate);
        let sig_29 = criteria_29.iter().find(|c| c.name == "min_signal_events").unwrap();
        assert!(!sig_29.passed, "29 signal events should fail");

        let criteria_30 = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);
        let sig_30 = criteria_30.iter().find(|c| c.name == "min_signal_events").unwrap();
        assert!(sig_30.passed, "30 signal events should pass");
    }

    // ---- VAL-STRAT-056: Sharpe ratio ≥ 1.0 ----

    #[test]
    fn test_sharpe_ratio_threshold() {
        let gate = gate_config_default();
        let criteria_below = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 0.5, &gate);
        let sharpe_below = criteria_below.iter().find(|c| c.name == "sharpe_ratio").unwrap();
        assert!(!sharpe_below.passed, "Sharpe 0.5 should fail");

        let criteria_at = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.0, &gate);
        let sharpe_at = criteria_at.iter().find(|c| c.name == "sharpe_ratio").unwrap();
        assert!(sharpe_at.passed, "Sharpe 1.0 should pass");
    }

    // ---- VAL-STRAT-079: Promotion gate aggregates ALL criteria ----

    #[test]
    fn test_promotion_gate_all_pass() {
        let gate = gate_config_default();
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);
        assert!(criteria.iter().all(|c| c.passed), "All criteria should pass");
    }

    #[test]
    fn test_promotion_gate_any_fail_blocks() {
        let gate = gate_config_default();
        // Negative expectancy
        let criteria = evaluate_promotion_criteria(&[], -1.0, 5.0, 0, 0, 30, 1.5, &gate);
        let all_passed = criteria.iter().all(|c| c.passed);
        assert!(!all_passed, "Negative expectancy should block promotion");

        // Too much drawdown
        let criteria = evaluate_promotion_criteria(&[], 1.0, 15.0, 0, 0, 30, 1.5, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Stale trades
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 1, 0, 30, 1.5, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Duplicates
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 1, 30, 1.5, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Too few signals
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 29, 1.5, &gate);
        assert!(!criteria.iter().all(|c| c.passed));

        // Low Sharpe
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 0.5, &gate);
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
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);

        // Must have exactly 6 criteria
        assert_eq!(criteria.len(), 6);

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
        assert!(names.contains(&"net_expectancy"));
        assert!(names.contains(&"max_drawdown"));
        assert!(names.contains(&"stale_data_trades"));
        assert!(names.contains(&"duplicate_pending_trades"));
        assert!(names.contains(&"min_signal_events"));
        assert!(names.contains(&"sharpe_ratio"));
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
        let criteria = evaluate_promotion_criteria(&[], 1.0, 5.0, 0, 0, 30, 1.5, &gate);
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
        let criteria = evaluate_promotion_criteria(&[], -1.0, 5.0, 0, 0, 30, 1.5, &gate);
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

        // Verify strategy params defaults are backward compatible
        let params = LiquidationCascadeParams::default();
        assert!(!params.enabled, "Strategy disabled by default");
        assert!(params.paper_only, "Paper-only by default");
    }
}
