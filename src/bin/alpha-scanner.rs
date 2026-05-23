//! alpha-scanner — Wallet discovery daemon for Hyperliquid profitable wallets.
//!
//! Fetches top wallets from Dextrabot API, enriches with Hypurrscan tags
//! and HL current positions, scores by composite metric, and outputs a
//! ranked watchlist to data/watchlist.json.
//!
//! Modes:
//!   --once    Single scan cycle, then exit
//!   --daemon  Continuous refresh every 6h (default)
//!
//! Scoring: composite_score = sharpe * log(|pnl| + 1) * consistency_factor

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "alpha-scanner",
    about = "Discover and rank profitable Hyperliquid wallets from Dextrabot, enrich with Hypurrscan tags and HL positions",
    version
)]
struct Args {
    /// Run a single scan cycle and exit (default: daemon mode)
    #[arg(long)]
    once: bool,

    /// Minimum Sharpe ratio filter (30-day). Reject if < this value.
    #[arg(long, default_value_t = 0.0, value_parser = validate_min_sharpe)]
    min_sharpe: f64,

    /// Minimum net PnL in USD (30-day). Reject if < this value.
    #[arg(long, default_value_t = 0.0, value_parser = validate_min_pnl)]
    min_pnl: f64,

    /// Maximum number of wallets in the output watchlist.
    #[arg(long, default_value_t = 50, value_parser = validate_watchlist_size)]
    watchlist_size: usize,

    /// Output file path for the ranked watchlist JSON.
    #[arg(long, default_value = "data/watchlist.json")]
    output: PathBuf,
}

fn validate_min_sharpe(v: &str) -> Result<f64, String> {
    let val: f64 = v.parse().map_err(|_| format!("invalid min-sharpe value: {}", v))?;
    if val < 0.0 {
        return Err(format!("min-sharpe must be >= 0, got {}", val));
    }
    Ok(val)
}

fn validate_min_pnl(v: &str) -> Result<f64, String> {
    let val: f64 = v.parse().map_err(|_| format!("invalid min-pnl value: {}", v))?;
    Ok(val)
}

fn validate_watchlist_size(v: &str) -> Result<usize, String> {
    let val: usize = v.parse().map_err(|_| format!("invalid watchlist-size value: {}", v))?;
    if val == 0 {
        return Err("watchlist-size must be > 0".to_string());
    }
    Ok(val)
}

// ── Constants ────────────────────────────────────────────────────────────────

const DEXTRADATA_BASE: &str = "https://dextradata.nftinit.io/api/hyper";
const HYPURRSCAN_BASE: &str = "https://api.hypurrscan.io";
const HYPURRSCAN_REFRESH_URL: &str = "https://hypurrscan.io/api/auth/refresh";
const HL_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 21600; // 6 hours

// ── Data Types ───────────────────────────────────────────────────────────────

/// A wallet entry fetched from Dextrabot (raw parse).
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct RawWallet {
    address: String,
    month_sharpe: f64,
    month_pnl: f64,
    week_sharpe: f64,
    week_pnl: f64,
    total_win_rate: f64,
    long_win_rate: f64,
    short_win_rate: f64,
    is_scalper: bool,
    avg_leverage: f64,
}

/// Hypurrscan tag response (GET /tags/{address}).
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
struct TagsResponse(Vec<String>);

/// JWT refresh response from Hypurrscan.
#[derive(Debug, Clone, Deserialize)]
struct JwtRefreshResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    jwt: Option<String>,
}

/// Simplified HL position for watchlist output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistPosition {
    pub coin: String,
    pub size: String,
    pub entry_px: String,
}

/// A single wallet in the ranked watchlist output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub address: String,
    pub score: f64,
    pub sharpe: f64,
    pub pnl: f64,
    pub tags: Vec<String>,
    pub positions: Vec<WatchlistPosition>,
    pub decaying: bool,
}

/// Top-level watchlist file output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchlist {
    pub generated_at: String,
    pub wallets: Vec<WatchlistEntry>,
}

/// Change report comparing two watchlists.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeReport {
    pub added: usize,
    pub removed: usize,
    pub score_changed: usize,
    pub details: ChangeReportDetails,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeReportDetails {
    pub added_addresses: Vec<String>,
    pub removed_addresses: Vec<String>,
    pub score_changes: Vec<ScoreChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreChange {
    pub address: String,
    pub old_score: f64,
    pub new_score: f64,
    pub delta: f64,
}

// ── JSON Parsing Helpers ─────────────────────────────────────────────────────

fn json_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

// ── Dextrabot Fetching ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct DextrabotResponse {
    count: Option<u64>,
    results: Option<Vec<serde_json::Value>>,
}

fn parse_raw_wallet(v: &serde_json::Value) -> Option<RawWallet> {
    let address = v.get("user_token")?.as_str()?.to_string();
    Some(RawWallet {
        address,
        month_sharpe: v.get("portfolio_perp_month_sharpe").map(json_f64).unwrap_or(0.0),
        month_pnl: v.get("portfolio_perp_month_pnl").map(json_f64).unwrap_or(0.0),
        week_sharpe: v.get("portfolio_perp_week_sharpe").map(json_f64).unwrap_or(0.0),
        week_pnl: v.get("portfolio_perp_week_pnl").map(json_f64).unwrap_or(0.0),
        total_win_rate: v.get("total_win_rate").map(json_f64).unwrap_or(0.0),
        long_win_rate: v.get("long_win_rate").map(json_f64).unwrap_or(0.0),
        short_win_rate: v.get("short_win_rate").map(json_f64).unwrap_or(0.0),
        is_scalper: v.get("is_scalper").and_then(|v| v.as_bool()).unwrap_or(false),
        avg_leverage: v.get("avg_uleverage_value").map(json_f64).unwrap_or(0.0),
    })
}

