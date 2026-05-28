//! pipeline — Orchestrator for the Alpha Discovery Engine.
//!
//! Launches and manages the full alpha pipeline as coordinated child processes:
//!
//!   1. alpha-scanner   → discovers profitable wallets → data/watchlist.json
//!   2. copy-trader     → mirrors wallet positions (paper mode) → data/copy-trades.json
//!   3. whale-watcher   → monitors large fills via WebSocket → data/whale-alerts.json
//!   4. zekt --paper    → runs strategy-trait strategies (funding-capture, etc.) → paper-trades.json
//!
//! Periodically generates combined PnL reports.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin pipeline -- --paper-balance 1000
//! cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48
//! cargo run --bin pipeline -- --once   # single scan + report, no daemon
//! cargo run --bin pipeline -- --help
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tracing::{debug, error, info, warn};

// ── CLI ──────────────────────────────────────────────────────────────────────

fn validate_duration(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid duration-hours value: {}", v))?;
    if val < 0.0 {
        return Err(format!("duration-hours must be >= 0, got {}", val));
    }
    Ok(val)
}

fn validate_paper_balance(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid paper-balance value: {}", v))?;
    if val <= 0.0 {
        return Err(format!("paper-balance must be > 0, got {}", val));
    }
    Ok(val)
}

fn validate_report_interval(v: &str) -> Result<u64, String> {
    let val: u64 = v
        .parse()
        .map_err(|_| format!("invalid report-interval value: {}", v))?;
    if val == 0 {
        return Err("report-interval must be > 0".to_string());
    }
    Ok(val)
}

#[derive(Parser, Debug)]
#[command(
    name = "pipeline",
    about = "Orchestrate the full Alpha Discovery Engine: alpha-scanner + copy-trader + whale-watcher + paper trading",
    version
)]
struct Args {
    /// Run a single scan + PnL report cycle and exit (no daemon).
    #[arg(long)]
    once: bool,

    /// Starting paper trading balance in USD.
    #[arg(long, default_value_t = 1000.0, value_parser = validate_paper_balance)]
    paper_balance: f64,

    /// Maximum runtime in hours (0 = unlimited, runs until Ctrl+C).
    #[arg(long, default_value_t = 0.0, value_parser = validate_duration)]
    duration_hours: f64,

    /// Combined PnL report generation interval in seconds.
    #[arg(long, default_value_t = 300, value_parser = validate_report_interval)]
    report_interval: u64,

    /// Path to watchlist JSON (alpha-scanner output).
    #[arg(long, default_value = "data/watchlist.json")]
    watchlist: PathBuf,

    /// Strategies to run in paper mode (comma-separated).
    #[arg(long, default_value = "funding-capture")]
    strategies: String,

    /// Markets to trade (comma-separated).
    #[arg(long, default_value = "BTC")]
    markets: String,

    /// Output directory for paper results.
    #[arg(long, default_value = "data/paper-results")]
    paper_output: String,

    /// Output path for combined PnL report.
    #[arg(long, default_value = "data/combined-pnl.json")]
    combined_output: PathBuf,

    /// Skip alpha-scanner (use existing watchlist).
    #[arg(long)]
    skip_scanner: bool,

    /// Skip copy-trader (only run scanner + whale-watcher + paper).
    #[arg(long)]
    skip_copy_trader: bool,

    /// Skip whale-watcher.
    #[arg(long)]
    skip_whale_watcher: bool,

    /// Skip paper trading (only run scanner + copy-trader + whale-watcher).
    #[arg(long)]
    skip_paper: bool,
}

// ── PnL tracker types (inline — same as src/pnl_tracker.rs) ─────────────────

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

/// Per-strategy PnL breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrategyPnl {
    strategy: String,
    total_trades: usize,
    closed_trades: usize,
    open_trades: usize,
    gross_pnl: f64,
    total_fees: f64,
    net_pnl: f64,
    win_count: usize,
    loss_count: usize,
    win_rate_pct: f64,
    largest_win: f64,
    largest_loss: f64,
}

/// Combined PnL report across all strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CombinedPnlReport {
    generated_at: String,
    total_net_pnl: f64,
    total_gross_pnl: f64,
    total_fees: f64,
    total_trades: usize,
    strategies: Vec<StrategyPnl>,
    data_sources: HashMap<String, String>,
    errors: Vec<String>,
}

