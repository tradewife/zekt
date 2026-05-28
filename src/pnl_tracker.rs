//! pnl_tracker — Combined PnL tracking across all alpha engine strategies.
//!
//! Reads trade logs from copy-trader (`data/copy-trades.json`), whale-watcher
//! (`data/whale-alerts.json`), and the main zekt paper engine (`paper-trades.json`,
//! `data/paper-results/summary.json`), then produces a unified PnL report.
//!
//! Output: `data/combined-pnl.json` (atomic write).

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ── Data types for reading external trade logs ──────────────────────────────

/// Copy trade entry (matches copy-trader output format).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CopyTrade {
    id: String,
    timestamp: String,
    wallet_address: String,
    market: String,
    direction: String,
    size_usd: f64,
    entry_price: f64,
    status: String,
    close_reason: Option<String>,
    exit_price: Option<f64>,
    pnl_usd: Option<f64>,
    whale_size_usd: f64,
    sizing_multiplier: f64,
    /// HL paper mode: entry fee (0.035% taker on notional).
    #[serde(default)]
    entry_fee: Option<f64>,
    /// HL paper mode: exit fee (0.035% taker on notional).
    #[serde(default)]
    exit_fee: Option<f64>,
    /// HL paper mode: borrow fee accrued over holding period.
    #[serde(default)]
    borrow_fee: Option<f64>,
    /// HL paper mode: hours the position was held.
    #[serde(default)]
    hours_held: Option<f64>,
}

/// Whale alert entry (matches whale-watcher output format).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhaleAlert {
    timestamp: String,
    wallet: String,
    coin: String,
    side: String,
    size: f64,
    price: f64,
    notional_usd: f64,
    alert_id: String,
    direction: String,
}

/// Paper trade record (matches risk.rs TradeRecord format).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TradeRecord {
    timestamp: String,
    strategy: String,
    asset: String,
    side: String,
    entry_price: f64,
    exit_price: f64,
    size_usd: f64,
    gross_pnl: f64,
    entry_fee: f64,
    exit_fee: f64,
    borrow_fee: f64,
    net_pnl: f64,
    exit_reason: String,
    leverage: f64,
    trade_date: String,
}

/// Paper results summary (matches MultiPaperEngine output).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaperSummary {
    #[serde(default)]
    start_time: String,
    #[serde(default)]
    end_time: String,
    #[serde(default)]
    starting_balance: f64,
    #[serde(default)]
    final_balance: f64,
    #[serde(default)]
    total_trades: usize,
    #[serde(default)]
    total_net_pnl: f64,
    #[serde(default)]
    total_fees: f64,
}

/// Fee breakdown from HlPaperEngine report.
#[derive(Debug, Clone, Default, Deserialize)]
struct HlFeeBreakdownLocal {
    #[serde(default)]
    taker_fees: f64,
    #[serde(default)]
    borrow_fees: f64,
    #[serde(default)]
    total: f64,
}

/// HlPaperEngine summary report (matches hl_paper.rs HlPaperSummary).
#[derive(Debug, Clone, Deserialize)]
struct HlPaperReportLocal {
    #[serde(default)]
    initial_balance: f64,
    #[serde(default)]
    final_balance: f64,
    #[serde(default)]
    net_pnl: f64,
    #[serde(default)]
    total_fees: HlFeeBreakdownLocal,
    #[serde(default)]
    total_trades: usize,
    #[serde(default)]
    win_rate: f64,
}

// ── Combined output types ───────────────────────────────────────────────────

/// Per-strategy PnL breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyPnl {
    pub strategy: String,
    pub total_trades: usize,
    pub closed_trades: usize,
    pub open_trades: usize,
    pub gross_pnl: f64,
    pub total_fees: f64,
    pub net_pnl: f64,
    pub win_count: usize,
    pub loss_count: usize,
    pub win_rate_pct: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    /// Path to the source data file for this strategy.
    #[serde(default)]
    pub data_source: String,
}

/// Combined PnL report across all strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombinedPnlReport {
    pub generated_at: String,
    pub total_net_pnl: f64,
    pub total_gross_pnl: f64,
    pub total_fees: f64,
    pub total_trades: usize,
    pub strategies: Vec<StrategyPnl>,
    pub data_sources: HashMap<String, String>,
    pub errors: Vec<String>,
    /// Initial account balance before any trades.
    #[serde(default)]
    pub initial_balance: f64,
    /// Current balance = initial_balance + sum(closed trade net_pnl).
    #[serde(default)]
    pub balance: f64,
}

// ── Tracker ─────────────────────────────────────────────────────────────────

/// Configuration for the combined PnL tracker.
#[derive(Debug, Clone)]
pub struct PnlTrackerConfig {
    pub copy_trades_path: PathBuf,
    pub whale_alerts_path: PathBuf,
    pub paper_trades_path: PathBuf,
    pub paper_summary_path: PathBuf,
    pub hl_paper_summary_path: PathBuf,
    pub output_path: PathBuf,
    pub initial_balance: f64,
}