async fn fetch_dextrabot_wallets(
    client: &Client,
    min_sharpe: f64,
    min_pnl: f64,
    limit: usize,
    offset: usize,
) -> Result<DextrabotResponse> {
    let url = format!(
        "{}/get_wallets_profit_new/?period=30&order=-portfolio_perp_month_sharpe&offset={}&limit={}&min_sharpe={}&min_pnl={}",
        DEXTRADATA_BASE, offset, limit, min_sharpe, min_pnl,
    );

    debug!(url = %url, "Fetching Dextrabot wallets");

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

async fn fetch_all_dextrabot_wallets(
    client: &Client,
    min_sharpe: f64,
    min_pnl: f64,
    watchlist_size: usize,
) -> Result<Vec<RawWallet>> {
    // Fetch enough pages to get watchlist_size * 2 wallets (some may be filtered)
    let fetch_target = (watchlist_size * 3).max(100);
    let page_size = 50;
    let mut all_wallets: Vec<RawWallet> = Vec::new();
    let mut offset = 0;

    loop {
        let remaining = fetch_target.saturating_sub(all_wallets.len());
        if remaining == 0 {
            break;
        }
        let limit = page_size.min(remaining);

        let resp = fetch_dextrabot_wallets(client, min_sharpe, min_pnl, limit, offset).await?;
        let results = resp.results.unwrap_or_default();
        let fetched = results.len();

        let parsed: Vec<RawWallet> = results.iter().filter_map(parse_raw_wallet).collect();
        all_wallets.extend(parsed);

        if fetched < limit {
            break;
        }

        offset += page_size;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    Ok(all_wallets)
}

// ── Client-side Filtering ────────────────────────────────────────────────────

fn apply_filters(
    wallets: Vec<RawWallet>,
    min_sharpe: f64,
    min_pnl: f64,
) -> Vec<RawWallet> {
    wallets
        .into_iter()
        .filter(|w| w.month_sharpe >= min_sharpe && w.month_pnl >= min_pnl)
        .collect()
}

// ── Hypurrscan Enrichment ────────────────────────────────────────────────────

async fn try_fetch_tags(
    client: &Client,
    url: &str,
    token: Option<&str>,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut req = client.get(url).timeout(Duration::from_secs(10));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {}", t));
    }
    req.send().await
}

async fn fetch_hypurrscan_tags(
    client: &Client,
    address: &str,
    jwt: &mut Option<String>,
    refresh_token: &Option<String>,
) -> Vec<String> {
    let url = format!("{}/tags/{}", HYPURRSCAN_BASE, address);

    // First attempt
    let resp = match try_fetch_tags(client, &url, jwt.as_deref()).await {
        Ok(r) => r,
        Err(e) => {
            debug!(address = &address[..address.len().min(12)], error = %e, "Hypurrscan tags request failed");
            return Vec::new();
        }
    };

    let status = resp.status();

    // If 401, attempt JWT refresh
    if status.as_u16() == 401 {
        if let Some(rt) = refresh_token {
            info!("Hypurrscan JWT expired, attempting refresh...");
            match refresh_jwt(client, rt).await {
                Ok(new_jwt) => {
                    info!("JWT refreshed successfully");
                    *jwt = Some(new_jwt.clone());
                    // Retry with new JWT
                    match try_fetch_tags(client, &url, Some(&new_jwt)).await {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                match resp.json::<TagsResponse>().await {
                                    Ok(tags) => return tags.0,
                                    Err(e) => {
                                        debug!(address = &address[..address.len().min(12)], error = %e, "Failed to parse tags");
                                        return Vec::new();
                                    }
                                }
                            } else {
                                debug!(status = %resp.status(), "Hypurrscan tags retry failed");
                                return Vec::new();
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "Hypurrscan retry request failed");
                            return Vec::new();
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "JWT refresh failed");
                    return Vec::new();
                }
            }
        } else {
            debug!("Hypurrscan returned 401 and no refresh token available");
            return Vec::new();
        }
    }

    if !status.is_success() {
        debug!(status = %status, address = &address[..address.len().min(12)], "Hypurrscan tags returned non-success");
        return Vec::new();
    }

    match resp.json::<TagsResponse>().await {
        Ok(tags) => tags.0,
        Err(e) => {
            debug!(address = &address[..address.len().min(12)], error = %e, "Failed to parse tags response");
            Vec::new()
        }
    }
}