/// Generate combined PnL report from all data sources.
fn generate_combined_report(
    copy_trades_path: &std::path::Path,
    whale_alerts_path: &std::path::Path,
    paper_trades_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<CombinedPnlReport> {
    let mut strategies: Vec<StrategyPnl> = Vec::new();
    let mut data_sources: HashMap<String, String> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    // Copy trader
    match read_copy_trades(copy_trades_path) {
        Ok(pnl) => {
            data_sources.insert("copy-trades".to_string(), copy_trades_path.display().to_string());
            strategies.push(pnl);
        }
        Err(e) => {
            let msg = format!("copy-trades: {:#}", e);
            debug!("{}", msg);
            errors.push(msg);
        }
    }

    // Whale watcher
    match read_whale_alerts_count(whale_alerts_path) {
        Ok(count) => {
            data_sources.insert("whale-alerts".to_string(), whale_alerts_path.display().to_string());
            if count > 0 {
                strategies.push(StrategyPnl {
                    strategy: "whale-watcher".to_string(),
                    total_trades: count,
                    closed_trades: 0,
                    open_trades: count,
                    gross_pnl: 0.0,
                    total_fees: 0.0,
                    net_pnl: 0.0,
                    win_count: 0,
                    loss_count: 0,
                    win_rate_pct: 0.0,
                    largest_win: 0.0,
                    largest_loss: 0.0,
                });
            }
        }
        Err(e) => {
            let msg = format!("whale-alerts: {:#}", e);
            debug!("{}", msg);
            errors.push(msg);
        }
    }

    // Paper trades
    match read_paper_trades(paper_trades_path) {
        Ok(pnl) => {
            data_sources.insert("paper-trades".to_string(), paper_trades_path.display().to_string());
            strategies.push(pnl);
        }
        Err(e) => {
            let msg = format!("paper-trades: {:#}", e);
            debug!("{}", msg);
            errors.push(msg);
        }
    }

    let total_net_pnl: f64 = strategies.iter().map(|s| s.net_pnl).sum();
    let total_gross_pnl: f64 = strategies.iter().map(|s| s.gross_pnl).sum();
    let total_fees: f64 = strategies.iter().map(|s| s.total_fees).sum();
    let total_trades: usize = strategies.iter().map(|s| s.total_trades).sum();

    let report = CombinedPnlReport {
        generated_at: Utc::now().to_rfc3339(),
        total_net_pnl,
        total_gross_pnl,
        total_fees,
        total_trades,
        strategies,
        data_sources,
        errors,
    };

    // Atomic write
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))?;
    }
    let tmp_path = output_path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(&report).context("serialize report")?;
    fs::write(&tmp_path, &json_str).with_context(|| format!("write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, output_path)
        .with_context(|| format!("rename to {}", output_path.display()))?;

    Ok(report)
}

fn read_copy_trades(path: &std::path::Path) -> Result<StrategyPnl> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let trades: Vec<CopyTrade> =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;

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

    let total_fees: f64 = trades.iter().map(|t| t.size_usd * 0.001 * 2.0).sum();

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
    })
}

fn read_whale_alerts_count(path: &std::path::Path) -> Result<usize> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let alerts: Vec<WhaleAlert> =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    Ok(alerts.len())
}

fn read_paper_trades(path: &std::path::Path) -> Result<StrategyPnl> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let trades: Vec<TradeRecord> =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;

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

    let unique_strategies: std::collections::HashSet<&str> =
        trades.iter().map(|t| t.strategy.as_str()).collect();
    let label = if unique_strategies.len() <= 1 {
        trades
            .first()
            .map(|t| t.strategy.clone())
            .unwrap_or_else(|| "paper-engine".to_string())
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
    })
}

