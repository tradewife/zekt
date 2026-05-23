//! scrape-dextrabot — Scrapes wallet addresses + metrics from Dextrabot's discover-wallets API.
//!
//! Source: https://app.dextrabot.com/discover-wallets
//! Backend: https://dextradata.nftinit.io/api/hyper/get_wallets_profit_new/
//!
//! Dextrabot tracks 100K+ Hyperliquid wallets with pre-computed Sharpe, PnL,
//! drawdown, and growth rate across 1D/7D/30D/90D/All timeframes. This scraper
//! applies Sharpe >= 1.0 + PnL >= $10K filters to find currently profitable
//! wallets with risk-adjusted returns.
//!
//! Output: data/wallets-dextrabot.json compatible with the Python analysis pipeline.
//! Wallet addresses from this source can be fed into `analyze-wallet` for fill-level
//! analysis and strategy classification.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "scrape-dextrabot",
    about = "Scrape profitable HL wallets from Dextrabot's discover-wallets API with Sharpe + PnL filtering",
    version
)]
struct Args {
    /// Output file path (JSON). Defaults to data/wallets-dextrabot.json
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Minimum Sharpe ratio filter (30-day)
    #[arg(long, default_value_t = 1.0)]
    min_sharpe: f64,

    /// Minimum net PnL in USD (30-day)
    #[arg(long, default_value_t = 10000.0)]
    min_pnl: f64,

    /// Maximum drawdown percentage (30-day, e.g., -20 means filter out > 20% DD)
    #[arg(long, default_value_t = -100.0)]
    max_drawdown: f64,

    /// Minimum completed trade count (30-day)
    #[arg(long, default_value_t = 10)]
    min_trades: u64,

    /// Period in days: 1, 7, 30, 90
    #[arg(long, default_value_t = 30)]
    period: u32,

    /// Maximum number of wallets to return (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    max_wallets: usize,

    /// Sort order: "sharpe", "pnl", "growth", "dd"
    #[arg(long, default_value = "sharpe")]
    sort_by: String,

    /// Also fetch fills from Hyperliquid for each wallet (slower, enables full pipeline)
    #[arg(long, default_value_t = false)]
    fetch_fills: bool,
}

// ── API Types ────────────────────────────────────────────────────────────────

const DEXTRADATA_BASE: &str = "https://dextradata.nftinit.io/api/hyper";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DextrabotResponse {
    count: Option<u64>,
    results: Option<Vec<serde_json::Value>>,
}

