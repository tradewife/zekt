//! Hyperliquid paper trading engine.
//!
//! `HlPaperEngine` simulates perpetual futures trading on Hyperliquid using
//! live price and funding-rate data from any `MarketDataProvider`.  It
//! maintains a position matrix keyed by `(strategy_name, market)`, accrues
//! realistic borrow fees each tick, and writes an atomic JSON report on
//! shutdown.
//!
//! Fee model (mirrors HL mainnet):
//! - Taker entry / exit fee: **0.035 %** of notional
//! - Hourly borrow fee: **0.01 %** of notional (accrued per tick)

use crate::funding_capture::{FundingRateCaptureStrategy, FundingSnapshot};
use crate::market_data::{
    MarketDataProvider, HL_BORROW_RATE_PER_HOUR, HL_TAKER_FEE_RATE,
};
use crate::signal::{ExitReason, Signal};
use crate::strategy::{PositionContext, Strategy};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// A simulated HL perp position with full fee tracking.
#[derive(Debug, Clone)]
pub(crate) struct HlPaperPosition {
    market: String,
    strategy_name: String,
    is_long: bool,
    entry_price: f64,
    size_usd: f64,
    entry_fee: f64,
    accrued_borrow_fee: f64,
    /// Highest price seen (for long trailing) or lowest (for short trailing).
    peak_price: f64,
    entry_time_ms: i64,
}

impl HlPaperPosition {
    /// Unrealised PnL as a fraction of `size_usd`.
    fn unrealized_pnl_pct(&self, current_price: f64) -> f64 {
        if self.entry_price == 0.0 {
            return 0.0;
        }
        if self.is_long {
            (current_price - self.entry_price) / self.entry_price * 100.0
        } else {
            (self.entry_price - current_price) / self.entry_price * 100.0
        }
    }

    /// Unrealised PnL in USD.
    fn unrealized_pnl_usd(&self, current_price: f64) -> f64 {
        self.size_usd * self.unrealized_pnl_pct(current_price) / 100.0
    }

    /// Hold duration in seconds (wall-clock).
    fn hold_secs(&self) -> u64 {
        let now_ms = Utc::now().timestamp_millis();
        ((now_ms - self.entry_time_ms).max(0) / 1000) as u64
    }

    /// Accrue borrow fee for one tick.
    fn accrue_borrow(&mut self, poll_interval_secs: u64) {
        let hours = poll_interval_secs as f64 / 3600.0;
        self.accrued_borrow_fee += self.size_usd * HL_BORROW_RATE_PER_HOUR * hours;
    }

    /// Update peak price (trailing-stop tracking).
    fn update_peak(&mut self, price: f64) {
        if self.is_long {
            if price > self.peak_price {
                self.peak_price = price;
            }
        } else if self.peak_price == 0.0 || price < self.peak_price {
            self.peak_price = price;
        }
    }
}

// ---------------------------------------------------------------------------
// Cell key
// ---------------------------------------------------------------------------

/// Position-matrix key: (strategy_name, market).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CellKey {
    strategy: String,
    market: String,
}

impl std::fmt::Display for CellKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.strategy, self.market)
    }
}

// ---------------------------------------------------------------------------
// Trade record (local, for report)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HlTradeRecord {
    strategy: String,
    market: String,
    direction: String,
    entry_price: f64,
    exit_price: f64,
    size_usd: f64,
    gross_pnl: f64,
    entry_fee: f64,
    exit_fee: f64,
    borrow_fee: f64,
    net_pnl: f64,
    hold_secs: u64,
    exit_reason: String,
    timestamp: String,
}

// ---------------------------------------------------------------------------
// Per-market stats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct MarketStats {
    market: String,
    trades: usize,
    wins: usize,
    losses: usize,
    pnl: f64,
    gross_pnl: f64,
    entry_fees: f64,
    exit_fees: f64,
    borrow_fees: f64,
    total_fees: f64,
    funding_captured: f64,
    win_rate: f64,
    /// Number of currently open positions for this market.
    open_position_count: usize,
    /// Total notional of open positions for this market.
    open_notional_usd: f64,
    /// Sum of unrealized PnL across open positions for this market.
    unrealized_pnl: f64,
    /// Individual trade PnLs for Sharpe.
    #[serde(skip_serializing)]
    trade_pnls: Vec<f64>,
}

impl MarketStats {
    fn record(
        &mut self,
        net_pnl: f64,
        gross_pnl: f64,
        entry_fee: f64,
        exit_fee: f64,
        borrow_fee: f64,
    ) {
        self.trades += 1;
        self.trade_pnls.push(net_pnl);
        self.pnl += net_pnl;
        self.gross_pnl += gross_pnl;
        self.entry_fees += entry_fee;
        self.exit_fees += exit_fee;
        self.borrow_fees += borrow_fee;
        self.total_fees += entry_fee + exit_fee + borrow_fee;
        if net_pnl > 0.0 {
            self.wins += 1;
        } else {
            self.losses += 1;
        }
    }

    fn finalize(&mut self) {
        self.win_rate = if self.trades > 0 {
            self.wins as f64 / self.trades as f64 * 100.0
        } else {
            0.0
        };
    }
}

// ---------------------------------------------------------------------------
// Open position report entry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OpenPositionReport {
    market: String,
    strategy: String,
    direction: String,
    entry_price: f64,
    current_price: f64,
    size_usd: f64,
    collateral_usd: f64,
    unrealized_pnl: f64,
    accrued_borrow_fee: f64,
    entry_fee: f64,
    funding_captured: f64,
    hold_time_secs: u64,
    entry_time: String,
}

// ---------------------------------------------------------------------------
// Summary report
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct HlPaperSummary {
    start_time: String,
    end_time: String,
    initial_balance: f64,
    final_balance: f64,
    net_pnl: f64,
    total_fees: FeeBreakdown,
    total_trades: usize,
    win_rate: f64,
    /// Sharpe ratio — `None` when fewer than 5 trades.
    sharpe_ratio: Option<f64>,
    /// Open positions with unrealized PnL (populated at report time).
    open_positions: Vec<OpenPositionReport>,
    markets: Vec<MarketStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FeeBreakdown {
    taker_fees: f64,
    borrow_fees: f64,
    total: f64,
}

// ---------------------------------------------------------------------------
// Engine configuration
// ---------------------------------------------------------------------------

/// Configuration for `HlPaperEngine`.
#[derive(Debug, Clone)]
pub struct HlPaperConfig {
    pub poll_interval_secs: u64,
    /// Maximum total notional across all positions.
    pub max_total_notional_usd: f64,
}

impl Default for HlPaperConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 300,
            max_total_notional_usd: 50_000.0,
        }
    }
}

// ---------------------------------------------------------------------------
// HlPaperEngine
// ---------------------------------------------------------------------------

/// Paper-trading engine that uses Hyperliquid data via a `MarketDataProvider`.
///
/// Maintains an independent position for each `(strategy, market)` cell,
/// accrues borrow fees per tick, and writes an atomic JSON report.
pub struct HlPaperEngine<P: MarketDataProvider> {
    provider: P,
    /// Per-cell strategy instances (each with independent state/price buffer).
    cell_strategies: HashMap<CellKey, Box<dyn Strategy>>,
    /// Strategy names for iteration order.
    strategy_names: Vec<String>,
    markets: Vec<String>,
    sim_balance: f64,
    initial_balance: f64,
    positions: HashMap<CellKey, HlPaperPosition>,
    trade_log: Vec<HlTradeRecord>,
    market_stats: HashMap<String, MarketStats>,
    poll_interval_secs: u64,
    max_total_notional_usd: f64,
    output_dir: String,
    running: Arc<AtomicBool>,
    start_time: DateTime<Utc>,
    /// Cooldown markers: cell key → cooldown-until timestamp (ms).
    cooldowns: HashMap<CellKey, i64>,
    /// Last known prices per market (updated each tick, used for report).
    last_prices: HashMap<String, f64>,
}

