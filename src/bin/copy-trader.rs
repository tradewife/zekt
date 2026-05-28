//! copy-trader — Position mirroring engine for Hyperliquid wallets.
//!
//! Loads a watchlist from alpha-scanner output, polls watched wallets' positions
//! every 30s via HL clearinghouseState, detects new/closed/modified positions,
//! and mirrors them in paper trading mode with configurable sizing and risk
//! management (max-position-pct, max-positions, stop-loss-pct).
//!
//! Output: `data/copy-trades.json` — trade log with atomic writes.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// ── Constants ────────────────────────────────────────────────────────────────

const HL_INFO_URL: &str = "https://api.hyperliquid.xyz/info";

/// Hyperliquid taker fee rate: 0.035% of notional per side.
const HL_TAKER_FEE_RATE: f64 = 0.00035;

/// Hyperliquid hourly borrow rate: 0.01% of notional per hour.
const HL_BORROW_RATE_PER_HOUR: f64 = 0.0001;

// ── CLI ──────────────────────────────────────────────────────────────────────

fn validate_max_position_pct(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid max-position-pct value: {}", v))?;
    if val <= 0.0 {
        return Err(format!("max-position-pct must be > 0, got {}", val));
    }
    Ok(val)
}

fn validate_max_positions(v: &str) -> Result<usize, String> {
    let val: usize = v
        .parse()
        .map_err(|_| format!("invalid max-positions value: {}", v))?;
    if val == 0 {
        return Err("max-positions must be > 0".to_string());
    }
    Ok(val)
}

fn validate_stop_loss_pct(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid stop-loss-pct value: {}", v))?;
    if val <= 0.0 {
        return Err(format!("stop-loss-pct must be > 0, got {}", val));
    }
    Ok(val)
}

fn validate_lag_secs(v: &str) -> Result<u64, String> {
    let val: i64 = v
        .parse()
        .map_err(|_| format!("invalid lag-secs value: {}", v))?;
    if val < 0 {
        return Err(format!("lag-secs must be >= 0, got {}", val));
    }
    Ok(val as u64)
}

fn validate_sizing_multiplier(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid sizing-multiplier value: {}", v))?;
    if val <= 0.0 {
        return Err(format!("sizing-multiplier must be > 0, got {}", val));
    }
    Ok(val)
}

#[derive(Parser, Debug)]
#[command(
    name = "copy-trader",
    about = "Mirror profitable Hyperliquid wallets' positions in paper trading mode",
    version
)]
struct Args {
    /// Paper trading mode (simulated).
    #[arg(long)]
    paper: bool,

    /// HL paper trading mode — paper trade with Hyperliquid fee model (0.035% taker, 0.01%/hr borrow).
    #[arg(long)]
    hl_paper: bool,

    /// Live trading mode (requires human approval gate).
    #[arg(long)]
    live: bool,

    /// Path to watchlist JSON from alpha-scanner output.
    #[arg(long)]
    watchlist: PathBuf,

    /// Maximum position size as percentage of account balance.
    #[arg(long, default_value_t = 10.0, value_parser = validate_max_position_pct)]
    max_position_pct: f64,

    /// Maximum number of concurrent mirrored positions.
    #[arg(long, default_value_t = 3, value_parser = validate_max_positions)]
    max_positions: usize,

    /// Stop-loss percentage for mirrored positions.
    #[arg(long, default_value_t = 5.0, value_parser = validate_stop_loss_pct)]
    stop_loss_pct: f64,

    /// Delay before mirroring a detected position (seconds).
    #[arg(long, default_value_t = 30, value_parser = validate_lag_secs)]
    lag_secs: u64,

    /// Position sizing multiplier: our_size = whale_size * multiplier.
    #[arg(long, default_value_t = 0.1, value_parser = validate_sizing_multiplier)]
    sizing_multiplier: f64,

    /// Output file path for copy trade log JSON.
    #[arg(long, default_value = "data/copy-trades.json")]
    output: PathBuf,

    /// Paper trading account balance (USD).
    #[arg(long, default_value_t = 10000.0)]
    paper_balance: f64,
}

impl Args {
    fn validate(&self) -> Result<()> {
        let mode_count = [self.paper, self.hl_paper, self.live]
            .iter()
            .filter(|&&b| b)
            .count();
        if mode_count == 0 {
            anyhow::bail!("must specify --paper, --hl-paper, or --live mode");
        }
        if mode_count > 1 {
            anyhow::bail!("cannot specify more than one of --paper, --hl-paper, --live");
        }
        if !self.watchlist.exists() {
            anyhow::bail!(
                "watchlist file not found: {}",
                self.watchlist.display()
            );
        }
        Ok(())
    }
}

// ── Data Types ───────────────────────────────────────────────────────────────

/// A wallet entry in the watchlist (matches alpha-scanner output format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub address: String,
    pub score: f64,
    #[serde(default)]
    pub sharpe: f64,
    #[serde(default)]
    pub pnl: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub positions: Vec<WatchlistPosition>,
    #[serde(default)]
    pub decaying: bool,
}

/// Simplified position from watchlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistPosition {
    pub coin: String,
    pub size: String,
    pub entry_px: String,
}

/// Top-level watchlist file (matches alpha-scanner output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchlist {
    pub generated_at: String,
    pub wallets: Vec<WalletEntry>,
}

/// Wallet entry from watchlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletEntry {
    pub address: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub sharpe: f64,
    #[serde(default)]
    pub pnl: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub positions: Vec<WatchlistPosition>,
    #[serde(default)]
    pub decaying: bool,
}

/// Direction of a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Long,
    Short,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Long => write!(f, "long"),
            Direction::Short => write!(f, "short"),
        }
    }
}

/// Status of a copy trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    Open,
    Closed,
}

impl std::fmt::Display for TradeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeStatus::Open => write!(f, "open"),
            TradeStatus::Closed => write!(f, "closed"),
        }
    }
}

/// Reason a copy trade was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    WalletClosed,
    StopLoss,
    ManualClose,
}

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloseReason::WalletClosed => write!(f, "wallet_closed"),
            CloseReason::StopLoss => write!(f, "stop_loss"),
            CloseReason::ManualClose => write!(f, "manual_close"),
        }
    }
}

/// A single copy trade entry in the trade log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyTrade {
    pub id: String,
    pub timestamp: String,
    pub wallet_address: String,
    pub market: String,
    pub direction: Direction,
    pub size_usd: f64,
    pub entry_price: f64,
    pub status: TradeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<CloseReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl_usd: Option<f64>,
    pub whale_size_usd: f64,
    pub sizing_multiplier: f64,
    /// HL paper mode: entry fee (0.035% taker on notional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_fee: Option<f64>,
    /// HL paper mode: exit fee (0.035% taker on notional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_fee: Option<f64>,
    /// HL paper mode: borrow fee accrued over holding period.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borrow_fee: Option<f64>,
    /// HL paper mode: hours the position was held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_held: Option<f64>,
}

// ── HL Position Types (inline, matching hl_info.rs format) ──────────────────

/// Simplified position from clearinghouseState.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletPosition {
    pub coin: String,
    pub size: String,
    pub entry_px: String,
    #[serde(default)]
    pub mark_px: Option<String>,
}

/// Result of position diff between two snapshots.
#[derive(Debug, Clone, Serialize)]
pub struct PositionDiff {
    pub new_positions: Vec<WalletPosition>,
    pub closed_positions: Vec<WalletPosition>,
}

/// Parse positions from HL clearinghouseState JSON.
pub fn parse_positions(raw: &serde_json::Value) -> Vec<WalletPosition> {
    let asset_positions = match raw.get("assetPositions").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut positions = Vec::new();
    for ap in asset_positions {
        let pos = match ap.get("position") {
            Some(p) => p,
            None => continue,
        };

        let size: f64 = pos
            .get("szi")
            .or(pos.get("size"))
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .unwrap_or(0.0);

        if size.abs() < f64::EPSILON {
            continue;
        }

        let coin = pos
            .get("coin")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();

        let entry_px = pos
            .get("entryPx")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0")
            .to_string();

        let mark_px = pos.get("markPx").and_then(|v| v.as_str()).map(|s| s.to_string());

        positions.push(WalletPosition {
            coin,
            size: format!("{}", size),
            entry_px,
            mark_px,
        });
    }
    positions
}

/// Detect new and closed positions between two snapshots.
pub fn detect_positions_diff(
    old: &[WalletPosition],
    new: &[WalletPosition],
) -> PositionDiff {
    let old_map: HashMap<&str, &WalletPosition> = old
        .iter()
        .filter_map(|p| {
            let sz: f64 = p.size.parse().ok()?;
            if sz.abs() > 0.0 {
                Some((p.coin.as_str(), p))
            } else {
                None
            }
        })
        .collect();

    let new_map: HashMap<&str, &WalletPosition> = new
        .iter()
        .filter_map(|p| {
            let sz: f64 = p.size.parse().ok()?;
            if sz.abs() > 0.0 {
                Some((p.coin.as_str(), p))
            } else {
                None
            }
        })
        .collect();

    let old_coins: HashSet<&str> = old_map.keys().copied().collect();
    let new_coins: HashSet<&str> = new_map.keys().copied().collect();

    let new_positions = new_coins
        .difference(&old_coins)
        .filter_map(|coin| new_map.get(coin).map(|p| (*p).clone()))
        .collect();

    let closed_positions = old_coins
        .difference(&new_coins)
        .filter_map(|coin| old_map.get(coin).map(|p| (*p).clone()))
        .collect();

    PositionDiff {
        new_positions,
        closed_positions,
    }
}

