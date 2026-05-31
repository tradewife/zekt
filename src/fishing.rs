//! Fishing Order Simulator — passive limit order ladder simulation at liquidation zone offsets.
//!
//! Models a configurable limit-order ladder placed at offsets from liquidation zones.
//! Simulates partial fills, maker/taker fee assumptions, adverse selection tracking,
//! missed fills, post-fill SL/TP, route cost integration, and spread/depth degradation
//! cancellation. Outputs a market-entry vs fishing-entry expectancy comparison.
//!
//! **Standalone module** — callable by `replay.rs` for composed replay flows.
//! No imports from engine, executor, flash_api, or strategy.
//! Uses `tracing` for all logging (never `println`).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::liquidity_memory::MemoryZone;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the fishing order ladder simulator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FishingLadderConfig {
    /// Basis-point offset from the zone midpoint for the first order.
    /// e.g., 10.0 means the first order is placed 10 bps away from zone midpoint.
    pub zone_offset_bps: f64,

    /// Number of tranches (orders) in the ladder.
    pub num_tranches: usize,

    /// Spacing between consecutive tranches in basis points.
    pub tranche_spacing_bps: f64,

    /// Maximum total risk across all fishing orders in USD.
    pub max_total_risk_usd: f64,

    /// Time in seconds after which unfilled orders are cancelled.
    pub expiry_secs: f64,

    /// Zone decay score threshold above which fishing orders are cancelled.
    /// If a zone's decay_score exceeds this value, orders at that zone are cancelled.
    pub cancel_on_decay_threshold: f64,

    /// If true, all fishing orders are cancelled when a cascade event is detected.
    pub cancel_on_cascade_threshold: bool,

    /// Fee rate for maker fills (passive, resting orders).
    pub maker_fee_bps: f64,

    /// Fee rate for taker fills (aggressive, spread-crossing fills).
    pub taker_fee_bps: f64,

    /// Post-fill stop-loss offset in basis points from the fill price.
    pub sl_offset_bps: f64,

    /// Post-fill take-profit offset in basis points from the fill price.
    pub tp_offset_bps: f64,

    /// Route cost in bps to deduct from fishing PnL.
    pub route_cost_bps: f64,

    /// Maximum spread (in percentage) above which fishing orders are cancelled.
    pub max_spread_pct: f64,

    /// Minimum order book depth (in USD) below which fishing orders are cancelled.
    pub min_depth_usd: f64,
}

impl Default for FishingLadderConfig {
    fn default() -> Self {
        Self {
            zone_offset_bps: 10.0,
            num_tranches: 3,
            tranche_spacing_bps: 5.0,
            max_total_risk_usd: 500.0,
            expiry_secs: 300.0,
            cancel_on_decay_threshold: 0.7,
            cancel_on_cascade_threshold: true,
            maker_fee_bps: 2.0,
            taker_fee_bps: 5.0,
            sl_offset_bps: 50.0,
            tp_offset_bps: 100.0,
            route_cost_bps: 3.0,
            max_spread_pct: 0.15,
            min_depth_usd: 10_000.0,
        }
    }
}

impl FishingLadderConfig {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.zone_offset_bps < 0.0 {
            anyhow::bail!("zone_offset_bps must be >= 0, got {}", self.zone_offset_bps);
        }
        if self.num_tranches == 0 {
            anyhow::bail!("num_tranches must be >= 1");
        }
        if self.tranche_spacing_bps < 0.0 {
            anyhow::bail!(
                "tranche_spacing_bps must be >= 0, got {}",
                self.tranche_spacing_bps
            );
        }
        if self.max_total_risk_usd <= 0.0 {
            anyhow::bail!(
                "max_total_risk_usd must be > 0, got {}",
                self.max_total_risk_usd
            );
        }
        if self.expiry_secs <= 0.0 {
            anyhow::bail!("expiry_secs must be > 0, got {}", self.expiry_secs);
        }
        if self.cancel_on_decay_threshold < 0.0 || self.cancel_on_decay_threshold > 1.0 {
            anyhow::bail!(
                "cancel_on_decay_threshold must be in [0.0, 1.0], got {}",
                self.cancel_on_decay_threshold
            );
        }
        if self.maker_fee_bps < 0.0 {
            anyhow::bail!("maker_fee_bps must be >= 0, got {}", self.maker_fee_bps);
        }
        if self.taker_fee_bps < 0.0 {
            anyhow::bail!("taker_fee_bps must be >= 0, got {}", self.taker_fee_bps);
        }
        if self.sl_offset_bps <= 0.0 {
            anyhow::bail!("sl_offset_bps must be > 0, got {}", self.sl_offset_bps);
        }
        if self.tp_offset_bps <= 0.0 {
            anyhow::bail!("tp_offset_bps must be > 0, got {}", self.tp_offset_bps);
        }
        if self.route_cost_bps < 0.0 {
            anyhow::bail!("route_cost_bps must be >= 0, got {}", self.route_cost_bps);
        }
        if self.max_spread_pct <= 0.0 {
            anyhow::bail!(
                "max_spread_pct must be > 0, got {}",
                self.max_spread_pct
            );
        }
        if self.min_depth_usd < 0.0 {
            anyhow::bail!(
                "min_depth_usd must be >= 0, got {}",
                self.min_depth_usd
            );
        }
        Ok(())
    }

    /// Per-tranche size in USD, derived from max_total_risk_usd and num_tranches.
    pub fn tranche_size_usd(&self) -> f64 {
        self.max_total_risk_usd / self.num_tranches as f64
    }
}

// ---------------------------------------------------------------------------
// Order State
// ---------------------------------------------------------------------------

/// Status of a single fishing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    /// Order is active, waiting to be filled.
    Active,
    /// Order was fully filled.
    Filled,
    /// Order was partially filled (some quantity remains).
    PartiallyFilled,
    /// Order was cancelled due to zone decay.
    CancelledDecay,
    /// Order was cancelled due to cascade signal.
    CancelledCascade,
    /// Order expired (unfilled past expiry_secs).
    Expired,
    /// Order was cancelled due to spread widening.
    CancelledSpread,
    /// Order was cancelled due to depth degradation.
    CancelledDepth,
    /// Order was missed — price passed through but order was not present.
    Missed,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Active => write!(f, "active"),
            OrderStatus::Filled => write!(f, "filled"),
            OrderStatus::PartiallyFilled => write!(f, "partially_filled"),
            OrderStatus::CancelledDecay => write!(f, "cancelled_decay"),
            OrderStatus::CancelledCascade => write!(f, "cancelled_cascade"),
            OrderStatus::Expired => write!(f, "expired"),
            OrderStatus::CancelledSpread => write!(f, "cancelled_spread"),
            OrderStatus::CancelledDepth => write!(f, "cancelled_depth"),
            OrderStatus::Missed => write!(f, "missed"),
        }
    }
}

/// A single fishing order in the simulation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FishingOrder {
    /// Order price (the limit price for this tranche).
    pub price: f64,
    /// Order size in USD.
    pub size_usd: f64,
    /// How much has been filled in USD.
    pub filled_usd: f64,
    /// Fill price (average for partial fills).
    pub fill_price: Option<f64>,
    /// Whether the fill was adverse-selected (price moved against after fill).
    pub adverse_selected: bool,
    /// Current status of this order.
    pub status: OrderStatus,
    /// Timestamp (ms) when the order was placed.
    pub placed_at_ms: i64,
    /// Index in the ladder (0 = closest to zone, num_tranches-1 = farthest).
    pub tranche_index: usize,
    /// Whether this was a maker fill (passive) or taker fill (aggressive).
    pub is_maker_fill: Option<bool>,
    /// Fee charged on the fill in USD.
    pub fee_usd: f64,
    /// Route cost deducted in USD.
    pub route_cost_usd: f64,
    /// Gross PnL (before fees and route cost) in USD.
    pub gross_pnl_usd: f64,
    /// Net PnL (after fees and route cost) in USD.
    pub net_pnl_usd: f64,
    /// Reason for cancellation, if cancelled.
    pub cancel_reason: Option<String>,
}

