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
        if !self.paper && !self.live {
            anyhow::bail!("must specify --paper or --live mode");
        }
        if self.paper && self.live {
            anyhow::bail!("cannot specify both --paper and --live");
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
                updated.pnl_usd = Some(unrealized_pnl_usd(
                    updated.size_usd,
                    updated.entry_price,
                    mark_price,
                    matches!(updated.direction, Direction::Long),
                ));

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
                updated.pnl_usd = Some(unrealized_pnl_usd(
                    updated.size_usd,
                    updated.entry_price,
                    current_price,
                    is_long,
                ));

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
    info!(
        mode = if args.paper { "paper" } else { "live" },
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
        anyhow::bail!("--live mode is not yet implemented. Use --paper for paper trading.");
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

        for wallet in &wallets {
            match fetch_wallet_positions(&client, &wallet.address).await {
                Ok(positions) => {
                    let mark_prices: HashMap<String, f64> = positions
                        .iter()
                        .filter_map(|p| {
                            let px: f64 = p.mark_px.as_ref()?.parse().ok()?;
                            Some((p.coin.clone(), px))
                        })
                        .collect();

                    match engine.process_wallet(&wallet.address, positions, &mark_prices) {
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

        let mark_prices: HashMap<String, f64> = HashMap::new();
        if let Err(e) = engine.check_stop_losses(&mark_prices) {
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
        };
        let s = serde_json::to_string(&t).unwrap();
        let p: CopyTrade = serde_json::from_str(&s).unwrap();
        assert_eq!(p.close_reason, Some(CloseReason::StopLoss));
    }

    #[test]
    fn test_trade_required_fields() {
        let t = CopyTrade {
            id: "i".into(), timestamp: "t".into(), wallet_address: "w".into(),
            market: "M".into(), direction: Direction::Long, size_usd: 1.0, entry_price: 1.0,
            status: TradeStatus::Open, close_reason: None, exit_price: None, pnl_usd: None,
            whale_size_usd: 1.0, sizing_multiplier: 0.1,
        };
        let v = serde_json::to_value(&t).unwrap();
        for f in &["id","timestamp","wallet_address","market","direction","size_usd","entry_price","status"] {
            assert!(v.get(*f).is_some(), "missing: {}", f);
        }
        assert!(v.get("close_reason").is_none());
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

    // === Helpers ===

    fn make_config() -> CopyTraderConfig {
        CopyTraderConfig { max_position_pct: 10.0, max_positions: 3, stop_loss_pct: 5.0,
            lag_secs: 30, sizing_multiplier: 0.1, paper_balance: 10000.0, poll_interval_secs: 30 }
    }

    fn make_trade(id: &str, w: &str, m: &str, s: TradeStatus) -> CopyTrade {
        CopyTrade { id: id.into(), timestamp: Utc::now().to_rfc3339(), wallet_address: w.into(),
            market: m.into(), direction: Direction::Long, size_usd: 100.0, entry_price: 60000.0,
            status: s, close_reason: None, exit_price: None, pnl_usd: None,
            whale_size_usd: 1000.0, sizing_multiplier: 0.1 }
    }

    fn make_wp(coin: &str, size: &str, entry_px: &str) -> WalletPosition {
        WalletPosition { coin: coin.into(), size: size.into(), entry_px: entry_px.into(), mark_px: Some(entry_px.into()) }
    }
}