impl Default for PnlTrackerConfig {
    fn default() -> Self {
        Self {
            copy_trades_path: PathBuf::from("data/copy-trades.json"),
            whale_alerts_path: PathBuf::from("data/whale-alerts.json"),
            paper_trades_path: PathBuf::from("paper-trades.json"),
            paper_summary_path: PathBuf::from("data/paper-results/summary.json"),
            hl_paper_summary_path: PathBuf::from("data/paper-results/hl-paper-summary.json"),
            output_path: PathBuf::from("data/combined-pnl.json"),
            initial_balance: 0.0,
        }
    }
}

/// Reads trade logs from all sources and produces a combined PnL report.
pub struct PnlTracker {
    config: PnlTrackerConfig,
}

impl PnlTracker {
    pub fn new(config: PnlTrackerConfig) -> Self {
        Self { config }
    }

    /// Generate a combined PnL report from all available data sources.
    pub fn generate_report(&self) -> Result<CombinedPnlReport> {
        let mut strategies: Vec<StrategyPnl> = Vec::new();
        let mut data_sources: HashMap<String, String> = HashMap::new();
        let mut errors: Vec<String> = Vec::new();

        // 1. Copy trader trades
        let copy_pnl = match self.read_copy_trades() {
            Ok(pnl) => {
                data_sources.insert(
                    "copy-trades".to_string(),
                    self.config.copy_trades_path.display().to_string(),
                );
                Some(pnl)
            }
            Err(e) => {
                let msg = format!("copy-trades: {:#}", e);
                debug!("{}", msg);
                errors.push(msg);
                None
            }
        };

        // 2. Whale watcher alerts (informational — no PnL, just signal tracking)
        let whale_count = match self.read_whale_alerts() {
            Ok(count) => {
                data_sources.insert(
                    "whale-alerts".to_string(),
                    self.config.whale_alerts_path.display().to_string(),
                );
                count
            }
            Err(e) => {
                let msg = format!("whale-alerts: {:#}", e);
                debug!("{}", msg);
                errors.push(msg);
                0
            }
        };

        // 3. Main zekt paper trades (strategy trait strategies)
        let paper_pnl = match self.read_paper_trades() {
            Ok(pnl) => {
                data_sources.insert(
                    "paper-trades".to_string(),
                    self.config.paper_trades_path.display().to_string(),
                );
                Some(pnl)
            }
            Err(e) => {
                let msg = format!("paper-trades: {:#}", e);
                debug!("{}", msg);
                errors.push(msg);
                None
            }
        };

        // 4. Paper summary (if available, for total balance info)
        let _paper_summary = match self.read_paper_summary() {
            Ok(summary) => {
                data_sources.insert(
                    "paper-summary".to_string(),
                    self.config.paper_summary_path.display().to_string(),
                );
                Some(summary)
            }
            Err(e) => {
                let msg = format!("paper-summary: {:#}", e);
                debug!("{}", msg);
                errors.push(msg);
                None
            }
        };

        // 5. HL paper summary (funding-capture strategy from HlPaperEngine)
        let hl_paper_pnl = match self.read_hl_paper_summary() {
            Ok(pnl) => {
                data_sources.insert(
                    "hl-paper-summary".to_string(),
                    self.config.hl_paper_summary_path.display().to_string(),
                );
                Some(pnl)
            }
            Err(e) => {
                let msg = format!("hl-paper-summary: {:#}", e);
                debug!("{}", msg);
                errors.push(msg);
                None
            }
        };

        // Aggregate
        if let Some(pnl) = copy_pnl {
            strategies.push(pnl);
        }

        // Add whale-watcher as informational strategy entry
        if whale_count > 0 {
            strategies.push(StrategyPnl {
                strategy: "whale-watcher".to_string(),
                total_trades: whale_count,
                closed_trades: 0,
                open_trades: whale_count,
                gross_pnl: 0.0,
                total_fees: 0.0,
                net_pnl: 0.0,
                win_count: 0,
                loss_count: 0,
                win_rate_pct: 0.0,
                largest_win: 0.0,
                largest_loss: 0.0,
                data_source: self.config.whale_alerts_path.display().to_string(),
            });
        }

        if let Some(pnl) = paper_pnl {
            strategies.push(pnl);
        }

        if let Some(pnl) = hl_paper_pnl {
            strategies.push(pnl);
        }

        let total_net_pnl: f64 = strategies.iter().map(|s| s.net_pnl).sum();
        let total_gross_pnl: f64 = strategies.iter().map(|s| s.gross_pnl).sum();
        let total_fees: f64 = strategies.iter().map(|s| s.total_fees).sum();
        let total_trades: usize = strategies.iter().map(|s| s.total_trades).sum();

        let initial_balance = self.config.initial_balance;
        let balance = initial_balance + total_net_pnl;

        let report = CombinedPnlReport {
            generated_at: Utc::now().to_rfc3339(),
            total_net_pnl,
            total_gross_pnl,
            total_fees,
            total_trades,
            strategies,
            data_sources,
            errors,
            initial_balance,
            balance,
        };

        Ok(report)
    }