impl FishingOrder {
    /// Create a new active fishing order.
    pub fn new(price: f64, size_usd: f64, placed_at_ms: i64, tranche_index: usize) -> Self {
        Self {
            price,
            size_usd,
            filled_usd: 0.0,
            fill_price: None,
            adverse_selected: false,
            status: OrderStatus::Active,
            placed_at_ms,
            tranche_index,
            is_maker_fill: None,
            fee_usd: 0.0,
            route_cost_usd: 0.0,
            gross_pnl_usd: 0.0,
            net_pnl_usd: 0.0,
            cancel_reason: None,
        }
    }

    /// Whether this order has any fill (full or partial).
    pub fn has_fill(&self) -> bool {
        self.filled_usd > 0.0
    }

    /// Remaining unfilled size in USD.
    pub fn remaining_usd(&self) -> f64 {
        (self.size_usd - self.filled_usd).max(0.0)
    }

    /// Whether this order is still active (can receive fills).
    pub fn is_active(&self) -> bool {
        self.status == OrderStatus::Active || self.status == OrderStatus::PartiallyFilled
    }

    /// Fill this order (partially or fully).
    ///
    /// `fill_price` is the actual execution price.
    /// `fill_amount_usd` is how much was filled in USD.
    /// `is_maker` is true if this was a passive (resting) fill.
    pub fn apply_fill(
        &mut self,
        fill_price: f64,
        fill_amount_usd: f64,
        is_maker: bool,
        config: &FishingLadderConfig,
    ) {
        let actual_fill = fill_amount_usd.min(self.remaining_usd());
        self.filled_usd += actual_fill;
        self.fill_price = Some(fill_price);
        self.is_maker_fill = Some(is_maker);

        // Fee calculation based on maker vs taker
        let fee_bps = if is_maker {
            config.maker_fee_bps
        } else {
            config.taker_fee_bps
        };
        self.fee_usd = self.filled_usd * (fee_bps / 10_000.0);

        // Route cost
        self.route_cost_usd = self.filled_usd * (config.route_cost_bps / 10_000.0);

        // Update status
        if self.filled_usd >= self.size_usd - 0.0001 {
            self.status = OrderStatus::Filled;
        } else {
            self.status = OrderStatus::PartiallyFilled;
        }
    }

    /// Compute PnL for a filled order given the exit price.
    /// For long fishing: pnl = (exit_price - fill_price) / fill_price * filled_usd
    /// For short fishing: pnl = (fill_price - exit_price) / fill_price * filled_usd
    pub fn compute_pnl(&mut self, exit_price: f64, is_long: bool) {
        if let Some(fp) = self.fill_price
            && fp > 0.0
        {
            let price_delta_pct = if is_long {
                (exit_price - fp) / fp
            } else {
                (fp - exit_price) / fp
            };
            self.gross_pnl_usd = price_delta_pct * self.filled_usd;
            self.net_pnl_usd = self.gross_pnl_usd - self.fee_usd - self.route_cost_usd;
        }
    }
}

// ---------------------------------------------------------------------------
// Post-fill SL/TP Result
// ---------------------------------------------------------------------------

/// Result of a post-fill SL/TP evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
pub enum SlTpResult {
    /// Neither SL nor TP hit.
    NoHit,
    /// Stop-loss hit.
    StopLossHit,
    /// Take-profit hit.
    TakeProfitHit,
}

/// Evaluate post-fill SL/TP for a filled order given the current price.
///
/// For a long fishing order:
/// - SL triggers when current_price <= fill_price * (1 - sl_offset_bps/10000)
/// - TP triggers when current_price >= fill_price * (1 + tp_offset_bps/10000)
///
/// For a short fishing order:
/// - SL triggers when current_price >= fill_price * (1 + sl_offset_bps/10000)
/// - TP triggers when current_price <= fill_price * (1 - tp_offset_bps/10000)
pub fn evaluate_sl_tp(
    fill_price: f64,
    current_price: f64,
    is_long: bool,
    sl_offset_bps: f64,
    tp_offset_bps: f64,
) -> SlTpResult {
    let sl_factor = sl_offset_bps / 10_000.0;
    let tp_factor = tp_offset_bps / 10_000.0;

    if is_long {
        let sl_price = fill_price * (1.0 - sl_factor);
        let tp_price = fill_price * (1.0 + tp_factor);
        if current_price <= sl_price {
            SlTpResult::StopLossHit
        } else if current_price >= tp_price {
            SlTpResult::TakeProfitHit
        } else {
            SlTpResult::NoHit
        }
    } else {
        let sl_price = fill_price * (1.0 + sl_factor);
        let tp_price = fill_price * (1.0 - tp_factor);
        if current_price >= sl_price {
            SlTpResult::StopLossHit
        } else if current_price <= tp_price {
            SlTpResult::TakeProfitHit
        } else {
            SlTpResult::NoHit
        }
    }
}

// ---------------------------------------------------------------------------
// Simulation Result
// ---------------------------------------------------------------------------

/// Summary result of a fishing order simulation run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FishingSimResult {
    /// Total number of orders placed.
    pub total_orders: usize,
    /// Number of orders that received at least a partial fill.
    pub filled_orders: usize,
    /// Number of fully filled orders.
    pub fully_filled_orders: usize,
    /// Number of partially filled orders.
    pub partially_filled_orders: usize,
    /// Fill rate = filled_orders / total_orders.
    pub fill_rate: f64,
    /// Number of fills that were adversely selected.
    pub adverse_fills: usize,
    /// Total number of fills (including partial).
    pub total_fills: usize,
    /// Adverse selection rate = adverse_fills / total_fills.
    pub adverse_selection_rate: f64,
    /// Average entry improvement in bps (how much better than market entry).
    pub avg_entry_improvement_bps: f64,
    /// Number of missed winning fills (price passed through but no order was there).
    pub missed_winners: usize,
    /// Number of missed losing fills (price passed through but no order was there).
    pub missed_losers: usize,
    /// Total gross PnL from all filled orders (before fees).
    pub total_gross_pnl_usd: f64,
    /// Total net PnL from all filled orders (after fees and route costs).
    pub total_net_pnl_usd: f64,
    /// Total fees paid across all fills.
    pub total_fees_usd: f64,
    /// Total route costs deducted.
    pub total_route_cost_usd: f64,
    /// Expectancy per trade with fishing entry (net).
    pub expectancy_fishing: f64,
    /// Expectancy per trade with market entry (net).
    pub expectancy_market: f64,
    /// Difference: fishing expectancy - market expectancy.
    pub expectancy_delta: f64,
    /// Number of orders cancelled due to zone decay.
    pub cancelled_decay: usize,
    /// Number of orders cancelled due to cascade.
    pub cancelled_cascade: usize,
    /// Number of orders cancelled due to spread degradation.
    pub cancelled_spread: usize,
    /// Number of orders cancelled due to depth degradation.
    pub cancelled_depth: usize,
    /// Number of expired orders.
    pub expired_orders: usize,
    /// Stop-loss hit count.
    pub sl_hit_count: usize,
    /// Take-profit hit count.
    pub tp_hit_count: usize,
}