/// Log the combined report summary to tracing.
fn log_report_summary(report: &CombinedPnlReport) {
    info!("=== Combined PnL Report ===");
    info!(
        "Generated: {} | Strategies: {} | Total trades: {}",
        report.generated_at,
        report.strategies.len(),
        report.total_trades,
    );
    info!(
        "Total: gross=${:.2} fees=${:.2} net=${:.2}",
        report.total_gross_pnl, report.total_fees, report.total_net_pnl,
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

// ── Managed child process ────────────────────────────────────────────────────

struct ManagedChild {
    name: String,
    child: Option<Child>,
}

impl ManagedChild {
    fn new(name: &str, child: Child) -> Self {
        Self {
            name: name.to_string(),
            child: Some(child),
        }
    }

    async fn check(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    anyhow::bail!(
                        "process '{}' exited unexpectedly with status: {}",
                        self.name,
                        status
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    anyhow::bail!("error checking process '{}': {}", self.name, e);
                }
            }
        }
        Ok(())
    }

    async fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill().await;
            let _ = child.wait().await;
            info!("Stopped: {}", self.name);
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}

// ── Pipeline runner ──────────────────────────────────────────────────────────

struct PipelineRunner {
    args: Args,
    running: Arc<AtomicBool>,
}

impl PipelineRunner {
    fn new(args: Args, running: Arc<AtomicBool>) -> Self {
        Self { args, running }
    }

    /// Find a binary in target/release or target/debug.
    fn find_bin(name: &str) -> Result<PathBuf> {
        let release = PathBuf::from(format!("target/release/{}", name));
        if release.exists() {
            return Ok(release);
        }
        let debug = PathBuf::from(format!("target/debug/{}", name));
        if debug.exists() {
            return Ok(debug);
        }
        anyhow::bail!(
            "binary '{}' not found in target/release or target/debug. Run `cargo build --release` first.",
            name
        )
    }

    /// Run a single cycle: scan → report.
    async fn run_once(&self) -> Result<()> {
        info!("=== Pipeline: single-cycle mode ===");

        if !self.args.skip_scanner {
            self.run_scanner_once().await?;
        } else {
            info!("Skipping alpha-scanner (--skip-scanner)");
        }

        self.do_generate_report()?;

        info!("=== Pipeline: single-cycle complete ===");
        Ok(())
    }

    /// Run the full daemon pipeline.
    async fn run_daemon(&self) -> Result<()> {
        let start = Utc::now();
        let max_duration = if self.args.duration_hours > 0.0 {
            Some(Duration::from_secs_f64(self.args.duration_hours * 3600.0))
        } else {
            None
        };

        info!("=== Pipeline: daemon mode ===");
        info!(
            "Paper balance: ${:.0} | Strategies: {} | Markets: {}",
            self.args.paper_balance, self.args.strategies, self.args.markets,
        );
        if let Some(dur) = max_duration {
            info!(
                "Max duration: {:.1}h ({:.0}min)",
                self.args.duration_hours,
                dur.as_secs_f64() / 60.0
            );
        } else {
            info!("Max duration: unlimited (Ctrl+C to stop)");
        }

        // Step 1: Initial alpha-scanner run
        if !self.args.skip_scanner {
            self.run_scanner_once().await?;
        }

        // Step 2: Launch daemon child processes
        let mut children: Vec<ManagedChild> = Vec::new();

        if !self.args.skip_copy_trader && self.args.watchlist.exists() {
            match self.launch_copy_trader().await {
                Ok(child) => children.push(child),
                Err(e) => error!("Failed to launch copy-trader: {:#}", e),
            }
        } else if !self.args.skip_copy_trader {
            warn!(
                "Skipping copy-trader: watchlist not found at {}",
                self.args.watchlist.display()
            );
        }

        if !self.args.skip_whale_watcher && self.args.watchlist.exists() {
            match self.launch_whale_watcher().await {
                Ok(child) => children.push(child),
                Err(e) => error!("Failed to launch whale-watcher: {:#}", e),
            }
        } else if !self.args.skip_whale_watcher {
            warn!(
                "Skipping whale-watcher: watchlist not found at {}",
                self.args.watchlist.display()
            );
        }

        if !self.args.skip_paper {
            match self.launch_paper_trading().await {
                Ok(child) => children.push(child),
                Err(e) => error!("Failed to launch paper trading: {:#}", e),
            }
        }

        if children.is_empty() {
            anyhow::bail!("No child processes launched — check watchlist and flags");
        }

        info!("{} child processes running", children.len());

        // Step 3: Monitoring loop
        let mut last_report = tokio::time::Instant::now();
        let report_interval = Duration::from_secs(self.args.report_interval);
        let mut last_scanner = tokio::time::Instant::now();
        let scanner_interval = Duration::from_secs(21600); // 6h

        loop {
            if !self.running.load(AtomicOrdering::Relaxed) {
                info!("Shutdown signal received");
                break;
            }

            if let Some(max_dur) = max_duration {
                let elapsed = Utc::now()
                    .signed_duration_since(start)
                    .to_std()
                    .unwrap_or(Duration::ZERO);
                if elapsed >= max_dur {
                    info!("Max duration reached ({:.1}h)", self.args.duration_hours);
                    break;
                }
            }

            for child in &mut children {
                if let Err(e) = child.check().await {
                    error!("{}", e);
                }
            }

            if !self.args.skip_scanner && last_scanner.elapsed() >= scanner_interval {
                info!("Periodic alpha-scanner refresh");
                match self.run_scanner_once().await {
                    Ok(()) => last_scanner = tokio::time::Instant::now(),
                    Err(e) => error!("Scanner refresh failed: {:#}", e),
                }
            }

            if last_report.elapsed() >= report_interval {
                match self.do_generate_report() {
                    Ok(report) => log_report_summary(&report),
                    Err(e) => debug!("Report generation failed: {:#}", e),
                }
                last_report = tokio::time::Instant::now();
            }

            tokio::time::sleep(Duration::from_secs(10)).await;
        }

        // Cleanup
        info!("Shutting down {} child processes...", children.len());
        for child in &mut children {
            child.kill().await;
        }

        self.do_generate_report()?;

        let elapsed = Utc::now()
            .signed_duration_since(start)
            .to_std()
            .unwrap_or(Duration::ZERO);
        info!(
            "=== Pipeline complete | elapsed: {:.1}h ===",
            elapsed.as_secs_f64() / 3600.0
        );

        Ok(())
    }