impl<P: MarketDataProvider> HlPaperEngine<P> {
    /// Create a new engine.
    ///
    /// * `provider` — data source (e.g. `HlDataProvider`, `MockDataProvider`).
    /// * `config`   — engine-level parameters (poll interval, exposure limits).
    /// * `strategy_factory` — closure that creates a new strategy by name.
    /// * `strategy_names` — list of strategy names to instantiate.
    /// * `markets`  — list of market symbols to trade.
    /// * `balance`  — starting simulated USDC balance.
    /// * `output_dir` — directory for the JSON report.
    pub fn new(
        provider: P,
        config: HlPaperConfig,
        strategy_names: Vec<String>,
        strategy_factory: &dyn Fn(&str) -> anyhow::Result<Box<dyn Strategy>>,
        markets: Vec<String>,
        balance: f64,
        output_dir: &str,
    ) -> anyhow::Result<Self> {
        let poll_interval_secs = config.poll_interval_secs;
        let max_total_notional_usd = config.max_total_notional_usd;

        let mut market_stats = HashMap::new();
        for m in &markets {
            market_stats.insert(
                m.clone(),
                MarketStats {
                    market: m.clone(),
                    ..Default::default()
                },
            );
        }

        // Create per-cell strategy instances.
        let mut cell_strategies = HashMap::new();
        for strat_name in &strategy_names {
            for market in &markets {
                let key = CellKey {
                    strategy: strat_name.clone(),
                    market: market.clone(),
                };
                let strat = strategy_factory(strat_name)?;
                cell_strategies.insert(key, strat);
            }
        }

        Ok(Self {
            provider,
            cell_strategies,
            strategy_names,
            markets,
            sim_balance: balance,
            initial_balance: balance,
            positions: HashMap::new(),
            trade_log: Vec::new(),
            market_stats,
            poll_interval_secs,
            max_total_notional_usd,
            output_dir: output_dir.to_string(),
            running: Arc::new(AtomicBool::new(true)),
            start_time: Utc::now(),
            cooldowns: HashMap::new(),
            last_prices: HashMap::new(),
        })
    }