/// Fetch wallet positions from HL Info API.
async fn fetch_wallet_positions(client: &Client, wallet: &str) -> Result<Vec<WalletPosition>> {
    let body = serde_json::json!({
        "type": "clearinghouseState",
        "user": wallet
    });

    let resp = client
        .post(HL_INFO_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("HL positions request failed for wallet {}", &wallet[..wallet.len().min(12)]))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("HL positions returned {} for {}: {}", status, &wallet[..wallet.len().min(12)], &text[..text.len().min(200)]);
    }

    let raw: serde_json::Value = resp.json().await?;
    Ok(parse_positions(&raw))
}

// ── Watchlist Loading ────────────────────────────────────────────────────────

pub fn load_watchlist(path: &PathBuf) -> Result<Vec<WalletEntry>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read watchlist file: {}", path.display()))?;

    if let Ok(watchlist) = serde_json::from_str::<Watchlist>(&content) {
        return Ok(watchlist
            .wallets
            .into_iter()
            .filter(|w| !w.address.is_empty())
            .collect());
    }

    if let Ok(wallets) = serde_json::from_str::<Vec<WalletEntry>>(&content) {
        return Ok(wallets
            .into_iter()
            .filter(|w| !w.address.is_empty())
            .collect());
    }

    anyhow::bail!("failed to parse watchlist file: {}", path.display())
}

// ── Position Sizing ──────────────────────────────────────────────────────────

pub fn calculate_position_size(
    whale_size_usd: f64,
    sizing_multiplier: f64,
    account_balance: f64,
    max_position_pct: f64,
) -> f64 {
    let raw_size = whale_size_usd * sizing_multiplier;
    let max_size = account_balance * (max_position_pct / 100.0);
    raw_size.min(max_size)
}

// ── Risk Management ──────────────────────────────────────────────────────────

pub fn can_open_position(open_count: usize, max_positions: usize) -> bool {
    open_count < max_positions
}

pub fn is_stop_loss_triggered(
    entry_price: f64,
    current_price: f64,
    is_long: bool,
    stop_loss_pct: f64,
) -> bool {
    if entry_price <= 0.0 {
        return false;
    }
    let pnl_pct = if is_long {
        (current_price - entry_price) / entry_price * 100.0
    } else {
        (entry_price - current_price) / entry_price * 100.0
    };
    pnl_pct <= -stop_loss_pct
}

pub fn unrealized_pnl_pct(entry_price: f64, current_price: f64, is_long: bool) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }
    if is_long {
        (current_price - entry_price) / entry_price * 100.0
    } else {
        (entry_price - current_price) / entry_price * 100.0
    }
}

pub fn unrealized_pnl_usd(
    size_usd: f64,
    entry_price: f64,
    current_price: f64,
    is_long: bool,
) -> f64 {
    size_usd * unrealized_pnl_pct(entry_price, current_price, is_long) / 100.0
}

pub fn direction_from_size(size: &str) -> Direction {
    let sz: f64 = size.parse().unwrap_or(0.0);
    if sz >= 0.0 {
        Direction::Long
    } else {
        Direction::Short
    }
}

pub fn notional_usd(size: &str, price: f64) -> f64 {
    let sz: f64 = size.parse().unwrap_or(0.0);
    sz.abs() * price
}

// ── Trade Log ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TradeLog {
    path: PathBuf,
    trades: Vec<CopyTrade>,
}

impl TradeLog {
    pub fn new(path: PathBuf) -> Self {
        let trades = Self::load_from_disk(&path).unwrap_or_default();
        Self { path, trades }
    }

    fn load_from_disk(path: &PathBuf) -> Result<Vec<CopyTrade>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        let trades: Vec<CopyTrade> = serde_json::from_str(&content)?;
        Ok(trades)
    }

    pub fn trades(&self) -> &[CopyTrade] {
        &self.trades
    }

    pub fn add_trade(&mut self, trade: CopyTrade) -> Result<()> {
        self.trades.push(trade);
        self.persist()
    }

    pub fn update_trade(&mut self, trade_id: &str, updated: CopyTrade) -> Result<()> {
        if let Some(trade) = self.trades.iter_mut().find(|t| t.id == trade_id) {
            *trade = updated;
            self.persist()
        } else {
            anyhow::bail!("trade not found: {}", trade_id)
        }
    }

    pub fn open_trades(&self) -> Vec<&CopyTrade> {
        self.trades
            .iter()
            .filter(|t| t.status == TradeStatus::Open)
            .collect()
    }

    pub fn find_open_trade(&self, wallet_address: &str, market: &str) -> Option<&CopyTrade> {
        self.trades.iter().find(|t| {
            t.status == TradeStatus::Open
                && t.wallet_address == wallet_address
                && t.market == market
        })
    }

    pub fn open_count(&self) -> usize {
        self.trades
            .iter()
            .filter(|t| t.status == TradeStatus::Open)
            .count()
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        let tmp_path = self.path.with_extension("json.tmp");
        let json_str =
            serde_json::to_string_pretty(&self.trades).context("failed to serialize trade log")?;
        fs::write(&tmp_path, &json_str)
            .with_context(|| format!("failed to write temp file: {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &self.path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp_path.display(),
                self.path.display()
            )
        })?;
        Ok(())
    }
}

// ── HL Paper Fee Helpers ─────────────────────────────────────────────────────

/// Calculate hours between an RFC3339 timestamp and now.
fn hours_since_timestamp(timestamp: &str) -> f64 {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return 0.0;
    };
    let now = Utc::now();
    let diff = now.naive_utc() - dt.naive_utc();
    diff.num_seconds() as f64 / 3600.0
}

/// Compute HL paper fees for opening a position.
fn hl_entry_fee(size_usd: f64) -> f64 {
    size_usd * HL_TAKER_FEE_RATE
}

/// Compute HL paper fees for closing a position.
fn hl_close_fees(size_usd: f64, hours_held: f64) -> (f64, f64) {
    let exit_fee = size_usd * HL_TAKER_FEE_RATE;
    let borrow_fee = size_usd * HL_BORROW_RATE_PER_HOUR * hours_held;
    (exit_fee, borrow_fee)
}

/// Compute net PnL for HL paper mode: gross_pnl - entry_fee - exit_fee - borrow_fee.
fn hl_net_pnl(gross_pnl: f64, entry_fee: f64, exit_fee: f64, borrow_fee: f64) -> f64 {
    gross_pnl - entry_fee - exit_fee - borrow_fee
}

// ── Engine ───────────────────────────────────────────────────────────────────

pub type SnapshotMap = HashMap<String, Vec<WalletPosition>>;

#[derive(Debug, Clone)]
pub struct CopyTraderConfig {
    pub max_position_pct: f64,
    pub max_positions: usize,
    pub stop_loss_pct: f64,
    pub lag_secs: u64,
    pub sizing_multiplier: f64,
    pub paper_balance: f64,
    pub poll_interval_secs: u64,
    /// When true, use Hyperliquid fee model (0.035% taker, 0.01%/hr borrow).
    pub hl_paper: bool,
}

pub struct CopyTraderEngine {
    config: CopyTraderConfig,
    trade_log: TradeLog,
    snapshots: SnapshotMap,
    account_balance: f64,
}

impl CopyTraderEngine {
    pub fn new(config: CopyTraderConfig, trade_log: TradeLog) -> Self {
        let account_balance = config.paper_balance;
        Self {
            config,
            trade_log,
            snapshots: HashMap::new(),
            account_balance,
        }
    }