    // ── Child process launchers ─────────────────────────────────────────────

    async fn run_scanner_once(&self) -> Result<()> {
        let bin = Self::find_bin("alpha-scanner")?;
        info!("Running alpha-scanner --once ...");

        let status = Command::new(&bin)
            .arg("--once")
            .arg("--output")
            .arg(&self.args.watchlist)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("failed to execute {}", bin.display()))?;

        if !status.success() {
            anyhow::bail!("alpha-scanner exited with status: {}", status);
        }

        info!("Alpha-scanner complete: {}", self.args.watchlist.display());
        Ok(())
    }

    async fn launch_copy_trader(&self) -> Result<ManagedChild> {
        let bin = Self::find_bin("copy-trader")?;
        info!(
            "Launching copy-trader --paper --watchlist {} ...",
            self.args.watchlist.display()
        );

        let child = Command::new(&bin)
            .arg("--paper")
            .arg("--watchlist")
            .arg(&self.args.watchlist)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn {}", bin.display()))?;

        Ok(ManagedChild::new("copy-trader", child))
    }

    async fn launch_whale_watcher(&self) -> Result<ManagedChild> {
        let bin = Self::find_bin("whale-watcher")?;
        info!(
            "Launching whale-watcher --watchlist {} ...",
            self.args.watchlist.display()
        );

        let child = Command::new(&bin)
            .arg("--watchlist")
            .arg(&self.args.watchlist)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn {}", bin.display()))?;

        Ok(ManagedChild::new("whale-watcher", child))
    }

    async fn launch_paper_trading(&self) -> Result<ManagedChild> {
        let bin = Self::find_bin("zekt")?;
        info!(
            "Launching zekt --paper --strategies {} --markets {} ...",
            self.args.strategies, self.args.markets,
        );

        let child = Command::new(&bin)
            .arg("--paper")
            .arg("--strategies")
            .arg(&self.args.strategies)
            .arg("--markets")
            .arg(&self.args.markets)
            .arg("--paper-balance")
            .arg(self.args.paper_balance.to_string())
            .arg("--paper-output")
            .arg(&self.args.paper_output)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn {}", bin.display()))?;

        Ok(ManagedChild::new("zekt-paper", child))
    }

    // ── PnL tracking ────────────────────────────────────────────────────────

    fn do_generate_report(&self) -> Result<CombinedPnlReport> {
        generate_combined_report(
            PathBuf::from("data/copy-trades.json").as_path(),
            PathBuf::from("data/whale-alerts.json").as_path(),
            PathBuf::from("paper-trades.json").as_path(),
            &self.args.combined_output,
        )
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .init();

    info!("=== Zekt Pipeline Orchestrator ===");

    fs::create_dir_all("data").context("create data directory")?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let _ = ctrlc::set_handler(move || {
        info!("Received shutdown signal (Ctrl+C)...");
        r.store(false, AtomicOrdering::Relaxed);
    });

    let runner = PipelineRunner::new(args, running);

    if runner.args.once {
        runner.run_once().await
    } else {
        runner.run_daemon().await
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_args_default_values() {
        let args = Args::try_parse_from(["pipeline"]).unwrap();
        assert!(!args.once);
        assert!((args.paper_balance - 1000.0).abs() < 0.01);
        assert_eq!(args.strategies, "funding-capture");
        assert_eq!(args.markets, "BTC");
        assert_eq!(args.report_interval, 300);
        assert!(!args.skip_scanner);
        assert!(!args.skip_copy_trader);
        assert!(!args.skip_whale_watcher);
        assert!(!args.skip_paper);
    }

    #[test]
    fn test_args_once_mode() {
        let args = Args::try_parse_from(["pipeline", "--once"]).unwrap();
        assert!(args.once);
    }

    #[test]
    fn test_args_custom_strategies() {
        let args = Args::try_parse_from([
            "pipeline",
            "--strategies",
            "funding-capture,momentum-scalper",
            "--markets",
            "BTC,SOL,ETH",
        ])
        .unwrap();
        assert_eq!(args.strategies, "funding-capture,momentum-scalper");
        assert_eq!(args.markets, "BTC,SOL,ETH");
    }

    #[test]
    fn test_args_skip_flags() {
        let args = Args::try_parse_from(["pipeline", "--skip-scanner", "--skip-paper"]).unwrap();
        assert!(args.skip_scanner);
        assert!(args.skip_paper);
        assert!(!args.skip_copy_trader);
        assert!(!args.skip_whale_watcher);
    }

    #[test]
    fn test_args_duration() {
        let args = Args::try_parse_from(["pipeline", "--duration-hours", "48"]).unwrap();
        assert!((args.duration_hours - 48.0).abs() < 0.01);
    }

    #[test]
    fn test_args_invalid_paper_balance() {
        let result = Args::try_parse_from(["pipeline", "--paper-balance", "-100"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_invalid_duration() {
        let result = Args::try_parse_from(["pipeline", "--duration-hours", "-1"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_invalid_report_interval() {
        let result = Args::try_parse_from(["pipeline", "--report-interval", "0"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_report_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let output = dir.path().join("combined-pnl.json");

        let report = generate_combined_report(
            dir.path().join("no-copy.json").as_path(),
            dir.path().join("no-whale.json").as_path(),
            dir.path().join("no-paper.json").as_path(),
            &output,
        )
        .unwrap();

        assert!(report.strategies.is_empty());
        assert_eq!(report.total_trades, 0);
        assert!(output.exists());
    }

    #[test]
    fn test_generate_report_with_data() {
        let dir = tempfile::TempDir::new().unwrap();

        let copy_path = dir.path().join("copy-trades.json");
        fs::write(
            &copy_path,
            r#"[{"id":"ct-1","timestamp":"2026-05-27T10:00:00Z","wallet_address":"0xabc","market":"BTC","direction":"long","size_usd":500.0,"entry_price":104000.0,"status":"closed","close_reason":"wallet_closed","exit_price":104500.0,"pnl_usd":25.0,"whale_size_usd":5000.0,"sizing_multiplier":0.1}]"#,
        ).unwrap();

        let whale_path = dir.path().join("whale-alerts.json");
        fs::write(
            &whale_path,
            r#"[{"timestamp":"2026-05-27T09:00:00Z","wallet":"0xabc","coin":"BTC","side":"buy","size":0.5,"price":104000.0,"notional_usd":52000.0,"alert_id":"wa-1","direction":"Open Long"}]"#,
        ).unwrap();

        let paper_path = dir.path().join("paper-trades.json");
        fs::write(
            &paper_path,
            r#"[{"timestamp":"2026-05-27T08:00:00Z","strategy":"funding-capture","asset":"BTC","side":"short","entry_price":104000.0,"exit_price":103800.0,"size_usd":200.0,"gross_pnl":0.38,"entry_fee":0.20,"exit_fee":0.20,"borrow_fee":0.05,"net_pnl":-0.07,"exit_reason":"time_stop","leverage":1.0,"trade_date":"2026-05-27"}]"#,
        ).unwrap();

        let output = dir.path().join("combined-pnl.json");

        let report = generate_combined_report(&copy_path, &whale_path, &paper_path, &output).unwrap();

        assert_eq!(report.strategies.len(), 3);
        assert_eq!(report.total_trades, 3); // 1 copy + 1 whale + 1 paper
        assert!(report.data_sources.contains_key("copy-trades"));
        assert!(report.data_sources.contains_key("whale-alerts"));
        assert!(report.data_sources.contains_key("paper-trades"));

        let written: CombinedPnlReport =
            serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
        assert_eq!(written.strategies.len(), 3);
    }

    #[test]
    fn test_read_copy_trades_wins_losses() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("copy-trades.json");
        let mut f = fs::File::create(&path).unwrap();
        write!(
            f,
            r#"[
                {{"id":"ct-1","timestamp":"2026-05-27T10:00:00Z","wallet_address":"0xabc","market":"BTC","direction":"long","size_usd":500.0,"entry_price":104000.0,"status":"closed","close_reason":"wallet_closed","exit_price":104500.0,"pnl_usd":25.0,"whale_size_usd":5000.0,"sizing_multiplier":0.1}},
                {{"id":"ct-2","timestamp":"2026-05-27T11:00:00Z","wallet_address":"0xdef","market":"ETH","direction":"short","size_usd":300.0,"entry_price":3800.0,"status":"closed","close_reason":"stop_loss","exit_price":3850.0,"pnl_usd":-15.0,"whale_size_usd":3000.0,"sizing_multiplier":0.1}},
                {{"id":"ct-3","timestamp":"2026-05-27T12:00:00Z","wallet_address":"0xabc","market":"SOL","direction":"long","size_usd":200.0,"entry_price":170.0,"status":"open","close_reason":null,"exit_price":null,"pnl_usd":null,"whale_size_usd":2000.0,"sizing_multiplier":0.1}}
            ]"#
        )
        .unwrap();

        let pnl = read_copy_trades(&path).unwrap();
        assert_eq!(pnl.total_trades, 3);
        assert_eq!(pnl.closed_trades, 2);
        assert_eq!(pnl.open_trades, 1);
        assert!((pnl.gross_pnl - 10.0).abs() < 0.01);
        assert_eq!(pnl.win_count, 1);
        assert_eq!(pnl.loss_count, 1);
    }

    #[test]
    fn test_read_paper_trades_multi_strategy() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("paper-trades.json");
        fs::write(
            &path,
            r#"[
                {"timestamp":"2026-05-27T08:00:00Z","strategy":"funding-capture","asset":"BTC","side":"short","entry_price":104000.0,"exit_price":103800.0,"size_usd":200.0,"gross_pnl":0.38,"entry_fee":0.20,"exit_fee":0.20,"borrow_fee":0.05,"net_pnl":-0.07,"exit_reason":"time_stop","leverage":1.0,"trade_date":"2026-05-27"},
                {"timestamp":"2026-05-27T09:00:00Z","strategy":"momentum-scalper","asset":"SOL","side":"long","entry_price":170.0,"exit_price":172.5,"size_usd":100.0,"gross_pnl":1.47,"entry_fee":0.10,"exit_fee":0.10,"borrow_fee":0.01,"net_pnl":1.26,"exit_reason":"take_profit","leverage":3.0,"trade_date":"2026-05-27"}
            ]"#,
        ).unwrap();

        let pnl = read_paper_trades(&path).unwrap();
        assert!(pnl.strategy.contains("paper-engine"));
        assert!(pnl.strategy.contains("funding-capture"));
        assert!(pnl.strategy.contains("momentum-scalper"));
        assert_eq!(pnl.total_trades, 2);
    }

    #[test]
    fn test_log_report_summary_no_panic() {
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
            }],
            data_sources: HashMap::new(),
            errors: vec![],
        };
        log_report_summary(&report);
    }

    #[test]
    fn test_managed_child_drop_safe() {
        let running = Arc::new(AtomicBool::new(true));
        let _runner = PipelineRunner::new(
            Args::try_parse_from(["pipeline", "--once"]).unwrap(),
            running,
        );
    }
}