    /// Return a clone of the running flag for external shutdown.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Execute one tick of the trading loop.
    ///
    /// 1. Fetch prices and push to strategies.
    /// 2. Fetch funding rates and push to funding-aware strategies.
    /// 3. Entry / exit detection per cell.
    /// 4. Borrow-fee accrual.
    pub async fn tick(&mut self) -> anyhow::Result<()> {
        let now_ms = Utc::now().timestamp_millis();

        // --- 1. Fetch funding rates (single API call) ---
        // The funding rates response includes `mark_px` for every market,
        // so we extract prices from it and only call get_price() as a
        // fallback for markets missing from the funding data.
        let funding_rates = match self.provider.get_funding_rates().await {
            Ok(rates) => rates,
            Err(e) => {
                debug!("[hl-paper] funding-rate fetch error: {:#}", e);
                vec![]
            }
        };

        // --- 2. Build per-market funding lookup + extract prices ---
        let mut funding_by_market: HashMap<String, FundingSnapshot> = HashMap::new();
        let mut prices: HashMap<String, f64> = HashMap::new();
        for fs in &funding_rates {
            let key = fs.coin.to_uppercase();
            funding_by_market.insert(key.clone(), fs.clone());
            // Extract mark price from funding data — avoids a separate get_price() call.
            if fs.mark_px > 0.0 {
                prices.insert(key, fs.mark_px);
            }
        }

        // Fallback: fetch prices for any market not covered by funding data.
        for market in &self.markets {
            if !prices.contains_key(market) {
                match self.provider.get_price(market).await {
                    Ok(price) => {
                        prices.insert(market.clone(), price);
                    }
                    Err(e) => {
                        warn!("[hl-paper] {} price fetch error: {:#}", market, e);
                    }
                }
            }
        }

        // Store last known prices for report computation.
        for (mkt, px) in &prices {
            self.last_prices.insert(mkt.clone(), *px);
        }

        // --- 3 & 4. Process each cell ---
        let keys: Vec<CellKey> = self
            .strategy_names
            .iter()
            .flat_map(|s| {
                self.markets
                    .iter()
                    .map(move |m| CellKey {
                        strategy: s.clone(),
                        market: m.clone(),
                    })
            })
            .collect();

        for key in keys {
            let price = match prices.get(&key.market) {
                Some(p) => *p,
                None => continue,
            };

            // Push price to this cell's strategy.
            if let Some(strat) = self.cell_strategies.get_mut(&key) {
                strat.push_price(price, now_ms);
            }

            // Push market-specific funding to funding-aware strategies.
            if key.strategy == "funding-capture"
                && let Some(fs) = funding_by_market.get(&key.market.to_uppercase())
                && let Some(strat) = self.cell_strategies.get_mut(&key)
            {
                strat.as_any_mut()
                    .downcast_mut::<FundingRateCaptureStrategy>()
                    .unwrap()
                    .push_funding(fs.clone());
            }

            let has_position = self.positions.contains_key(&key);

            if has_position {
                self.manage_cell(&key, price)?;
            } else {
                self.handle_no_position_cell(&key, price)?;
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Entry
    // -----------------------------------------------------------------------

    fn handle_no_position_cell(&mut self, key: &CellKey, current_price: f64) -> anyhow::Result<()> {
        // Check cooldown.
        if let Some(until_ms) = self.cooldowns.get(key) {
            if Utc::now().timestamp_millis() < *until_ms {
                return Ok(());
            } else {
                self.cooldowns.remove(key);
            }
        }

        let strat = match self.cell_strategies.get_mut(key) {
            Some(s) => s,
            None => return Ok(()),
        };
        let params = strat.parameters().clone();
        let clip_size_usd = params.clip_size_usd;
        let leverage = if params.clip_size_usd > 0.0 {
            // Use leverage from the strategy's generic params if exposed.
            // For funding-capture, leverage is embedded in the strategy.
            1.0 // default 1x; strategies can override via size computation
        } else {
            1.0
        };
        let size_usd = clip_size_usd * leverage;

        // Balance check.
        if self.sim_balance < clip_size_usd {
            debug!(
                "[{}] Insufficient balance: ${:.2} < ${:.2}",
                key, self.sim_balance, clip_size_usd
            );
            return Ok(());
        }

        // Total exposure check.
        let current_total: f64 = self.positions.values().map(|p| p.size_usd).sum();
        if current_total + size_usd > self.max_total_notional_usd {
            debug!(
                "[{}] Exposure limit: ${:.2} + ${:.2} > ${:.2}",
                key, current_total, size_usd, self.max_total_notional_usd
            );
            return Ok(());
        }

        let snapshot = strat.snapshot();
        let bias = params.direction_bias.to_lowercase();
        let signal = strat.detect_entry(&snapshot);

        match signal {
            Signal::MomentumLong { strength, velocity_pct } if bias != "short" => {
                self.open_position(key, true, clip_size_usd, leverage, current_price, strength, velocity_pct)?;
            }
            Signal::MomentumShort { strength, velocity_pct } if bias != "long" => {
                self.open_position(key, false, clip_size_usd, leverage, current_price, strength, velocity_pct)?;
            }
            _ => {}
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn open_position(
        &mut self,
        key: &CellKey,
        is_long: bool,
        clip_size_usd: f64,
        leverage: f64,
        current_price: f64,
        strength: f64,
        velocity_pct: f64,
    ) -> anyhow::Result<()> {
        let size_usd = clip_size_usd * leverage;
        let entry_fee = size_usd * HL_TAKER_FEE_RATE;

        // Deduct collateral from simulated balance.
        self.sim_balance -= clip_size_usd;

        let direction = if is_long { "LONG" } else { "SHORT" };
        info!(
            ">>> [hl-paper] [{}] OPEN {} ${:.0} x {:.1}x @ ${:.2} | fee=${:.4} | strength={:.0} vel={:.3}%",
            key, direction, clip_size_usd, leverage, current_price, entry_fee, strength, velocity_pct
        );

        let pos = HlPaperPosition {
            market: key.market.clone(),
            strategy_name: key.strategy.clone(),
            is_long,
            entry_price: current_price,
            size_usd,
            entry_fee,
            accrued_borrow_fee: 0.0,
            peak_price: current_price,
            entry_time_ms: Utc::now().timestamp_millis(),
        };

        self.positions.insert(key.clone(), pos);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Exit / manage
    // -----------------------------------------------------------------------

    fn manage_cell(
        &mut self,
        key: &CellKey,
        current_price: f64,
    ) -> anyhow::Result<()> {
        let pos = match self.positions.get_mut(key) {
            Some(p) => p,
            None => return Ok(()),
        };

        // Accrue borrow fee.
        pos.accrue_borrow(self.poll_interval_secs);
        // Update trailing peak.
        pos.update_peak(current_price);

        let params = match self.cell_strategies.get(key) {
            Some(s) => s.parameters().clone(),
            None => return Ok(()),
        };

        let hold_secs = pos.hold_secs();

        let ctx = PositionContext {
            is_long: pos.is_long,
            entry_price: pos.entry_price,
            current_price,
            peak_price: pos.peak_price,
            hold_secs,
            max_hold_secs: params.max_hold_secs,
            take_profit_pct: params.take_profit_pct,
            stop_loss_pct: params.stop_loss_pct,
            trailing_stop_pct: params.trailing_stop_pct,
            trailing_activation_pct: params.trailing_activation_pct,
        };

        // Software-side SL/TP/trailing (fast exits before strategy signal).
        if let Some(reason) = check_soft_exits(&ctx) {
            // Need to drop borrows before close_position.
            drop(params);
            self.close_position(key, current_price, reason)?;
            return Ok(());
        }

        let strat = match self.cell_strategies.get(key) {
            Some(s) => s,
            None => return Ok(()),
        };
        let snapshot = strat.snapshot();

        let exit_signal = strat.detect_exit(&snapshot, &ctx);

        match exit_signal {
            Some(Signal::ExitLong { reason } | Signal::ExitShort { reason }) => {
                self.close_position(key, current_price, reason)?;
            }
            Some(_) => {}
            None => {
                let pos = self.positions.get(key).unwrap();
                debug!(
                    "[{}] Holding {} {} @ ${:.2} | uPnL=${:.2} ({:.2}%) | fees=${:.4} | hold={}s",
                    key,
                    if pos.is_long { "LONG" } else { "SHORT" },
                    pos.market,
                    current_price,
                    pos.unrealized_pnl_usd(current_price),
                    pos.unrealized_pnl_pct(current_price),
                    pos.entry_fee + pos.accrued_borrow_fee,
                    pos.hold_secs(),
                );
            }
        }

        Ok(())
    }

    fn close_position(
        &mut self,
        key: &CellKey,
        exit_price: f64,
        reason: ExitReason,
    ) -> anyhow::Result<()> {
        let pos = match self.positions.remove(key) {
            Some(p) => p,
            None => return Ok(()),
        };

        // Gross PnL.
        let gross_pnl = pos.unrealized_pnl_usd(exit_price);
        let exit_fee = pos.size_usd * HL_TAKER_FEE_RATE;
        let net_pnl = gross_pnl - exit_fee - pos.accrued_borrow_fee;

        // Recover clip from initial deduction: we deducted clip_size_usd at open.
        // size_usd = clip * leverage, so clip = size_usd / leverage.
        let leverage = match self.cell_strategies.get(key) {
            Some(s) => {
                let clip = s.parameters().clip_size_usd;
                if clip > 0.0 { pos.size_usd / clip } else { 1.0 }
            }
            None => 1.0,
        };
        let clip_collateral = pos.size_usd / leverage.max(1.0);

        self.sim_balance += clip_collateral + net_pnl;

        let direction = if pos.is_long { "LONG" } else { "SHORT" };
        let hold_mins = pos.hold_secs() as f64 / 60.0;

        info!(
            "<<< [hl-paper] [{}] CLOSE {} {} | ${:.2} -> ${:.2} | reason={:?} | hold={:.1}min",
            key, direction, pos.market, pos.entry_price, exit_price, reason, hold_mins
        );
        info!(
            "    gross=${:.2} entry_fee=${:.4} exit_fee=${:.4} borrow_fee=${:.4} | net=${:.2}",
            gross_pnl, pos.entry_fee, exit_fee, pos.accrued_borrow_fee, net_pnl
        );

        // Record trade.
        let record = HlTradeRecord {
            strategy: key.strategy.clone(),
            market: key.market.clone(),
            direction: direction.to_string(),
            entry_price: pos.entry_price,
            exit_price,
            size_usd: pos.size_usd,
            gross_pnl,
            entry_fee: pos.entry_fee,
            exit_fee,
            borrow_fee: pos.accrued_borrow_fee,
            net_pnl,
            hold_secs: pos.hold_secs(),
            exit_reason: format!("{:?}", reason),
            timestamp: Utc::now().to_rfc3339(),
        };
        self.trade_log.push(record);

        // Update market stats.
        if let Some(stats) = self.market_stats.get_mut(&key.market) {
            stats.record(net_pnl, gross_pnl, pos.entry_fee, exit_fee, pos.accrued_borrow_fee);
        }

        // Cooldown after loss.
        if net_pnl < 0.0
            && let Some(strat) = self.cell_strategies.get(key)
        {
            let cooldown_secs = strat.parameters().cooldown_after_loss_secs;
            let until_ms = Utc::now().timestamp_millis() + (cooldown_secs as i64 * 1000);
            self.cooldowns.insert(key.clone(), until_ms);
        }

        info!("[hl-paper] [{}] Balance: ${:.2}", key, self.sim_balance);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Report
    // -----------------------------------------------------------------------

    /// Write the PnL summary to `data/paper-results/hl-paper-summary.json`
    /// using atomic writes (`.tmp` then `rename`).
    pub fn write_report(&self) -> anyhow::Result<()> {
        let mut market_stats: Vec<MarketStats> = self
            .market_stats
            .values()
            .cloned()
            .collect();

        // --- Build open positions report & compute equity ---
        let mut open_positions: Vec<OpenPositionReport> = Vec::new();
        let mut per_market_open_count: HashMap<String, usize> = HashMap::new();
        let mut per_market_open_notional: HashMap<String, f64> = HashMap::new();
        let mut per_market_unrealized: HashMap<String, f64> = HashMap::new();
        let mut total_open_equity: f64 = 0.0;

        for (key, pos) in &self.positions {
            let current_price = self
                .last_prices
                .get(&key.market)
                .copied()
                .unwrap_or(pos.entry_price);

            let unrealized_pnl = pos.unrealized_pnl_usd(current_price);

            // Compute collateral: clip_size_usd that was deducted from sim_balance.
            let leverage = match self.cell_strategies.get(key) {
                Some(s) => {
                    let clip = s.parameters().clip_size_usd;
                    if clip > 0.0 {
                        pos.size_usd / clip
                    } else {
                        1.0
                    }
                }
                None => 1.0,
            };
            let collateral_usd = pos.size_usd / leverage.max(1.0);

            let position_equity = collateral_usd + unrealized_pnl;
            total_open_equity += position_equity;

            *per_market_open_count.entry(key.market.clone()).or_insert(0) += 1;
            *per_market_open_notional
                .entry(key.market.clone())
                .or_insert(0.0) += pos.size_usd;
            *per_market_unrealized
                .entry(key.market.clone())
                .or_insert(0.0) += unrealized_pnl;

            let entry_time = DateTime::from_timestamp_millis(pos.entry_time_ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();

            open_positions.push(OpenPositionReport {
                market: pos.market.clone(),
                strategy: pos.strategy_name.clone(),
                direction: if pos.is_long {
                    "long".to_string()
                } else {
                    "short".to_string()
                },
                entry_price: pos.entry_price,
                current_price,
                size_usd: pos.size_usd,
                collateral_usd,
                unrealized_pnl,
                accrued_borrow_fee: pos.accrued_borrow_fee,
                entry_fee: pos.entry_fee,
                funding_captured: 0.0, // Not tracked per-position yet
                hold_time_secs: pos.hold_secs(),
                entry_time,
            });
        }

        // Sort open positions by market for deterministic output.
        open_positions.sort_by(|a, b| a.market.cmp(&b.market).then(a.strategy.cmp(&b.strategy)));

        // Equity = cash balance + open position equity.
        let equity = self.sim_balance + total_open_equity;
        let net_pnl = equity - self.initial_balance;

        // --- Per-market stats ---
        for ms in &mut market_stats {
            ms.open_position_count = *per_market_open_count.get(&ms.market).unwrap_or(&0);
            ms.open_notional_usd = *per_market_open_notional.get(&ms.market).unwrap_or(&0.0);
            ms.unrealized_pnl = *per_market_unrealized.get(&ms.market).unwrap_or(&0.0);
            ms.finalize();
        }
        market_stats.sort_by(|a, b| a.market.cmp(&b.market));

        let total_trades: usize = market_stats.iter().map(|m| m.trades).sum();
        let total_wins: usize = market_stats.iter().map(|m| m.wins).sum();
        let win_rate = if total_trades > 0 {
            total_wins as f64 / total_trades as f64 * 100.0
        } else {
            0.0
        };

        let taker_fees: f64 = market_stats.iter().map(|m| m.entry_fees + m.exit_fees).sum();
        let borrow_fees: f64 = market_stats.iter().map(|m| m.borrow_fees).sum();
        let total_fees = taker_fees + borrow_fees;

        // Sharpe ratio (null if < 5 trades).
        let all_pnls: Vec<f64> = market_stats
            .iter()
            .flat_map(|m| m.trade_pnls.iter().copied())
            .collect();
        let sharpe_ratio = compute_sharpe(&all_pnls);

        let summary = HlPaperSummary {
            start_time: self.start_time.to_rfc3339(),
            end_time: Utc::now().to_rfc3339(),
            initial_balance: self.initial_balance,
            final_balance: equity,
            net_pnl,
            total_fees: FeeBreakdown {
                taker_fees,
                borrow_fees,
                total: total_fees,
            },
            total_trades,
            win_rate,
            sharpe_ratio,
            open_positions,
            markets: market_stats,
        };

        std::fs::create_dir_all(&self.output_dir)?;

        let output_path = format!("{}/hl-paper-summary.json", self.output_dir);
        let tmp_path = format!("{}.tmp", output_path);
        let json = serde_json::to_string_pretty(&summary)?;
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &output_path)?;

        info!("[hl-paper] Report written to {}", output_path);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Async run loop
    // -----------------------------------------------------------------------

    /// Run the engine until `shutdown_handle()` is unset or an error occurs.
    pub async fn run(mut self) -> anyhow::Result<()> {
        info!(
            "=== HL Paper Trading Engine === balance=${:.2} markets={} strategies={}",
            self.initial_balance,
            self.markets.join(","),
            self.strategy_names.join(","),
        );

        while self.running.load(Ordering::Relaxed) {
            if let Err(e) = self.tick().await {
                error!("[hl-paper] tick error: {:#}", e);
                // Interruptible error-backoff sleep (checks running every second).
                let deadline =
                    tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
                while self.running.load(Ordering::Relaxed)
                    && tokio::time::Instant::now() < deadline
                {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
                continue;
            }
            // Interruptible poll-interval sleep (checks running every second).
            let deadline = tokio::time::Instant::now()
                + tokio::time::Duration::from_secs(self.poll_interval_secs);
            while self.running.load(Ordering::Relaxed)
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        // Shutdown report.
        self.write_report()?;

        let stats = self.trade_log.len();
        info!(
            "[hl-paper] Final: {} trades | balance ${:.2} (started ${:.2})",
            stats, self.sim_balance, self.initial_balance
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Accessors for testing
    // -----------------------------------------------------------------------

    /// Current simulated balance.
    #[allow(dead_code)]
    pub fn balance(&self) -> f64 {
        self.sim_balance
    }

    /// Number of open positions.
    #[allow(dead_code)]
    pub fn open_position_count(&self) -> usize {
        self.positions.len()
    }

    /// Borrow check for a specific cell.
    #[allow(dead_code)]
    pub fn has_position(&self, strategy: &str, market: &str) -> bool {
        let key = CellKey {
            strategy: strategy.to_string(),
            market: market.to_string(),
        };
        self.positions.contains_key(&key)
    }

    /// Get a reference to a position (for testing).
    #[allow(dead_code)]
    pub fn get_position(&self, strategy: &str, market: &str) -> Option<&HlPaperPosition> {
        let key = CellKey {
            strategy: strategy.to_string(),
            market: market.to_string(),
        };
        self.positions.get(&key)
    }

    /// Number of completed trades.
    #[allow(dead_code)]
    pub fn trade_count(&self) -> usize {
        self.trade_log.len()
    }

    /// Trade log accessor for testing.
    #[allow(dead_code)]
    pub fn trades(&self) -> &[HlTradeRecord] {
        &self.trade_log
    }

    /// Market stats accessor for testing.
    #[allow(dead_code)]
    pub fn market_stats(&self, market: &str) -> Option<&MarketStats> {
        self.market_stats.get(market)
    }

    /// Cooldowns accessor for testing.
    #[allow(dead_code)]
    pub fn has_cooldown(&self, strategy: &str, market: &str) -> bool {
        let key = CellKey {
            strategy: strategy.to_string(),
            market: market.to_string(),
        };
        self.cooldowns.contains_key(&key)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check software-side stop-loss, take-profit, trailing, and time stop.
fn check_soft_exits(ctx: &PositionContext) -> Option<ExitReason> {
    let pnl_pct = if ctx.is_long {
        (ctx.current_price - ctx.entry_price) / ctx.entry_price * 100.0
    } else {
        (ctx.entry_price - ctx.current_price) / ctx.entry_price * 100.0
    };

    // Stop-loss.
    if pnl_pct <= -ctx.stop_loss_pct {
        return Some(ExitReason::StopLoss);
    }

    // Take-profit.
    if pnl_pct >= ctx.take_profit_pct {
        return Some(ExitReason::TakeProfit);
    }

    // Trailing stop.
    if ctx.trailing_stop_pct > 0.0 && ctx.trailing_activation_pct > 0.0 {
        let peak_profit_pct = if ctx.entry_price > 0.0 {
            if ctx.is_long {
                (ctx.peak_price - ctx.entry_price) / ctx.entry_price * 100.0
            } else {
                (ctx.entry_price - ctx.peak_price) / ctx.entry_price * 100.0
            }
        } else {
            0.0
        };
        if peak_profit_pct >= ctx.trailing_activation_pct {
            let retracement_pct = if ctx.peak_price > 0.0 {
                if ctx.is_long {
                    (ctx.peak_price - ctx.current_price) / ctx.peak_price * 100.0
                } else {
                    (ctx.current_price - ctx.peak_price) / ctx.peak_price * 100.0
                }
            } else {
                0.0
            };
            if retracement_pct >= ctx.trailing_stop_pct {
                return Some(ExitReason::TrailingStop);
            }
        }
    }

    // Time stop.
    if ctx.hold_secs >= ctx.max_hold_secs {
        return Some(ExitReason::TimeStop);
    }

    None
}

/// Compute annualised Sharpe ratio from a list of per-trade PnLs.
/// Returns `None` if there are fewer than 5 trades.
fn compute_sharpe(pnls: &[f64]) -> Option<f64> {
    if pnls.len() < 5 {
        return None;
    }
    let n = pnls.len() as f64;
    let mean = pnls.iter().sum::<f64>() / n;
    let variance = pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let std_dev = variance.sqrt();
    if std_dev <= 0.0 {
        return Some(0.0);
    }
    // Annualise: assume ~252 trading days, ~10 trades/day ≈ 2520 trades/year
    Some((mean / std_dev) * (2520.0_f64).sqrt())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::funding_capture::FundingCaptureParams;
    use crate::market_data::MockDataProvider;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn default_funding_params() -> FundingCaptureParams {
        FundingCaptureParams {
            min_annualized_rate_pct: 20.0,
            exit_annualized_rate_pct: 5.0,
            max_position_hours: 72,
            leverage: 1.0,
            clip_size_usd: 100.0,
            confirmation_ticks: 2,
            stop_loss_pct: 3.0,
            cooldown_after_loss_secs: 300,
            use_native_tp_sl: true,
            funding_interval_hours: 8,
        }
    }

    fn make_funding_snapshot(coin: &str, rate_pct: f64, mark_px: f64) -> FundingSnapshot {
        FundingSnapshot {
            coin: coin.to_string(),
            annualized_rate_pct: rate_pct,
            raw_funding_rate: rate_pct / 100.0 / (365.0 * 3.0),
            mark_px,
            open_interest_usd: 1_000_000.0,
            timestamp_ms: 1_700_000_000_000,
        }
    }

    fn make_mock_provider(
        prices: HashMap<String, f64>,
        funding: Vec<FundingSnapshot>,
    ) -> MockDataProvider {
        MockDataProvider::new(prices, funding)
    }

    fn build_engine(
        provider: MockDataProvider,
        markets: Vec<&str>,
        balance: f64,
    ) -> HlPaperEngine<MockDataProvider> {
        let params = default_funding_params();
        let strategy_names = vec!["funding-capture".to_string()];
        let markets_owned: Vec<String> = markets.iter().map(|s| s.to_string()).collect();

        HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            strategy_names,
            &|name| {
                match name {
                    "funding-capture" => Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
                    other => Err(anyhow::anyhow!("Unknown strategy: {}", other)),
                }
            },
            markets_owned,
            balance,
            "/tmp/hl-paper-test",
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // Engine opens short on funding signal
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_opens_short_on_funding_signal() {
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Tick twice so confirmation_ticks=2 is satisfied across pushes.
        // First tick pushes funding, second tick fires entry.

        // Need to manually push funding into strategy because tick() fetches fresh
        // funding data each call. We set mock to always return high funding.
        engine.tick().await.unwrap();

        // After first tick, check if position opened (might need 2 ticks
        // because push_funding happens in tick and detect_entry follows).
        // With funding data available, the strategy should accumulate consecutive
        // ticks and eventually fire.
        let mut prices3 = HashMap::new();
        prices3.insert("BTC".to_string(), 60000.0);
        engine.provider = make_mock_provider(
            prices3,
            vec![
                make_funding_snapshot("BTC", 25.0, 60000.0),
                make_funding_snapshot("BTC", 25.0, 60000.0),
            ],
        );
        engine.tick().await.unwrap();

        assert!(
            engine.has_position("funding-capture", "BTC"),
            "Expected a SHORT position to be opened for BTC"
        );

        let pos = engine.get_position("funding-capture", "BTC").unwrap();
        assert!(!pos.is_long, "Funding capture should be SHORT");
        assert!((pos.entry_price - 60000.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Engine closes on funding drop
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_closes_on_funding_drop() {
        let high_funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];

        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, high_funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Open position.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // Now drop funding below exit threshold.
        let low_funding = vec![make_funding_snapshot("BTC", 2.0, 60000.0)];
        let prices2 = {
            let mut p = HashMap::new();
            p.insert("BTC".to_string(), 60000.0);
            p
        };
        engine.provider = make_mock_provider(prices2, low_funding);
        engine.tick().await.unwrap();

        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "Position should be closed after funding drops"
        );
        assert!(engine.trade_count() >= 1);
    }

    // -----------------------------------------------------------------------
    // Borrow fee accrual
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_accrues_borrow_fees() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Open position.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // Tick a few more times with funding still high (to keep position open).
        let keep_funding = vec![make_funding_snapshot("BTC", 10.0, 60000.0)];
        for _ in 0..5 {
            engine.provider = make_mock_provider(
                {
                    let mut p = HashMap::new();
                    p.insert("BTC".to_string(), 60000.0);
                    p
                },
                keep_funding.clone(),
            );
            engine.tick().await.unwrap();
        }

        let pos = engine.get_position("funding-capture", "BTC").unwrap();
        assert!(
            pos.accrued_borrow_fee > 0.0,
            "Borrow fee should have accrued: got {}",
            pos.accrued_borrow_fee
        );

        // Verify: each tick accrues size_usd * HL_BORROW_RATE_PER_HOUR * (poll_interval_secs / 3600)
        // poll_interval_secs = 5 (default)
        let expected_per_tick = pos.size_usd * HL_BORROW_RATE_PER_HOUR * (5.0 / 3600.0);
        // 5 manage ticks + ticks during opening (at least 1 manage tick)
        assert!(
            pos.accrued_borrow_fee >= expected_per_tick,
            "Borrow fee should be >= one tick's accrual"
        );
    }

    // -----------------------------------------------------------------------
    // Net PnL calculation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_net_pnl_calculation() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Open.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        let pos = engine.get_position("funding-capture", "BTC").unwrap();
        let entry_fee = pos.entry_fee;

        // Close with funding drop — price moved in our favor (short, price drops).
        let low_funding = vec![make_funding_snapshot("BTC", 2.0, 59000.0)];
        let prices2 = {
            let mut p = HashMap::new();
            p.insert("BTC".to_string(), 59000.0);
            p
        };
        engine.provider = make_mock_provider(prices2, low_funding);
        engine.tick().await.unwrap();

        assert_eq!(engine.trade_count(), 1);
        let trade = &engine.trades()[0];
        let expected_exit_fee = trade.size_usd * HL_TAKER_FEE_RATE;
        assert!((trade.exit_fee - expected_exit_fee).abs() < 1e-10);
        // net_pnl = gross_pnl - exit_fee - borrow_fee
        let expected_net = trade.gross_pnl - trade.exit_fee - trade.borrow_fee;
        assert!(
            (trade.net_pnl - expected_net).abs() < 1e-10,
            "net_pnl should be gross - exit_fee - borrow_fee: got {} vs expected {}",
            trade.net_pnl,
            expected_net
        );
        // Entry fee should match.
        assert!((trade.entry_fee - entry_fee).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // Rejects insufficient balance
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_rejects_insufficient_balance() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        // Balance of $50 is less than clip_size_usd of $100.
        let mut engine = build_engine(provider, vec!["BTC"], 50.0);

        engine.tick().await.unwrap();
        engine.tick().await.unwrap();

        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "Should NOT open position with insufficient balance"
        );
        assert_eq!(engine.open_position_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Multi-market independent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_multi_market_independent() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("ETH", 25.0, 3000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("ETH", 25.0, 3000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);
        prices.insert("ETH".to_string(), 3000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC", "ETH"], 1000.0);

        engine.tick().await.unwrap();
        engine.tick().await.unwrap();

        // Both markets should have positions.
        assert!(engine.has_position("funding-capture", "BTC"));
        assert!(engine.has_position("funding-capture", "ETH"));

        // Close BTC only.
        let btc_close_funding = vec![
            make_funding_snapshot("BTC", 2.0, 60000.0),
            make_funding_snapshot("ETH", 10.0, 3000.0),
        ];
        let mut prices2 = HashMap::new();
        prices2.insert("BTC".to_string(), 60000.0);
        prices2.insert("ETH".to_string(), 3000.0);
        engine.provider = make_mock_provider(prices2, btc_close_funding);
        engine.tick().await.unwrap();

        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "BTC should be closed"
        );
        assert!(
            engine.has_position("funding-capture", "ETH"),
            "ETH should still be open"
        );
    }

    // -----------------------------------------------------------------------
    // Entry fee calculation
    // -----------------------------------------------------------------------

    #[test]
    fn test_entry_fee_calculation() {
        // $200 notional at 0.035% = $0.07
        let notional = 200.0;
        let fee = notional * HL_TAKER_FEE_RATE;
        assert!((fee - 0.07).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // Exit fee calculation
    // -----------------------------------------------------------------------

    #[test]
    fn test_exit_fee_calculation() {
        let notional = 200.0;
        let fee = notional * HL_TAKER_FEE_RATE;
        assert!((fee - 0.07).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // Borrow fee accrual (unit)
    // -----------------------------------------------------------------------

    #[test]
    fn test_borrow_fee_accrual() {
        // $200 size, 0.01%/hr, 3 ticks of 5s each
        let size_usd = 200.0;
        let poll_secs = 5u64;
        let ticks = 3;
        let mut accrued = 0.0;
        for _ in 0..ticks {
            accrued += size_usd * HL_BORROW_RATE_PER_HOUR * (poll_secs as f64 / 3600.0);
        }
        let expected = 200.0 * 0.0001 * (5.0 / 3600.0) * 3.0;
        assert!((accrued - expected).abs() < 1e-15);
        assert!(accrued > 0.0);
    }

    // -----------------------------------------------------------------------
    // Sharpe ratio null under 5 trades
    // -----------------------------------------------------------------------

    #[test]
    fn test_sharpe_ratio_null_under_5_trades() {
        let pnls = vec![1.0, 2.0, -0.5, 3.0]; // 4 trades
        assert!(compute_sharpe(&pnls).is_none());
    }

    // -----------------------------------------------------------------------
    // Sharpe ratio computed over 5 trades
    // -----------------------------------------------------------------------

    #[test]
    fn test_sharpe_ratio_computed_over_5_trades() {
        let pnls = vec![1.0, 2.0, -0.5, 3.0, 0.5]; // 5 trades
        let sharpe = compute_sharpe(&pnls);
        assert!(sharpe.is_some());
        let s = sharpe.unwrap();
        assert!(s.is_finite());
        // Mean = 1.2, should have a positive Sharpe.
        assert!(s > 0.0);
    }

    // -----------------------------------------------------------------------
    // Report atomic writes
    // -----------------------------------------------------------------------

    #[test]
    fn test_report_atomic_writes() {
        let dir = format!("/tmp/hl-paper-test-atomic-{}", std::process::id());
        let prices = HashMap::new();
        let provider = make_mock_provider(prices, vec![]);
        let params = default_funding_params();
        let engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        assert!(std::path::Path::new(&path).exists());

        // .tmp should not exist (renamed).
        assert!(!std::path::Path::new(&format!("{}.tmp", path)).exists());

        // Parse and validate.
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(summary["initial_balance"], 1000.0);
        assert!(summary["total_trades"].is_number());
        assert!(summary["sharpe_ratio"].is_null()); // < 5 trades → null
        // open_positions array should exist (empty when no positions).
        assert!(summary["open_positions"].is_array());
        assert_eq!(summary["open_positions"].as_array().unwrap().len(), 0);

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Report per-market breakdown
    // -----------------------------------------------------------------------

    #[test]
    fn test_report_per_market_breakdown() {
        let dir = format!("/tmp/hl-paper-test-market-{}", std::process::id());
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);
        prices.insert("ETH".to_string(), 3000.0);

        let provider = make_mock_provider(prices, vec![]);
        let params = default_funding_params();
        let engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string(), "ETH".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();

        let markets = summary["markets"].as_array().unwrap();
        assert_eq!(markets.len(), 2);

        let market_names: Vec<&str> = markets
            .iter()
            .map(|m| m["market"].as_str().unwrap())
            .collect();
        assert!(market_names.contains(&"BTC"));
        assert!(market_names.contains(&"ETH"));

        // Per-market open position fields should exist.
        for m in markets {
            assert!(m["open_position_count"].is_number());
            assert!(m["open_notional_usd"].is_number());
            assert!(m["unrealized_pnl"].is_number());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Report fee decomposition
    // -----------------------------------------------------------------------

    #[test]
    fn test_report_fee_decomposition() {
        let dir = format!("/tmp/hl-paper-test-fees-{}", std::process::id());
        let prices = HashMap::new();
        let provider = make_mock_provider(prices, vec![]);
        let params = default_funding_params();
        let engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();

        let fees = &summary["total_fees"];
        assert!(fees["taker_fees"].is_number());
        assert!(fees["borrow_fees"].is_number());
        assert!(fees["total"].is_number());
        // taker + borrow = total
        let taker = fees["taker_fees"].as_f64().unwrap();
        let borrow = fees["borrow_fees"].as_f64().unwrap();
        let total = fees["total"].as_f64().unwrap();
        assert!((taker + borrow - total).abs() < 1e-10);

        // open_positions should exist (empty).
        assert!(summary["open_positions"].is_array());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Stop-loss closes position
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_stop_loss_closes_position() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Open position.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // Price moves against the short by >3% (stop_loss_pct = 3.0).
        // Short: loss when price goes up. 3% of 60000 = 61800.
        let loss_price = 62000.0;
        let keep_funding = vec![make_funding_snapshot("BTC", 10.0, loss_price)];
        let mut prices2 = HashMap::new();
        prices2.insert("BTC".to_string(), loss_price);
        engine.provider = make_mock_provider(prices2, keep_funding);
        engine.tick().await.unwrap();

        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "Stop-loss should close the short position"
        );

        let trade = &engine.trades()[0];
        assert_eq!(trade.exit_reason, "StopLoss");
    }

    // -----------------------------------------------------------------------
    // Trailing stop tracks peak
    // -----------------------------------------------------------------------

    #[test]
    fn test_trailing_stop_tracks_peak() {
        let mut pos = HlPaperPosition {
            market: "BTC".to_string(),
            strategy_name: "funding-capture".to_string(),
            is_long: false,
            entry_price: 60000.0,
            size_usd: 100.0,
            entry_fee: 0.0,
            accrued_borrow_fee: 0.0,
            peak_price: 60000.0,
            entry_time_ms: Utc::now().timestamp_millis(),
        };

        // Short: peak_price tracks lowest price.
        pos.update_peak(58000.0);
        assert_eq!(pos.peak_price, 58000.0);

        pos.update_peak(59000.0);
        assert_eq!(pos.peak_price, 58000.0, "Peak should not change for short when price rises");

        pos.update_peak(57000.0);
        assert_eq!(pos.peak_price, 57000.0);
    }

    // -----------------------------------------------------------------------
    // Cooldown after loss
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_cooldown_after_loss() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        // Open.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // Close at a loss (price moved against short → price goes up > 3%).
        let loss_price = 62000.0;
        let close_funding = vec![make_funding_snapshot("BTC", 2.0, loss_price)];
        let mut prices2 = HashMap::new();
        prices2.insert("BTC".to_string(), loss_price);
        engine.provider = make_mock_provider(prices2, close_funding);
        engine.tick().await.unwrap();

        assert!(!engine.has_position("funding-capture", "BTC"));
        assert_eq!(engine.trade_count(), 1);
        assert!(engine.trades()[0].net_pnl < 0.0, "Trade should be a loss");

        // Check cooldown is set.
        assert!(engine.has_cooldown("funding-capture", "BTC"));

        // Try to open again immediately — should be blocked by cooldown.
        let reentry_funding = vec![
            make_funding_snapshot("BTC", 25.0, 62000.0),
            make_funding_snapshot("BTC", 25.0, 62000.0),
        ];
        let mut prices3 = HashMap::new();
        prices3.insert("BTC".to_string(), 62000.0);
        engine.provider = make_mock_provider(prices3, reentry_funding);
        engine.tick().await.unwrap();
        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "Should not re-enter during cooldown"
        );
    }

    // -----------------------------------------------------------------------
    // Balance updated on close
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_balance_updated_on_close() {
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);
        let initial = engine.balance();

        // Open (deducts clip).
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));
        let after_open = engine.balance();
        assert!(after_open < initial, "Balance should decrease after opening");

        // Close with funding drop.
        let close_funding = vec![make_funding_snapshot("BTC", 2.0, 60000.0)];
        let mut prices2 = HashMap::new();
        prices2.insert("BTC".to_string(), 60000.0);
        engine.provider = make_mock_provider(prices2, close_funding);
        engine.tick().await.unwrap();

        let after_close = engine.balance();
        // After close, balance should be > after_open (collateral returned + net PnL).
        assert!(
            after_close > after_open,
            "Balance after close ({}) should be > after open ({})",
            after_close,
            after_open
        );
    }

    // -----------------------------------------------------------------------
    // Push funding called per tick (verify strategy has funding data)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_push_funding_called_per_tick() {
        let funding = vec![make_funding_snapshot("BTC", 30.0, 60000.0)];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        engine.tick().await.unwrap();

        // Verify the strategy has received funding data.
        let key = CellKey {
            strategy: "funding-capture".to_string(),
            market: "BTC".to_string(),
        };
        if let Some(strat) = engine.cell_strategies.get(&key) {
            if let Some(fc) = strat.as_any().downcast_ref::<FundingRateCaptureStrategy>() {
                assert!(
                    fc.current_rate() > 0.0,
                    "Strategy should have funding data after tick"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // HlPaperPosition — unrealised PnL
    // -----------------------------------------------------------------------

    #[test]
    fn test_position_unrealized_pnl_long_profit() {
        let pos = HlPaperPosition {
            market: "BTC".to_string(),
            strategy_name: "test".to_string(),
            is_long: true,
            entry_price: 100.0,
            size_usd: 1000.0,
            entry_fee: 0.0,
            accrued_borrow_fee: 0.0,
            peak_price: 105.0,
            entry_time_ms: 0,
        };
        // Price went from 100 to 105 → +5% → $50 PnL.
        assert!((pos.unrealized_pnl_usd(105.0) - 50.0).abs() < 0.01);
        assert!((pos.unrealized_pnl_pct(105.0) - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_position_unrealized_pnl_short_profit() {
        let pos = HlPaperPosition {
            market: "BTC".to_string(),
            strategy_name: "test".to_string(),
            is_long: false,
            entry_price: 100.0,
            size_usd: 1000.0,
            entry_fee: 0.0,
            accrued_borrow_fee: 0.0,
            peak_price: 95.0,
            entry_time_ms: 0,
        };
        // Short: price went from 100 to 95 → +5% → $50 PnL.
        assert!((pos.unrealized_pnl_usd(95.0) - 50.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // check_soft_exits — unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_soft_exit_stop_loss() {
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 96.0, // -4% < -3% SL
            peak_price: 100.0,
            hold_secs: 10,
            max_hold_secs: 3600,
            take_profit_pct: 5.0,
            stop_loss_pct: 3.0,
            trailing_stop_pct: 1.0,
            trailing_activation_pct: 2.0,
        };
        assert_eq!(check_soft_exits(&ctx), Some(ExitReason::StopLoss));
    }

    #[test]
    fn test_soft_exit_take_profit() {
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 106.0, // +6% > 5% TP
            peak_price: 106.0,
            hold_secs: 10,
            max_hold_secs: 3600,
            take_profit_pct: 5.0,
            stop_loss_pct: 3.0,
            trailing_stop_pct: 1.0,
            trailing_activation_pct: 2.0,
        };
        assert_eq!(check_soft_exits(&ctx), Some(ExitReason::TakeProfit));
    }

    #[test]
    fn test_soft_exit_time_stop() {
        let ctx = PositionContext {
            is_long: false,
            entry_price: 100.0,
            current_price: 99.0,
            peak_price: 99.0,
            hold_secs: 3600,
            max_hold_secs: 3600,
            take_profit_pct: 5.0,
            stop_loss_pct: 3.0,
            trailing_stop_pct: 0.0,
            trailing_activation_pct: 0.0,
        };
        assert_eq!(check_soft_exits(&ctx), Some(ExitReason::TimeStop));
    }

    #[test]
    fn test_soft_exit_trailing_stop() {
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 103.0, // Retraced from peak 105 to 103
            peak_price: 105.0,
            hold_secs: 10,
            max_hold_secs: 3600,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            trailing_stop_pct: 1.0,
            trailing_activation_pct: 2.0,
        };
        // Peak profit = (105-100)/100 = 5% > 2% activation
        // Retracement = (105-103)/105 = 1.9% > 1.0% trailing
        assert_eq!(check_soft_exits(&ctx), Some(ExitReason::TrailingStop));
    }

    #[test]
    fn test_soft_exit_no_exit() {
        let ctx = PositionContext {
            is_long: true,
            entry_price: 100.0,
            current_price: 101.0,
            peak_price: 102.0,
            hold_secs: 10,
            max_hold_secs: 3600,
            take_profit_pct: 5.0,
            stop_loss_pct: 3.0,
            trailing_stop_pct: 1.0,
            trailing_activation_pct: 2.0,
        };
        assert!(check_soft_exits(&ctx).is_none());
    }

    // -----------------------------------------------------------------------
    // Compute Sharpe
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_sharpe_empty() {
        assert!(compute_sharpe(&[]).is_none());
    }

    #[test]
    fn test_compute_sharpe_four_trades() {
        assert!(compute_sharpe(&[1.0, 2.0, 3.0, 4.0]).is_none());
    }

    #[test]
    fn test_compute_sharpe_five_constant_pnls() {
        // All same → std_dev = 0 → Sharpe = 0.0.
        let sharpe = compute_sharpe(&[1.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(sharpe, Some(0.0));
    }

    // -----------------------------------------------------------------------
    // Cell key
    // -----------------------------------------------------------------------

    #[test]
    fn test_cell_key_hash_and_eq() {
        let k1 = CellKey {
            strategy: "funding-capture".to_string(),
            market: "BTC".to_string(),
        };
        let k2 = CellKey {
            strategy: "funding-capture".to_string(),
            market: "BTC".to_string(),
        };
        let k3 = CellKey {
            strategy: "funding-capture".to_string(),
            market: "ETH".to_string(),
        };
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);

        let mut map = HashMap::new();
        map.insert(k1.clone(), 1);
        assert_eq!(map.get(&k2), Some(&1));
    }

    // -----------------------------------------------------------------------
    // Market stats
    // -----------------------------------------------------------------------

    #[test]
    fn test_market_stats_record_and_finalize() {
        let mut stats = MarketStats {
            market: "BTC".to_string(),
            ..Default::default()
        };
        stats.record(5.0, 6.0, 0.5, 0.5, 0.0);
        stats.record(-2.0, -1.5, 0.5, 0.5, 0.5);
        stats.finalize();

        assert_eq!(stats.trades, 2);
        assert_eq!(stats.wins, 1);
        assert_eq!(stats.losses, 1);
        assert!((stats.pnl - 3.0).abs() < 1e-10);
        assert!((stats.win_rate - 50.0).abs() < 1e-10);
        assert!((stats.total_fees - 2.5).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // HlPaperConfig default
    // -----------------------------------------------------------------------

    #[test]
    fn test_config_default() {
        let config = HlPaperConfig::default();
        assert_eq!(config.poll_interval_secs, 300);
        assert_eq!(config.max_total_notional_usd, 50_000.0);
    }

    // -----------------------------------------------------------------------
    // Report includes correct fields when there are trades
    // -----------------------------------------------------------------------

    #[test]
    fn test_report_with_trades() {
        let dir = format!("/tmp/hl-paper-test-trades-{}", std::process::id());
        let prices = HashMap::new();
        let provider = make_mock_provider(prices, vec![]);
        let params = default_funding_params();
        let mut engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        // Manually inject a trade record to test reporting.
        engine.trade_log.push(HlTradeRecord {
            strategy: "funding-capture".to_string(),
            market: "BTC".to_string(),
            direction: "SHORT".to_string(),
            entry_price: 60000.0,
            exit_price: 59000.0,
            size_usd: 100.0,
            gross_pnl: 16.67,
            entry_fee: 0.035,
            exit_fee: 0.035,
            borrow_fee: 0.01,
            net_pnl: 16.59,
            hold_secs: 3600,
            exit_reason: "ReversalDetected".to_string(),
            timestamp: Utc::now().to_rfc3339(),
        });

        // Update market stats manually.
        if let Some(stats) = engine.market_stats.get_mut("BTC") {
            stats.record(16.59, 16.67, 0.035, 0.035, 0.01);
        }

        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(summary["total_trades"], 1);
        assert!(summary["win_rate"].as_f64().unwrap() > 0.0);
        assert!(summary["sharpe_ratio"].is_null()); // Still < 5 in market_stats

        // open_positions should exist (empty — no open positions).
        assert!(summary["open_positions"].is_array());
        assert_eq!(summary["open_positions"].as_array().unwrap().len(), 0);

        let markets = summary["markets"].as_array().unwrap();
        let btc = markets.iter().find(|m| m["market"] == "BTC").unwrap();
        assert_eq!(btc["trades"], 1);
        assert!(btc["pnl"].as_f64().unwrap() > 0.0);
        assert!((btc["total_fees"].as_f64().unwrap() - 0.08).abs() < 1e-10);
        // Per-market open position fields.
        assert_eq!(btc["open_position_count"], 0);
        assert!((btc["open_notional_usd"].as_f64().unwrap()).abs() < 1e-10);
        assert!((btc["unrealized_pnl"].as_f64().unwrap()).abs() < 1e-10);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Report includes open positions and correct equity
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_report_includes_open_positions_with_equity() {
        let dir = format!("/tmp/hl-paper-test-open-pos-{}", std::process::id());
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let params = default_funding_params();
        let mut engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        // Open position.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // sim_balance should be 1000 - 100 (clip) = 900
        assert!(
            (engine.balance() - 900.0).abs() < 1e-10,
            "Balance after open should be 900, got {}",
            engine.balance()
        );

        // Write report while position is open — price unchanged at 60000.
        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();

        // final_balance = equity = sim_balance (900) + collateral (100) + unrealized_pnl (0)
        let final_bal = summary["final_balance"].as_f64().unwrap();
        assert!(
            (final_bal - 1000.0).abs() < 1e-10,
            "final_balance should be ~1000 (equity), got {}",
            final_bal
        );

        // net_pnl = equity - initial = 0 (no price movement yet)
        let net_pnl = summary["net_pnl"].as_f64().unwrap();
        assert!(
            net_pnl.abs() < 1e-10,
            "net_pnl should be ~0, got {}",
            net_pnl
        );

        // open_positions should contain one entry.
        let open = summary["open_positions"].as_array().unwrap();
        assert_eq!(open.len(), 1);
        let pos = &open[0];
        assert_eq!(pos["market"].as_str().unwrap(), "BTC");
        assert_eq!(pos["strategy"].as_str().unwrap(), "funding-capture");
        assert_eq!(pos["direction"].as_str().unwrap(), "short");
        assert!((pos["entry_price"].as_f64().unwrap() - 60000.0).abs() < 0.01);
        assert!((pos["current_price"].as_f64().unwrap() - 60000.0).abs() < 0.01);
        assert!((pos["collateral_usd"].as_f64().unwrap() - 100.0).abs() < 1e-10);
        assert!((pos["unrealized_pnl"].as_f64().unwrap()).abs() < 1e-10);

        // Per-market stats should show open position data.
        let markets = summary["markets"].as_array().unwrap();
        let btc = markets.iter().find(|m| m["market"] == "BTC").unwrap();
        assert_eq!(btc["open_position_count"], 1);
        assert!(btc["open_notional_usd"].as_f64().unwrap() > 0.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Report equity reflects unrealized loss
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_report_equity_reflects_unrealized_loss() {
        let dir = format!("/tmp/hl-paper-test-equity-loss-{}", std::process::id());
        let funding = vec![
            make_funding_snapshot("BTC", 25.0, 60000.0),
            make_funding_snapshot("BTC", 25.0, 60000.0),
        ];
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, funding);
        let params = default_funding_params();
        let mut engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        // Open position.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert!(engine.has_position("funding-capture", "BTC"));

        // Price moves against short (up 1%): 60000 -> 60600.
        let new_price = 60600.0;
        let keep_funding = vec![make_funding_snapshot("BTC", 10.0, new_price)];
        engine.provider = make_mock_provider(
            {
                let mut p = HashMap::new();
                p.insert("BTC".to_string(), new_price);
                p
            },
            keep_funding,
        );
        engine.tick().await.unwrap();

        // Write report.
        engine.write_report().unwrap();

        let path = format!("{}/hl-paper-summary.json", dir);
        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Short BTC: price went up → unrealized loss.
        // unrealized_pnl = size_usd * (entry - current) / entry = 100 * (60000 - 60600) / 60000 = -1.0
        let open = summary["open_positions"].as_array().unwrap();
        assert_eq!(open.len(), 1);
        let upnl = open[0]["unrealized_pnl"].as_f64().unwrap();
        assert!(upnl < 0.0, "unrealized_pnl should be negative for short with price rise, got {}", upnl);

        // final_balance = 900 (sim) + 100 (collateral) + (-1.0) (unrealized) = 999.0
        let final_bal = summary["final_balance"].as_f64().unwrap();
        assert!(
            (final_bal - 999.0).abs() < 0.1,
            "final_balance should be ~999, got {}",
            final_bal
        );

        // net_pnl = equity - initial = -1.0
        let net_pnl = summary["net_pnl"].as_f64().unwrap();
        assert!(
            net_pnl < 0.0,
            "net_pnl should be negative, got {}",
            net_pnl
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Multiple ticks with no funding data → no position
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_no_position_without_funding() {
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = make_mock_provider(prices, vec![]);
        let mut engine = build_engine(provider, vec!["BTC"], 1000.0);

        engine.tick().await.unwrap();
        engine.tick().await.unwrap();

        assert!(
            !engine.has_position("funding-capture", "BTC"),
            "Should not open position without funding data"
        );
    }

    // -----------------------------------------------------------------------
    // Report is written after run() exits
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_report_written_after_run_exits() {
        let dir = format!("/tmp/hl-paper-test-run-exit-{}", std::process::id());
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);
        let provider = make_mock_provider(prices, vec![]);
        let params = default_funding_params();
        let engine = HlPaperEngine::new(
            provider,
            HlPaperConfig::default(),
            vec!["funding-capture".to_string()],
            &|_name| Ok(Box::new(FundingRateCaptureStrategy::new(params.clone()))),
            vec!["BTC".to_string()],
            1000.0,
            &dir,
        ).unwrap();

        // Stop immediately so the run loop exits without ticking.
        engine.running.store(false, Ordering::Relaxed);

        engine.run().await.unwrap();

        // Verify the report file was written.
        let path = format!("{}/hl-paper-summary.json", dir);
        assert!(
            std::path::Path::new(&path).exists(),
            "Report should be written after run() exits"
        );

        let content = std::fs::read_to_string(&path).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(summary["initial_balance"], 1000.0);
        assert_eq!(summary["total_trades"], 0);
        // open_positions should be present (empty).
        assert!(summary["open_positions"].is_array());
        assert_eq!(summary["open_positions"].as_array().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