async fn refresh_jwt(client: &Client, refresh_token: &str) -> Result<String> {
    let body = serde_json::json!({
        "refresh_token": refresh_token
    });

    let resp = client
        .post(HYPURRSCAN_REFRESH_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("JWT refresh request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("JWT refresh returned {}: {}", status, &text[..text.len().min(200)]);
    }

    let refresh_resp: JwtRefreshResponse = resp
        .json()
        .await
        .context("Failed to parse JWT refresh response")?;

    // Try multiple possible field names for the new token
    let new_jwt = refresh_resp
        .access_token
        .or(refresh_resp.token)
        .or(refresh_resp.jwt)
        .context("JWT refresh response missing token field")?;

    Ok(new_jwt)
}

// ── HL Position Enrichment ───────────────────────────────────────────────────

async fn fetch_hl_positions(client: &Client, address: &str) -> Vec<WatchlistPosition> {
    let body = serde_json::json!({
        "type": "clearinghouseState",
        "user": address
    });

    let resp = match client
        .post(HL_INFO_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            debug!(address = &address[..address.len().min(12)], error = %e, "HL positions request failed");
            return Vec::new();
        }
    };

    if !resp.status().is_success() {
        debug!(status = %resp.status(), address = &address[..address.len().min(12)], "HL positions returned non-success");
        return Vec::new();
    }

    let raw: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            debug!(address = &address[..address.len().min(12)], error = %e, "Failed to parse HL positions");
            return Vec::new();
        }
    };

    parse_hl_positions(&raw)
}

fn parse_hl_positions(raw: &serde_json::Value) -> Vec<WatchlistPosition> {
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

        let size_str = format!("{}", size);

        positions.push(WatchlistPosition {
            coin,
            size: size_str,
            entry_px,
        });
    }

    positions
}

// ── Scoring ──────────────────────────────────────────────────────────────────

/// Compute composite score: sharpe * log(|pnl| + 1) * consistency_factor
///
/// consistency_factor is the total_win_rate clamped to [0.01, 1.0].
/// For wallets with win_rate = 0, we use a small floor of 0.01 to avoid
/// zeroing out the score entirely.
pub fn compute_composite_score(sharpe: f64, pnl: f64, win_rate: f64) -> f64 {
    let consistency = win_rate.clamp(0.01, 1.0);
    let pnl_component = if pnl.abs() > 0.0 {
        (pnl.abs() + 1.0).ln()
    } else {
        0.0
    };
    sharpe * pnl_component * consistency
}

// ── Decay Detection ──────────────────────────────────────────────────────────

/// Detect wallet decay: 30d profitable but 7d negative PnL.
pub fn detect_decay(month_pnl: f64, week_pnl: f64) -> bool {
    month_pnl > 0.0 && week_pnl < 0.0
}

// ── Watchlist Diffing ────────────────────────────────────────────────────────

/// Compare two watchlists and produce a change report.
pub fn diff_watchlists(old: &Watchlist, new: &Watchlist) -> ChangeReport {
    let old_map: std::collections::HashMap<&str, &WatchlistEntry> = old
        .wallets
        .iter()
        .map(|w| (w.address.as_str(), w))
        .collect();

    let new_map: std::collections::HashMap<&str, &WatchlistEntry> = new
        .wallets
        .iter()
        .map(|w| (w.address.as_str(), w))
        .collect();

    let old_addrs: HashSet<&str> = old_map.keys().copied().collect();
    let new_addrs: HashSet<&str> = new_map.keys().copied().collect();

    let added: Vec<String> = new_addrs
        .difference(&old_addrs)
        .map(|a| a.to_string())
        .collect();

    let removed: Vec<String> = old_addrs
        .difference(&new_addrs)
        .map(|a| a.to_string())
        .collect();

    let score_changes: Vec<ScoreChange> = old_addrs
        .intersection(&new_addrs)
        .filter_map(|addr| {
            let old_entry = old_map.get(addr)?;
            let new_entry = new_map.get(addr)?;
            let delta = new_entry.score - old_entry.score;
            if delta.abs() > f64::EPSILON {
                Some(ScoreChange {
                    address: addr.to_string(),
                    old_score: old_entry.score,
                    new_score: new_entry.score,
                    delta,
                })
            } else {
                None
            }
        })
        .collect();

    ChangeReport {
        added: added.len(),
        removed: removed.len(),
        score_changed: score_changes.len(),
        details: ChangeReportDetails {
            added_addresses: added,
            removed_addresses: removed,
            score_changes,
        },
    }
}

// ── Output ───────────────────────────────────────────────────────────────────