    pub fn process_wallet(
        &mut self,
        wallet_address: &str,
        current_positions: Vec<WalletPosition>,
        mark_prices: &HashMap<String, f64>,
    ) -> Result<usize> {
        let prev = self
            .snapshots
            .get(wallet_address)
            .cloned()
            .unwrap_or_default();

        let diff = detect_positions_diff(&prev, &current_positions);
        let mut new_count = 0;

        for pos in &diff.new_positions {
            let entry_price: f64 = pos.entry_px.parse().unwrap_or_else(|_| {
                mark_prices.get(&pos.coin).copied().unwrap_or(0.0)
            });

            let mark_price = mark_prices.get(&pos.coin).copied().unwrap_or(entry_price);
            let whale_size = notional_usd(&pos.size, mark_price);
            let direction = direction_from_size(&pos.size);

            if !can_open_position(self.trade_log.open_count(), self.config.max_positions) {
                info!(
                    wallet = &wallet_address[..wallet_address.len().min(12)],
                    market = %pos.coin,
                    "skipped: max positions reached ({})",
                    self.config.max_positions
                );
                continue;
            }

            let our_size = calculate_position_size(
                whale_size,
                self.config.sizing_multiplier,
                self.account_balance,
                self.config.max_position_pct,
            );

            if our_size <= 0.0 {
                continue;
            }

            let trade = CopyTrade {
                id: format!(
                    "ct-{}-{:04}",
                    Utc::now().timestamp_millis(),
                    rand::random::<u16>()
                ),
                timestamp: Utc::now().to_rfc3339(),
                wallet_address: wallet_address.to_string(),
                market: pos.coin.clone(),
                direction,
                size_usd: our_size,
                entry_price,
                status: TradeStatus::Open,
                close_reason: None,
                exit_price: None,
                pnl_usd: None,
                whale_size_usd: whale_size,
                sizing_multiplier: self.config.sizing_multiplier,
                entry_fee: if self.config.hl_paper {
                    Some(hl_entry_fee(our_size))
                } else {
                    None
                },
                exit_fee: None,
                borrow_fee: None,
                hours_held: None,
            };

            info!(
                wallet = &wallet_address[..wallet_address.len().min(12)],
                market = %pos.coin,
                direction = %direction,
                size_usd = our_size,
                entry_price = entry_price,
                whale_size = whale_size,
                "new position detected — mirroring"
            );

            self.trade_log.add_trade(trade)?;
            new_count += 1;
        }

        for pos in &diff.closed_positions {
            if let Some(trade) = self
                .trade_log
                .find_open_trade(wallet_address, &pos.coin)
                .cloned()
            {
                let mark_price = mark_prices
                    .get(&pos.coin)
                    .copied()
                    .unwrap_or(trade.entry_price);

                let mut updated = trade.clone();
                updated.status = TradeStatus::Closed;
                updated.close_reason = Some(CloseReason::WalletClosed);
                updated.exit_price = Some(mark_price);

                let gross_pnl = unrealized_pnl_usd(
                    updated.size_usd,
                    updated.entry_price,
                    mark_price,
                    matches!(updated.direction, Direction::Long),
                );

                if self.config.hl_paper {
                    let hours = hours_since_timestamp(&updated.timestamp);
                    let entry_fee = updated.entry_fee.unwrap_or(hl_entry_fee(updated.size_usd));
                    let (exit_fee, borrow_fee) = hl_close_fees(updated.size_usd, hours);
                    let net = hl_net_pnl(gross_pnl, entry_fee, exit_fee, borrow_fee);
                    updated.entry_fee = Some(entry_fee);
                    updated.exit_fee = Some(exit_fee);
                    updated.borrow_fee = Some(borrow_fee);
                    updated.hours_held = Some(hours);
                    updated.pnl_usd = Some(net);
                } else {
                    updated.pnl_usd = Some(gross_pnl);
                }

                self.account_balance += updated.pnl_usd.unwrap_or(0.0);

                info!(
                    wallet = &wallet_address[..wallet_address.len().min(12)],
                    market = %pos.coin,
                    exit_price = mark_price,
                    pnl = updated.pnl_usd.unwrap_or(0.0),
                    "wallet closed position — mirroring closure"
                );

                let id = updated.id.clone();
                self.trade_log.update_trade(&id, updated)?;
            }
        }

        self.snapshots
            .insert(wallet_address.to_string(), current_positions);
        Ok(new_count)
    }

    pub fn check_stop_losses(&mut self, mark_prices: &HashMap<String, f64>) -> Result<usize> {
        let open_ids: Vec<String> = self
            .trade_log
            .open_trades()
            .iter()
            .map(|t| t.id.clone())
            .collect();

        let mut closed_count = 0;

        for trade_id in open_ids {
            let trade = self
                .trade_log
                .trades()
                .iter()
                .find(|t| t.id == trade_id)
                .cloned();
            let trade = match trade {
                Some(t) if t.status == TradeStatus::Open => t,
                _ => continue,
            };

            let is_long = matches!(trade.direction, Direction::Long);
            let current_price = mark_prices
                .get(&trade.market)
                .copied()
                .unwrap_or(trade.entry_price);

            if is_stop_loss_triggered(trade.entry_price, current_price, is_long, self.config.stop_loss_pct) {
                let mut updated = trade.clone();
                updated.status = TradeStatus::Closed;
                updated.close_reason = Some(CloseReason::StopLoss);
                updated.exit_price = Some(current_price);

                let gross_pnl = unrealized_pnl_usd(
                    updated.size_usd,
                    updated.entry_price,
                    current_price,
                    is_long,
                );

                if self.config.hl_paper {
                    let hours = hours_since_timestamp(&updated.timestamp);
                    let entry_fee = updated.entry_fee.unwrap_or(hl_entry_fee(updated.size_usd));
                    let (exit_fee, borrow_fee) = hl_close_fees(updated.size_usd, hours);
                    let net = hl_net_pnl(gross_pnl, entry_fee, exit_fee, borrow_fee);
                    updated.entry_fee = Some(entry_fee);
                    updated.exit_fee = Some(exit_fee);
                    updated.borrow_fee = Some(borrow_fee);
                    updated.hours_held = Some(hours);
                    updated.pnl_usd = Some(net);
                } else {
                    updated.pnl_usd = Some(gross_pnl);
                }

                self.account_balance += updated.pnl_usd.unwrap_or(0.0);

                info!(
                    market = %trade.market,
                    wallet = &trade.wallet_address[..trade.wallet_address.len().min(12)],
                    pnl = updated.pnl_usd.unwrap_or(0.0),
                    "stop-loss triggered at {:.2}% loss",
                    self.config.stop_loss_pct
                );

                let id = updated.id.clone();
                self.trade_log.update_trade(&id, updated)?;
                closed_count += 1;
            }
        }

        Ok(closed_count)
    }

    pub fn account_balance(&self) -> f64 {
        self.account_balance
    }

    pub fn trade_log(&self) -> &TradeLog {
        &self.trade_log
    }