    /// Write the combined report to JSON (atomic write).
    pub fn write_report(&self, report: &CombinedPnlReport) -> Result<()> {
        if let Some(parent) = self.config.output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create dir: {}", parent.display()))?;
        }

        let tmp_path = self.config.output_path.with_extension("json.tmp");
        let json_str = serde_json::to_string_pretty(report)
            .context("serialize combined report")?;
        fs::write(&tmp_path, &json_str)
            .with_context(|| format!("write tmp: {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &self.config.output_path)
            .with_context(|| format!("rename to {}", self.config.output_path.display()))?;

        info!(
            "Combined PnL report written to {} ({} strategies, net PnL: ${:.2})",
            self.config.output_path.display(),
            report.strategies.len(),
            report.total_net_pnl,
        );

        Ok(())
    }

    /// Generate and write the report in one call.
    pub fn run(&self) -> Result<CombinedPnlReport> {
        let report = self.generate_report()?;
        self.write_report(&report)?;
        Ok(report)
    }

    // ── Internal readers ────────────────────────────────────────────────────

    fn read_copy_trades(&self) -> Result<StrategyPnl> {
        let path = &self.config.copy_trades_path;
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        let data = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let trades: Vec<CopyTrade> = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;

        let total = trades.len();
        let closed: Vec<&CopyTrade> = trades.iter().filter(|t| t.status == "closed").collect();
        let open = total - closed.len();

        let mut gross_pnl = 0.0_f64;
        let mut win_count = 0usize;
        let mut loss_count = 0usize;
        let mut largest_win = 0.0_f64;
        let mut largest_loss = 0.0_f64;

        for t in &closed {
            let pnl = t.pnl_usd.unwrap_or(0.0);
            gross_pnl += pnl;
            if pnl >= 0.0 {
                win_count += 1;
                largest_win = largest_win.max(pnl);
            } else {
                loss_count += 1;
                largest_loss = largest_loss.min(pnl);
            }
        }

        let closed_count = closed.len();
        let win_rate = if closed_count > 0 {
            win_count as f64 / closed_count as f64 * 100.0
        } else {
            0.0
        };

        // Use actual fees from HL paper mode when available, otherwise estimate 0.1% per side.
        let total_fees: f64 = trades
            .iter()
            .map(|t| {
                if t.entry_fee.is_some() || t.exit_fee.is_some() {
                    t.entry_fee.unwrap_or(0.0)
                        + t.exit_fee.unwrap_or(0.0)
                        + t.borrow_fee.unwrap_or(0.0)
                } else {
                    t.size_usd * 0.001 * 2.0
                }
            })
            .sum();

        Ok(StrategyPnl {
            strategy: "copy-trader".to_string(),
            total_trades: total,
            closed_trades: closed_count,
            open_trades: open,
            gross_pnl,
            total_fees,
            net_pnl: gross_pnl - total_fees,
            win_count,
            loss_count,
            win_rate_pct: win_rate,
            largest_win,
            largest_loss,
            data_source: path.display().to_string(),
        })
    }

    fn read_whale_alerts(&self) -> Result<usize> {
        let path = &self.config.whale_alerts_path;
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        let data = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let alerts: Vec<WhaleAlert> = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;

        Ok(alerts.len())
    }

    fn read_paper_trades(&self) -> Result<StrategyPnl> {
        let path = &self.config.paper_trades_path;
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        let data = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let trades: Vec<TradeRecord> = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;

        let total = trades.len();
        let mut gross_pnl = 0.0_f64;
        let mut total_fees = 0.0_f64;
        let mut win_count = 0usize;
        let mut loss_count = 0usize;
        let mut largest_win = 0.0_f64;
        let mut largest_loss = 0.0_f64;

        for t in &trades {
            gross_pnl += t.gross_pnl;
            total_fees += t.entry_fee + t.exit_fee + t.borrow_fee;
            if t.net_pnl >= 0.0 {
                win_count += 1;
                largest_win = largest_win.max(t.net_pnl);
            } else {
                loss_count += 1;
                largest_loss = largest_loss.min(t.net_pnl);
            }
        }

        let win_rate = if total > 0 {
            win_count as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        // Group by strategy name for display
        let strategy_name = trades
            .first()
            .map(|t| t.strategy.clone())
            .unwrap_or_else(|| "paper-engine".to_string());

        // If multiple strategies, aggregate under a single label
        let unique_strategies: std::collections::HashSet<&str> =
            trades.iter().map(|t| t.strategy.as_str()).collect();
        let label = if unique_strategies.len() <= 1 {
            strategy_name
        } else {
            format!(
                "paper-engine ({})",
                unique_strategies.iter().cloned().collect::<Vec<_>>().join(",")
            )
        };

        Ok(StrategyPnl {
            strategy: label,
            total_trades: total,
            closed_trades: total,
            open_trades: 0,
            gross_pnl,
            total_fees,
            net_pnl: gross_pnl - total_fees,
            win_count,
            loss_count,
            win_rate_pct: win_rate,
            largest_win,
            largest_loss,
            data_source: path.display().to_string(),
        })
    }

    fn read_paper_summary(&self) -> Result<PaperSummary> {
        let path = &self.config.paper_summary_path;
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        let data = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let summary: PaperSummary = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(summary)
    }

    /// Read HlPaperEngine summary as a funding-capture strategy entry.
    fn read_hl_paper_summary(&self) -> Result<StrategyPnl> {
        let path = &self.config.hl_paper_summary_path;
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        let data = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let summary: HlPaperReportLocal = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;

        let total_fees = summary.total_fees.total;
        let net_pnl = summary.net_pnl;
        // gross_pnl = net_pnl + total_fees (since net = gross - fees)
        let gross_pnl = net_pnl + total_fees;

        Ok(StrategyPnl {
            strategy: "funding-capture".to_string(),
            total_trades: summary.total_trades,
            closed_trades: summary.total_trades,
            open_trades: 0,
            gross_pnl,
            total_fees,
            net_pnl,
            win_count: 0,
            loss_count: 0,
            win_rate_pct: summary.win_rate,
            largest_win: 0.0,
            largest_loss: 0.0,
            data_source: path.display().to_string(),
        })
    }
}

/// Log the combined report summary to tracing.
pub fn log_report_summary(report: &CombinedPnlReport) {
    info!("=== Combined PnL Report ===");
    info!(
        "Generated: {} | Strategies: {} | Total trades: {}",
        report.generated_at, report.strategies.len(), report.total_trades,
    );
    info!(
        "Total: gross=${:.2} fees=${:.2} net=${:.2}",
        report.total_gross_pnl, report.total_fees, report.total_net_pnl,
    );
    info!(
        "Balance: initial=${:.2} current=${:.2}",
        report.initial_balance, report.balance,
    );

    for s in &report.strategies {
        info!(
            "  [{}] trades={} closed={} net_pnl=${:.2} win_rate={:.1}% (wins={} losses={})",
            s.strategy,
            s.total_trades,
            s.closed_trades,
            s.net_pnl,
            s.win_rate_pct,
            s.win_count,
            s.loss_count,
        );
    }

    if !report.errors.is_empty() {
        warn!("Data source errors:");
        for e in &report.errors {
            warn!("  - {}", e);
        }
    }
}

/// Standalone function to generate a combined PnL report from funding-capture
/// (HlPaperEngine) and copy-trader sources.
///
/// Reads both data sources, aggregates into a combined report, and writes
/// it atomically to `output_path`.
///
/// # Arguments
/// * `hl_paper_summary_path` - Path to HlPaperEngine summary JSON
/// * `copy_trades_path` - Path to copy-trader trade log JSON
/// * `output_path` - Path for the combined report output
/// * `initial_balance` - Starting account balance for balance tracking
pub fn generate_combined_report(
    hl_paper_summary_path: &Path,
    copy_trades_path: &Path,
    output_path: &Path,
    initial_balance: f64,
) -> Result<CombinedPnlReport> {
    let config = PnlTrackerConfig {
        hl_paper_summary_path: hl_paper_summary_path.to_path_buf(),
        copy_trades_path: copy_trades_path.to_path_buf(),
        initial_balance,
        output_path: output_path.to_path_buf(),
        ..PnlTrackerConfig::default()
    };

    let tracker = PnlTracker::new(config);
    let report = tracker.run()?;

    Ok(report)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_copy_trades_json() -> &'static str {
        r#"[
            {
                "id": "ct-001",
                "timestamp": "2026-05-27T10:00:00Z",
                "wallet_address": "0xabc",
                "market": "BTC",
                "direction": "long",
                "size_usd": 500.0,
                "entry_price": 104000.0,
                "status": "closed",
                "close_reason": "wallet_closed",
                "exit_price": 104500.0,
                "pnl_usd": 25.0,
                "whale_size_usd": 5000.0,
                "sizing_multiplier": 0.1
            },
            {
                "id": "ct-002",
                "timestamp": "2026-05-27T11:00:00Z",
                "wallet_address": "0xdef",
                "market": "ETH",
                "direction": "short",
                "size_usd": 300.0,
                "entry_price": 3800.0,
                "status": "closed",
                "close_reason": "stop_loss",
                "exit_price": 3850.0,
                "pnl_usd": -15.0,
                "whale_size_usd": 3000.0,
                "sizing_multiplier": 0.1
            },
            {
                "id": "ct-003",
                "timestamp": "2026-05-27T12:00:00Z",
                "wallet_address": "0xabc",
                "market": "SOL",
                "direction": "long",
                "size_usd": 200.0,
                "entry_price": 170.0,
                "status": "open",
                "close_reason": null,
                "exit_price": null,
                "pnl_usd": null,
                "whale_size_usd": 2000.0,
                "sizing_multiplier": 0.1
            }
        ]"#
    }