/// Extract a float from a JSON value that might be a number or string.
fn json_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Extract an int from a JSON value that might be a number or string.
fn json_u64(v: &serde_json::Value) -> u64 {
    match v {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_json::Value::String(s) => s.parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

/// Extract a bool from a JSON value.
fn json_bool(v: &serde_json::Value) -> bool {
    v.as_bool().unwrap_or(false)
}

/// Parsed wallet metrics extracted from the raw JSON.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ParsedWallet {
    address: String,
    avg_leverage: f64,
    is_scalper: bool,
    margin_roi: f64,
    margin_used: f64,
    funding: f64,
    total_win_rate: f64,
    long_win_rate: f64,
    short_win_rate: f64,
    long_kar: f64,
    short_kar: f64,
    // Period-specific metrics
    day_sharpe: f64, day_pnl: f64, day_dd: f64, day_growth: f64, day_calc: u64,
    week_sharpe: f64, week_pnl: f64, week_dd: f64, week_growth: f64, week_calc: u64,
    month_sharpe: f64, month_pnl: f64, month_dd: f64, month_growth: f64, month_calc: u64,
    qtr_sharpe: f64, qtr_pnl: f64, qtr_dd: f64, qtr_growth: f64, qtr_calc: u64,
    all_sharpe: f64, all_pnl: f64, all_dd: f64, all_growth: f64, all_calc: u64, all_value: f64,
    open_positions: Vec<serde_json::Value>,
}

impl ParsedWallet {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let address = v.get("user_token")?.as_str()?.to_string();
        Some(Self {
            address,
            avg_leverage: v.get("avg_uleverage_value").map(json_f64).unwrap_or(0.0),
            is_scalper: v.get("is_scalper").map(json_bool).unwrap_or(false),
            margin_roi: v.get("margin_roi").map(json_f64).unwrap_or(0.0),
            margin_used: v.get("margin_used").map(json_f64).unwrap_or(0.0),
            funding: v.get("funding").map(json_f64).unwrap_or(0.0),
            total_win_rate: v.get("total_win_rate").map(json_f64).unwrap_or(0.0),
            long_win_rate: v.get("long_win_rate").map(json_f64).unwrap_or(0.0),
            short_win_rate: v.get("short_win_rate").map(json_f64).unwrap_or(0.0),
            long_kar: v.get("long_kar").map(json_f64).unwrap_or(0.0),
            short_kar: v.get("short_kar").map(json_f64).unwrap_or(0.0),
            day_sharpe: v.get("portfolio_perp_day_sharpe").map(json_f64).unwrap_or(0.0),
            day_pnl: v.get("portfolio_perp_day_pnl").map(json_f64).unwrap_or(0.0),
            day_dd: v.get("portfolio_perp_day_dd").map(json_f64).unwrap_or(0.0),
            day_growth: v.get("portfolio_perp_day_growth_rate").map(json_f64).unwrap_or(0.0),
            day_calc: v.get("portfolio_perp_day_calc_count").map(json_u64).unwrap_or(0),
            week_sharpe: v.get("portfolio_perp_week_sharpe").map(json_f64).unwrap_or(0.0),
            week_pnl: v.get("portfolio_perp_week_pnl").map(json_f64).unwrap_or(0.0),
            week_dd: v.get("portfolio_perp_week_dd").map(json_f64).unwrap_or(0.0),
            week_growth: v.get("portfolio_perp_week_growth_rate").map(json_f64).unwrap_or(0.0),
            week_calc: v.get("portfolio_perp_week_calc_count").map(json_u64).unwrap_or(0),
            month_sharpe: v.get("portfolio_perp_month_sharpe").map(json_f64).unwrap_or(0.0),
            month_pnl: v.get("portfolio_perp_month_pnl").map(json_f64).unwrap_or(0.0),
            month_dd: v.get("portfolio_perp_month_dd").map(json_f64).unwrap_or(0.0),
            month_growth: v.get("portfolio_perp_month_growth_rate").map(json_f64).unwrap_or(0.0),
            month_calc: v.get("portfolio_perp_month_calc_count").map(json_u64).unwrap_or(0),
            qtr_sharpe: v.get("portfolio_perp_3month_sharpe").map(json_f64).unwrap_or(0.0),
            qtr_pnl: v.get("portfolio_perp_3month_pnl").map(json_f64).unwrap_or(0.0),
            qtr_dd: v.get("portfolio_perp_3month_dd").map(json_f64).unwrap_or(0.0),
            qtr_growth: v.get("portfolio_perp_3month_growth_rate").map(json_f64).unwrap_or(0.0),
            qtr_calc: v.get("portfolio_perp_3month_calc_count").map(json_u64).unwrap_or(0),
            all_sharpe: v.get("portfolio_perp_all_time_sharpe").map(json_f64).unwrap_or(0.0),
            all_pnl: v.get("portfolio_perp_all_time_pnl").map(json_f64).unwrap_or(0.0),
            all_dd: v.get("portfolio_perp_all_time_dd").map(json_f64).unwrap_or(0.0),
            all_growth: v.get("portfolio_perp_all_time_growth_rate").map(json_f64).unwrap_or(0.0),
            all_calc: v.get("portfolio_perp_all_time_calc_count").map(json_u64).unwrap_or(0),
            all_value: v.get("portfolio_perp_all_time_value").map(json_f64).unwrap_or(0.0),
            open_positions: v.get("open_positions")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn calc_count(&self, period: u32) -> u64 {
        match period {
            1 => self.day_calc,
            7 => self.week_calc,
            30 => self.month_calc,
            90 => self.qtr_calc,
            _ => self.month_calc,
        }
    }

    fn dd(&self, period: u32) -> f64 {
        match period {
            1 => self.day_dd,
            7 => self.week_dd,
            30 => self.month_dd,
            90 => self.qtr_dd,
            _ => self.month_dd,
        }
    }
}

// ── Output Types (compatible with existing pipeline) ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DextrabotWalletOutput {
    /// Wallet address (EVM 0x hex)
    pub address: String,
    /// Source platform
    pub source: String,
    /// 30-day Sharpe ratio from Dextrabot
    pub sharpe_30d: f64,
    /// 30-day net PnL in USD
    pub pnl_30d: f64,
    /// 30-day max drawdown percentage
    pub drawdown_30d: f64,
    /// 30-day growth rate percentage
    pub growth_rate_30d: f64,
    /// 30-day number of calculation data points
    pub calc_count_30d: u64,
    /// 90-day Sharpe ratio
    pub sharpe_90d: f64,
    /// 90-day net PnL in USD
    pub pnl_90d: f64,
    /// All-time Sharpe ratio
    pub sharpe_all: f64,
    /// All-time net PnL in USD
    pub pnl_all: f64,
    /// Average leverage used
    pub avg_leverage: f64,
    /// Whether the wallet is flagged as scalper
    pub is_scalper: bool,
    /// Margin ROI percentage
    pub margin_roi: f64,
    /// Number of open positions
    pub open_position_count: usize,
    /// Portfolio value (all-time)
    pub portfolio_value: f64,
    /// ISO 8601 timestamp when scraped
    pub scraped_at: String,
    /// Fill records (empty unless --fetch-fills)
    pub fills: Vec<serde_json::Value>,
    /// Markets from open positions
    pub markets_traded: Vec<String>,
}

// ── API Fetching ─────────────────────────────────────────────────────────────

fn order_param(sort_by: &str, period: u32) -> String {
    match sort_by {
        "pnl" => {
            let field = period_field(period, "pnl");
            format!("-{}", field)
        }
        "growth" => {
            let field = period_field(period, "growth_rate");
            format!("-{}", field)
        }
        "dd" => {
            period_field(period, "dd") // ascending = smallest drawdown first
        }
        _ => {
            // "sharpe" default
            let field = period_field(period, "sharpe");
            format!("-{}", field)
        }
    }
}

fn period_field(period: u32, metric: &str) -> String {
    match period {
        1 => format!("portfolio_perp_day_{}", metric),
        7 => format!("portfolio_perp_week_{}", metric),
        30 => format!("portfolio_perp_month_{}", metric),
        90 => format!("portfolio_perp_3month_{}", metric),
        _ => format!("portfolio_perp_month_{}", metric),
    }
}

async fn fetch_dextrabot_wallets(
    client: &Client,
    period: u32,
    order: &str,
    min_sharpe: f64,
    min_pnl: f64,
    limit: usize,
    offset: usize,
) -> Result<DextrabotResponse> {
    let url = format!(
        "{}/get_wallets_profit_new/?period={}&order={}&offset={}&limit={}&min_sharpe={}&min_pnl={}",
        DEXTRADATA_BASE,
        period,
        order,
        offset,
        limit,
        min_sharpe,
        min_pnl,
    );

    debug!(url = %url, "Fetching Dextrabot wallets");
    info!(%url, "Dextrabot API request");

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("Dextrabot API request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Dextrabot API returned {}: {}", status, &body[..body.len().min(200)]);
    }

    let data: DextrabotResponse = resp
        .json()
        .await
        .context("Failed to parse Dextrabot response")?;

    Ok(data)
}

/// Fetch fill data from Hyperliquid for a wallet address.
async fn fetch_hl_fills(
    client: &Client,
    address: &str,
) -> Result<Vec<serde_json::Value>> {
    let thirty_days_ago_ms = Utc::now()
        .checked_sub_signed(chrono::Duration::days(30))
        .unwrap_or_else(Utc::now)
        .timestamp_millis();

    let body = serde_json::json!({
        "type": "userFillsByTime",
        "user": address,
        "startTime": thirty_days_ago_ms.max(0)
    });

    let resp = client
        .post("https://api.hyperliquid.xyz/info")
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("HL userFillsByTime request failed")?;

    let fills: Vec<serde_json::Value> = resp
        .json()
        .await
        .context("Failed to parse HL fills response")?;

    Ok(fills)
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .init();

    info!("=== scrape-dextrabot ===");
    info!(
        period = args.period,
        min_sharpe = args.min_sharpe,
        min_pnl = args.min_pnl,
        max_drawdown = args.max_drawdown,
        min_trades = args.min_trades,
        sort_by = %args.sort_by,
        "Filters"
    );

    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("data/wallets-dextrabot.json"));

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    let order = order_param(&args.sort_by, args.period);
    info!(order = %order, "Sort order");

    // Paginated fetch
    let page_size = 50;
    let mut all_wallets: Vec<ParsedWallet> = Vec::new();
    let mut offset = 0;
    let mut total_count: Option<u64> = None;
    let max_fetch = if args.max_wallets > 0 {
        args.max_wallets
    } else {
        500 // Safety limit
    };

    loop {
        let limit = page_size.min(max_fetch.saturating_sub(all_wallets.len()));
        if limit == 0 {
            break;
        }

        info!(offset, limit, "Fetching page from Dextrabot API");

        let resp = fetch_dextrabot_wallets(
            &client,
            args.period,
            &order,
            args.min_sharpe,
            args.min_pnl,
            limit,
            offset,
        )
        .await?;

        let count = resp.count.unwrap_or(0);
        if total_count.is_none() {
            total_count = Some(count);
            info!(total_matching = count, "Total wallets matching filters");
        }

        let results = resp.results.unwrap_or_default();
        let fetched = results.len();
        let parsed: Vec<ParsedWallet> = results
            .iter()
            .filter_map(ParsedWallet::from_json)
            .collect();
        info!(fetched, parsed = parsed.len(), cumulative = all_wallets.len() + parsed.len(), "Page fetched");

        all_wallets.extend(parsed);

        if fetched < limit || all_wallets.len() >= max_fetch {
            break;
        }

        offset += page_size;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    info!(total = all_wallets.len(), "Total wallets fetched from Dextrabot");

    // Client-side filter: min trades (API doesn't support this filter directly)
    if args.min_trades > 0 {
        let before = all_wallets.len();
        all_wallets.retain(|w| w.calc_count(args.period) >= args.min_trades);
        let after = all_wallets.len();
        if before != after {
            info!(before, after, "Applied min_trades={}", args.min_trades);
        }
    }

    // Client-side filter: max drawdown (DD values are negative, e.g., -21.46 = 21.46% drawdown)
    if args.max_drawdown > -100.0 {
        let before = all_wallets.len();
        all_wallets.retain(|w| w.dd(args.period) >= args.max_drawdown);
        let after = all_wallets.len();
        if before != after {
            info!(before, after, "Applied max_drawdown={}%", args.max_drawdown);
        }
    }

    if all_wallets.is_empty() {
        warn!("No wallets found matching filters");
        let empty: Vec<DextrabotWalletOutput> = vec![];
        let json = serde_json::to_string_pretty(&empty)?;
        fs::write(&output_path, &json)?;
        info!(path = %output_path.display(), "Wrote empty output");
        return Ok(());
    }

    // Convert to output format
    let mut outputs: Vec<DextrabotWalletOutput> = Vec::new();
    let now = Utc::now().to_rfc3339();

    for w in &all_wallets {
        // Extract markets from open positions
        let mut markets: Vec<String> = Vec::new();
        for pos in &w.open_positions {
            if let Some(coin) = pos.get("coin").and_then(|v| v.as_str()) {
                markets.push(coin.to_string());
            }
        }
        markets.sort();
        markets.dedup();

        let mut output = DextrabotWalletOutput {
            address: w.address.clone(),
            source: "dextrabot".to_string(),
            sharpe_30d: w.month_sharpe,
            pnl_30d: w.month_pnl,
            drawdown_30d: w.month_dd,
            growth_rate_30d: w.month_growth,
            calc_count_30d: w.month_calc,
            sharpe_90d: w.qtr_sharpe,
            pnl_90d: w.qtr_pnl,
            sharpe_all: w.all_sharpe,
            pnl_all: w.all_pnl,
            avg_leverage: w.avg_leverage,
            is_scalper: w.is_scalper,
            margin_roi: w.margin_roi,
            open_position_count: w.open_positions.len(),
            portfolio_value: w.all_value,
            scraped_at: now.clone(),
            fills: Vec::new(),
            markets_traded: markets,
        };

        // Optionally fetch fills from Hyperliquid
        if args.fetch_fills {
            match fetch_hl_fills(&client, &w.address).await {
                Ok(fills) => {
                    info!(
                        address = &w.address[..12],
                        fills = fills.len(),
                        "Fetched HL fills"
                    );
                    output.fills = fills;
                }
                Err(e) => {
                    warn!(
                        address = &w.address[..12],
                        error = %e,
                        "Failed to fetch HL fills"
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        outputs.push(output);
    }

    // Summary
    info!("=== Dextrabot Wallet Summary ===");
    for (i, w) in outputs.iter().enumerate() {
        info!(
            "  #{:<3} {} sharpe_30d={:.2} pnl_30d=${:.0} dd_30d={:.1}% growth={:.1}% lev={:.1} scalper={} markets=[{}]",
            i + 1,
            &w.address[..12],
            w.sharpe_30d,
            w.pnl_30d,
            w.drawdown_30d,
            w.growth_rate_30d,
            w.avg_leverage,
            w.is_scalper,
            w.markets_traded.join(","),
        );
    }

    // Atomic write
    if let Some(parent) = output_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .context(format!("Failed to create directory: {}", parent.display()))?;
    }

    let tmp_path = output_path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&outputs)
        .context("Failed to serialize output")?;
    fs::write(&tmp_path, &json)
        .context(format!("Failed to write temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &output_path)
        .context(format!("Failed to rename to {}", output_path.display()))?;

    info!(
        path = %output_path.display(),
        count = outputs.len(),
        "Output written successfully"
    );

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_param_sharpe() {
        assert_eq!(order_param("sharpe", 30), "-portfolio_perp_month_sharpe");
        assert_eq!(order_param("sharpe", 7), "-portfolio_perp_week_sharpe");
        assert_eq!(order_param("sharpe", 1), "-portfolio_perp_day_sharpe");
        assert_eq!(order_param("sharpe", 90), "-portfolio_perp_3month_sharpe");
    }

    #[test]
    fn test_order_param_pnl() {
        assert_eq!(order_param("pnl", 30), "-portfolio_perp_month_pnl");
    }

    #[test]
    fn test_order_param_growth() {
        assert_eq!(order_param("growth", 30), "-portfolio_perp_month_growth_rate");
    }

    #[test]
    fn test_order_param_dd() {
        assert_eq!(order_param("dd", 30), "portfolio_perp_month_dd");
    }

    #[test]
    fn test_period_field() {
        assert_eq!(period_field(1, "sharpe"), "portfolio_perp_day_sharpe");
        assert_eq!(period_field(7, "pnl"), "portfolio_perp_week_pnl");
        assert_eq!(period_field(30, "dd"), "portfolio_perp_month_dd");
        assert_eq!(period_field(90, "growth_rate"), "portfolio_perp_3month_growth_rate");
    }

    #[test]
    fn test_wallet_output_serialization() {
        let output = DextrabotWalletOutput {
            address: "0xabc123".to_string(),
            source: "dextrabot".to_string(),
            sharpe_30d: 1.5,
            pnl_30d: 50000.0,
            drawdown_30d: -5.2,
            growth_rate_30d: 33.0,
            calc_count_30d: 32,
            sharpe_90d: 1.2,
            pnl_90d: 120000.0,
            sharpe_all: 0.8,
            pnl_all: 500000.0,
            avg_leverage: 3.0,
            is_scalper: true,
            margin_roi: 15.5,
            open_position_count: 2,
            portfolio_value: 100000.0,
            scraped_at: "2026-05-23T00:00:00Z".to_string(),
            fills: vec![],
            markets_traded: vec!["BTC".to_string(), "ETH".to_string()],
        };

        let json = serde_json::to_string(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["address"], "0xabc123");
        assert_eq!(parsed["source"], "dextrabot");
        assert!((parsed["sharpe_30d"].as_f64().unwrap() - 1.5).abs() < 0.001);
        assert!(parsed["is_scalper"].as_bool().unwrap());
    }

    #[test]
    fn test_dextrabot_response_parsing() {
        let raw = serde_json::json!({
            "count": 5,
            "results": [
                {
                    "user_token": "0xabc123",
                    "portfolio_perp_month_sharpe": 1.5,
                    "portfolio_perp_month_pnl": 50000.0,
                    "portfolio_perp_month_dd": -5.2,
                    "portfolio_perp_month_growth_rate": 33.0,
                    "portfolio_perp_month_calc_count": 32,
                    "is_scalper": true,
                    "avg_uleverage_value": 3.0,
                    "open_positions": [
                        {"coin": "BTC", "szi": 1.5, "direction": "long"}
                    ]
                }
            ]
        });

        let resp: DextrabotResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(resp.count, Some(5));
        assert_eq!(resp.results.unwrap().len(), 1);
    }

    #[test]
    fn test_empty_response() {
        let raw = serde_json::json!({
            "count": 0,
            "results": []
        });

        let resp: DextrabotResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(resp.count, Some(0));
        assert!(resp.results.unwrap().is_empty());
    }
}