impl Default for FishingSimResult {
    fn default() -> Self {
        Self {
            total_orders: 0,
            filled_orders: 0,
            fully_filled_orders: 0,
            partially_filled_orders: 0,
            fill_rate: 0.0,
            adverse_fills: 0,
            total_fills: 0,
            adverse_selection_rate: 0.0,
            avg_entry_improvement_bps: 0.0,
            missed_winners: 0,
            missed_losers: 0,
            total_gross_pnl_usd: 0.0,
            total_net_pnl_usd: 0.0,
            total_fees_usd: 0.0,
            total_route_cost_usd: 0.0,
            expectancy_fishing: 0.0,
            expectancy_market: 0.0,
            expectancy_delta: 0.0,
            cancelled_decay: 0,
            cancelled_cascade: 0,
            cancelled_spread: 0,
            cancelled_depth: 0,
            expired_orders: 0,
            sl_hit_count: 0,
            tp_hit_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Market Conditions Snapshot
// ---------------------------------------------------------------------------

/// Current market conditions passed to the simulator for each tick.
#[derive(Debug, Clone, Default)]
pub struct MarketConditions {
    /// Current price.
    pub price: f64,
    /// Candle high price (for fill simulation).
    pub high: f64,
    /// Candle low price (for fill simulation).
    pub low: f64,
    /// Current timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Current bid-ask spread in percentage.
    pub spread_pct: f64,
    /// Current order book depth in USD.
    pub depth_usd: f64,
    /// Whether a cascade event is occurring.
    pub cascade_detected: bool,
    /// Zone decay scores (zone_id -> decay_score).
    pub zone_decay_scores: Vec<(usize, f64)>,
}

// ---------------------------------------------------------------------------
// Fishing Simulator
// ---------------------------------------------------------------------------

/// The fishing order ladder simulator.
///
/// Places a configurable limit-order ladder at offsets from a liquidation zone.
/// On each tick, it processes fills, expiry, cancellations, and SL/TP.
pub struct FishingSimulator {
    config: FishingLadderConfig,
    orders: Vec<FishingOrder>,
    /// Track which zone each order belongs to (index into zones list).
    order_zone_ids: Vec<usize>,
    /// Whether fishing is long-side (buy orders below market) or short-side.
    is_long: bool,
    /// Market price at the time of ladder placement.
    entry_market_price: f64,
    /// Simulation results accumulator.
    result: FishingSimResult,
    /// Whether the ladder has been placed.
    ladder_placed: bool,
    /// Track missed fills: (price, would_have_been_profitable).
    missed_fill_prices: Vec<(f64, bool)>,
}

impl FishingSimulator {
    /// Create a new fishing simulator with the given configuration.
    pub fn new(config: FishingLadderConfig, is_long: bool) -> Self {
        Self {
            config,
            orders: Vec::new(),
            order_zone_ids: Vec::new(),
            is_long,
            entry_market_price: 0.0,
            result: FishingSimResult::default(),
            ladder_placed: false,
            missed_fill_prices: Vec::new(),
        }
    }

    /// Place a fishing order ladder at the given zone.
    ///
    /// For long fishing: orders are placed BELOW the zone midpoint.
    /// For short fishing: orders are placed ABOVE the zone midpoint.
    ///
    /// Each tranche is spaced by `tranche_spacing_bps` from the previous one,
    /// starting at `zone_offset_bps` from the zone midpoint.
    pub fn place_ladder(&mut self, zone: &MemoryZone, zone_id: usize, market_price: f64, timestamp_ms: i64) {
        self.entry_market_price = market_price;
        let zone_mid = (zone.low + zone.high) / 2.0;
        let tranche_size = self.config.tranche_size_usd();

        for i in 0..self.config.num_tranches {
            let offset_bps = self.config.zone_offset_bps + (i as f64) * self.config.tranche_spacing_bps;
            let offset_factor = offset_bps / 10_000.0;

            let order_price = if self.is_long {
                // Long fishing: buy below zone midpoint
                zone_mid * (1.0 - offset_factor)
            } else {
                // Short fishing: sell above zone midpoint
                zone_mid * (1.0 + offset_factor)
            };

            let order = FishingOrder::new(order_price, tranche_size, timestamp_ms, i);
            self.orders.push(order);
            self.order_zone_ids.push(zone_id);
        }

        self.ladder_placed = true;
        self.result.total_orders = self.orders.len();
    }

    /// Process a single tick of the simulation.
    ///
    /// Returns a list of (order_index, SlTpResult) for filled orders that triggered SL/TP.
    pub fn tick(&mut self, market: &MarketConditions) -> Vec<(usize, SlTpResult)> {
        let mut sl_tp_results = Vec::new();

        // 1. Check for cascade cancellation
        if market.cascade_detected && self.config.cancel_on_cascade_threshold {
            for order in &mut self.orders {
                if order.is_active() {
                    order.status = OrderStatus::CancelledCascade;
                    order.cancel_reason = Some("cascade_detected".to_string());
                    self.result.cancelled_cascade += 1;
                }
            }
            return sl_tp_results; // All orders cancelled, nothing more to do
        }

        // 2. Check spread/depth degradation — cancel active orders if market is degraded
        if market.spread_pct > self.config.max_spread_pct {
            for order in self.orders.iter_mut() {
                if order.is_active() {
                    order.status = OrderStatus::CancelledSpread;
                    order.cancel_reason = Some(format!("spread {:.4}% > max {:.4}%",
                        market.spread_pct, self.config.max_spread_pct));
                    self.result.cancelled_spread += 1;
                }
            }
        } else if market.depth_usd < self.config.min_depth_usd && market.depth_usd >= 0.0 {
            for order in self.orders.iter_mut() {
                if order.is_active() {
                    order.status = OrderStatus::CancelledDepth;
                    order.cancel_reason = Some(format!("depth ${:.0} < min ${:.0}",
                        market.depth_usd, self.config.min_depth_usd));
                    self.result.cancelled_depth += 1;
                }
            }
        }

        // 3. Check zone decay cancellation
        for (zone_id, decay_score) in &market.zone_decay_scores {
            if *decay_score > self.config.cancel_on_decay_threshold {
                for (i, order) in self.orders.iter_mut().enumerate() {
                    if order.is_active() && self.order_zone_ids[i] == *zone_id {
                        order.status = OrderStatus::CancelledDecay;
                        order.cancel_reason = Some(format!("zone {} decay {:.3} > threshold {:.3}",
                            zone_id, decay_score, self.config.cancel_on_decay_threshold));
                        self.result.cancelled_decay += 1;
                    }
                }
            }
        }

        // 4. Check expiry
        let expiry_ms = (self.config.expiry_secs * 1000.0) as i64;
        for order in &mut self.orders {
            if order.is_active()
                && !order.has_fill()
                && market.timestamp_ms - order.placed_at_ms > expiry_ms
            {
                order.status = OrderStatus::Expired;
                order.cancel_reason = Some("expired".to_string());
                self.result.expired_orders += 1;
            }
        }

        // 5. Process fills based on candle high/low
        for order in self.orders.iter_mut() {
            if !order.is_active() {
                continue;
            }

            let filled = if self.is_long {
                // Long order: fill when low touches or goes below order price
                market.low <= order.price
            } else {
                // Short order: fill when high touches or goes above order price
                market.high >= order.price
            };

            if filled {
                // Maker fill (order was resting on the book)
                let fill_price = order.price;
                let fill_amount = order.remaining_usd();
                order.apply_fill(fill_price, fill_amount, true, &self.config);

                // Compute entry improvement vs market
                if self.entry_market_price > 0.0 {
                    let improvement_bps = if self.is_long {
                        (self.entry_market_price - fill_price) / self.entry_market_price * 10_000.0
                    } else {
                        (fill_price - self.entry_market_price) / self.entry_market_price * 10_000.0
                    };
                    // Will be averaged later
                    self.result.avg_entry_improvement_bps += improvement_bps;
                }
            }
        }

        // 6. Evaluate SL/TP for filled orders
        for (i, order) in self.orders.iter_mut().enumerate() {
            if order.has_fill() && order.fill_price.is_some() && !order.adverse_selected {
                let fill_price = order.fill_price.unwrap();
                let sl_tp = evaluate_sl_tp(
                    fill_price,
                    market.price,
                    self.is_long,
                    self.config.sl_offset_bps,
                    self.config.tp_offset_bps,
                );

                match sl_tp {
                    SlTpResult::StopLossHit => {
                        let sl_price = if self.is_long {
                            fill_price * (1.0 - self.config.sl_offset_bps / 10_000.0)
                        } else {
                            fill_price * (1.0 + self.config.sl_offset_bps / 10_000.0)
                        };
                        order.compute_pnl(sl_price, self.is_long);
                        order.adverse_selected = true;
                        self.result.sl_hit_count += 1;
                        sl_tp_results.push((i, SlTpResult::StopLossHit));
                    }
                    SlTpResult::TakeProfitHit => {
                        let tp_price = if self.is_long {
                            fill_price * (1.0 + self.config.tp_offset_bps / 10_000.0)
                        } else {
                            fill_price * (1.0 - self.config.tp_offset_bps / 10_000.0)
                        };
                        order.compute_pnl(tp_price, self.is_long);
                        order.adverse_selected = true;
                        self.result.tp_hit_count += 1;
                        sl_tp_results.push((i, SlTpResult::TakeProfitHit));
                    }
                    SlTpResult::NoHit => {}
                }
            }
        }

        // 7. Track missed fills — check if price passed through order levels
        // that were not active (cancelled/expired). After recording, change
        // status to OrderStatus::Missed to prevent double-counting on
        // subsequent ticks.
        for order in self.orders.iter_mut() {
            if order.status == OrderStatus::CancelledCascade
                || order.status == OrderStatus::CancelledDecay
                || order.status == OrderStatus::Expired
                || order.status == OrderStatus::CancelledSpread
                || order.status == OrderStatus::CancelledDepth
            {
                let would_have_filled = if self.is_long {
                    market.low <= order.price
                } else {
                    market.high >= order.price
                };

                if would_have_filled {
                    // Determine if it would have been profitable
                    let would_be_profitable = if self.is_long {
                        market.price > order.price
                    } else {
                        market.price < order.price
                    };
                    self.missed_fill_prices.push((order.price, would_be_profitable));
                    if would_be_profitable {
                        self.result.missed_winners += 1;
                    } else {
                        self.result.missed_losers += 1;
                    }
                    // Mark as Missed to prevent re-counting on future ticks
                    order.status = OrderStatus::Missed;
                }
            }
        }

        sl_tp_results
    }

    /// Mark an order as adversely selected (price moved against after fill).
    pub fn mark_adverse_selection(&mut self, order_index: usize) {
        if let Some(order) = self.orders.get_mut(order_index)
            && order.has_fill()
        {
            order.adverse_selected = true;
        }
    }

    /// Finalize the simulation and compute aggregate results.
    pub fn finalize(mut self) -> FishingSimResult {
        // Count fills
        let mut fill_count = 0;
        let mut full_fill_count = 0;
        let mut partial_fill_count = 0;
        let mut adverse_count = 0;
        let mut total_gross = 0.0;
        let mut total_net = 0.0;
        let mut total_fees = 0.0;
        let mut total_route_cost = 0.0;
        let mut improvement_count = 0;

        for order in &self.orders {
            if order.has_fill() {
                fill_count += 1;
                if order.status == OrderStatus::Filled {
                    full_fill_count += 1;
                } else if order.status == OrderStatus::PartiallyFilled {
                    partial_fill_count += 1;
                }
                if order.adverse_selected {
                    adverse_count += 1;
                }
                total_gross += order.gross_pnl_usd;
                total_net += order.net_pnl_usd;
                total_fees += order.fee_usd;
                total_route_cost += order.route_cost_usd;
                if order.fill_price.is_some() {
                    improvement_count += 1;
                }
            }
        }

        self.result.filled_orders = fill_count;
        self.result.fully_filled_orders = full_fill_count;
        self.result.partially_filled_orders = partial_fill_count;
        self.result.adverse_fills = adverse_count;
        self.result.total_fills = fill_count;

        // Fill rate
        if self.result.total_orders > 0 {
            self.result.fill_rate = fill_count as f64 / self.result.total_orders as f64;
        }

        // Adverse selection rate
        if fill_count > 0 {
            self.result.adverse_selection_rate = adverse_count as f64 / fill_count as f64;
        }

        // Average entry improvement
        if improvement_count > 0 {
            self.result.avg_entry_improvement_bps /= improvement_count as f64;
        }

        self.result.total_gross_pnl_usd = total_gross;
        self.result.total_net_pnl_usd = total_net;
        self.result.total_fees_usd = total_fees;
        self.result.total_route_cost_usd = total_route_cost;

        // Compute expectancy
        // Fishing expectancy: avg net PnL per filled order
        if fill_count > 0 {
            self.result.expectancy_fishing = total_net / fill_count as f64;
        }

        // Market expectancy: what would have happened if we entered at market price
        // with taker fees instead of fishing with maker fees
        self.result.expectancy_market = self.compute_market_expectancy();

        self.result.expectancy_delta = self.result.expectancy_fishing - self.result.expectancy_market;

        self.result
    }

    /// Compute the market-entry expectancy for comparison.
    ///
    /// Simulates entering at the market price with taker fees, using the same
    /// SL/TP offsets. This provides a baseline to compare against fishing entry.
    fn compute_market_expectancy(&self) -> f64 {
        if self.entry_market_price <= 0.0 || self.orders.is_empty() {
            return 0.0;
        }

        let market_price = self.entry_market_price;
        let sl_price = if self.is_long {
            market_price * (1.0 - self.config.sl_offset_bps / 10_000.0)
        } else {
            market_price * (1.0 + self.config.sl_offset_bps / 10_000.0)
        };
        let tp_price = if self.is_long {
            market_price * (1.0 + self.config.tp_offset_bps / 10_000.0)
        } else {
            market_price * (1.0 - self.config.tp_offset_bps / 10_000.0)
        };

        // For each filled order, compute what market entry would have yielded
        let mut market_pnl_total = 0.0;
        let mut market_fill_count = 0;

        for order in &self.orders {
            if !order.has_fill() {
                continue;
            }
            market_fill_count += 1;

            // Market entry: same SL/TP logic but at market price with taker fees
            let size_usd = order.filled_usd;
            let taker_fee = size_usd * (self.config.taker_fee_bps / 10_000.0);
            let route_cost = size_usd * (self.config.route_cost_bps / 10_000.0);

            // Determine if the market entry would have been stopped out or hit TP
            // based on what actually happened to the fishing order
            if order.adverse_selected {
                // SL hit: loss at SL price
                let gross_pnl = if self.is_long {
                    (sl_price - market_price) / market_price * size_usd
                } else {
                    (market_price - sl_price) / market_price * size_usd
                };
                market_pnl_total += gross_pnl - taker_fee - route_cost;
            } else if order.gross_pnl_usd > 0.0 {
                // TP hit: gain at TP price
                let gross_pnl = if self.is_long {
                    (tp_price - market_price) / market_price * size_usd
                } else {
                    (market_price - tp_price) / market_price * size_usd
                };
                market_pnl_total += gross_pnl - taker_fee - route_cost;
            }
        }

        if market_fill_count > 0 {
            market_pnl_total / market_fill_count as f64
        } else {
            0.0
        }
    }

    /// Get a reference to the current orders.
    pub fn orders(&self) -> &[FishingOrder] {
        &self.orders
    }

    /// Get a mutable reference to the orders.
    pub fn orders_mut(&mut self) -> &mut [FishingOrder] {
        &mut self.orders
    }

    /// Whether the ladder has been placed.
    pub fn is_ladder_placed(&self) -> bool {
        self.ladder_placed
    }

    /// Get the config reference.
    pub fn config(&self) -> &FishingLadderConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Standalone simulation function for replay pipeline integration
// ---------------------------------------------------------------------------

/// Run a complete fishing simulation over a price series.
///
/// Takes a zone, a list of market conditions (one per candle/tick), and
/// configuration. Returns the complete simulation result.
///
/// This is the primary entry point for the replay pipeline to call.
pub fn run_fishing_simulation(
    zone: &MemoryZone,
    is_long: bool,
    candles: &[MarketConditions],
    config: &FishingLadderConfig,
) -> FishingSimResult {
    if candles.is_empty() {
        return FishingSimResult::default();
    }

    let mut sim = FishingSimulator::new(config.clone(), is_long);

    // Place ladder at the first candle's price
    let first_price = candles[0].price;
    let first_ts = candles[0].timestamp_ms;
    sim.place_ladder(zone, 0, first_price, first_ts);

    // Process each candle
    for candle in candles {
        sim.tick(candle);
    }

    sim.finalize()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquidation::LiquidationZone;

    // Helper: create a default test config
    fn test_config() -> FishingLadderConfig {
        FishingLadderConfig {
            zone_offset_bps: 10.0,
            num_tranches: 3,
            tranche_spacing_bps: 5.0,
            max_total_risk_usd: 300.0,
            expiry_secs: 300.0,
            cancel_on_decay_threshold: 0.7,
            cancel_on_cascade_threshold: true,
            maker_fee_bps: 2.0,
            taker_fee_bps: 5.0,
            sl_offset_bps: 50.0,
            tp_offset_bps: 100.0,
            route_cost_bps: 3.0,
            max_spread_pct: 0.15,
            min_depth_usd: 10_000.0,
        }
    }

    // Helper: create a test memory zone centered at `zone_mid` with "long" side_at_risk
    fn test_zone(zone_mid: f64) -> MemoryZone {
        let base = LiquidationZone {
            price: zone_mid,
            side_at_risk: "long".to_string(),
            estimated_notional_usd: 100_000.0,
            wallet_count: 5,
            distance_bps: 200.0,
            confidence: 0.8,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        MemoryZone::from_liquidation_zone(&base, 1_700_000_000_000, 20.0)
    }

    // Helper: create a short-side zone
    fn test_zone_short(zone_mid: f64) -> MemoryZone {
        let base = LiquidationZone {
            price: zone_mid,
            side_at_risk: "short".to_string(),
            estimated_notional_usd: 100_000.0,
            wallet_count: 5,
            distance_bps: 200.0,
            confidence: 0.8,
            source_mix: vec!["hyperliquid_positions".to_string()],
        };
        MemoryZone::from_liquidation_zone(&base, 1_700_000_000_000, 20.0)
    }

    // Helper: basic market conditions
    fn market_tick(price: f64, high: f64, low: f64, ts: i64) -> MarketConditions {
        MarketConditions {
            price,
            high,
            low,
            timestamp_ms: ts,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        }
    }

    // =======================================================================
    // Config tests
    // =======================================================================

    #[test]
    fn test_config_default_validates() {
        let config = FishingLadderConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_rejects_negative_offset() {
        let mut config = test_config();
        config.zone_offset_bps = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_tranches() {
        let mut config = test_config();
        config.num_tranches = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_negative_spacing() {
        let mut config = test_config();
        config.tranche_spacing_bps = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_risk() {
        let mut config = test_config();
        config.max_total_risk_usd = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_zero_expiry() {
        let mut config = test_config();
        config.expiry_secs = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_rejects_out_of_range_decay_threshold() {
        let mut config = test_config();
        config.cancel_on_decay_threshold = 1.5;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_tranche_size() {
        let config = test_config();
        // 300.0 / 3 = 100.0
        assert!((config.tranche_size_usd() - 100.0).abs() < 0.001);
    }

    // =======================================================================
    // VAL-FISHING-001: Ladder placement at configured offsets
    // =======================================================================

    #[test]
    fn test_ladder_placement_long_offsets() {
        let config = test_config();
        let zone = test_zone(100.0);
        let zone_mid = (zone.low + zone.high) / 2.0;

        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        assert!(sim.is_ladder_placed());
        assert_eq!(sim.orders().len(), 3);

        // Order 0: zone_mid * (1 - 10/10000) = zone_mid * 0.999
        let expected_0 = zone_mid * (1.0 - 10.0 / 10_000.0);
        assert!((sim.orders()[0].price - expected_0).abs() < 0.001);

        // Order 1: zone_mid * (1 - 15/10000) = zone_mid * 0.9985
        let expected_1 = zone_mid * (1.0 - 15.0 / 10_000.0);
        assert!((sim.orders()[1].price - expected_1).abs() < 0.001);

        // Order 2: zone_mid * (1 - 20/10000) = zone_mid * 0.998
        let expected_2 = zone_mid * (1.0 - 20.0 / 10_000.0);
        assert!((sim.orders()[2].price - expected_2).abs() < 0.001);

        // All orders should be active
        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::Active);
        }

        // Total risk should equal max_total_risk_usd
        let total_risk: f64 = sim.orders().iter().map(|o| o.size_usd).sum();
        assert!((total_risk - config.max_total_risk_usd).abs() < 0.01);
    }

    #[test]
    fn test_ladder_placement_short_offsets() {
        let config = test_config();
        let zone = test_zone_short(100.0);
        let zone_mid = (zone.low + zone.high) / 2.0;

        let mut sim = FishingSimulator::new(config, false);
        sim.place_ladder(&zone, 0, 99.0, 1_700_000_000_000);

        assert_eq!(sim.orders().len(), 3);

        // Short: orders ABOVE zone midpoint
        let expected_0 = zone_mid * (1.0 + 10.0 / 10_000.0);
        assert!((sim.orders()[0].price - expected_0).abs() < 0.001);

        let expected_1 = zone_mid * (1.0 + 15.0 / 10_000.0);
        assert!((sim.orders()[1].price - expected_1).abs() < 0.001);

        let expected_2 = zone_mid * (1.0 + 20.0 / 10_000.0);
        assert!((sim.orders()[2].price - expected_2).abs() < 0.001);
    }

    #[test]
    fn test_ladder_order_count_matches_config() {
        let mut config = test_config();
        config.num_tranches = 5;
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);
        assert_eq!(sim.orders().len(), 5);
    }

    // =======================================================================
    // VAL-FISHING-002: Partial fill modeling
    // =======================================================================

    #[test]
    fn test_partial_fill_tracking() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Manually apply a partial fill
        let order_price = sim.orders()[0].price;
        let order = &mut sim.orders_mut()[0];
        order.apply_fill(order_price, 50.0, true, &config);

        assert_eq!(order.status, OrderStatus::PartiallyFilled);
        assert!((order.filled_usd - 50.0).abs() < 0.001);
        assert!((order.remaining_usd() - 50.0).abs() < 0.001);
        assert!(order.has_fill());
        assert!(order.is_active());
    }

    #[test]
    fn test_full_fill_updates_status() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let order = &mut sim.orders_mut()[0];
        let fill_amount = order.size_usd;
        order.apply_fill(order.price, fill_amount, true, &config);

        assert_eq!(order.status, OrderStatus::Filled);
        assert!((order.remaining_usd()).abs() < 0.001);
    }

    // =======================================================================
    // VAL-FISHING-003: Adverse selection tracking
    // =======================================================================

    #[test]
    fn test_adverse_selection_rate() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Fill all 3 orders
        let zone_mid = (zone.low + zone.high) / 2.0;
        for order in sim.orders_mut() {
            order.apply_fill(order.price, order.size_usd, true, &config);
        }

        // Mark 2 as adversely selected
        sim.mark_adverse_selection(0);
        sim.mark_adverse_selection(1);

        let result = sim.finalize();
        assert_eq!(result.adverse_fills, 2);
        assert_eq!(result.total_fills, 3);
        assert!((result.adverse_selection_rate - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_adverse_selection_zero_when_no_fills() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let result = sim.finalize();
        assert_eq!(result.adverse_fills, 0);
        assert_eq!(result.total_fills, 0);
        assert!((result.adverse_selection_rate).abs() < 0.001);
    }

    // =======================================================================
    // VAL-FISHING-004: Cancel on zone decay
    // =======================================================================

    #[test]
    fn test_cancel_on_zone_decay() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Decay zone 0 above threshold
        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![(0, 0.8)], // zone 0 decayed to 0.8 > 0.7
        };

        sim.tick(&market);

        // All orders at zone 0 should be cancelled due to decay
        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::CancelledDecay);
        }
    }

    #[test]
    fn test_no_cancel_when_decay_below_threshold() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![(0, 0.3)], // below 0.7 threshold
        };

        sim.tick(&market);

        // Orders should still be active (or filled if price hit them)
        for order in sim.orders() {
            assert!(order.status == OrderStatus::Active || order.status == OrderStatus::Filled);
        }
    }

    // =======================================================================
    // VAL-FISHING-005: Cancel on cascade signal
    // =======================================================================

    #[test]
    fn test_cancel_on_cascade() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true, // Cascade!
            zone_decay_scores: vec![],
        };

        sim.tick(&market);

        // All orders should be cancelled
        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::CancelledCascade);
        }
    }

    #[test]
    fn test_no_fills_after_cascade() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // First tick: cascade
        let cascade_market = MarketConditions {
            price: 99.0,
            high: 99.5,
            low: 98.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true,
            zone_decay_scores: vec![],
        };
        sim.tick(&cascade_market);

        // All orders cancelled
        for order in sim.orders() {
            assert!(order.status == OrderStatus::CancelledCascade);
            assert!(!order.has_fill());
        }
    }

    // =======================================================================
    // VAL-FISHING-006: Post-fill SL/TP
    // =======================================================================

    #[test]
    fn test_post_fill_sl_hit_long() {
        // Long fishing: SL when price drops below fill_price * (1 - sl_offset_bps/10000)
        let result = evaluate_sl_tp(100.0, 99.4, true, 50.0, 100.0);
        // SL at 100 * (1 - 50/10000) = 99.5, current 99.4 < 99.5 → SL hit
        assert_eq!(result, SlTpResult::StopLossHit);
    }

    #[test]
    fn test_post_fill_tp_hit_long() {
        // Long fishing: TP when price rises above fill_price * (1 + tp_offset_bps/10000)
        let result = evaluate_sl_tp(100.0, 101.1, true, 50.0, 100.0);
        // TP at 100 * (1 + 100/10000) = 101.0, current 101.1 > 101.0 → TP hit
        assert_eq!(result, SlTpResult::TakeProfitHit);
    }

    #[test]
    fn test_post_fill_no_hit_long() {
        let result = evaluate_sl_tp(100.0, 100.5, true, 50.0, 100.0);
        assert_eq!(result, SlTpResult::NoHit);
    }

    #[test]
    fn test_post_fill_sl_hit_short() {
        // Short fishing: SL when price rises above fill_price * (1 + sl_offset_bps/10000)
        let result = evaluate_sl_tp(100.0, 100.6, false, 50.0, 100.0);
        // SL at 100 * (1 + 50/10000) = 100.5, current 100.6 > 100.5 → SL hit
        assert_eq!(result, SlTpResult::StopLossHit);
    }

    #[test]
    fn test_post_fill_tp_hit_short() {
        // Short fishing: TP when price drops below fill_price * (1 - tp_offset_bps/10000)
        let result = evaluate_sl_tp(100.0, 98.9, false, 50.0, 100.0);
        // TP at 100 * (1 - 100/10000) = 99.0, current 98.9 < 99.0 → TP hit
        assert_eq!(result, SlTpResult::TakeProfitHit);
    }

    #[test]
    fn test_sl_tp_in_simulation_tick() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Fill order 0 by pushing price low enough
        let zone_mid = (zone.low + zone.high) / 2.0;
        let fill_price = sim.orders()[0].price;

        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: fill_price - 0.1, // trigger fill
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Order should be filled
        assert!(sim.orders()[0].has_fill());

        // Now push price to TP level
        let tp_price = fill_price * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.1,
            high: tp_price + 0.2,
            low: fill_price,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        let results = sim.tick(&tp_market);

        // Should have a TP hit
        assert!(results.iter().any(|(_, r)| *r == SlTpResult::TakeProfitHit));
    }

    // =======================================================================
    // VAL-FISHING-006a: Maker/taker fee assumptions modeled correctly
    // =======================================================================

    #[test]
    fn test_maker_fee_deduction() {
        let config = test_config(); // maker_fee_bps = 2.0
        let mut order = FishingOrder::new(100.0, 100.0, 1_700_000_000_000, 0);
        order.apply_fill(100.0, 100.0, true, &config); // maker fill

        // Fee = 100.0 * (2.0 / 10000) = 0.02
        assert!((order.fee_usd - 0.02).abs() < 0.001);
        assert_eq!(order.is_maker_fill, Some(true));
    }

    #[test]
    fn test_taker_fee_deduction() {
        let config = test_config(); // taker_fee_bps = 5.0
        let mut order = FishingOrder::new(100.0, 100.0, 1_700_000_000_000, 0);
        order.apply_fill(100.0, 100.0, false, &config); // taker fill

        // Fee = 100.0 * (5.0 / 10000) = 0.05
        assert!((order.fee_usd - 0.05).abs() < 0.001);
        assert_eq!(order.is_maker_fill, Some(false));
    }

    #[test]
    fn test_fee_differential_reflected_in_pnl() {
        let config = test_config();
        let size = 100.0;

        // Maker fill
        let mut maker_order = FishingOrder::new(100.0, size, 1_700_000_000_000, 0);
        maker_order.apply_fill(100.0, size, true, &config);

        // Taker fill
        let mut taker_order = FishingOrder::new(100.0, size, 1_700_000_000_000, 0);
        taker_order.apply_fill(100.0, size, false, &config);

        // The difference in fees should match the fee differential
        let fee_diff = taker_order.fee_usd - maker_order.fee_usd;
        let expected_diff = size * ((config.taker_fee_bps - config.maker_fee_bps) / 10_000.0);
        assert!((fee_diff - expected_diff).abs() < 0.001);
    }

    // =======================================================================
    // VAL-FISHING-006b: Missed fills tracked
    // =======================================================================

    #[test]
    fn test_missed_winners_tracked_after_cascade_cancel() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Cancel orders via cascade
        let cancel_market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true,
            zone_decay_scores: vec![],
        };
        sim.tick(&cancel_market);

        // Now price drops below the cancelled order prices and recovers (missed winner)
        let order_price = sim.orders()[0].price;
        let missed_market = MarketConditions {
            price: 101.0, // price recovered above order price
            high: 101.5,
            low: order_price - 0.5, // price went below cancelled order
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&missed_market);

        let result = sim.finalize();
        assert!(result.missed_winners >= 1, "Should have at least 1 missed winner");
    }

    #[test]
    fn test_missed_losers_tracked() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Cancel orders via cascade
        let cancel_market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true,
            zone_decay_scores: vec![],
        };
        sim.tick(&cancel_market);

        // Price drops below order but stays below (missed loser)
        let order_price = sim.orders()[0].price;
        let missed_market = MarketConditions {
            price: order_price - 1.0, // price stays below order price → would have been loser
            high: order_price - 0.5,
            low: order_price - 2.0,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&missed_market);

        let result = sim.finalize();
        assert!(result.missed_losers >= 1, "Should have at least 1 missed loser");
    }

    // =======================================================================
    // VAL-FISHING-006c: Route cost integration
    // =======================================================================

    #[test]
    fn test_route_cost_deducted_from_fill() {
        let config = test_config(); // route_cost_bps = 3.0
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let fill_price = sim.orders()[0].price;
        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: fill_price - 0.1,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        let order = &sim.orders()[0];
        assert!(order.has_fill());

        // Route cost = filled_usd * (route_cost_bps / 10000)
        let expected_route_cost = order.filled_usd * (config.route_cost_bps / 10_000.0);
        assert!((order.route_cost_usd - expected_route_cost).abs() < 0.001);
    }

    #[test]
    fn test_route_cost_in_final_pnl() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let fill_price = sim.orders()[0].price;
        let order_size = sim.orders()[0].size_usd;

        // Fill order
        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: fill_price - 0.1,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Verify route cost deducted from fill
        let order = &sim.orders()[0];
        let expected_route_cost = order.filled_usd * (config.route_cost_bps / 10_000.0);
        assert!((order.route_cost_usd - expected_route_cost).abs() < 0.001);

        // Trigger TP
        let tp_price = fill_price * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.1,
            high: tp_price + 0.2,
            low: fill_price,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&tp_market);

        // Check PnL relationship before finalize
        let order = &sim.orders()[0];
        if order.has_fill() {
            let expected_net = order.gross_pnl_usd - order.fee_usd - order.route_cost_usd;
            assert!((order.net_pnl_usd - expected_net).abs() < 0.01);
        }

        let result = sim.finalize();
        assert!(result.total_route_cost_usd > 0.0, "Route cost should be > 0");
    }

    // =======================================================================
    // VAL-FISHING-006d: Spread/depth degradation cancels orders
    // =======================================================================

    #[test]
    fn test_cancel_on_spread_degradation() {
        let config = test_config(); // max_spread_pct = 0.15
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.20, // > 0.15
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::CancelledSpread);
        }
    }

    #[test]
    fn test_cancel_on_depth_degradation() {
        let config = test_config(); // min_depth_usd = 10_000.0
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05, // OK
            depth_usd: 5_000.0, // < 10_000
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::CancelledDepth);
        }
    }

    #[test]
    fn test_no_cancel_when_spread_and_depth_ok() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5, // doesn't trigger fills
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05, // OK
            depth_usd: 50_000.0, // OK
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::Active);
        }
    }

    // =======================================================================
    // VAL-FISHING-007: Market-entry vs fishing-entry expectancy comparison
    // =======================================================================

    #[test]
    fn test_expectancy_comparison_output() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let fill_price = sim.orders()[0].price;

        // Fill order 0
        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: fill_price - 0.1,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Trigger TP
        let tp_price = fill_price * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.1,
            high: tp_price + 0.2,
            low: fill_price,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&tp_market);

        let result = sim.finalize();

        // Both expectancy values should be populated
        // Fishing should have positive expectancy for this TP scenario
        // Market entry should also have positive but potentially different
        assert!(result.total_orders > 0);
        assert!(result.expectancy_fishing != 0.0 || result.expectancy_market != 0.0);

        // Delta should be computed
        let expected_delta = result.expectancy_fishing - result.expectancy_market;
        assert!((result.expectancy_delta - expected_delta).abs() < 0.001);
    }

    #[test]
    fn test_fishing_improvement_over_market() {
        // Fishing at a better price with maker fees should improve expectancy
        let config = test_config();
        let zone = test_zone(100.0);
        let zone_mid = (zone.low + zone.high) / 2.0;

        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Fill order 0 at a price below market (improvement)
        let fill_price = sim.orders()[0].price;

        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: fill_price - 0.1,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Trigger TP
        let tp_price = fill_price * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.1,
            high: tp_price + 0.2,
            low: fill_price,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&tp_market);

        let result = sim.finalize();

        // Fishing entry at a lower price with maker fees should have better expectancy
        // than market entry at higher price with taker fees
        assert!(result.expectancy_fishing > result.expectancy_market,
            "Fishing expectancy ({}) should be > market expectancy ({})",
            result.expectancy_fishing, result.expectancy_market);
    }

    // =======================================================================
    // Order expiry tests
    // =======================================================================

    #[test]
    fn test_order_expiry() {
        let config = test_config(); // expiry_secs = 300
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Advance time beyond expiry (301 seconds)
        let expired_market = MarketConditions {
            price: 101.0, // price stays above order, no fill
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000 + 301_000, // 301 seconds later
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&expired_market);

        let result = sim.finalize();
        assert!(result.expired_orders > 0, "Should have expired orders");
    }

    #[test]
    fn test_order_not_expired_before_timeout() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Within expiry window (200 seconds < 300)
        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000 + 200_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        for order in sim.orders() {
            assert_ne!(order.status, OrderStatus::Expired);
        }
    }

    // =======================================================================
    // Standalone simulation function test
    // =======================================================================

    #[test]
    fn test_run_fishing_simulation() {
        let config = test_config();
        let zone = test_zone(100.0);

        let candles = vec![
            market_tick(101.0, 101.5, 100.5, 1_700_000_000_000),
            market_tick(100.0, 100.5, 99.0, 1_700_000_000_100),   // drops to fill range
            market_tick(101.5, 102.0, 101.0, 1_700_000_000_200),  // recovers to TP
        ];

        let result = run_fishing_simulation(&zone, true, &candles, &config);

        assert!(result.total_orders > 0);
        assert!(result.filled_orders > 0, "Should have fills from the price drop");
    }

    #[test]
    fn test_run_fishing_simulation_empty_candles() {
        let config = test_config();
        let zone = test_zone(100.0);
        let candles: Vec<MarketConditions> = vec![];

        let result = run_fishing_simulation(&zone, true, &candles, &config);
        assert_eq!(result.total_orders, 0);
    }

    // =======================================================================
    // Fill rate computation
    // =======================================================================

    #[test]
    fn test_fill_rate_computation() {
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Use a candle that only triggers the first order (highest buy for long fishing)
        // For long fishing: order 0 is highest buy. Set low just below order 0 but
        // above order 1 and 2 (which are lower).
        let fill_price_0 = sim.orders()[0].price;
        let fill_price_1 = sim.orders()[1].price;
        // Set low between order 0 and order 1 prices
        let low_price = (fill_price_0 + fill_price_1) / 2.0;
        let fill_market = MarketConditions {
            price: fill_price_0 + 0.01, // price is above order 0
            high: 101.0,
            low: low_price, // triggers only order 0
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        let result = sim.finalize();
        // 1 filled out of 3
        assert_eq!(result.filled_orders, 1);
        assert!((result.fill_rate - 1.0 / 3.0).abs() < 0.01);
    }

    // =======================================================================
    // Edge case: cascade cancellation disabled
    // =======================================================================

    #[test]
    fn test_no_cascade_cancel_when_disabled() {
        let mut config = test_config();
        config.cancel_on_cascade_threshold = false;

        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        // Orders should NOT be cancelled (cascade cancel disabled)
        for order in sim.orders() {
            assert_ne!(order.status, OrderStatus::CancelledCascade);
        }
    }

    // =======================================================================
    // OrderStatus display test
    // =======================================================================

    #[test]
    fn test_order_status_display() {
        assert_eq!(OrderStatus::Active.to_string(), "active");
        assert_eq!(OrderStatus::Filled.to_string(), "filled");
        assert_eq!(OrderStatus::PartiallyFilled.to_string(), "partially_filled");
        assert_eq!(OrderStatus::CancelledDecay.to_string(), "cancelled_decay");
        assert_eq!(OrderStatus::CancelledCascade.to_string(), "cancelled_cascade");
        assert_eq!(OrderStatus::Expired.to_string(), "expired");
        assert_eq!(OrderStatus::CancelledSpread.to_string(), "cancelled_spread");
        assert_eq!(OrderStatus::CancelledDepth.to_string(), "cancelled_depth");
        assert_eq!(OrderStatus::Missed.to_string(), "missed");
    }

    // =======================================================================
    // Multiple zone scenario
    // =======================================================================

    #[test]
    fn test_multiple_zones_decay_independent() {
        let config = test_config();
        let zone0 = test_zone(100.0);
        let zone1 = test_zone(95.0);

        let mut sim = FishingSimulator::new(config.clone(), true);

        // Place ladder at zone 0
        sim.place_ladder(&zone0, 0, 101.0, 1_700_000_000_000);

        // Place ladder at zone 1 (in a real scenario this would be a separate sim,
        // but we test zone-specific decay by having orders at different zone IDs)
        // For this test, we manually add orders for zone 1
        let zone1_mid = (zone1.low + zone1.high) / 2.0;
        for i in 0..3 {
            let offset = (config.zone_offset_bps + (i as f64) * config.tranche_spacing_bps) / 10_000.0;
            let price = zone1_mid * (1.0 - offset);
            let order = FishingOrder::new(price, config.tranche_size_usd(), 1_700_000_000_000, i);
            sim.orders.push(order);
            sim.order_zone_ids.push(1); // zone 1
        }
        sim.result.total_orders = sim.orders.len();

        // Decay only zone 0
        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![(0, 0.8)], // only zone 0 decayed
        };
        sim.tick(&market);

        // Zone 0 orders cancelled, zone 1 orders still active
        for (i, order) in sim.orders().iter().enumerate() {
            if sim.order_zone_ids[i] == 0 {
                assert_eq!(order.status, OrderStatus::CancelledDecay);
            } else {
                assert_eq!(order.status, OrderStatus::Active);
            }
        }
    }

    // =======================================================================
    // Serialization test
    // =======================================================================

    #[test]
    fn test_fishing_sim_result_serialization() {
        let result = FishingSimResult {
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
            expectancy_delta: 3.0,
            cancelled_decay: 1,
            cancelled_cascade: 0,
            cancelled_spread: 0,
            cancelled_depth: 0,
            expired_orders: 2,
            sl_hit_count: 1,
            tp_hit_count: 2,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: FishingSimResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_fishing_order_serialization() {
        let order = FishingOrder {
            price: 100.0,
            size_usd: 100.0,
            filled_usd: 50.0,
            fill_price: Some(100.0),
            adverse_selected: false,
            status: OrderStatus::PartiallyFilled,
            placed_at_ms: 1_700_000_000_000,
            tranche_index: 0,
            is_maker_fill: Some(true),
            fee_usd: 0.01,
            route_cost_usd: 0.015,
            gross_pnl_usd: 0.5,
            net_pnl_usd: 0.475,
            cancel_reason: None,
        };

        let json = serde_json::to_string(&order).unwrap();
        let deserialized: FishingOrder = serde_json::from_str(&json).unwrap();
        assert_eq!(order, deserialized);
    }

    // =======================================================================
    // Full end-to-end simulation test
    // =======================================================================

    #[test]
    fn test_full_simulation_long_tp() {
        let config = test_config();
        let zone = test_zone(100.0);

        // Scenario: Place ladder, fill all orders, hit TP on all
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let fill_price_0 = sim.orders()[0].price;

        // Fill all orders by dropping price significantly
        let fill_market = MarketConditions {
            price: fill_price_0 - 0.5,
            high: 101.0,
            low: sim.orders()[2].price - 0.5, // drop below all order prices
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Verify all filled
        for order in sim.orders() {
            assert!(order.has_fill(), "Order should be filled");
        }

        // Push price up to TP
        let tp_price = fill_price_0 * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.5,
            high: tp_price + 1.0,
            low: fill_price_0,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&tp_market);

        let result = sim.finalize();

        assert_eq!(result.filled_orders, 3);
        assert!(result.tp_hit_count > 0);
        assert!(result.total_net_pnl_usd > 0.0, "Should have positive net PnL on TP");
    }

    #[test]
    fn test_full_simulation_long_sl() {
        let config = test_config();
        let zone = test_zone(100.0);

        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        let fill_price_0 = sim.orders()[0].price;

        // Fill all orders
        let fill_market = MarketConditions {
            price: fill_price_0 - 0.5,
            high: 101.0,
            low: sim.orders()[2].price - 0.5,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);

        // Push price down to SL
        let sl_price = fill_price_0 * (1.0 - config.sl_offset_bps / 10_000.0);
        let sl_market = MarketConditions {
            price: sl_price - 0.5,
            high: fill_price_0,
            low: sl_price - 1.0,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&sl_market);

        let result = sim.finalize();

        assert_eq!(result.filled_orders, 3);
        assert!(result.sl_hit_count > 0);
        assert!(result.total_net_pnl_usd < 0.0, "Should have negative net PnL on SL");
    }

    // =======================================================================
    // Regression: TP re-counting bug fix
    // =======================================================================

    #[test]
    fn test_tp_hit_not_recounted_on_subsequent_ticks() {
        // BUG: adverse_selected not set after TP hit, causing duplicate TP
        // counts on subsequent ticks. Fix: set adverse_selected = true in
        // TakeProfitHit branch to prevent re-evaluation.
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config.clone(), true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Fill ONLY order 0 by setting low between order 0 and order 1 prices
        let fill_price_0 = sim.orders()[0].price;
        let fill_price_1 = sim.orders()[1].price;
        let fill_market = MarketConditions {
            price: 100.5,
            high: 101.0,
            low: (fill_price_0 + fill_price_1) / 2.0, // between order 0 and 1
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&fill_market);
        assert!(sim.orders()[0].has_fill());
        assert!(!sim.orders()[1].has_fill(), "Only order 0 should be filled");
        assert!(!sim.orders()[2].has_fill(), "Only order 0 should be filled");

        // Tick 1: push price to TP level for order 0
        let tp_price = fill_price_0 * (1.0 + config.tp_offset_bps / 10_000.0);
        let tp_market = MarketConditions {
            price: tp_price + 0.5,
            high: tp_price + 1.0,
            low: fill_price_0,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        let results = sim.tick(&tp_market);
        assert!(results.iter().any(|(_, r)| *r == SlTpResult::TakeProfitHit),
            "First tick at TP should trigger TakeProfitHit");

        // Tick 2: price still above TP — should NOT trigger another TP count
        let tp_market2 = MarketConditions {
            price: tp_price + 1.0,
            high: tp_price + 2.0,
            low: tp_price + 0.5,
            timestamp_ms: 1_700_000_000_300,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        let results2 = sim.tick(&tp_market2);
        assert!(results2.is_empty(),
            "Subsequent tick at TP should NOT trigger another TP hit");

        // Tick 3: yet another tick at TP — still should not re-trigger
        let tp_market3 = MarketConditions {
            price: tp_price + 2.0,
            high: tp_price + 3.0,
            low: tp_price + 1.0,
            timestamp_ms: 1_700_000_000_400,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        let results3 = sim.tick(&tp_market3);
        assert!(results3.is_empty(),
            "Third tick at TP should NOT trigger another TP hit");

        // Verify the order is marked as adversely selected (preventing re-evaluation)
        assert!(sim.orders()[0].adverse_selected,
            "Order should have adverse_selected=true after TP hit to prevent re-evaluation");

        let result = sim.finalize();
        assert_eq!(result.tp_hit_count, 1,
            "TP should be counted exactly once, got {}", result.tp_hit_count);
    }

    // =======================================================================
    // Regression: Missed fills double-counting bug fix
    // =======================================================================

    #[test]
    fn test_missed_fill_recorded_once_not_on_every_tick() {
        // BUG: cancelled/expired orders checked for missed fills on every tick
        // without deduplication. Fix: after recording a missed fill, change
        // order status to OrderStatus::Missed to prevent re-counting.
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Cancel orders via cascade
        let cancel_market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: 100.5,
            timestamp_ms: 1_700_000_000_000,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: true,
            zone_decay_scores: vec![],
        };
        sim.tick(&cancel_market);

        // Verify all orders are cancelled
        for order in sim.orders() {
            assert_eq!(order.status, OrderStatus::CancelledCascade);
        }

        let order_price = sim.orders()[0].price;

        // Tick 1: price drops below cancelled order and recovers → missed winner
        let missed_market1 = MarketConditions {
            price: 101.0, // recovered above order price → would have been profitable
            high: 101.5,
            low: order_price - 0.5, // price went below cancelled order
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&missed_market1);

        // After the first tick recording the missed fill, order status should change
        assert_eq!(sim.orders()[0].status, OrderStatus::Missed,
            "Order status should change to Missed after recording missed fill");

        // Tick 2: same conditions — should NOT double-count
        let missed_market2 = MarketConditions {
            price: 101.5,
            high: 102.0,
            low: order_price - 1.0,
            timestamp_ms: 1_700_000_000_200,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&missed_market2);

        // Tick 3: same conditions again — still should NOT triple-count
        let missed_market3 = MarketConditions {
            price: 102.0,
            high: 102.5,
            low: order_price - 1.5,
            timestamp_ms: 1_700_000_000_300,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&missed_market3);

        let result = sim.finalize();

        // Each cancelled order should contribute at most 1 missed fill
        // With 3 cancelled orders, missed_winners should be exactly 3 (one per order)
        // NOT 9 (3 orders × 3 ticks)
        assert_eq!(result.missed_winners, 3,
            "Each order should be counted as missed winner exactly once, got {}",
            result.missed_winners);
    }

    #[test]
    fn test_missed_fill_status_prevents_recount() {
        // Direct test: an order with status Missed should not be re-evaluated
        // for missed fills on subsequent ticks.
        let config = test_config();
        let zone = test_zone(100.0);
        let mut sim = FishingSimulator::new(config, true);
        sim.place_ladder(&zone, 0, 101.0, 1_700_000_000_000);

        // Manually set an order to Missed status
        sim.orders_mut()[0].status = OrderStatus::Missed;
        sim.orders_mut()[0].cancel_reason = Some("test_missed".to_string());

        let order_price = sim.orders()[0].price;

        // Tick with price passing through the missed order's level
        let market = MarketConditions {
            price: 101.0,
            high: 101.5,
            low: order_price - 0.5,
            timestamp_ms: 1_700_000_000_100,
            spread_pct: 0.05,
            depth_usd: 50_000.0,
            cascade_detected: false,
            zone_decay_scores: vec![],
        };
        sim.tick(&market);

        let result = sim.finalize();
        // The manually-set Missed order should NOT be counted again
        // (only the other 2 active orders might contribute if they get cancelled)
        assert_eq!(result.missed_winners, 0,
            "Already-Missed order should not be counted again");
    }
}