    fn make_whale_alerts_json() -> &'static str {
        r#"[
            {
                "timestamp": "2026-05-27T09:00:00Z",
                "wallet": "0xabc",
                "coin": "BTC",
                "side": "buy",
                "size": 0.5,
                "price": 104000.0,
                "notional_usd": 52000.0,
                "alert_id": "wa-001",
                "direction": "Open Long"
            }
        ]"#
    }

    fn make_paper_trades_json() -> &'static str {
        r#"[
            {
                "timestamp": "2026-05-27T08:00:00Z",
                "strategy": "funding-capture",
                "asset": "BTC",
                "side": "short",
                "entry_price": 104000.0,
                "exit_price": 103800.0,
                "size_usd": 200.0,
                "gross_pnl": 0.38,
                "entry_fee": 0.20,
                "exit_fee": 0.20,
                "borrow_fee": 0.05,
                "net_pnl": -0.07,
                "exit_reason": "time_stop",
                "leverage": 1.0,
                "trade_date": "2026-05-27"
            },
            {
                "timestamp": "2026-05-27T09:00:00Z",
                "strategy": "momentum-scalper",
                "asset": "SOL",
                "side": "long",
                "entry_price": 170.0,
                "exit_price": 172.5,
                "size_usd": 100.0,
                "gross_pnl": 1.47,
                "entry_fee": 0.10,
                "exit_fee": 0.10,
                "borrow_fee": 0.01,
                "net_pnl": 1.26,
                "exit_reason": "take_profit",
                "leverage": 3.0,
                "trade_date": "2026-05-27"
            }
        ]"#
    }

    #[test]
    fn test_read_copy_trades() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("copy-trades.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(make_copy_trades_json().as_bytes()).unwrap();

        let config = PnlTrackerConfig {
            copy_trades_path: path.clone(),
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let pnl = tracker.read_copy_trades().unwrap();

        assert_eq!(pnl.strategy, "copy-trader");
        assert_eq!(pnl.total_trades, 3);
        assert_eq!(pnl.closed_trades, 2);
        assert_eq!(pnl.open_trades, 1);
        assert!((pnl.gross_pnl - 10.0).abs() < 0.01); // 25 + (-15)
        assert_eq!(pnl.win_count, 1);
        assert_eq!(pnl.loss_count, 1);
        assert!((pnl.largest_win - 25.0).abs() < 0.01);
        assert!((pnl.largest_loss - (-15.0)).abs() < 0.01);
    }

    #[test]
    fn test_read_whale_alerts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("whale-alerts.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(make_whale_alerts_json().as_bytes()).unwrap();

        let config = PnlTrackerConfig {
            whale_alerts_path: path,
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let count = tracker.read_whale_alerts().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_read_paper_trades() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("paper-trades.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(make_paper_trades_json().as_bytes()).unwrap();

        let config = PnlTrackerConfig {
            paper_trades_path: path,
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let pnl = tracker.read_paper_trades().unwrap();

        assert!(pnl.strategy.contains("paper-engine"));
        assert_eq!(pnl.total_trades, 2);
        assert_eq!(pnl.closed_trades, 2);
        assert_eq!(pnl.open_trades, 0);
        // gross: 0.38 + 1.47 = 1.85
        assert!((pnl.gross_pnl - 1.85).abs() < 0.01);
        // fees: (0.20+0.20+0.05) + (0.10+0.10+0.01) = 0.45 + 0.21 = 0.66
        assert!((pnl.total_fees - 0.66).abs() < 0.01);
        // net: 1.85 - 0.66 = 1.19
        assert!((pnl.net_pnl - 1.19).abs() < 0.01);
        assert_eq!(pnl.win_count, 1);
        assert_eq!(pnl.loss_count, 1);
    }

    #[test]
    fn test_full_combined_report() {
        let dir = TempDir::new().unwrap();

        let copy_path = dir.path().join("copy-trades.json");
        fs::write(&copy_path, make_copy_trades_json()).unwrap();

        let whale_path = dir.path().join("whale-alerts.json");
        fs::write(&whale_path, make_whale_alerts_json()).unwrap();

        let paper_path = dir.path().join("paper-trades.json");
        fs::write(&paper_path, make_paper_trades_json()).unwrap();

        let output_path = dir.path().join("combined-pnl.json");

        let config = PnlTrackerConfig {
            copy_trades_path: copy_path,
            whale_alerts_path: whale_path,
            paper_trades_path: paper_path,
            paper_summary_path: dir.path().join("nonexistent.json"),
            hl_paper_summary_path: dir.path().join("nonexistent-hl.json"),
            output_path: output_path.clone(),
            initial_balance: 1000.0,
        };

        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // 3 strategies: copy-trader, whale-watcher, paper-engine
        assert_eq!(report.strategies.len(), 3);
        assert_eq!(report.total_trades, 6); // 3 copy + 1 whale + 2 paper
        assert!(report.data_sources.contains_key("copy-trades"));
        assert!(report.data_sources.contains_key("whale-alerts"));
        assert!(report.data_sources.contains_key("paper-trades"));

        // Verify output file was written
        assert!(output_path.exists());
        let written: CombinedPnlReport =
            serde_json::from_str(&fs::read_to_string(&output_path).unwrap()).unwrap();
        assert_eq!(written.strategies.len(), 3);

        // Verify total PnL aggregation
        let copy_pnl: f64 = report.strategies.iter()
            .find(|s| s.strategy == "copy-trader")
            .map(|s| s.net_pnl)
            .unwrap();
        let paper_pnl: f64 = report.strategies.iter()
            .find(|s| s.strategy.contains("paper-engine"))
            .map(|s| s.net_pnl)
            .unwrap();
        assert!((report.total_net_pnl - copy_pnl - paper_pnl).abs() < 0.01);
    }

    #[test]
    fn test_missing_files_graceful() {
        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join("combined-pnl.json");

        let config = PnlTrackerConfig {
            copy_trades_path: dir.path().join("no-copy.json"),
            whale_alerts_path: dir.path().join("no-whale.json"),
            paper_trades_path: dir.path().join("no-paper.json"),
            paper_summary_path: dir.path().join("no-summary.json"),
            hl_paper_summary_path: dir.path().join("no-hl-summary.json"),
            output_path,
            initial_balance: 0.0,
        };

        let tracker = PnlTracker::new(config);
        let report = tracker.generate_report().unwrap();

        // Should succeed with empty results
        assert!(report.strategies.is_empty());
        assert_eq!(report.total_trades, 0);
        assert!(!report.errors.is_empty());
    }

    #[test]
    fn test_log_report_summary() {
        let report = CombinedPnlReport {
            generated_at: "2026-05-27T00:00:00Z".to_string(),
            total_net_pnl: 42.50,
            total_gross_pnl: 50.00,
            total_fees: 7.50,
            total_trades: 10,
            strategies: vec![StrategyPnl {
                strategy: "test".to_string(),
                total_trades: 10,
                closed_trades: 8,
                open_trades: 2,
                gross_pnl: 50.0,
                total_fees: 7.5,
                net_pnl: 42.5,
                win_count: 6,
                loss_count: 2,
                win_rate_pct: 75.0,
                largest_win: 20.0,
                largest_loss: -5.0,
                data_source: String::new(),
            }],
            data_sources: HashMap::new(),
            errors: vec![],
            initial_balance: 1000.0,
            balance: 1042.5,
        };
        // Should not panic
        log_report_summary(&report);
    }

    #[test]
    fn test_empty_copy_trades() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("copy-trades.json");
        fs::write(&path, "[]").unwrap();

        let config = PnlTrackerConfig {
            copy_trades_path: path,
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let pnl = tracker.read_copy_trades().unwrap();

        assert_eq!(pnl.total_trades, 0);
        assert_eq!(pnl.closed_trades, 0);
        assert!((pnl.gross_pnl - 0.0).abs() < 0.01);
        assert!((pnl.net_pnl - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_strategy_pnl_serde_roundtrip() {
        let pnl = StrategyPnl {
            strategy: "funding-capture".to_string(),
            total_trades: 5,
            closed_trades: 3,
            open_trades: 2,
            gross_pnl: 100.0,
            total_fees: 10.0,
            net_pnl: 90.0,
            win_count: 3,
            loss_count: 0,
            win_rate_pct: 100.0,
            largest_win: 50.0,
            largest_loss: 0.0,
            data_source: "data/test.json".to_string(),
        };
        let json = serde_json::to_string(&pnl).unwrap();
        let parsed: StrategyPnl = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.strategy, "funding-capture");
        assert_eq!(parsed.total_trades, 5);
        assert!((parsed.net_pnl - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_combined_report_serde_roundtrip() {
        let report = CombinedPnlReport {
            generated_at: "2026-05-27T00:00:00Z".to_string(),
            total_net_pnl: 42.50,
            total_gross_pnl: 50.00,
            total_fees: 7.50,
            total_trades: 10,
            strategies: vec![],
            data_sources: HashMap::new(),
            errors: vec!["some error".to_string()],
            initial_balance: 1000.0,
            balance: 1042.5,
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: CombinedPnlReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.generated_at, "2026-05-27T00:00:00Z");
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn test_read_paper_summary() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("summary.json");
        fs::write(
            &path,
            r#"{"start_time":"2026-05-27T00:00:00Z","end_time":"2026-05-27T12:00:00Z","starting_balance":1000.0,"final_balance":1050.0,"total_trades":5,"total_net_pnl":50.0,"total_fees":10.0}"#,
        ).unwrap();

        let config = PnlTrackerConfig {
            paper_summary_path: path,
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let summary = tracker.read_paper_summary().unwrap();
        assert!((summary.final_balance - 1050.0).abs() < 0.01);
        assert_eq!(summary.total_trades, 5);
    }

    // ── HL paper summary tests ──────────────────────────────────────────────

    fn make_hl_paper_summary_json() -> &'static str {
        r#"{
            "start_time": "2026-05-27T00:00:00Z",
            "end_time": "2026-05-27T12:00:00Z",
            "initial_balance": 1000.0,
            "final_balance": 1050.0,
            "net_pnl": 42.5,
            "total_fees": {
                "taker_fees": 5.0,
                "borrow_fees": 2.5,
                "total": 7.5
            },
            "total_trades": 10,
            "win_rate": 70.0,
            "sharpe_ratio": 1.8,
            "markets": []
        }"#
    }

    #[test]
    fn test_read_hl_paper_summary() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hl-paper-summary.json");
        fs::write(&path, make_hl_paper_summary_json()).unwrap();

        let config = PnlTrackerConfig {
            hl_paper_summary_path: path,
            ..PnlTrackerConfig::default()
        };
        let tracker = PnlTracker::new(config);
        let pnl = tracker.read_hl_paper_summary().unwrap();

        assert_eq!(pnl.strategy, "funding-capture");
        assert_eq!(pnl.total_trades, 10);
        assert_eq!(pnl.closed_trades, 10);
        // gross_pnl = net_pnl + total_fees = 42.5 + 7.5 = 50.0
        assert!((pnl.gross_pnl - 50.0).abs() < 0.01);
        assert!((pnl.total_fees - 7.5).abs() < 0.01);
        assert!((pnl.net_pnl - 42.5).abs() < 0.01);
        assert!(!pnl.data_source.is_empty());
    }

    // ── Combined report tests (8 tests as specified) ────────────────────────

    fn make_combined_report_setup(dir: &tempfile::TempDir) -> PnlTrackerConfig {
        let copy_path = dir.path().join("copy-trades.json");
        fs::write(&copy_path, make_copy_trades_json()).unwrap();

        let hl_path = dir.path().join("hl-paper-summary.json");
        fs::write(&hl_path, make_hl_paper_summary_json()).unwrap();

        let output_path = dir.path().join("combined-pnl.json");

        PnlTrackerConfig {
            copy_trades_path: copy_path,
            hl_paper_summary_path: hl_path,
            output_path,
            initial_balance: 10000.0,
            ..PnlTrackerConfig::default()
        }
    }

    #[test]
    fn test_combined_report_has_all_fields() {
        let dir = TempDir::new().unwrap();
        let config = make_combined_report_setup(&dir);
        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // Verify top-level fields
        assert!(!report.generated_at.is_empty());
        assert!(report.total_trades > 0);
        assert!(report.total_fees >= 0.0);
        assert!(report.total_gross_pnl.is_finite());
        assert!(report.total_net_pnl.is_finite());
        assert!(!report.strategies.is_empty());
        assert!(!report.data_sources.is_empty());
        assert!(report.initial_balance > 0.0);
        assert!(report.balance.is_finite());

        // Verify each strategy has required fields
        for s in &report.strategies {
            assert!(!s.strategy.is_empty());
            assert!(s.total_trades > 0 || s.open_trades > 0);
            assert!(s.gross_pnl.is_finite());
            assert!(s.total_fees >= 0.0);
            assert!(s.net_pnl.is_finite());
        }
    }

    #[test]
    fn test_combined_report_fee_consistency() {
        let dir = TempDir::new().unwrap();
        let config = make_combined_report_setup(&dir);
        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // total_fees == sum(strategies[].total_fees)
        let sum_fees: f64 = report.strategies.iter().map(|s| s.total_fees).sum();
        assert!(
            (report.total_fees - sum_fees).abs() < 0.01,
            "total_fees ({}) should equal sum of strategy fees ({})",
            report.total_fees,
            sum_fees
        );
    }

    #[test]
    fn test_combined_report_net_pnl_formula() {
        let dir = TempDir::new().unwrap();
        let config = make_combined_report_setup(&dir);
        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // Each strategy: net_pnl == gross_pnl - total_fees
        for s in &report.strategies {
            let expected_net = s.gross_pnl - s.total_fees;
            assert!(
                (s.net_pnl - expected_net).abs() < 0.01,
                "strategy {} net_pnl ({}) should equal gross ({}) - fees ({})",
                s.strategy,
                s.net_pnl,
                s.gross_pnl,
                s.total_fees
            );
        }

        // Total level
        let expected_total_net = report.total_gross_pnl - report.total_fees;
        assert!(
            (report.total_net_pnl - expected_total_net).abs() < 0.01,
            "total_net_pnl ({}) should equal total_gross ({}) - total_fees ({})",
            report.total_net_pnl,
            report.total_gross_pnl,
            report.total_fees
        );
    }

    #[test]
    fn test_combined_report_balance_tracking() {
        let dir = TempDir::new().unwrap();
        let config = make_combined_report_setup(&dir);
        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // balance == initial_balance + total_net_pnl
        let expected_balance = report.initial_balance + report.total_net_pnl;
        assert!(
            (report.balance - expected_balance).abs() < 0.01,
            "balance ({}) should equal initial ({}) + net_pnl ({})",
            report.balance,
            report.initial_balance,
            report.total_net_pnl
        );
    }

    #[test]
    fn test_combined_report_with_empty_sources() {
        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join("combined-pnl.json");

        // Only provide copy trades, no HL paper summary
        let copy_path = dir.path().join("copy-trades.json");
        fs::write(&copy_path, make_copy_trades_json()).unwrap();

        let config = PnlTrackerConfig {
            copy_trades_path: copy_path,
            hl_paper_summary_path: dir.path().join("no-hl-summary.json"),
            output_path,
            initial_balance: 5000.0,
            ..PnlTrackerConfig::default()
        };

        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // Should succeed with at least copy-trader
        assert!(
            report
                .strategies
                .iter()
                .any(|s| s.strategy == "copy-trader"),
            "Should have copy-trader strategy"
        );
        // Should have errors for missing sources
        assert!(!report.errors.is_empty());
        // Balance should still be computed
        assert!((report.initial_balance - 5000.0).abs() < 0.01);
        let expected_balance = report.initial_balance + report.total_net_pnl;
        assert!((report.balance - expected_balance).abs() < 0.01);
    }

    #[test]
    fn test_combined_report_atomic_writes() {
        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join("combined-pnl.json");
        let tmp_path = dir.path().join("combined-pnl.json.tmp");

        let copy_path = dir.path().join("copy-trades.json");
        fs::write(&copy_path, make_copy_trades_json()).unwrap();

        let hl_path = dir.path().join("hl-paper-summary.json");
        fs::write(&hl_path, make_hl_paper_summary_json()).unwrap();

        let config = PnlTrackerConfig {
            copy_trades_path: copy_path,
            hl_paper_summary_path: hl_path,
            output_path: output_path.clone(),
            initial_balance: 1000.0,
            ..PnlTrackerConfig::default()
        };

        let tracker = PnlTracker::new(config);
        let _report = tracker.run().unwrap();

        // Output file should exist
        assert!(output_path.exists(), "Output file should exist after write");
        // .tmp file should NOT exist (renamed)
        assert!(
            !tmp_path.exists(),
            ".tmp file should have been renamed to final path"
        );

        // Output should be valid JSON
        let content = fs::read_to_string(&output_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.get("generated_at").is_some());
        assert!(parsed.get("strategies").is_some());
    }

    #[test]
    fn test_combined_report_single_strategy() {
        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join("combined-pnl.json");

        // Only HL paper summary, no copy trades
        let hl_path = dir.path().join("hl-paper-summary.json");
        fs::write(&hl_path, make_hl_paper_summary_json()).unwrap();

        let config = PnlTrackerConfig {
            copy_trades_path: dir.path().join("no-copy.json"),
            hl_paper_summary_path: hl_path,
            output_path,
            initial_balance: 5000.0,
            ..PnlTrackerConfig::default()
        };

        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // Should have exactly funding-capture strategy
        assert_eq!(report.strategies.len(), 1);
        assert_eq!(report.strategies[0].strategy, "funding-capture");
        assert!(
            report
                .strategies
                .iter()
                .any(|s| s.data_source.contains("hl-paper-summary")),
            "funding-capture should have data_source set"
        );

        // Totals should equal single strategy
        let s = &report.strategies[0];
        assert!((report.total_net_pnl - s.net_pnl).abs() < 0.01);
        assert!((report.total_gross_pnl - s.gross_pnl).abs() < 0.01);
        assert!((report.total_fees - s.total_fees).abs() < 0.01);
    }

    #[test]
    fn test_combined_report_timestamp() {
        let dir = TempDir::new().unwrap();
        let config = make_combined_report_setup(&dir);
        let tracker = PnlTracker::new(config);
        let report = tracker.run().unwrap();

        // Timestamp should be valid RFC3339
        assert!(
            chrono::DateTime::parse_from_rfc3339(&report.generated_at).is_ok(),
            "generated_at should be valid RFC3339: {}",
            report.generated_at
        );

        // Timestamp should be recent (within last 60 seconds)
        let parsed =
            chrono::DateTime::parse_from_rfc3339(&report.generated_at).unwrap();
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(parsed.with_timezone(&chrono::Utc));
        assert!(
            diff.num_seconds().abs() < 60,
            "generated_at should be recent, but was {} seconds from now",
            diff.num_seconds()
        );
    }

    // ── generate_combined_report() standalone function test ─────────────────

    #[test]
    fn test_generate_combined_report_standalone() {
        let dir = TempDir::new().unwrap();

        let copy_path = dir.path().join("copy-trades.json");
        fs::write(&copy_path, make_copy_trades_json()).unwrap();

        let hl_path = dir.path().join("hl-paper-summary.json");
        fs::write(&hl_path, make_hl_paper_summary_json()).unwrap();

        let output_path = dir.path().join("combined-pnl.json");

        let report = generate_combined_report(
            &hl_path,
            &copy_path,
            &output_path,
            10000.0,
        )
        .unwrap();

        assert!(output_path.exists());
        assert!(!report.strategies.is_empty());
        assert!((report.initial_balance - 10000.0).abs() < 0.01);

        // Verify balance tracking
        let expected_balance = report.initial_balance + report.total_net_pnl;
        assert!((report.balance - expected_balance).abs() < 0.01);

        // Verify fee consistency
        let sum_fees: f64 = report.strategies.iter().map(|s| s.total_fees).sum();
        assert!((report.total_fees - sum_fees).abs() < 0.01);
    }
}