    pub fn config(&self) -> &CopyTraderConfig {
        &self.config
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    args.validate()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .init();

    info!("=== copy-trader ===");
    let mode_str = if args.hl_paper {
        "hl-paper"
    } else if args.paper {
        "paper"
    } else {
        "live"
    };
    info!(
        mode = mode_str,
        watchlist = %args.watchlist.display(),
        max_position_pct = args.max_position_pct,
        max_positions = args.max_positions,
        stop_loss_pct = args.stop_loss_pct,
        lag_secs = args.lag_secs,
        sizing_multiplier = args.sizing_multiplier,
        output = %args.output.display(),
        paper_balance = args.paper_balance,
        "Configuration"
    );

    if args.live {
        anyhow::bail!("--live mode is not yet implemented. Use --paper or --hl-paper for paper trading.");
    }

    let wallets = load_watchlist(&args.watchlist)?;
    if wallets.is_empty() {
        info!("no wallets to monitor — exiting");
        return Ok(());
    }
    info!(count = wallets.len(), "Loaded watchlist");

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    let trade_log = TradeLog::new(args.output.clone());
    let config = CopyTraderConfig {
        max_position_pct: args.max_position_pct,
        max_positions: args.max_positions,
        stop_loss_pct: args.stop_loss_pct,
        lag_secs: args.lag_secs,
        sizing_multiplier: args.sizing_multiplier,
        paper_balance: args.paper_balance,
        poll_interval_secs: 30,
        hl_paper: args.hl_paper,
    };
    let mut engine = CopyTraderEngine::new(config, trade_log);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        info!("Received SIGINT, shutting down...");
        r.store(false, AtomicOrdering::SeqCst);
    })
    .context("Failed to set ctrlc handler")?;

    info!(poll_interval_secs = engine.config().poll_interval_secs, "Starting polling loop");

    loop {
        if !running.load(AtomicOrdering::SeqCst) {
            info!("Shutting down...");
            break;
        }

        let mut total_new = 0;
        let mut total_errors = 0;
        let mut accumulated_mark_prices: HashMap<String, f64> = HashMap::new();

        for wallet in &wallets {
            match fetch_wallet_positions(&client, &wallet.address).await {
                Ok(positions) => {
                    let wallet_mark_prices: HashMap<String, f64> = positions
                        .iter()
                        .filter_map(|p| {
                            let px: f64 = p.mark_px.as_ref()?.parse().ok()?;
                            Some((p.coin.clone(), px))
                        })
                        .collect();

                    // Accumulate mark prices across all wallets for stop-loss checks
                    for (coin, px) in &wallet_mark_prices {
                        accumulated_mark_prices.entry(coin.clone()).or_insert(*px);
                    }

                    match engine.process_wallet(&wallet.address, positions, &wallet_mark_prices) {
                        Ok(count) => total_new += count,
                        Err(e) => {
                            warn!(
                                wallet = &wallet.address[..wallet.address.len().min(12)],
                                error = %e,
                                "Failed to process wallet"
                            );
                            total_errors += 1;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        wallet = &wallet.address[..wallet.address.len().min(12)],
                        error = %e,
                        "API failure fetching positions — will retry next cycle"
                    );
                    total_errors += 1;
                }
            }
        }

        if let Err(e) = engine.check_stop_losses(&accumulated_mark_prices) {
            error!(error = %e, "Failed to check stop-losses");
        }

        if total_new > 0 || total_errors > 0 {
            debug!(
                new_mirrored = total_new,
                errors = total_errors,
                open_positions = engine.trade_log().open_count(),
                balance = engine.account_balance(),
                "Poll cycle completed"
            );
        }

        let sleep_duration = Duration::from_secs(engine.config().poll_interval_secs);
        let start = std::time::Instant::now();
        while start.elapsed() < sleep_duration {
            if !running.load(AtomicOrdering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    let open = engine.trade_log().open_trades();
    let total_trades = engine.trade_log().trades().len();

    info!("=== Copy Trader Shutdown ===");
    info!(total_trades = total_trades, open_positions = open.len(), account_balance = engine.account_balance(), "Final state");

    for trade in &open {
        info!(
            "  {} {} ${:.2} @ ${:.2} (wallet: {}...)",
            trade.direction,
            trade.market,
            trade.size_usd,
            trade.entry_price,
            &trade.wallet_address[..trade.wallet_address.len().min(12)]
        );
    }

    let closed: Vec<&CopyTrade> = engine
        .trade_log()
        .trades()
        .iter()
        .filter(|t| t.status == TradeStatus::Closed)
        .collect();
    if !closed.is_empty() {
        let pnl: f64 = closed.iter().filter_map(|t| t.pnl_usd).sum();
        let wins = closed.iter().filter(|t| t.pnl_usd.unwrap_or(0.0) > 0.0).count();
        let wr = wins as f64 / closed.len() as f64 * 100.0;
        info!(closed_trades = closed.len(), total_pnl = pnl, win_rate = format!("{:.1}%", wr), "Closed trade summary");
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    macro_rules! hashmap {
        ($($key:expr => $val:expr),* $(,)?) => {{
            #[allow(unused_mut)]
            let mut m = std::collections::HashMap::new();
            $(
                m.insert($key.to_string(), $val);
            )*
            m
        }};
    }

    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    // === CLI Validation (12 tests) ===

    #[test]
    fn test_validate_max_position_pct_valid() {
        assert!(validate_max_position_pct("1.0").is_ok());
        assert!(validate_max_position_pct("0.1").is_ok());
    }

    #[test]
    fn test_validate_max_position_pct_zero_rejected() {
        assert!(validate_max_position_pct("0.0").is_err());
    }

    #[test]
    fn test_validate_max_position_pct_negative_rejected() {
        assert!(validate_max_position_pct("-1.0").is_err());
    }

    #[test]
    fn test_validate_max_position_pct_non_numeric() {
        assert!(validate_max_position_pct("abc").is_err());
    }

    #[test]
    fn test_validate_max_positions_valid() {
        assert!(validate_max_positions("1").is_ok());
        assert!(validate_max_positions("10").is_ok());
    }

    #[test]
    fn test_validate_max_positions_zero_rejected() {
        assert!(validate_max_positions("0").is_err());
    }

    #[test]
    fn test_validate_lag_secs_valid() {
        assert!(validate_lag_secs("0").is_ok());
        assert!(validate_lag_secs("30").is_ok());
    }

    #[test]
    fn test_validate_lag_secs_negative_rejected() {
        assert!(validate_lag_secs("-1").is_err());
    }

    #[test]
    fn test_validate_sizing_multiplier_valid() {
        assert!(validate_sizing_multiplier("0.1").is_ok());
    }

    #[test]
    fn test_validate_sizing_multiplier_zero_rejected() {
        assert!(validate_sizing_multiplier("0.0").is_err());
    }

    #[test]
    fn test_validate_stop_loss_valid() {
        assert!(validate_stop_loss_pct("5.0").is_ok());
    }

    #[test]
    fn test_validate_stop_loss_zero_rejected() {
        assert!(validate_stop_loss_pct("0.0").is_err());
    }

    // === Watchlist Loading (8 tests) ===

    #[test]
    fn test_load_watchlist_alpha_format() {
        let content = json!({"generated_at":"2026-05-23T00:00:00Z","wallets":[
            {"address":"0xabc","score":10.5,"sharpe":2.0,"pnl":50000.0,"tags":[],"positions":[],"decaying":false},
            {"address":"0xdef","score":8.0,"decaying":true}
        ]});
        let f = write_temp(&serde_json::to_string(&content).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].address, "0xabc");
        assert!(!w[0].decaying);
        assert!(w[1].decaying);
    }

    #[test]
    fn test_load_watchlist_array_format() {
        let f = write_temp(&serde_json::to_string(&json!([{"address":"0xabc","score":5.0}])).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn test_load_watchlist_empty_array() {
        let f = write_temp("[]");
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert!(w.is_empty());
    }

    #[test]
    fn test_load_watchlist_empty_wallets() {
        let f = write_temp(&serde_json::to_string(&json!({"generated_at":"t","wallets":[]})).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert!(w.is_empty());
    }

    #[test]
    fn test_load_watchlist_missing_file() {
        assert!(load_watchlist(&PathBuf::from("/tmp/no-such-xyz.json")).is_err());
    }

    #[test]
    fn test_load_watchlist_invalid_json() {
        let f = write_temp("{broken");
        assert!(load_watchlist(&f.path().to_path_buf()).is_err());
    }

    #[test]
    fn test_load_watchlist_filters_empty_address() {
        let f = write_temp(&serde_json::to_string(&json!([{"address":"0xabc","score":5.0},{"address":"","score":3.0}])).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn test_load_watchlist_defaults() {
        let f = write_temp(&serde_json::to_string(&json!([{"address":"0xabc","score":5.0}])).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w[0].sharpe, 0.0);
        assert!(w[0].tags.is_empty());
    }

    // === Position Sizing (7 tests) ===

    #[test]
    fn test_sizing_basic() {
        assert!((calculate_position_size(10000.0, 0.1, 10000.0, 10.0) - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_sizing_capped() {
        assert!((calculate_position_size(100000.0, 0.1, 1000.0, 10.0) - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_sizing_no_cap() {
        assert!((calculate_position_size(100.0, 0.1, 10000.0, 10.0) - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_sizing_zero_balance() {
        assert_eq!(calculate_position_size(10000.0, 0.1, 0.0, 10.0), 0.0);
    }

    #[test]
    fn test_sizing_small_whale() {
        assert!((calculate_position_size(50.0, 0.1, 10000.0, 10.0) - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_sizing_exact_boundary() {
        assert!((calculate_position_size(10000.0, 0.1, 10000.0, 10.0) - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_sizing_high_multiplier() {
        assert!((calculate_position_size(5000.0, 2.0, 10000.0, 10.0) - 1000.0).abs() < 0.01);
    }

    // === Max Positions (4 tests) ===

    #[test]
    fn test_can_open_below() { assert!(can_open_position(0, 3)); assert!(can_open_position(2, 3)); }

    #[test]
    fn test_can_open_at_limit() { assert!(!can_open_position(3, 3)); }

    #[test]
    fn test_can_open_above() { assert!(!can_open_position(4, 3)); }

    #[test]
    fn test_can_open_zero_max() { assert!(!can_open_position(0, 0)); }

    // === Stop-Loss (8 tests) ===

    #[test]
    fn test_sl_triggered_long() { assert!(is_stop_loss_triggered(100.0, 94.0, true, 5.0)); }

    #[test]
    fn test_sl_not_triggered_long() { assert!(!is_stop_loss_triggered(100.0, 96.0, true, 5.0)); }

    #[test]
    fn test_sl_triggered_short() { assert!(is_stop_loss_triggered(100.0, 106.0, false, 5.0)); }

    #[test]
    fn test_sl_not_triggered_short() { assert!(!is_stop_loss_triggered(100.0, 104.0, false, 5.0)); }

    #[test]
    fn test_sl_exact_threshold() { assert!(is_stop_loss_triggered(100.0, 95.0, true, 5.0)); }

    #[test]
    fn test_sl_zero_entry() { assert!(!is_stop_loss_triggered(0.0, 50.0, true, 5.0)); }

    #[test]
    fn test_sl_long_profitable() { assert!(!is_stop_loss_triggered(100.0, 110.0, true, 5.0)); }

    #[test]
    fn test_sl_short_profitable() { assert!(!is_stop_loss_triggered(100.0, 90.0, false, 5.0)); }

    // === PnL (6 tests) ===

    #[test]
    fn test_pnl_pct_long_profit() { assert!((unrealized_pnl_pct(100.0, 105.0, true) - 5.0).abs() < 0.01); }

    #[test]
    fn test_pnl_pct_long_loss() { assert!((unrealized_pnl_pct(100.0, 95.0, true) - (-5.0)).abs() < 0.01); }

    #[test]
    fn test_pnl_pct_short_profit() { assert!((unrealized_pnl_pct(100.0, 95.0, false) - 5.0).abs() < 0.01); }

    #[test]
    fn test_pnl_pct_short_loss() { assert!((unrealized_pnl_pct(100.0, 105.0, false) - (-5.0)).abs() < 0.01); }

    #[test]
    fn test_pnl_usd_basic() { assert!((unrealized_pnl_usd(1000.0, 100.0, 105.0, true) - 50.0).abs() < 0.01); }

    #[test]
    fn test_pnl_usd_loss() { assert!((unrealized_pnl_usd(1000.0, 100.0, 94.0, true) - (-60.0)).abs() < 0.01); }

    // === Direction & Notional (5 tests) ===

    #[test]
    fn test_direction_positive() { assert_eq!(direction_from_size("0.5"), Direction::Long); }

    #[test]
    fn test_direction_negative() { assert_eq!(direction_from_size("-2.0"), Direction::Short); }

    #[test]
    fn test_direction_zero() { assert_eq!(direction_from_size("0.0"), Direction::Long); }

    #[test]
    fn test_notional_basic() { assert!((notional_usd("0.5", 60000.0) - 30000.0).abs() < 0.01); }

    #[test]
    fn test_notional_short() { assert!((notional_usd("-2.0", 3000.0) - 6000.0).abs() < 0.01); }

    // === Position Parsing (4 tests) ===

    #[test]
    fn test_parse_positions_multiple() {
        let raw = json!({"assetPositions":[
            {"position":{"coin":"BTC","szi":"0.5","entryPx":"60000.0","markPx":"61000.0"}},
            {"position":{"coin":"ETH","szi":"-2.0","entryPx":"3000.0"}}
        ]});
        let p = parse_positions(&raw);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].coin, "BTC");
        assert_eq!(p[1].coin, "ETH");
    }

    #[test]
    fn test_parse_positions_empty() {
        assert!(parse_positions(&json!({"assetPositions":[]})).is_empty());
    }

    #[test]
    fn test_parse_positions_zero_filtered() {
        let raw = json!({"assetPositions":[
            {"position":{"coin":"BTC","szi":"0.5","entryPx":"60000.0"}},
            {"position":{"coin":"ETH","szi":"0.0","entryPx":"3000.0"}}
        ]});
        assert_eq!(parse_positions(&raw).len(), 1);
    }

    #[test]
    fn test_parse_positions_size_field_fallback() {
        let raw = json!({"assetPositions":[{"position":{"coin":"SOL","size":"10.0","entryPx":"150.0"}}]});
        let p = parse_positions(&raw);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].coin, "SOL");
    }

    // === Position Diff (5 tests) ===

    #[test]
    fn test_diff_new_from_empty() {
        let old: Vec<WalletPosition> = vec![];
        let new = vec![make_wp("BTC", "0.5", "60000.0")];
        let d = detect_positions_diff(&old, &new);
        assert_eq!(d.new_positions.len(), 1);
        assert!(d.closed_positions.is_empty());
    }

    #[test]
    fn test_diff_closed() {
        let old = vec![make_wp("BTC", "0.5", "60000.0")];
        let new: Vec<WalletPosition> = vec![];
        let d = detect_positions_diff(&old, &new);
        assert!(d.new_positions.is_empty());
        assert_eq!(d.closed_positions.len(), 1);
    }

    #[test]
    fn test_diff_no_change() {
        let old = vec![make_wp("BTC", "0.5", "60000.0")];
        let new = vec![make_wp("BTC", "0.5", "60000.0")];
        let d = detect_positions_diff(&old, &new);
        assert!(d.new_positions.is_empty());
        assert!(d.closed_positions.is_empty());
    }

    #[test]
    fn test_diff_mixed() {
        let old = vec![make_wp("BTC", "0.5", "60000.0"), make_wp("ETH", "2.0", "3000.0")];
        let new = vec![make_wp("BTC", "0.5", "60000.0"), make_wp("SOL", "10.0", "150.0")];
        let d = detect_positions_diff(&old, &new);
        assert_eq!(d.new_positions.len(), 1);
        assert_eq!(d.closed_positions.len(), 1);
    }

    #[test]
    fn test_diff_ignores_zero() {
        let old = vec![make_wp("BTC", "0.5", "60000.0")];
        let new = vec![make_wp("BTC", "0.5", "60000.0"), make_wp("ETH", "0.0", "3000.0")];
        let d = detect_positions_diff(&old, &new);
        assert!(d.new_positions.is_empty());
    }

    // === TradeLog (8 tests) ===

    #[test]
    fn test_tradelog_new_empty() {
        let dir = std::env::temp_dir().join("ct-test-empty");
        let _ = fs::create_dir_all(&dir);
        let log = TradeLog::new(dir.join("t.json"));
        assert!(log.trades().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_add() {
        let dir = std::env::temp_dir().join("ct-test-add");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        assert_eq!(log.trades().len(), 1);
        assert_eq!(log.open_count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_update() {
        let dir = std::env::temp_dir().join("ct-test-upd");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        let mut c = make_trade("t1", "0xa", "BTC", TradeStatus::Closed);
        c.close_reason = Some(CloseReason::WalletClosed);
        log.update_trade("t1", c).unwrap();
        assert_eq!(log.open_count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_find_open() {
        let dir = std::env::temp_dir().join("ct-test-find");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        assert!(log.find_open_trade("0xa", "BTC").is_some());
        assert!(log.find_open_trade("0xa", "ETH").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_find_ignores_closed() {
        let dir = std::env::temp_dir().join("ct-test-fic");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        let c = make_trade("t1", "0xa", "BTC", TradeStatus::Closed);
        log.update_trade("t1", c).unwrap();
        assert!(log.find_open_trade("0xa", "BTC").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_open_list() {
        let dir = std::env::temp_dir().join("ct-test-ol");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        log.add_trade(make_trade("t2", "0xb", "ETH", TradeStatus::Open)).unwrap();
        log.add_trade(make_trade("t3", "0xc", "SOL", TradeStatus::Closed)).unwrap();
        assert_eq!(log.open_trades().len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_valid_json() {
        let dir = std::env::temp_dir().join("ct-test-vj");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        log.add_trade(make_trade("t1", "0xa", "BTC", TradeStatus::Open)).unwrap();
        let content = fs::read_to_string(&p).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_array());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tradelog_update_nonexistent() {
        let dir = std::env::temp_dir().join("ct-test-ne");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("t.json");
        let mut log = TradeLog::new(p.clone());
        assert!(log.update_trade("nope", make_trade("nope", "0xa", "BTC", TradeStatus::Closed)).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    // === Engine: New Position (3 tests) ===

    #[test]
    fn test_engine_new_position() {
        let dir = std::env::temp_dir().join("ct-eng-new");
        let _ = fs::create_dir_all(&dir);
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(make_config(), tl);
        eng.process_wallet("0xa", vec![], &hashmap!()).unwrap();
        let c = eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(c, 1);
        assert_eq!(eng.trade_log().open_count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_engine_max_positions() {
        let dir = std::env::temp_dir().join("ct-eng-max");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.max_positions = 2;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        let ps = vec![make_wp("BTC","0.5","60000.0"), make_wp("ETH","2.0","3000.0")];
        eng.process_wallet("0xa", ps, &hashmap!("BTC"=>60000.0,"ETH"=>3000.0)).unwrap();
        assert_eq!(eng.trade_log().open_count(), 2);
        let ps2 = vec![make_wp("BTC","0.5","60000.0"), make_wp("ETH","2.0","3000.0"), make_wp("SOL","10.0","150.0")];
        eng.process_wallet("0xa", ps2, &hashmap!("BTC"=>60000.0,"ETH"=>3000.0,"SOL"=>150.0)).unwrap();
        assert_eq!(eng.trade_log().open_count(), 2); // SOL skipped
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_engine_sizing() {
        let dir = std::env::temp_dir().join("ct-eng-sz");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.sizing_multiplier = 0.1; cfg.paper_balance = 10000.0; cfg.max_position_pct = 10.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        // whale=30000, raw=3000, max=1000 → capped at 1000
        assert!((eng.trade_log().trades()[0].size_usd - 1000.0).abs() < 0.01);
        let _ = fs::remove_dir_all(&dir);
    }

    // === Engine: Closure (1 test) ===

    #[test]
    fn test_engine_wallet_closure() {
        let dir = std::env::temp_dir().join("ct-eng-cl");
        let _ = fs::create_dir_all(&dir);
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(make_config(), tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        eng.process_wallet("0xa", vec![], &hashmap!("BTC"=>61000.0)).unwrap();
        assert_eq!(eng.trade_log().open_count(), 0);
        let t = &eng.trade_log().trades()[0];
        assert_eq!(t.close_reason, Some(CloseReason::WalletClosed));
        assert_eq!(t.exit_price, Some(61000.0));
        let _ = fs::remove_dir_all(&dir);
    }

    // === Engine: Stop-Loss (3 tests) ===

    #[test]
    fn test_engine_sl_triggered() {
        let dir = std::env::temp_dir().join("ct-eng-sl");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.stop_loss_pct = 5.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        let c = eng.check_stop_losses(&hashmap!("BTC"=>56400.0)).unwrap();
        assert_eq!(c, 1);
        assert_eq!(eng.trade_log().trades()[0].close_reason, Some(CloseReason::StopLoss));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_engine_sl_not_triggered() {
        let dir = std::env::temp_dir().join("ct-eng-sln");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.stop_loss_pct = 5.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(eng.check_stop_losses(&hashmap!("BTC"=>58200.0)).unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_engine_sl_short() {
        let dir = std::env::temp_dir().join("ct-eng-sls");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.stop_loss_pct = 5.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","-0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(eng.check_stop_losses(&hashmap!("BTC"=>63600.0)).unwrap(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    // === Engine: Snapshot Persistence (1 test) ===

    #[test]
    fn test_engine_snapshot_persistence() {
        let dir = std::env::temp_dir().join("ct-eng-sp");
        let _ = fs::create_dir_all(&dir);
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(make_config(), tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        let c = eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(c, 0); // no duplicate
        assert_eq!(eng.trade_log().open_count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    // === Engine: Balance (2 tests) ===

    #[test]
    fn test_engine_balance_profit() {
        let dir = std::env::temp_dir().join("ct-eng-bp");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.paper_balance = 10000.0; cfg.max_position_pct = 100.0; cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        let b = eng.account_balance();
        eng.process_wallet("0xa", vec![], &hashmap!("BTC"=>63000.0)).unwrap();
        assert!(eng.account_balance() > b);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_engine_balance_loss() {
        let dir = std::env::temp_dir().join("ct-eng-bl");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.paper_balance = 10000.0; cfg.max_position_pct = 100.0; cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet("0xa", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        eng.process_wallet("0xa", vec![], &hashmap!("BTC"=>57000.0)).unwrap();
        assert!(eng.account_balance() < 10000.0);
        let _ = fs::remove_dir_all(&dir);
    }

    // === Display (3 tests) ===

    #[test]
    fn test_direction_display() {
        assert_eq!(format!("{}", Direction::Long), "long");
        assert_eq!(format!("{}", Direction::Short), "short");
    }

    #[test]
    fn test_status_display() {
        assert_eq!(format!("{}", TradeStatus::Open), "open");
        assert_eq!(format!("{}", TradeStatus::Closed), "closed");
    }

    #[test]
    fn test_close_reason_display() {
        assert_eq!(format!("{}", CloseReason::WalletClosed), "wallet_closed");
        assert_eq!(format!("{}", CloseReason::StopLoss), "stop_loss");
    }

    // === Serialization (3 tests) ===

    #[test]
    fn test_trade_serde_roundtrip() {
        let t = CopyTrade {
            id: "id-1".into(), timestamp: "2026-05-23T00:00:00Z".into(),
            wallet_address: "0xabc".into(), market: "BTC".into(),
            direction: Direction::Long, size_usd: 100.0, entry_price: 60000.0,
            status: TradeStatus::Open, close_reason: None, exit_price: None, pnl_usd: None,
            whale_size_usd: 1000.0, sizing_multiplier: 0.1,
            entry_fee: None, exit_fee: None, borrow_fee: None, hours_held: None,
        };
        let s = serde_json::to_string(&t).unwrap();
        let p: CopyTrade = serde_json::from_str(&s).unwrap();
        assert_eq!(p.id, "id-1");
        assert_eq!(p.direction, Direction::Long);
    }

    #[test]
    fn test_trade_closed_serde() {
        let t = CopyTrade {
            id: "id-2".into(), timestamp: "t".into(), wallet_address: "0xd".into(),
            market: "ETH".into(), direction: Direction::Short, size_usd: 50.0, entry_price: 3000.0,
            status: TradeStatus::Closed, close_reason: Some(CloseReason::StopLoss),
            exit_price: Some(3150.0), pnl_usd: Some(-2.5),
            whale_size_usd: 500.0, sizing_multiplier: 0.1,
            entry_fee: Some(0.0175), exit_fee: Some(0.0175), borrow_fee: Some(0.005), hours_held: Some(1.0),
        };
        let s = serde_json::to_string(&t).unwrap();
        let p: CopyTrade = serde_json::from_str(&s).unwrap();
        assert_eq!(p.close_reason, Some(CloseReason::StopLoss));
        assert!((p.entry_fee.unwrap() - 0.0175).abs() < 1e-10);
    }

    #[test]
    fn test_trade_required_fields() {
        let t = CopyTrade {
            id: "i".into(), timestamp: "t".into(), wallet_address: "w".into(),
            market: "M".into(), direction: Direction::Long, size_usd: 1.0, entry_price: 1.0,
            status: TradeStatus::Open, close_reason: None, exit_price: None, pnl_usd: None,
            whale_size_usd: 1.0, sizing_multiplier: 0.1,
            entry_fee: None, exit_fee: None, borrow_fee: None, hours_held: None,
        };
        let v = serde_json::to_value(&t).unwrap();
        for f in &["id","timestamp","wallet_address","market","direction","size_usd","entry_price","status"] {
            assert!(v.get(*f).is_some(), "missing: {}", f);
        }
        assert!(v.get("close_reason").is_none());
        assert!(v.get("entry_fee").is_none());
        assert!(v.get("exit_fee").is_none());
        assert!(v.get("borrow_fee").is_none());
        assert!(v.get("hours_held").is_none());
    }

    // === Integration: Full Pipeline (2 tests) ===

    #[test]
    fn test_full_pipeline() {
        let dir = std::env::temp_dir().join("ct-full");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config(); cfg.max_position_pct = 100.0; cfg.sizing_multiplier = 0.1; cfg.stop_loss_pct = 5.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        // Open
        let c = eng.process_wallet("0xw", vec![make_wp("BTC","1.0","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(c, 1);
        let t = &eng.trade_log().trades()[0];
        assert_eq!(t.direction, Direction::Long);
        assert!((t.size_usd - 6000.0).abs() < 0.01);
        // No change
        let c = eng.process_wallet("0xw", vec![make_wp("BTC","1.0","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(c, 0);
        // Close
        eng.process_wallet("0xw", vec![], &hashmap!("BTC"=>62000.0)).unwrap();
        let t = &eng.trade_log().trades()[0];
        assert_eq!(t.close_reason, Some(CloseReason::WalletClosed));
        let exp = 6000.0 * (62000.0 - 60000.0) / 60000.0;
        assert!((t.pnl_usd.unwrap() - exp).abs() < 0.01);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_multi_wallet_pipeline() {
        let dir = std::env::temp_dir().join("ct-multi");
        let _ = fs::create_dir_all(&dir);
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(make_config(), tl);
        eng.process_wallet("w1", vec![make_wp("BTC","0.5","60000.0")], &hashmap!("BTC"=>60000.0)).unwrap();
        eng.process_wallet("w2", vec![make_wp("ETH","2.0","3000.0")], &hashmap!("ETH"=>3000.0)).unwrap();
        assert_eq!(eng.trade_log().open_count(), 2);
        eng.process_wallet("w1", vec![], &hashmap!("BTC"=>60000.0)).unwrap();
        assert_eq!(eng.trade_log().open_count(), 1);
        assert_eq!(eng.trade_log().trades().len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    // === Bug-Fix: Stop-Loss with Real Mark Prices (6 tests) ===

    #[test]
    fn test_stop_loss_fires_long() {
        // Long position with price dropped 6% (below 5% threshold)
        let dir = std::env::temp_dir().join("ct-sl-long");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        // Open long BTC at 60000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();
        // Price drops 6% → 56400
        let mark_prices = hashmap!("BTC" => 56400.0);
        let closed = eng.check_stop_losses(&mark_prices).unwrap();
        assert_eq!(closed, 1);
        let trade = &eng.trade_log().trades()[0];
        assert_eq!(trade.status, TradeStatus::Closed);
        assert_eq!(trade.close_reason, Some(CloseReason::StopLoss));
        assert!(trade.pnl_usd.unwrap() < 0.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stop_loss_fires_short() {
        // Short position with price risen 6% (above 5% threshold)
        let dir = std::env::temp_dir().join("ct-sl-short");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        // Open short ETH at 3000
        eng.process_wallet(
            "0xa",
            vec![make_wp("ETH", "-2.0", "3000.0")],
            &hashmap!("ETH" => 3000.0),
        )
        .unwrap();
        // Price rises 6% → 3180
        let mark_prices = hashmap!("ETH" => 3180.0);
        let closed = eng.check_stop_losses(&mark_prices).unwrap();
        assert_eq!(closed, 1);
        let trade = &eng.trade_log().trades()[0];
        assert_eq!(trade.status, TradeStatus::Closed);
        assert_eq!(trade.close_reason, Some(CloseReason::StopLoss));
        assert!(trade.pnl_usd.unwrap() < 0.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stop_loss_no_trigger() {
        // Position within stop-loss threshold — should NOT close
        let dir = std::env::temp_dir().join("ct-sl-no");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();
        // Price only drops 3% → 58200 (within 5% threshold)
        let mark_prices = hashmap!("BTC" => 58200.0);
        let closed = eng.check_stop_losses(&mark_prices).unwrap();
        assert_eq!(closed, 0);
        assert_eq!(eng.trade_log().open_count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_mark_prices_populated() {
        // Verify that mark_prices HashMap is correctly populated from position data
        let positions = vec![
            WalletPosition {
                coin: "BTC".into(),
                size: "1.0".into(),
                entry_px: "60000.0".into(),
                mark_px: Some("61000.0".into()),
            },
            WalletPosition {
                coin: "ETH".into(),
                size: "5.0".into(),
                entry_px: "3000.0".into(),
                mark_px: Some("3100.0".into()),
            },
            WalletPosition {
                coin: "SOL".into(),
                size: "100.0".into(),
                entry_px: "150.0".into(),
                mark_px: None, // No mark price
            },
        ];

        let mark_prices: HashMap<String, f64> = positions
            .iter()
            .filter_map(|p| {
                let px: f64 = p.mark_px.as_ref()?.parse().ok()?;
                Some((p.coin.clone(), px))
            })
            .collect();

        // Should have 2 entries (BTC, ETH), not SOL (no mark_px)
        assert_eq!(mark_prices.len(), 2);
        assert!((mark_prices["BTC"] - 61000.0).abs() < 0.01);
        assert!((mark_prices["ETH"] - 3100.0).abs() < 0.01);
        assert!(!mark_prices.contains_key("SOL"));
    }

    #[test]
    fn test_stop_loss_no_empty_hashmap() {
        // Verify stop-loss does NOT fire when mark_prices is empty —
        // this proves the bug (empty HashMap) doesn't silently prevent stop-losses
        // by showing that with real data it works, and empty data means no closure
        // when the fallback is the entry price (which equals the entry, so no loss)
        let dir = std::env::temp_dir().join("ct-sl-empty");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        // With empty HashMap, check_stop_losses falls back to entry_price → no loss
        let empty: HashMap<String, f64> = HashMap::new();
        let closed = eng.check_stop_losses(&empty).unwrap();
        assert_eq!(
            closed, 0,
            "Empty HashMap should not trigger stop-loss (falls back to entry price = no loss)"
        );
        assert_eq!(eng.trade_log().open_count(), 1);

        // But with real mark prices showing a 6% drop, it SHOULD fire
        let real_prices = hashmap!("BTC" => 56400.0);
        let closed = eng.check_stop_losses(&real_prices).unwrap();
        assert_eq!(
            closed, 1,
            "Real mark prices should trigger stop-loss for 6% drop"
        );
        assert_eq!(eng.trade_log().open_count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stop_loss_accumulated_mark_prices_multi_wallet() {
        // Simulate the main-loop pattern: accumulate mark prices across wallets
        let dir = std::env::temp_dir().join("ct-sl-acc");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Wallet 1: BTC position
        eng.process_wallet(
            "w1",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();
        // Wallet 2: ETH position
        eng.process_wallet(
            "w2",
            vec![make_wp("ETH", "2.0", "3000.0")],
            &hashmap!("ETH" => 3000.0),
        )
        .unwrap();

        // Accumulated mark prices from both wallets
        let mut accumulated: HashMap<String, f64> = HashMap::new();
        accumulated.insert("BTC".to_string(), 56400.0); // -6% → triggers SL
        accumulated.insert("ETH".to_string(), 2940.0); // -2% → no SL

        let closed = eng.check_stop_losses(&accumulated).unwrap();
        assert_eq!(closed, 1, "Only BTC should trigger stop-loss");
        let btc_trade = eng
            .trade_log()
            .trades()
            .iter()
            .find(|t| t.market == "BTC")
            .unwrap();
        assert_eq!(btc_trade.close_reason, Some(CloseReason::StopLoss));
        let eth_trade = eng
            .trade_log()
            .trades()
            .iter()
            .find(|t| t.market == "ETH")
            .unwrap();
        assert_eq!(eth_trade.status, TradeStatus::Open);
        let _ = fs::remove_dir_all(&dir);
    }

    // === HL Paper Mode Tests (13 tests) ===

    #[test]
    fn test_hl_paper_cli_flag() {
        // Verify --hl-paper flag is parsed correctly
        let args = Args::try_parse_from(["copy-trader", "--hl-paper", "--watchlist", "/tmp/test.json"]);
        assert!(args.is_ok(), "Failed to parse --hl-paper flag");
        let args = args.unwrap();
        assert!(args.hl_paper);
        assert!(!args.paper);
        assert!(!args.live);
    }

    #[test]
    fn test_hl_paper_cli_conflict() {
        // Cannot use --hl-paper with --paper
        let args = Args::try_parse_from(["copy-trader", "--hl-paper", "--paper", "--watchlist", "/tmp/test.json"]);
        let args = args.unwrap();
        assert!(args.validate().is_err(), "Should reject --hl-paper + --paper");
    }

    #[test]
    fn test_hl_paper_mode_fee_tracking() {
        // When HL paper mode is enabled, entry_fee is recorded when opening a position
        let dir = std::env::temp_dir().join("ct-hl-fees");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        let trade = &eng.trade_log().trades()[0];
        assert!(trade.entry_fee.is_some(), "entry_fee should be recorded in HL paper mode");
        let expected_entry_fee = trade.size_usd * HL_TAKER_FEE_RATE;
        assert!(
            (trade.entry_fee.unwrap() - expected_entry_fee).abs() < 1e-10,
            "entry_fee mismatch"
        );
        assert!(trade.exit_fee.is_none(), "exit_fee should be None for open trade");
        assert!(trade.borrow_fee.is_none(), "borrow_fee should be None for open trade");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_net_pnl_calculation() {
        // net_pnl = gross_pnl - entry_fee - exit_fee - borrow_fee
        let dir = std::env::temp_dir().join("ct-hl-net");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Open long BTC at 60000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        let trade = &eng.trade_log().trades()[0];
        let size_usd = trade.size_usd;
        let entry_fee = trade.entry_fee.unwrap();

        // Close when price goes to 63000 (+5%)
        eng.process_wallet("0xa", vec![], &hashmap!("BTC" => 63000.0)).unwrap();

        let trade = &eng.trade_log().trades()[0];
        assert_eq!(trade.status, TradeStatus::Closed);

        let gross_pnl = size_usd * (63000.0 - 60000.0) / 60000.0;
        let exit_fee = trade.exit_fee.unwrap();
        let borrow_fee = trade.borrow_fee.unwrap();
        let hours_held = trade.hours_held.unwrap();
        let expected_net = gross_pnl - entry_fee - exit_fee - borrow_fee;

        assert!(
            (trade.pnl_usd.unwrap() - expected_net).abs() < 1e-6,
            "net_pnl should be gross_pnl minus all fees"
        );

        // Verify exit_fee = size_usd * HL_TAKER_FEE_RATE
        assert!(
            (exit_fee - size_usd * HL_TAKER_FEE_RATE).abs() < 1e-10,
            "exit_fee should be taker fee on notional"
        );

        // Verify borrow_fee = size_usd * HL_BORROW_RATE_PER_HOUR * hours_held
        assert!(
            (borrow_fee - size_usd * HL_BORROW_RATE_PER_HOUR * hours_held).abs() < 1e-6,
            "borrow_fee should be size_usd * rate * hours"
        );

        // Net PnL should be less than gross PnL (fees reduce it)
        assert!(trade.pnl_usd.unwrap() < gross_pnl, "net should be less than gross after fees");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_position_size_cap() {
        // Position size capped at account_balance * max_position_pct / 100
        let dir = std::env::temp_dir().join("ct-hl-cap");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.paper_balance = 5000.0;
        cfg.max_position_pct = 20.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Whale has $100k BTC position — our cap should be 5000 * 20% = $1000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "100000.0")],
            &hashmap!("BTC" => 100000.0),
        )
        .unwrap();

        let trade = &eng.trade_log().trades()[0];
        assert!(
            (trade.size_usd - 1000.0).abs() < 0.01,
            "size should be capped at 1000 (5000 * 20%)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_borrow_fee() {
        // Borrow fee accrues based on hours held
        // borrow_fee = notional * 0.01%/hr * hours_held
        let notional = 10000.0;
        let hours = 5.0;
        let expected_borrow = notional * HL_BORROW_RATE_PER_HOUR * hours;
        // 10000 * 0.0001 * 5 = 5.0
        assert!(
            (expected_borrow - 5.0).abs() < 1e-10,
            "borrow fee should be $5 for $10k over 5 hours"
        );

        // Also verify through the helper
        let (_, borrow_fee) = hl_close_fees(notional, hours);
        assert!((borrow_fee - expected_borrow).abs() < 1e-10);

        // Zero hours → zero borrow
        let (_, zero_borrow) = hl_close_fees(notional, 0.0);
        assert!((zero_borrow - 0.0).abs() < 1e-15);
    }

    #[test]
    fn test_hl_paper_api_failure_graceful() {
        // Simulate API failure scenario: engine continues processing after error
        let dir = std::env::temp_dir().join("ct-hl-api");
        let _ = fs::create_dir_all(&dir);
        let cfg = make_hl_config();
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Process wallet 1 successfully
        let result = eng.process_wallet(
            "w1",
            vec![make_wp("BTC", "0.5", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        );
        assert!(result.is_ok(), "First wallet should succeed");
        assert_eq!(result.unwrap(), 1);

        // Simulate API failure by passing empty positions (wallet went away)
        // This should not crash — just no diff detected
        let result = eng.process_wallet("w2", vec![], &hashmap!());
        assert!(result.is_ok(), "Empty positions should not crash");
        assert_eq!(result.unwrap(), 0);

        // Engine should still be functional
        assert_eq!(eng.trade_log().open_count(), 1);
        assert_eq!(eng.account_balance(), 10000.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_max_positions() {
        // Max positions limit enforced in HL paper mode
        let dir = std::env::temp_dir().join("ct-hl-maxpos");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.max_positions = 2;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Open 2 positions (max)
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "0.5", "60000.0"), make_wp("ETH", "2.0", "3000.0")],
            &hashmap!("BTC" => 60000.0, "ETH" => 3000.0),
        )
        .unwrap();
        assert_eq!(eng.trade_log().open_count(), 2);

        // Third position should be skipped
        eng.process_wallet(
            "0xb",
            vec![make_wp("SOL", "10.0", "150.0")],
            &hashmap!("SOL" => 150.0),
        )
        .unwrap();
        assert_eq!(eng.trade_log().open_count(), 2, "Third position should be skipped");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_position_diff_new() {
        // Detect new positions in HL paper mode
        let old: Vec<WalletPosition> = vec![];
        let new = vec![
            make_wp("BTC", "0.5", "60000.0"),
            make_wp("ETH", "2.0", "3000.0"),
        ];
        let diff = detect_positions_diff(&old, &new);
        assert_eq!(diff.new_positions.len(), 2, "Both positions should be new");
        assert!(diff.closed_positions.is_empty());
    }

    #[test]
    fn test_position_diff_closed() {
        // Detect closed positions in HL paper mode
        let old = vec![
            make_wp("BTC", "0.5", "60000.0"),
            make_wp("ETH", "2.0", "3000.0"),
        ];
        let new = vec![make_wp("BTC", "0.5", "60000.0")];
        let diff = detect_positions_diff(&old, &new);
        assert_eq!(diff.closed_positions.len(), 1, "ETH should be closed");
        assert_eq!(diff.closed_positions[0].coin, "ETH");
        assert!(diff.new_positions.is_empty());
    }

    #[test]
    fn test_hl_paper_balance_tracking() {
        // Balance updated correctly after close in HL paper mode (including fees)
        let dir = std::env::temp_dir().join("ct-hl-bal");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.paper_balance = 10000.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        let initial_balance = eng.account_balance();

        // Open long BTC position (size = $60000 at 100% balance would be capped to $10000)
        // With max_position_pct=100 and balance=10000, sizing_multiplier=1.0:
        // raw_size = 60000 * 1.0 = 60000, max = 10000 * 1.0 = 10000 → capped to 10000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        // Balance unchanged while position is open
        assert!(
            (eng.account_balance() - initial_balance).abs() < 1e-10,
            "Balance should not change on open"
        );

        // Close with profit: price goes to 66000 (+10%)
        eng.process_wallet("0xa", vec![], &hashmap!("BTC" => 66000.0)).unwrap();

        let final_balance = eng.account_balance();
        assert!(
            final_balance > initial_balance,
            "Balance should increase after profitable close"
        );

        // Verify: gross_pnl = 10000 * (66000-60000)/60000 = 1000
        // fees: entry=10000*0.00035=3.5, exit=10000*0.00035=3.5, borrow varies by time
        // net_pnl = 1000 - 3.5 - 3.5 - borrow_fee < 1000
        let trade = &eng.trade_log().trades()[0];
        let net_pnl = trade.pnl_usd.unwrap();
        assert!(net_pnl < 1000.0, "Net PnL should be less than gross due to fees");
        assert!(net_pnl > 0.0, "Net PnL should still be positive for 10% gain");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_stop_loss_with_fees() {
        // Stop-loss in HL paper mode includes fee deduction
        let dir = std::env::temp_dir().join("ct-hl-sl");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.stop_loss_pct = 5.0;
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Open long BTC at 60000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        // Price drops 6% → stop-loss triggers
        let closed = eng.check_stop_losses(&hashmap!("BTC" => 56400.0)).unwrap();
        assert_eq!(closed, 1);

        let trade = &eng.trade_log().trades()[0];
        assert_eq!(trade.status, TradeStatus::Closed);
        assert_eq!(trade.close_reason, Some(CloseReason::StopLoss));
        assert!(trade.exit_fee.is_some(), "exit_fee should be recorded");
        assert!(trade.borrow_fee.is_some(), "borrow_fee should be recorded");
        assert!(trade.hours_held.is_some(), "hours_held should be recorded");

        // Net PnL should be worse than gross due to fees
        let gross_pnl = trade.size_usd * (56400.0 - 60000.0) / 60000.0;
        assert!(trade.pnl_usd.unwrap() < gross_pnl, "Net should be less than gross after fees");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_no_fee_in_normal_mode() {
        // When hl_paper=false, no fees are tracked
        let dir = std::env::temp_dir().join("ct-hl-nofee");
        let _ = fs::create_dir_all(&dir);
        let cfg = make_config(); // hl_paper: false
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "0.5", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        let trade = &eng.trade_log().trades()[0];
        assert!(trade.entry_fee.is_none(), "No entry_fee in normal paper mode");
        assert!(trade.exit_fee.is_none());
        assert!(trade.borrow_fee.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_paper_loss_with_fees() {
        // Losing trade in HL paper mode: fees make the loss worse
        let dir = std::env::temp_dir().join("ct-hl-loss");
        let _ = fs::create_dir_all(&dir);
        let mut cfg = make_hl_config();
        cfg.max_position_pct = 100.0;
        cfg.sizing_multiplier = 1.0;
        let tl = TradeLog::new(dir.join("t.json"));
        let mut eng = CopyTraderEngine::new(cfg, tl);

        // Open long at 60000
        eng.process_wallet(
            "0xa",
            vec![make_wp("BTC", "1.0", "60000.0")],
            &hashmap!("BTC" => 60000.0),
        )
        .unwrap();

        // Close at 57000 (-5%)
        eng.process_wallet("0xa", vec![], &hashmap!("BTC" => 57000.0)).unwrap();

        let trade = &eng.trade_log().trades()[0];
        let gross_pnl = trade.size_usd * (57000.0 - 60000.0) / 60000.0; // negative
        let net_pnl = trade.pnl_usd.unwrap();

        assert!(net_pnl < gross_pnl, "Fees should make the loss worse");
        assert!(net_pnl < 0.0, "Net PnL should be negative");

        let entry_fee = trade.entry_fee.unwrap();
        let exit_fee = trade.exit_fee.unwrap();
        let borrow_fee = trade.borrow_fee.unwrap();
        let expected_net = gross_pnl - entry_fee - exit_fee - borrow_fee;
        assert!((net_pnl - expected_net).abs() < 1e-6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hl_fee_helper_functions() {
        // Unit test the fee helper functions directly
        let size = 10000.0;

        // Entry fee
        let entry = hl_entry_fee(size);
        assert!((entry - 10000.0 * 0.00035).abs() < 1e-10);
        assert!((entry - 3.5).abs() < 1e-10);

        // Close fees for 10 hours held
        let (exit_fee, borrow_fee) = hl_close_fees(size, 10.0);
        assert!((exit_fee - 3.5).abs() < 1e-10);
        assert!((borrow_fee - 10000.0 * 0.0001 * 10.0).abs() < 1e-10);
        assert!((borrow_fee - 10.0).abs() < 1e-10);

        // Net PnL
        let net = hl_net_pnl(100.0, 3.5, 3.5, 10.0);
        assert!((net - 83.0).abs() < 1e-10);

        // Negative net
        let net_loss = hl_net_pnl(-50.0, 3.5, 3.5, 10.0);
        assert!((net_loss - (-67.0)).abs() < 1e-10);
    }

    // === Helpers ===

    fn make_config() -> CopyTraderConfig {
        CopyTraderConfig { max_position_pct: 10.0, max_positions: 3, stop_loss_pct: 5.0,
            lag_secs: 30, sizing_multiplier: 0.1, paper_balance: 10000.0, poll_interval_secs: 30,
            hl_paper: false }
    }

    fn make_hl_config() -> CopyTraderConfig {
        CopyTraderConfig { max_position_pct: 10.0, max_positions: 3, stop_loss_pct: 5.0,
            lag_secs: 30, sizing_multiplier: 0.1, paper_balance: 10000.0, poll_interval_secs: 30,
            hl_paper: true }
    }

    fn make_trade(id: &str, w: &str, m: &str, s: TradeStatus) -> CopyTrade {
        CopyTrade { id: id.into(), timestamp: Utc::now().to_rfc3339(), wallet_address: w.into(),
            market: m.into(), direction: Direction::Long, size_usd: 100.0, entry_price: 60000.0,
            status: s, close_reason: None, exit_price: None, pnl_usd: None,
            whale_size_usd: 1000.0, sizing_multiplier: 0.1,
            entry_fee: None, exit_fee: None, borrow_fee: None, hours_held: None }
    }

    fn make_wp(coin: &str, size: &str, entry_px: &str) -> WalletPosition {
        WalletPosition { coin: coin.into(), size: size.into(), entry_px: entry_px.into(), mark_px: Some(entry_px.into()) }
    }
}