fn atomic_write_json(path: &PathBuf, data: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .context(format!("Failed to create directory: {}", parent.display()))?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(data)
        .context("Failed to serialize watchlist")?;
    fs::write(&tmp_path, &json_str)
        .context(format!("Failed to write temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .context(format!("Failed to rename to {}", path.display()))?;

    Ok(())
}

fn load_previous_watchlist(path: &PathBuf) -> Option<Watchlist> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

// ── Scan Cycle ───────────────────────────────────────────────────────────────

async fn run_scan_cycle(
    client: &Client,
    args: &Args,
    jwt: &mut Option<String>,
) -> Result<Watchlist> {
    info!(
        min_sharpe = args.min_sharpe,
        min_pnl = args.min_pnl,
        watchlist_size = args.watchlist_size,
        output = %args.output.display(),
        "Starting scan cycle"
    );

    // 1. Fetch wallets from Dextrabot
    let raw_wallets = fetch_all_dextrabot_wallets(
        client,
        args.min_sharpe,
        args.min_pnl,
        args.watchlist_size,
    )
    .await
    .context("Failed to fetch wallets from Dextrabot")?;

    info!(fetched = raw_wallets.len(), "Fetched wallets from Dextrabot");

    // 2. Apply client-side filters
    let filtered = apply_filters(raw_wallets, args.min_sharpe, args.min_pnl);
    info!(
        fetched = filtered.len(),
        passed_filters = filtered.len(),
        "Applied filters"
    );

    if filtered.is_empty() {
        info!("No wallets passed filters");
        let watchlist = Watchlist {
            generated_at: Utc::now().to_rfc3339(),
            wallets: Vec::new(),
        };
        return Ok(watchlist);
    }

    // 3. Enrich with Hypurrscan tags and HL positions, compute scores
    let refresh_token = std::env::var("HYPURRSCAN_REFRESH_TOKEN").ok();
    let mut entries: Vec<WatchlistEntry> = Vec::new();

    for wallet in &filtered {
        // Fetch Hypurrscan tags
        let tags = fetch_hypurrscan_tags(client, &wallet.address, jwt, &refresh_token).await;

        // Fetch HL positions
        let positions = fetch_hl_positions(client, &wallet.address).await;

        // Compute score
        let score = compute_composite_score(wallet.month_sharpe, wallet.month_pnl, wallet.total_win_rate);

        // Detect decay
        let decaying = detect_decay(wallet.month_pnl, wallet.week_pnl);

        entries.push(WatchlistEntry {
            address: wallet.address.clone(),
            score,
            sharpe: wallet.month_sharpe,
            pnl: wallet.month_pnl,
            tags,
            positions,
            decaying,
        });

        // Small delay to respect rate limits
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 4. Sort by score descending
    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });

    // 5. Truncate to watchlist_size
    entries.truncate(args.watchlist_size);

    info!(
        total = entries.len(),
        decaying = entries.iter().filter(|e| e.decaying).count(),
        "Scored and ranked wallets"
    );

    if entries.iter().all(|e| !e.decaying) {
        info!("no decaying wallets detected");
    }

    let watchlist = Watchlist {
        generated_at: Utc::now().to_rfc3339(),
        wallets: entries,
    };

    Ok(watchlist)
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

    info!("=== alpha-scanner ===");
    info!(
        mode = if args.once { "once" } else { "daemon" },
        min_sharpe = args.min_sharpe,
        min_pnl = args.min_pnl,
        watchlist_size = args.watchlist_size,
        output = %args.output.display(),
        "Configuration"
    );

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    let mut jwt = std::env::var("HYPURRSCAN_JWT").ok();

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        info!("Received SIGINT, shutting down...");
        r.store(false, AtomicOrdering::SeqCst);
    })
    .context("Failed to set ctrlc handler")?;

    loop {
        // Load previous watchlist for diffing
        let previous = load_previous_watchlist(&args.output);

        // Run scan cycle
        match run_scan_cycle(&client, &args, &mut jwt).await {
            Ok(watchlist) => {
                // Generate change report if we have a previous watchlist
                if let Some(ref prev) = previous {
                    let report = diff_watchlists(prev, &watchlist);
                    info!(
                        "change report: {} added, {} removed, {} score-changed",
                        report.added, report.removed, report.score_changed
                    );
                    if !report.details.added_addresses.is_empty() {
                        debug!(addresses = ?report.details.added_addresses, "Added wallets");
                    }
                    if !report.details.removed_addresses.is_empty() {
                        debug!(addresses = ?report.details.removed_addresses, "Removed wallets");
                    }
                    for sc in &report.details.score_changes {
                        debug!(
                            address = &sc.address[..sc.address.len().min(12)],
                            old = sc.old_score,
                            new = sc.new_score,
                            delta = sc.delta,
                            "Score change"
                        );
                    }
                }

                // Write output
                let json = serde_json::to_value(&watchlist)
                    .context("Failed to serialize watchlist")?;
                atomic_write_json(&args.output, &json)?;
                info!(
                    path = %args.output.display(),
                    count = watchlist.wallets.len(),
                    "Watchlist written"
                );
            }
            Err(e) => {
                error!(error = %e, "Scan cycle failed");
                // On API failure: exit 1 with error
                std::process::exit(1);
            }
        }

        if args.once {
            return Ok(());
        }

        // Daemon mode: sleep until next cycle
        info!(
            interval_secs = DEFAULT_REFRESH_INTERVAL_SECS,
            "Sleeping until next scan cycle..."
        );
        let sleep_duration = Duration::from_secs(DEFAULT_REFRESH_INTERVAL_SECS);
        let start = std::time::Instant::now();
        while start.elapsed() < sleep_duration {
            if !running.load(AtomicOrdering::SeqCst) {
                info!("Shutting down daemon...");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =======================================================================
    // CLI validation
    // =======================================================================

    #[test]
    fn test_validate_min_sharpe_valid() {
        assert!(validate_min_sharpe("0.0").is_ok());
        assert!(validate_min_sharpe("1.5").is_ok());
        assert!(validate_min_sharpe("100.0").is_ok());
    }

    #[test]
    fn test_validate_min_sharpe_negative_rejected() {
        assert!(validate_min_sharpe("-1.0").is_err());
    }

    #[test]
    fn test_validate_min_sharpe_non_numeric_rejected() {
        assert!(validate_min_sharpe("abc").is_err());
    }

    #[test]
    fn test_validate_watchlist_size_valid() {
        assert!(validate_watchlist_size("1").is_ok());
        assert!(validate_watchlist_size("50").is_ok());
        assert!(validate_watchlist_size("1000").is_ok());
    }

    #[test]
    fn test_validate_watchlist_size_zero_rejected() {
        assert!(validate_watchlist_size("0").is_err());
    }

    #[test]
    fn test_validate_watchlist_size_non_numeric_rejected() {
        assert!(validate_watchlist_size("abc").is_err());
    }

    #[test]
    fn test_validate_min_pnl_valid() {
        assert!(validate_min_pnl("0.0").is_ok());
        assert!(validate_min_pnl("5000.0").is_ok());
        assert!(validate_min_pnl("-100.0").is_ok()); // negative allowed (filter for losses)
    }

    #[test]
    fn test_validate_min_pnl_non_numeric_rejected() {
        assert!(validate_min_pnl("abc").is_err());
    }

    // =======================================================================
    // Composite scoring
    // =======================================================================

    #[test]
    fn test_score_basic() {
        // sharpe=2.0, pnl=10000.0, win_rate=0.6
        // log(10001) ≈ 9.21
        // score = 2.0 * 9.21 * 0.6 ≈ 11.05
        let score = compute_composite_score(2.0, 10000.0, 0.6);
        assert!(score > 10.0 && score < 12.0, "score={}", score);
    }

    #[test]
    fn test_score_zero_sharpe() {
        let score = compute_composite_score(0.0, 10000.0, 0.6);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_score_zero_pnl() {
        let score = compute_composite_score(2.0, 0.0, 0.6);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_score_zero_win_rate() {
        // win_rate=0 gets clamped to 0.01
        let score = compute_composite_score(2.0, 10000.0, 0.0);
        assert!(score > 0.0, "score should be positive with floor consistency");
    }

    #[test]
    fn test_score_high_sharpe_high_pnl() {
        let score_high = compute_composite_score(5.0, 500000.0, 0.7);
        let score_low = compute_composite_score(1.0, 5000.0, 0.5);
        assert!(score_high > score_low);
    }

    #[test]
    fn test_score_negative_pnl_positive_sharpe() {
        // Negative PnL still contributes via abs()
        let score = compute_composite_score(2.0, -5000.0, 0.6);
        assert!(score > 0.0, "negative PnL should still produce positive score");
    }

    #[test]
    fn test_score_ordering_matches_expectation() {
        let s1 = compute_composite_score(3.0, 100000.0, 0.8);
        let s2 = compute_composite_score(2.0, 50000.0, 0.6);
        let s3 = compute_composite_score(1.5, 10000.0, 0.4);
        assert!(s1 > s2);
        assert!(s2 > s3);
    }

    #[test]
    fn test_score_win_rate_clamped_to_one() {
        let score = compute_composite_score(2.0, 10000.0, 1.5);
        // win_rate 1.5 clamped to 1.0
        let expected_sharpe = 2.0;
        let expected_pnl_comp = (10001.0_f64).ln();
        let expected = expected_sharpe * expected_pnl_comp * 1.0;
        assert!((score - expected).abs() < 0.001, "score={}", score);
    }

    // =======================================================================
    // Decay detection
    // =======================================================================

    #[test]
    fn test_decay_profitable_then_losing() {
        assert!(detect_decay(10000.0, -500.0));
    }

    #[test]
    fn test_decay_not_decaying_both_positive() {
        assert!(!detect_decay(10000.0, 500.0));
    }

    #[test]
    fn test_decay_not_decaying_both_negative() {
        assert!(!detect_decay(-1000.0, -500.0));
    }

    #[test]
    fn test_decay_zero_month_pnl() {
        assert!(!detect_decay(0.0, -500.0));
    }

    #[test]
    fn test_decay_zero_week_pnl() {
        assert!(!detect_decay(10000.0, 0.0));
    }

    #[test]
    fn test_decay_large_negative_week() {
        assert!(detect_decay(50000.0, -20000.0));
    }

    // =======================================================================
    // Client-side filtering
    // =======================================================================

    #[test]
    fn test_filter_passes_all() {
        let wallets = vec![
            make_raw_wallet("0xa", 2.0, 10000.0, 0.6),
            make_raw_wallet("0xb", 3.0, 50000.0, 0.7),
        ];
        let result = apply_filters(wallets, 1.0, 5000.0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_removes_low_sharpe() {
        let wallets = vec![
            make_raw_wallet("0xa", 2.0, 10000.0, 0.6),
            make_raw_wallet("0xb", 0.5, 50000.0, 0.7),
        ];
        let result = apply_filters(wallets, 1.0, 5000.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].address, "0xa");
    }

    #[test]
    fn test_filter_removes_low_pnl() {
        let wallets = vec![
            make_raw_wallet("0xa", 2.0, 10000.0, 0.6),
            make_raw_wallet("0xb", 3.0, 1000.0, 0.7),
        ];
        let result = apply_filters(wallets, 1.0, 5000.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].address, "0xa");
    }

    #[test]
    fn test_filter_removes_both() {
        let wallets = vec![
            make_raw_wallet("0xa", 0.5, 1000.0, 0.6),
            make_raw_wallet("0xb", 3.0, 1000.0, 0.7),
            make_raw_wallet("0xc", 0.5, 50000.0, 0.7),
        ];
        let result = apply_filters(wallets, 1.0, 5000.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_exact_threshold_passes() {
        let wallets = vec![make_raw_wallet("0xa", 2.0, 5000.0, 0.6)];
        let result = apply_filters(wallets, 2.0, 5000.0);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_empty_input() {
        let result = apply_filters(vec![], 1.0, 5000.0);
        assert!(result.is_empty());
    }

    // =======================================================================
    // Watchlist diffing
    // =======================================================================

    #[test]
    fn test_diff_empty_to_populated() {
        let old = make_watchlist(vec![]);
        let new = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
        ]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 2);
        assert_eq!(report.removed, 0);
        assert_eq!(report.score_changed, 0);
    }

    #[test]
    fn test_diff_populated_to_empty() {
        let old = make_watchlist(vec![
            make_entry("0xa", 10.0),
        ]);
        let new = make_watchlist(vec![]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 0);
        assert_eq!(report.removed, 1);
        assert_eq!(report.score_changed, 0);
    }

    #[test]
    fn test_diff_score_changes() {
        let old = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
        ]);
        let new = make_watchlist(vec![
            make_entry("0xa", 12.0),
            make_entry("0xb", 8.0),
        ]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.score_changed, 1);
        assert_eq!(report.details.score_changes[0].address, "0xa");
        assert!((report.details.score_changes[0].delta - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_diff_mixed_changes() {
        let old = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
            make_entry("0xc", 6.0),
        ]);
        let new = make_watchlist(vec![
            make_entry("0xa", 11.0), // score changed
            make_entry("0xd", 9.0),  // added
            // 0xb unchanged, 0xc removed
            make_entry("0xb", 8.0),
        ]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 1);
        assert_eq!(report.removed, 1);
        assert_eq!(report.score_changed, 1);
    }

    #[test]
    fn test_diff_no_changes() {
        let old = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
        ]);
        let new = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
        ]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.score_changed, 0);
    }

    #[test]
    fn test_diff_both_empty() {
        let old = make_watchlist(vec![]);
        let new = make_watchlist(vec![]);
        let report = diff_watchlists(&old, &new);
        assert_eq!(report.added, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.score_changed, 0);
    }

    #[test]
    fn test_diff_details_added_addresses() {
        let old = make_watchlist(vec![make_entry("0xa", 10.0)]);
        let new = make_watchlist(vec![
            make_entry("0xa", 10.0),
            make_entry("0xb", 8.0),
            make_entry("0xc", 6.0),
        ]);
        let report = diff_watchlists(&old, &new);
        let mut added = report.details.added_addresses.clone();
        added.sort();
        assert_eq!(added, vec!["0xb", "0xc"]);
    }

    // =======================================================================
    // HL positions parsing
    // =======================================================================

    #[test]
    fn test_parse_hl_positions_multiple() {
        let raw = json!({
            "assetPositions": [
                {
                    "position": {
                        "coin": "BTC",
                        "szi": "0.5",
                        "entryPx": "60000.0"
                    }
                },
                {
                    "position": {
                        "coin": "ETH",
                        "szi": "-2.0",
                        "entryPx": "3000.0"
                    }
                }
            ]
        });
        let positions = parse_hl_positions(&raw);
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].coin, "BTC");
        assert_eq!(positions[0].size, "0.5");
        assert_eq!(positions[0].entry_px, "60000.0");
        assert_eq!(positions[1].coin, "ETH");
        assert!((positions[1].size.parse::<f64>().unwrap() - (-2.0)).abs() < 0.001);
    }

    #[test]
    fn test_parse_hl_positions_empty() {
        let raw = json!({
            "assetPositions": []
        });
        let positions = parse_hl_positions(&raw);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_parse_hl_positions_missing_field() {
        let raw = json!({});
        let positions = parse_hl_positions(&raw);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_parse_hl_positions_zero_size_filtered() {
        let raw = json!({
            "assetPositions": [
                {
                    "position": {
                        "coin": "BTC",
                        "szi": "0.5",
                        "entryPx": "60000.0"
                    }
                },
                {
                    "position": {
                        "coin": "ETH",
                        "szi": "0.0",
                        "entryPx": "3000.0"
                    }
                }
            ]
        });
        let positions = parse_hl_positions(&raw);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].coin, "BTC");
    }

    #[test]
    fn test_parse_hl_positions_size_field_fallback() {
        // Some responses use "size" instead of "szi"
        let raw = json!({
            "assetPositions": [
                {
                    "position": {
                        "coin": "SOL",
                        "size": "10.0",
                        "entryPx": "150.0"
                    }
                }
            ]
        });
        let positions = parse_hl_positions(&raw);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].coin, "SOL");
        assert_eq!(positions[0].size, "10");
    }

    // =======================================================================
    // RawWallet parsing
    // =======================================================================

    #[test]
    fn test_parse_raw_wallet_full() {
        let raw = json!({
            "user_token": "0xabc123",
            "portfolio_perp_month_sharpe": 2.5,
            "portfolio_perp_month_pnl": 50000.0,
            "portfolio_perp_week_sharpe": 1.8,
            "portfolio_perp_week_pnl": -1000.0,
            "total_win_rate": 0.65,
            "long_win_rate": 0.7,
            "short_win_rate": 0.6,
            "is_scalper": true,
            "avg_uleverage_value": 3.5,
        });

        let wallet = parse_raw_wallet(&raw).unwrap();
        assert_eq!(wallet.address, "0xabc123");
        assert!((wallet.month_sharpe - 2.5).abs() < 0.001);
        assert!((wallet.month_pnl - 50000.0).abs() < 0.001);
        assert!((wallet.week_sharpe - 1.8).abs() < 0.001);
        assert!((wallet.week_pnl - (-1000.0)).abs() < 0.001);
        assert!((wallet.total_win_rate - 0.65).abs() < 0.001);
        assert!(wallet.is_scalper);
        assert!((wallet.avg_leverage - 3.5).abs() < 0.001);
    }

    #[test]
    fn test_parse_raw_wallet_missing_token_returns_none() {
        let raw = json!({
            "portfolio_perp_month_sharpe": 2.5,
        });
        assert!(parse_raw_wallet(&raw).is_none());
    }

    #[test]
    fn test_parse_raw_wallet_defaults() {
        let raw = json!({
            "user_token": "0xabc",
        });
        let wallet = parse_raw_wallet(&raw).unwrap();
        assert_eq!(wallet.month_sharpe, 0.0);
        assert_eq!(wallet.month_pnl, 0.0);
        assert_eq!(wallet.total_win_rate, 0.0);
        assert!(!wallet.is_scalper);
    }

    // =======================================================================
    // Watchlist serialization
    // =======================================================================

    #[test]
    fn test_watchlist_serialization_roundtrip() {
        let watchlist = Watchlist {
            generated_at: "2026-05-23T00:00:00Z".to_string(),
            wallets: vec![
                WatchlistEntry {
                    address: "0xabc".to_string(),
                    score: 11.5,
                    sharpe: 2.5,
                    pnl: 50000.0,
                    tags: vec!["HFUN Bot User".to_string()],
                    positions: vec![WatchlistPosition {
                        coin: "BTC".to_string(),
                        size: "0.5".to_string(),
                        entry_px: "60000.0".to_string(),
                    }],
                    decaying: false,
                },
            ],
        };

        let json_str = serde_json::to_string(&watchlist).unwrap();
        let parsed: Watchlist = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.generated_at, "2026-05-23T00:00:00Z");
        assert_eq!(parsed.wallets.len(), 1);
        assert_eq!(parsed.wallets[0].address, "0xabc");
        assert!((parsed.wallets[0].score - 11.5).abs() < 0.001);
        assert!(!parsed.wallets[0].decaying);
        assert_eq!(parsed.wallets[0].tags.len(), 1);
        assert_eq!(parsed.wallets[0].positions.len(), 1);
    }

    #[test]
    fn test_watchlist_empty_wallets() {
        let watchlist = Watchlist {
            generated_at: Utc::now().to_rfc3339(),
            wallets: vec![],
        };
        let json_str = serde_json::to_string(&watchlist).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["wallets"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_watchlist_entry_tags_never_null() {
        let entry = WatchlistEntry {
            address: "0xabc".to_string(),
            score: 5.0,
            sharpe: 2.0,
            pnl: 10000.0,
            tags: vec![], // empty, not null
            positions: vec![],
            decaying: false,
        };
        let json_str = serde_json::to_string(&entry).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        // tags must be [], not null
        assert!(parsed["tags"].is_array());
        assert_eq!(parsed["tags"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_watchlist_entry_positions_never_null() {
        let entry = WatchlistEntry {
            address: "0xabc".to_string(),
            score: 5.0,
            sharpe: 2.0,
            pnl: 10000.0,
            tags: vec![],
            positions: vec![],
            decaying: false,
        };
        let json_str = serde_json::to_string(&entry).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed["positions"].is_array());
        assert_eq!(parsed["positions"].as_array().unwrap().len(), 0);
    }

    // =======================================================================
    // Change report
    // =======================================================================

    #[test]
    fn test_change_report_serialization() {
        let report = ChangeReport {
            added: 3,
            removed: 1,
            score_changed: 5,
            details: ChangeReportDetails {
                added_addresses: vec!["0xa".to_string(), "0xb".to_string(), "0xc".to_string()],
                removed_addresses: vec!["0xd".to_string()],
                score_changes: vec![ScoreChange {
                    address: "0xe".to_string(),
                    old_score: 10.0,
                    new_score: 12.0,
                    delta: 2.0,
                }],
            },
        };
        let json_str = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["added"], 3);
        assert_eq!(parsed["removed"], 1);
        assert_eq!(parsed["score_changed"], 5);
    }

    // =======================================================================
    // Atomic file write
    // =======================================================================

    #[test]
    fn test_atomic_write_json() {
        let dir = std::env::temp_dir().join("alpha-scanner-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test-watchlist.json");

        let data = serde_json::json!({
            "generated_at": "2026-05-23T00:00:00Z",
            "wallets": []
        });

        atomic_write_json(&path, &data).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["wallets"].as_array().unwrap().len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write_creates_parent_dir() {
        let dir = std::env::temp_dir().join("alpha-scanner-test-nested").join("a").join("b");
        let path = dir.join("test.json");

        let data = serde_json::json!({"test": true});
        atomic_write_json(&path, &data).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["test"], true);

        let _ = fs::remove_dir_all(std::env::temp_dir().join("alpha-scanner-test-nested"));
    }

    // =======================================================================
    // Load previous watchlist
    // =======================================================================

    #[test]
    fn test_load_previous_watchlist_valid() {
        let dir = std::env::temp_dir().join("alpha-scanner-load-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("prev.json");

        let data = json!({
            "generated_at": "2026-05-23T00:00:00Z",
            "wallets": [
                {
                    "address": "0xabc",
                    "score": 10.0,
                    "sharpe": 2.0,
                    "pnl": 50000.0,
                    "tags": [],
                    "positions": [],
                    "decaying": false
                }
            ]
        });
        fs::write(&path, serde_json::to_string(&data).unwrap()).unwrap();

        let result = load_previous_watchlist(&path);
        assert!(result.is_some());
        let wl = result.unwrap();
        assert_eq!(wl.wallets.len(), 1);
        assert_eq!(wl.wallets[0].address, "0xabc");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_previous_watchlist_missing_file() {
        let result = load_previous_watchlist(&PathBuf::from("/tmp/nonexistent-xyz.json"));
        assert!(result.is_none());
    }

    #[test]
    fn test_load_previous_watchlist_invalid_json() {
        let dir = std::env::temp_dir().join("alpha-scanner-invalid-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("bad.json");
        fs::write(&path, "{broken").unwrap();

        let result = load_previous_watchlist(&path);
        assert!(result.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    // =======================================================================
    // Sorting verification
    // =======================================================================

    #[test]
    fn test_wallets_sorted_descending_by_score() {
        let mut entries = [
            make_entry("0xa", 5.0),
            make_entry("0xb", 15.0),
            make_entry("0xc", 10.0),
        ];
        entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

        assert!((entries[0].score - 15.0).abs() < 0.001);
        assert!((entries[1].score - 10.0).abs() < 0.001);
        assert!((entries[2].score - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_truncate_to_watchlist_size() {
        let mut entries: Vec<WatchlistEntry> = (0..100)
            .map(|i| make_entry(&format!("0x{:02x}", i), (100 - i) as f64))
            .collect();
        entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        entries.truncate(20);

        assert_eq!(entries.len(), 20);
        // First entry should have highest score
        assert!((entries[0].score - 100.0).abs() < 0.001);
        assert!((entries[19].score - 81.0).abs() < 0.001);
    }

    // =======================================================================
    // Integration-style: scoring + decay + sorting pipeline
    // =======================================================================

    #[test]
    fn test_full_scoring_pipeline() {
        let wallets = [
            make_raw_wallet("0x1", 3.0, 100000.0, 0.8),  // high everything
            make_raw_wallet("0x2", 2.0, 50000.0, 0.6),   // medium
            make_raw_wallet("0x3", 1.0, 5000.0, 0.4),    // low
        ];

        let mut entries: Vec<WatchlistEntry> = wallets
            .iter()
            .map(|w| {
                let score = compute_composite_score(w.month_sharpe, w.month_pnl, w.total_win_rate);
                WatchlistEntry {
                    address: w.address.clone(),
                    score,
                    sharpe: w.month_sharpe,
                    pnl: w.month_pnl,
                    tags: vec![],
                    positions: vec![],
                    decaying: detect_decay(w.month_pnl, w.week_pnl),
                }
            })
            .collect();

        entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

        // Verify descending order
        for i in 0..entries.len() - 1 {
            assert!(entries[i].score >= entries[i + 1].score);
        }

        // Verify scores are positive
        for e in &entries {
            assert!(e.score > 0.0);
        }
    }

    #[test]
    fn test_decay_detected_in_pipeline() {
        let mut wallet = make_raw_wallet("0x1", 3.0, 100000.0, 0.8);
        wallet.week_pnl = -5000.0; // Losing recently

        let decaying = detect_decay(wallet.month_pnl, wallet.week_pnl);
        assert!(decaying);
    }

    #[test]
    fn test_no_decay_in_pipeline() {
        let wallet = make_raw_wallet("0x1", 3.0, 100000.0, 0.8);
        let decaying = detect_decay(wallet.month_pnl, wallet.week_pnl);
        assert!(!decaying);
    }

    // =======================================================================
    // json_f64 helper
    // =======================================================================

    #[test]
    fn test_json_f64_number() {
        assert!((json_f64(&json!(3.15)) - 3.15).abs() < 0.001);
    }

    #[test]
    fn test_json_f64_string() {
        assert!((json_f64(&json!("42.5")) - 42.5).abs() < 0.001);
    }

    #[test]
    fn test_json_f64_null() {
        assert_eq!(json_f64(&json!(null)), 0.0);
    }

    #[test]
    fn test_json_f64_invalid_string() {
        assert_eq!(json_f64(&json!("not_a_number")), 0.0);
    }

    // =======================================================================
    // DextrabotResponse parsing
    // =======================================================================

    #[test]
    fn test_dextrabot_response_parse() {
        let raw = json!({
            "count": 5,
            "results": [
                {
                    "user_token": "0xabc",
                    "portfolio_perp_month_sharpe": 2.0,
                    "portfolio_perp_month_pnl": 10000.0,
                }
            ]
        });
        let resp: DextrabotResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(resp.count, Some(5));
        let results = resp.results.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_dextrabot_response_empty() {
        let raw = json!({
            "count": 0,
            "results": []
        });
        let resp: DextrabotResponse = serde_json::from_value(raw).unwrap();
        assert!(resp.results.unwrap().is_empty());
    }

    // ── Test Helpers ──────────────────────────────────────────────────────

    fn make_raw_wallet(
        address: &str,
        month_sharpe: f64,
        month_pnl: f64,
        win_rate: f64,
    ) -> RawWallet {
        RawWallet {
            address: address.to_string(),
            month_sharpe,
            month_pnl,
            week_sharpe: month_sharpe * 0.8,
            week_pnl: month_pnl * 0.2,
            total_win_rate: win_rate,
            long_win_rate: win_rate,
            short_win_rate: win_rate,
            is_scalper: false,
            avg_leverage: 3.0,
        }
    }

    fn make_entry(address: &str, score: f64) -> WatchlistEntry {
        WatchlistEntry {
            address: address.to_string(),
            score,
            sharpe: score * 0.3,
            pnl: score * 1000.0,
            tags: vec![],
            positions: vec![],
            decaying: false,
        }
    }

    fn make_watchlist(entries: Vec<WatchlistEntry>) -> Watchlist {
        Watchlist {
            generated_at: Utc::now().to_rfc3339(),
            wallets: entries,
        }
    }
}
