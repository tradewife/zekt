//! whale-watcher — Real-time whale monitor for Hyperliquid wallets.
//!
//! Connects to `wss://api.hyperliquid.xyz/ws`, subscribes to `userFills`
//! for each watched wallet, detects large position entries (>=$10K notional),
//! emits structured alerts to a JSON-lines file, and tracks alert accuracy
//! with 1-hour follow-up price checks.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin whale-watcher -- --watchlist data/watchlist.json
//! cargo run --bin whale-watcher -- --watchlist data/watchlist.json --min-notional 50000 --output data/whale-alerts.json
//! cargo run --bin whale-watcher -- --help
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream};
use tracing::{debug, error, info, warn};

/// Type alias for the WebSocket stream returned by `connect_async`.
type WsStream = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ── Constants ────────────────────────────────────────────────────────────────

const HL_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const HL_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const DEFAULT_MIN_NOTIONAL: f64 = 10_000.0;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;
const INITIAL_RECONNECT_DELAY_SECS: u64 = 1;

// ── CLI ──────────────────────────────────────────────────────────────────────

fn validate_min_notional(v: &str) -> Result<f64, String> {
    let val: f64 = v
        .parse()
        .map_err(|_| format!("invalid min-notional value: {}", v))?;
    if val <= 0.0 {
        return Err(format!("min-notional must be > 0, got {}", val));
    }
    Ok(val)
}

#[derive(Parser, Debug)]
#[command(
    name = "whale-watcher",
    about = "Monitor watched wallets for large position entries via Hyperliquid WebSocket",
    version
)]
struct Args {
    /// Path to watchlist JSON file (from alpha-scanner).
    #[arg(long, default_value = "data/watchlist.json")]
    watchlist: PathBuf,

    /// Minimum notional (USD) to trigger an alert.
    #[arg(
        long,
        default_value_t = DEFAULT_MIN_NOTIONAL,
        value_parser = validate_min_notional
    )]
    min_notional: f64,

    /// Output path for whale alerts (JSON lines).
    #[arg(long, default_value = "data/whale-alerts.json")]
    output: PathBuf,

    /// Path for accuracy tracking records (JSON lines).
    #[arg(long, default_value = "data/whale-accuracy.json")]
    accuracy_output: PathBuf,
}

impl Args {
    fn validate(&self) -> Result<()> {
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

/// Wallet entry from the alpha-scanner watchlist JSON.
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
    pub positions: Vec<serde_json::Value>,
    #[serde(default)]
    pub decaying: bool,
}

/// Top-level watchlist file format from alpha-scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchlist {
    #[serde(default)]
    pub generated_at: String,
    pub wallets: Vec<WalletEntry>,
}

/// A parsed fill event extracted from WebSocket messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillEvent {
    pub coin: String,
    pub side: String,
    pub px: String,
    pub sz: String,
    pub time: i64,
    pub dir: String,
    pub hash: String,
    #[serde(default)]
    pub start_position: Option<String>,
    #[serde(default)]
    pub closed_pnl: Option<String>,
    #[serde(default)]
    pub fee: Option<String>,
}

impl FillEvent {
    /// Parse fill price as f64.
    pub fn price_f64(&self) -> f64 {
        self.px.parse().unwrap_or(0.0)
    }

    /// Parse fill size as f64.
    pub fn size_f64(&self) -> f64 {
        self.sz.parse().unwrap_or(0.0)
    }

    /// Compute notional value (size × price).
    pub fn notional(&self) -> f64 {
        self.size_f64().abs() * self.price_f64()
    }

    /// Determine if this is an opening position (new entry).
    pub fn is_open(&self) -> bool {
        self.dir.starts_with("Open")
    }

    /// Determine the normalized side: "buy" or "sell".
    pub fn side_normalized(&self) -> &str {
        match self.side.as_str() {
            "B" => "buy",
            "A" => "sell",
            other => other,
        }
    }
}

/// A whale alert emitted for large position entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleAlert {
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Wallet address that triggered the fill.
    pub wallet: String,
    /// Market coin (e.g. "BTC").
    pub coin: String,
    /// Normalized side: "buy" or "sell".
    pub side: String,
    /// Fill size (absolute value).
    pub size: f64,
    /// Fill price.
    pub price: f64,
    /// Notional value in USD.
    pub notional_usd: f64,
    /// Unique alert identifier.
    pub alert_id: String,
    /// Direction string from the fill (e.g. "Open Long").
    pub direction: String,
}

/// Accuracy record for a 1-hour follow-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccuracyRecord {
    /// ID of the original alert.
    pub alert_id: String,
    /// Wallet address.
    pub wallet: String,
    /// Market coin.
    pub coin: String,
    /// Original alert timestamp.
    pub alert_timestamp: String,
    /// Original entry price.
    pub entry_price: f64,
    /// Original side ("buy" or "sell").
    pub side: String,
    /// Price at follow-up time (1h later).
    pub followup_price: f64,
    /// Whether the price moved in the whale's direction.
    pub direction_correct: bool,
    /// Price change percentage.
    pub price_change_pct: f64,
    /// Follow-up timestamp.
    pub followup_timestamp: String,
}

/// WebSocket subscription message.
#[derive(Debug, Serialize)]
struct SubscribeMsg {
    method: String,
    subscription: Subscription,
}

#[derive(Debug, Serialize)]
struct Subscription {
    #[serde(rename = "type")]
    sub_type: String,
    user: String,
}

// ── Watchlist Loading ────────────────────────────────────────────────────────

/// Load and deduplicate wallet addresses from the watchlist file.
///
/// Accepts two formats:
/// 1. Alpha-scanner format: `{"generated_at": "...", "wallets": [...]}`
/// 2. Simple array format: `[{"address": "...", ...}, ...]`
pub fn load_watchlist(path: &PathBuf) -> Result<Vec<WalletEntry>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read watchlist file: {}", path.display()))?;

    // Try alpha-scanner format first
    if let Ok(watchlist) = serde_json::from_str::<Watchlist>(&content) {
        let wallets: Vec<WalletEntry> = watchlist
            .wallets
            .into_iter()
            .filter(|w| !w.address.trim().is_empty())
            .collect();
        return Ok(deduplicate_wallets(wallets));
    }

    // Try simple array format
    if let Ok(wallets) = serde_json::from_str::<Vec<WalletEntry>>(&content) {
        let wallets: Vec<WalletEntry> = wallets
            .into_iter()
            .filter(|w| !w.address.trim().is_empty())
            .collect();
        return Ok(deduplicate_wallets(wallets));
    }

    anyhow::bail!("failed to parse watchlist file: {}", path.display())
}

/// Remove duplicate wallet addresses, keeping the first occurrence.
fn deduplicate_wallets(wallets: Vec<WalletEntry>) -> Vec<WalletEntry> {
    let mut seen = HashSet::new();
    wallets
        .into_iter()
        .filter(|w| {
            let addr = w.address.to_lowercase();
            seen.insert(addr)
        })
        .collect()
}

/// Extract unique wallet addresses from the watchlist.
pub fn extract_addresses(wallets: &[WalletEntry]) -> Vec<String> {
    wallets.iter().map(|w| w.address.clone()).collect()
}

// ── Alert File Operations ────────────────────────────────────────────────────

/// Append a JSON-line alert to the output file.
pub fn append_alert(path: &PathBuf, alert: &WhaleAlert) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open alert file: {}", path.display()))?;

    let mut line = serde_json::to_string(alert).context("failed to serialize alert")?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .with_context(|| format!("failed to write alert to {}", path.display()))?;

    file.flush()
        .with_context(|| format!("failed to flush alert file: {}", path.display()))?;

    Ok(())
}

/// Append a JSON-line accuracy record to the accuracy file.
pub fn append_accuracy_record(path: &PathBuf, record: &AccuracyRecord) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open accuracy file: {}", path.display()))?;

    let mut line = serde_json::to_string(record).context("failed to serialize accuracy record")?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .with_context(|| format!("failed to write accuracy record to {}", path.display()))?;

    Ok(())
}

// ── Fill Processing ──────────────────────────────────────────────────────────

/// Process a raw WebSocket text message, extracting fill events.
/// Returns a vector of (wallet_address, fill_events) pairs.
pub fn parse_ws_message(msg: &str) -> Option<Vec<(String, Vec<FillEvent>)>> {
    let raw: serde_json::Value = serde_json::from_str(msg).ok()?;

    // Check if this is a userFills channel message
    if raw.get("channel").and_then(|v| v.as_str()) != Some("userFills") {
        return None;
    }

    let data = raw.get("data")?;

    // The data may contain a nested structure or flat fills array
    // Hyperliquid format: {"channel":"userFills","data":{"fills":[...]}}
    // Also handle: {"channel":"userFills","data":[...]} (flat array)

    let fills: Vec<FillEvent> = if let Some(fills_arr) = data.get("fills") {
        serde_json::from_value(fills_arr.clone()).ok()?
    } else if data.is_array() {
        serde_json::from_value(data.clone()).ok()?
    } else {
        return None;
    };

    // Try to extract wallet address from the message
    // Hyperliquid includes the user in subscription responses but not always in data
    // We may need to track subscription → wallet mapping externally
    let wallet = data
        .get("user")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Some(vec![(wallet, fills)])
}

/// Process a fill event and decide whether to generate an alert.
pub fn process_fill(
    fill: &FillEvent,
    wallet: &str,
    min_notional: f64,
    seen_fills: &mut HashSet<String>,
) -> Option<WhaleAlert> {
    // Deduplicate by hash
    if !seen_fills.insert(fill.hash.clone()) {
        return None;
    }

    let notional = fill.notional();

    // Filter by minimum notional
    if notional < min_notional {
        debug!(
            coin = %fill.coin,
            notional = notional,
            threshold = min_notional,
            "Fill below threshold — skipping"
        );
        return None;
    }

    Some(WhaleAlert {
        timestamp: Utc::now().to_rfc3339(),
        wallet: wallet.to_string(),
        coin: fill.coin.clone(),
        side: fill.side_normalized().to_string(),
        size: fill.size_f64().abs(),
        price: fill.price_f64(),
        notional_usd: notional,
        alert_id: generate_alert_id(),
        direction: fill.dir.clone(),
    })
}

/// Generate a unique alert ID.
pub fn generate_alert_id() -> String {
    format!(
        "ww-{}-{:04x}",
        Utc::now().timestamp_millis(),
        rand::random::<u16>()
    )
}

// ── Accuracy Tracking ────────────────────────────────────────────────────────

/// Fetch the current price of a coin from Hyperliquid Info API.
pub async fn fetch_current_price(coin: &str) -> Result<f64> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let body = serde_json::json!({
        "type": "allMids"
    });

    let resp = client
        .post(HL_INFO_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .with_context(|| "failed to fetch allMids from HL API")?;

    if !resp.status().is_success() {
        anyhow::bail!("HL allMids returned status {}", resp.status());
    }

    let mids: HashMap<String, String> = resp.json().await?;
    let price_str = mids
        .get(coin)
        .ok_or_else(|| anyhow::anyhow!("coin {} not found in allMids response", coin))?;

    price_str
        .parse::<f64>()
        .with_context(|| format!("failed to parse price for {}: {}", coin, price_str))
}

/// Determine whether the price moved in the whale's predicted direction.
pub fn check_direction_correct(entry_price: f64, current_price: f64, side: &str) -> bool {
    if entry_price <= 0.0 {
        return false;
    }
    match side {
        "buy" => current_price > entry_price,
        "sell" => current_price < entry_price,
        _ => false,
    }
}

/// Compute the price change percentage from entry to current.
pub fn price_change_pct(entry_price: f64, current_price: f64) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }
    (current_price - entry_price) / entry_price * 100.0
}

/// Spawn a background task to check alert accuracy after a delay.
pub fn spawn_accuracy_check(
    alert: WhaleAlert,
    accuracy_output: PathBuf,
    delay: Duration,
    running: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        // Wait for the configured delay (1 hour by default)
        let sleep_steps = delay.as_secs() / 10;
        for _ in 0..sleep_steps {
            if !running.load(AtomicOrdering::SeqCst) {
                debug!(alert_id = %alert.alert_id, "Accuracy check cancelled — shutting down");
                return;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }

        // Fetch current price
        match fetch_current_price(&alert.coin).await {
            Ok(current_price) => {
                let direction_correct =
                    check_direction_correct(alert.price, current_price, &alert.side);
                let change_pct = price_change_pct(alert.price, current_price);

                let record = AccuracyRecord {
                    alert_id: alert.alert_id.clone(),
                    wallet: alert.wallet.clone(),
                    coin: alert.coin.clone(),
                    alert_timestamp: alert.timestamp.clone(),
                    entry_price: alert.price,
                    side: alert.side.clone(),
                    followup_price: current_price,
                    direction_correct,
                    price_change_pct: change_pct,
                    followup_timestamp: Utc::now().to_rfc3339(),
                };

                info!(
                    alert_id = %record.alert_id,
                    coin = %record.coin,
                    direction_correct = direction_correct,
                    price_change_pct = format!("{:.2}%", change_pct),
                    "Accuracy follow-up completed"
                );

                if let Err(e) = append_accuracy_record(&accuracy_output, &record) {
                    error!(error = %e, "Failed to write accuracy record");
                }
            }
            Err(e) => {
                warn!(
                    alert_id = %alert.alert_id,
                    coin = %alert.coin,
                    error = %e,
                    "Failed to fetch price for accuracy check"
                );
            }
        }
    });
}

// ── WebSocket Operations ─────────────────────────────────────────────────────

/// Build a subscribe message for a wallet.
pub fn build_subscribe_msg(wallet: &str) -> String {
    let msg = SubscribeMsg {
        method: "subscribe".to_string(),
        subscription: Subscription {
            sub_type: "userFills".to_string(),
            user: wallet.to_string(),
        },
    };
    serde_json::to_string(&msg).expect("subscribe message serialization should not fail")
}

/// Connect to Hyperliquid WebSocket with retry and exponential backoff.
pub async fn connect_with_retry(
    max_retries: u32,
) -> Result<WsStream> {
    let mut attempt = 0u32;
    let mut delay_secs = INITIAL_RECONNECT_DELAY_SECS;

    loop {
        attempt += 1;
        info!(
            attempt = attempt,
            max_retries = max_retries,
            url = HL_WS_URL,
            "Connecting to Hyperliquid WebSocket"
        );

        match connect_async(HL_WS_URL).await {
            Ok((ws_stream, _response)) => {
                info!("WebSocket connection established");
                return Ok(ws_stream);
            }
            Err(e) => {
                let is_last = attempt >= max_retries;
                let level = if is_last {
                    tracing::Level::ERROR
                } else {
                    tracing::Level::WARN
                };

                if level == tracing::Level::ERROR {
                    error!(error = %e, attempt = attempt, "WebSocket connection failed");
                } else {
                    warn!(
                        error = %e,
                        attempt = attempt,
                        retry_in_secs = delay_secs,
                        "WebSocket connection failed — retrying"
                    );
                }

                if is_last {
                    anyhow::bail!(
                        "failed to connect to WebSocket after {} attempts: {}",
                        attempt,
                        e
                    );
                }

                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

/// Send subscribe messages for all wallets.
pub async fn subscribe_wallets(
    ws: &mut WsStream,
    addresses: &[String],
) -> Result<usize> {
    let mut count = 0;
    for addr in addresses {
        let msg = build_subscribe_msg(addr);
        ws.send(Message::Text(msg.into()))
            .await
            .with_context(|| format!("failed to subscribe for wallet {}", &addr[..addr.len().min(12)]))?;
        count += 1;
        debug!(wallet = &addr[..addr.len().min(12)], "Subscribed to userFills");
    }
    info!(subscription_count = count, "Subscribed to all wallet fill streams");
    Ok(count)
}

// ── Alert ID Generation for Tests ───────────────────────────────────────────

/// Generate a deterministic alert ID for testing.
pub fn generate_alert_id_with_seed(ts_ms: i64, seed: u16) -> String {
    format!("ww-{}-{:04x}", ts_ms, seed)
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    args.validate()?;

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .init();

    info!("=== whale-watcher ===");
    info!(
        watchlist = %args.watchlist.display(),
        min_notional = args.min_notional,
        output = %args.output.display(),
        accuracy_output = %args.accuracy_output.display(),
        "Configuration"
    );

    // Load watchlist
    let wallets = load_watchlist(&args.watchlist)?;
    if wallets.is_empty() {
        info!("no wallets to monitor — exiting");
        return Ok(());
    }

    let addresses = extract_addresses(&wallets);
    info!(
        total_wallets = wallets.len(),
        unique_addresses = addresses.len(),
        "Loaded watchlist"
    );

    // Set up graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        info!("Received SIGINT, shutting down...");
        r.store(false, AtomicOrdering::SeqCst);
    })
    .context("Failed to set ctrlc handler")?;

    // Deduplication state for fill hashes
    let seen_fills = Arc::new(tokio::sync::Mutex::new(HashSet::<String>::new()));

    // Main WebSocket loop with reconnection
    loop {
        if !running.load(AtomicOrdering::SeqCst) {
            info!("Shutting down...");
            break;
        }

        // Connect with retry
        let mut ws = match connect_with_retry(10).await {
            Ok(ws) => ws,
            Err(e) => {
                error!(error = %e, "Failed to establish WebSocket connection");
                if !running.load(AtomicOrdering::SeqCst) {
                    break;
                }
                warn!("Will retry connection in 30s...");
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        // Subscribe to all wallets
        if let Err(e) = subscribe_wallets(&mut ws, &addresses).await {
            error!(error = %e, "Failed to subscribe to wallets");
            continue;
        }

        // Message processing loop
        loop {
            if !running.load(AtomicOrdering::SeqCst) {
                break;
            }

            let msg = tokio::select! {
                msg = ws.next() => msg,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    // Periodic check of running flag
                    continue;
                }
            };

            match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Some(wallet_fills) = parse_ws_message(&text) {
                        for (wallet, fills) in wallet_fills {
                            let mut seen = seen_fills.lock().await;
                            for fill in fills {
                                if let Some(alert) =
                                    process_fill(&fill, &wallet, args.min_notional, &mut seen)
                                {
                                    info!(
                                        alert_id = %alert.alert_id,
                                        wallet = &alert.wallet[..alert.wallet.len().min(12)],
                                        coin = %alert.coin,
                                        side = %alert.side,
                                        notional_usd = alert.notional_usd,
                                        "Whale alert triggered"
                                    );

                                    // Write alert to file
                                    if let Err(e) = append_alert(&args.output, &alert) {
                                        error!(error = %e, "Failed to write alert");
                                    }

                                    // Spawn accuracy check (1 hour)
                                    spawn_accuracy_check(
                                        alert,
                                        args.accuracy_output.clone(),
                                        Duration::from_secs(3600),
                                        running.clone(),
                                    );
                                }
                            }
                        }
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    debug!("Received ping, sending pong");
                    if let Err(e) = ws.send(Message::Pong(data)).await {
                        warn!(error = %e, "Failed to send pong");
                    }
                }
                Some(Ok(Message::Close(reason))) => {
                    warn!(?reason, "WebSocket closed by server");
                    break;
                }
                Some(Err(e)) => {
                    warn!(error = %e, "WebSocket read error");
                    break;
                }
                None => {
                    warn!("WebSocket stream ended — will reconnect");
                    break;
                }
                _ => {
                    // Ignore binary, pong, and frame messages
                }
            }
        }

        // If we exited the inner loop but running is still true, reconnect
        if running.load(AtomicOrdering::SeqCst) {
            warn!("WebSocket disconnected — reconnecting in 5s...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    info!("=== Whale Watcher Shutdown ===");
    info!(
        alerts_file = %args.output.display(),
        accuracy_file = %args.accuracy_output.display(),
        "Output files"
    );

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    // Helper: write content to a temp file
    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    // ── CLI Validation (3 tests) ────────────────────────────────────────

    #[test]
    fn test_validate_min_notional_valid() {
        assert!(validate_min_notional("10000").is_ok());
        assert!(validate_min_notional("50000.5").is_ok());
        assert!(validate_min_notional("0.01").is_ok());
    }

    #[test]
    fn test_validate_min_notional_negative_rejected() {
        assert!(validate_min_notional("-1.0").is_err());
    }

    #[test]
    fn test_validate_min_notional_zero_rejected() {
        assert!(validate_min_notional("0.0").is_err());
    }

    #[test]
    fn test_validate_min_notional_non_numeric() {
        assert!(validate_min_notional("abc").is_err());
    }

    // ── Watchlist Loading (6 tests) ─────────────────────────────────────

    #[test]
    fn test_load_watchlist_alpha_format() {
        let content = json!({
            "generated_at": "2026-05-23T00:00:00Z",
            "wallets": [
                {"address": "0xaaa", "score": 10.0, "sharpe": 2.0, "pnl": 50000.0,
                 "tags": ["whale"], "positions": [], "decaying": false},
                {"address": "0xbbb", "score": 8.0, "sharpe": 1.5, "pnl": 30000.0,
                 "tags": [], "positions": [], "decaying": true}
            ]
        });
        let f = write_temp(&serde_json::to_string(&content).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].address, "0xaaa");
        assert!(!w[0].decaying);
        assert!(w[1].decaying);
    }

    #[test]
    fn test_load_watchlist_array_format() {
        let content = json!([
            {"address": "0xaaa", "score": 5.0},
            {"address": "0xbbb", "score": 3.0}
        ]);
        let f = write_temp(&serde_json::to_string(&content).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn test_load_watchlist_empty_wallets() {
        let content = json!({"generated_at": "t", "wallets": []});
        let f = write_temp(&serde_json::to_string(&content).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        assert!(w.is_empty());
    }

    #[test]
    fn test_load_watchlist_missing_file() {
        assert!(load_watchlist(&PathBuf::from("/tmp/no-such-whale-xyz.json")).is_err());
    }

    #[test]
    fn test_load_watchlist_invalid_json() {
        let f = write_temp("{broken");
        assert!(load_watchlist(&f.path().to_path_buf()).is_err());
    }

    #[test]
    fn test_load_watchlist_deduplication() {
        let content = json!([
            {"address": "0xAaA", "score": 10.0},
            {"address": "0xaaa", "score": 5.0},
            {"address": "0xbbb", "score": 3.0}
        ]);
        let f = write_temp(&serde_json::to_string(&content).unwrap());
        let w = load_watchlist(&f.path().to_path_buf()).unwrap();
        // Case-insensitive dedup: "0xAaA" and "0xaaa" are the same
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].address, "0xAaA");
        assert_eq!(w[0].score, 10.0); // first occurrence kept
    }

    // ── Fill Parsing (5 tests) ──────────────────────────────────────────

    #[test]
    fn test_fill_event_parse() {
        let fill: FillEvent = serde_json::from_value(json!({
            "coin": "BTC",
            "side": "B",
            "px": "105000.0",
            "sz": "0.5",
            "time": 1719403211633_i64,
            "dir": "Open Long",
            "hash": "0xabc123",
            "startPosition": "0.0",
            "closedPnl": "0.0",
            "fee": "1.05"
        }))
        .unwrap();

        assert_eq!(fill.coin, "BTC");
        assert_eq!(fill.side, "B");
        assert!((fill.price_f64() - 105000.0).abs() < 0.01);
        assert!((fill.size_f64() - 0.5).abs() < 0.001);
        assert!((fill.notional() - 52500.0).abs() < 0.01);
        assert!(fill.is_open());
        assert_eq!(fill.side_normalized(), "buy");
    }

    #[test]
    fn test_fill_event_sell_side() {
        let fill: FillEvent = serde_json::from_value(json!({
            "coin": "ETH", "side": "A", "px": "3000.0", "sz": "10.0",
            "time": 1234_i64, "dir": "Close Long", "hash": "0xdef"
        }))
        .unwrap();

        assert_eq!(fill.side_normalized(), "sell");
        assert!(!fill.is_open());
    }

    #[test]
    fn test_fill_notional_calculation() {
        let fill: FillEvent = serde_json::from_value(json!({
            "coin": "SOL", "side": "B", "px": "150.0", "sz": "100.0",
            "time": 0_i64, "dir": "Open Long", "hash": "0x1"
        }))
        .unwrap();
        assert!((fill.notional() - 15000.0).abs() < 0.01);
    }

    #[test]
    fn test_fill_zero_size() {
        let fill: FillEvent = serde_json::from_value(json!({
            "coin": "BTC", "side": "B", "px": "100.0", "sz": "0.0",
            "time": 0_i64, "dir": "Open Long", "hash": "0x2"
        }))
        .unwrap();
        assert!((fill.notional() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_fill_short_direction() {
        let fill: FillEvent = serde_json::from_value(json!({
            "coin": "BTC", "side": "A", "px": "60000.0", "sz": "1.0",
            "time": 0_i64, "dir": "Open Short", "hash": "0x3"
        }))
        .unwrap();
        assert!(fill.is_open());
        assert_eq!(fill.side_normalized(), "sell");
        assert!((fill.notional() - 60000.0).abs() < 0.01);
    }

    // ── Threshold Filtering (4 tests) ───────────────────────────────────

    #[test]
    fn test_process_fill_above_threshold() {
        let fill = FillEvent {
            coin: "BTC".into(),
            side: "B".into(),
            px: "60000.0".into(),
            sz: "1.0".into(),
            time: 0,
            dir: "Open Long".into(),
            hash: "hash1".into(),
            start_position: None,
            closed_pnl: None,
            fee: None,
        };
        let mut seen = HashSet::new();
        let alert = process_fill(&fill, "0xwallet", 10000.0, &mut seen);
        assert!(alert.is_some());
        let a = alert.unwrap();
        assert!((a.notional_usd - 60000.0).abs() < 0.01);
        assert_eq!(a.coin, "BTC");
        assert_eq!(a.side, "buy");
    }

    #[test]
    fn test_process_fill_below_threshold() {
        let fill = FillEvent {
            coin: "BTC".into(),
            side: "B".into(),
            px: "60000.0".into(),
            sz: "0.1".into(),
            time: 0,
            dir: "Open Long".into(),
            hash: "hash2".into(),
            start_position: None,
            closed_pnl: None,
            fee: None,
        };
        let mut seen = HashSet::new();
        let alert = process_fill(&fill, "0xwallet", 10000.0, &mut seen);
        assert!(alert.is_none());
    }

    #[test]
    fn test_process_fill_exact_threshold() {
        let fill = FillEvent {
            coin: "BTC".into(),
            side: "B".into(),
            px: "10000.0".into(),
            sz: "1.0".into(),
            time: 0,
            dir: "Open Long".into(),
            hash: "hash3".into(),
            start_position: None,
            closed_pnl: None,
            fee: None,
        };
        let mut seen = HashSet::new();
        let alert = process_fill(&fill, "0xwallet", 10000.0, &mut seen);
        assert!(alert.is_some());
    }

    #[test]
    fn test_process_fill_deduplication() {
        let fill = FillEvent {
            coin: "BTC".into(),
            side: "B".into(),
            px: "60000.0".into(),
            sz: "1.0".into(),
            time: 0,
            dir: "Open Long".into(),
            hash: "hash4".into(),
            start_position: None,
            closed_pnl: None,
            fee: None,
        };
        let mut seen = HashSet::new();
        let a1 = process_fill(&fill, "0xwallet", 10000.0, &mut seen);
        let a2 = process_fill(&fill, "0xwallet", 10000.0, &mut seen);
        assert!(a1.is_some());
        assert!(a2.is_none()); // duplicate hash
    }

    // ── Alert Format (3 tests) ──────────────────────────────────────────

    #[test]
    fn test_alert_required_fields() {
        let alert = WhaleAlert {
            timestamp: "2026-05-23T00:00:00Z".into(),
            wallet: "0xabc".into(),
            coin: "BTC".into(),
            side: "buy".into(),
            size: 1.0,
            price: 60000.0,
            notional_usd: 60000.0,
            alert_id: "ww-123-0001".into(),
            direction: "Open Long".into(),
        };
        let v = serde_json::to_value(&alert).unwrap();
        for field in &[
            "timestamp",
            "wallet",
            "coin",
            "side",
            "size",
            "price",
            "notional_usd",
            "alert_id",
        ] {
            assert!(v.get(*field).is_some(), "missing field: {}", field);
        }
    }

    #[test]
    fn test_alert_json_line_format() {
        let dir = std::env::temp_dir().join("ww-test-alert-fmt");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("alerts.json");

        let alert = WhaleAlert {
            timestamp: "2026-05-23T00:00:00Z".into(),
            wallet: "0xabc".into(),
            coin: "BTC".into(),
            side: "buy".into(),
            size: 1.0,
            price: 60000.0,
            notional_usd: 60000.0,
            alert_id: "ww-123-0001".into(),
            direction: "Open Long".into(),
        };

        append_alert(&path, &alert).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["alert_id"], "ww-123-0001");
        assert_eq!(parsed["coin"], "BTC");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_alert_append_multiple() {
        let dir = std::env::temp_dir().join("ww-test-alert-append");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("alerts.json");

        for i in 0..3 {
            let alert = WhaleAlert {
                timestamp: format!("2026-05-23T0{}:00:00Z", i),
                wallet: "0xabc".into(),
                coin: "BTC".into(),
                side: "buy".into(),
                size: 1.0,
                price: 60000.0,
                notional_usd: 60000.0,
                alert_id: format!("ww-{}-{:04x}", 1000 + i, i),
                direction: "Open Long".into(),
            };
            append_alert(&path, &alert).unwrap();
        }

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);

        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.get("alert_id").is_some());
        }

        let _ = fs::remove_dir_all(&dir);
    }

    // ── Accuracy Tracking (4 tests) ─────────────────────────────────────

    #[test]
    fn test_direction_correct_buy_price_up() {
        assert!(check_direction_correct(100.0, 110.0, "buy"));
    }

    #[test]
    fn test_direction_correct_buy_price_down() {
        assert!(!check_direction_correct(100.0, 90.0, "buy"));
    }

    #[test]
    fn test_direction_correct_sell_price_down() {
        assert!(check_direction_correct(100.0, 90.0, "sell"));
    }

    #[test]
    fn test_direction_correct_sell_price_up() {
        assert!(!check_direction_correct(100.0, 110.0, "sell"));
    }

    #[test]
    fn test_price_change_pct_positive() {
        assert!((price_change_pct(100.0, 110.0) - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_price_change_pct_negative() {
        assert!((price_change_pct(100.0, 90.0) - (-10.0)).abs() < 0.01);
    }

    #[test]
    fn test_price_change_pct_zero_entry() {
        assert!((price_change_pct(0.0, 100.0)).abs() < 0.01);
    }

    // ── Accuracy Record Format (2 tests) ────────────────────────────────

    #[test]
    fn test_accuracy_record_serialization() {
        let record = AccuracyRecord {
            alert_id: "ww-123-0001".into(),
            wallet: "0xabc".into(),
            coin: "BTC".into(),
            alert_timestamp: "2026-05-23T00:00:00Z".into(),
            entry_price: 60000.0,
            side: "buy".into(),
            followup_price: 63000.0,
            direction_correct: true,
            price_change_pct: 5.0,
            followup_timestamp: "2026-05-23T01:00:00Z".into(),
        };
        let v = serde_json::to_value(&record).unwrap();
        assert_eq!(v["direction_correct"], true);
        assert_eq!(v["alert_id"], "ww-123-0001");
    }

    #[test]
    fn test_accuracy_record_file_append() {
        let dir = std::env::temp_dir().join("ww-test-acc");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("accuracy.json");

        let record = AccuracyRecord {
            alert_id: "ww-123-0001".into(),
            wallet: "0xabc".into(),
            coin: "BTC".into(),
            alert_timestamp: "2026-05-23T00:00:00Z".into(),
            entry_price: 60000.0,
            side: "buy".into(),
            followup_price: 63000.0,
            direction_correct: true,
            price_change_pct: 5.0,
            followup_timestamp: "2026-05-23T01:00:00Z".into(),
        };

        append_accuracy_record(&path, &record).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["direction_correct"], true);

        let _ = fs::remove_dir_all(&dir);
    }

    // ── WebSocket Message Parsing (3 tests) ─────────────────────────────

    #[test]
    fn test_parse_ws_message_valid() {
        let msg = json!({
            "channel": "userFills",
            "data": {
                "fills": [
                    {
                        "coin": "BTC",
                        "side": "B",
                        "px": "105000.0",
                        "sz": "0.5",
                        "time": 1719403211633_i64,
                        "dir": "Open Long",
                        "hash": "0xabc",
                        "startPosition": "0.0"
                    }
                ]
            }
        })
        .to_string();

        let result = parse_ws_message(&msg).unwrap();
        assert_eq!(result.len(), 1);
        let (_wallet, fills) = &result[0];
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].coin, "BTC");
    }

    #[test]
    fn test_parse_ws_message_wrong_channel() {
        let msg = json!({"channel": "l2Book", "data": {}}).to_string();
        assert!(parse_ws_message(&msg).is_none());
    }

    #[test]
    fn test_parse_ws_message_invalid_json() {
        assert!(parse_ws_message("not json").is_none());
    }

    // ── Subscribe Message (2 tests) ─────────────────────────────────────

    #[test]
    fn test_build_subscribe_msg() {
        let msg = build_subscribe_msg("0xabc123");
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["method"], "subscribe");
        assert_eq!(parsed["subscription"]["type"], "userFills");
        assert_eq!(parsed["subscription"]["user"], "0xabc123");
    }

    #[test]
    fn test_build_subscribe_msg_format() {
        let msg = build_subscribe_msg("0xdef");
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert!(parsed.is_object());
        assert_eq!(parsed["method"], "subscribe");
    }

    // ── Deduplication (2 tests) ─────────────────────────────────────────

    #[test]
    fn test_deduplicate_case_insensitive() {
        let wallets = vec![
            WalletEntry {
                address: "0xAaA".into(),
                score: 1.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
            WalletEntry {
                address: "0xaaa".into(),
                score: 2.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
            WalletEntry {
                address: "0xBBB".into(),
                score: 3.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
        ];
        let deduped = deduplicate_wallets(wallets);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].score, 1.0); // first occurrence
    }

    #[test]
    fn test_deduplicate_no_duplicates() {
        let wallets = vec![
            WalletEntry {
                address: "0xaaa".into(),
                score: 1.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
            WalletEntry {
                address: "0xbbb".into(),
                score: 2.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
        ];
        let deduped = deduplicate_wallets(wallets);
        assert_eq!(deduped.len(), 2);
    }

    // ── Alert ID Generation (1 test) ────────────────────────────────────

    #[test]
    fn test_generate_alert_id_format() {
        let id = generate_alert_id();
        assert!(id.starts_with("ww-"));
        let parts: Vec<&str> = id.split('-').collect();
        assert!(parts.len() >= 3);
    }

    // ── Integration: Full Fill → Alert Pipeline (1 test) ────────────────

    #[test]
    fn test_full_fill_to_alert_pipeline() {
        // Simulate a large fill event coming through WebSocket
        let ws_msg = json!({
            "channel": "userFills",
            "data": {
                "fills": [
                    {
                        "coin": "BTC",
                        "side": "B",
                        "px": "105000.0",
                        "sz": "1.0",
                        "time": 1719403211633_i64,
                        "dir": "Open Long",
                        "hash": "unique-hash-1",
                        "startPosition": "0.0"
                    }
                ]
            }
        })
        .to_string();

        // Parse WebSocket message
        let parsed = parse_ws_message(&ws_msg).expect("should parse WS message");
        assert_eq!(parsed.len(), 1);
        let (wallet, fills) = &parsed[0];
        assert_eq!(fills.len(), 1);

        // Process fill with $10K threshold
        let mut seen = HashSet::new();
        let alert = process_fill(&fills[0], wallet, 10000.0, &mut seen)
            .expect("should generate alert for $105K fill");

        // Verify alert fields
        assert_eq!(alert.coin, "BTC");
        assert_eq!(alert.side, "buy");
        assert!((alert.notional_usd - 105000.0).abs() < 0.01);
        assert!((alert.size - 1.0).abs() < 0.001);
        assert!((alert.price - 105000.0).abs() < 0.01);
        assert!(alert.alert_id.starts_with("ww-"));
        assert_eq!(alert.direction, "Open Long");
        assert!(!alert.timestamp.is_empty());
        assert!(!alert.wallet.is_empty());

        // Verify JSON serialization
        let json = serde_json::to_value(&alert).unwrap();
        for field in &[
            "timestamp",
            "wallet",
            "coin",
            "side",
            "size",
            "price",
            "notional_usd",
            "alert_id",
        ] {
            assert!(json.get(*field).is_some(), "missing: {}", field);
        }
    }

    // ── Extract Addresses (1 test) ──────────────────────────────────────

    #[test]
    fn test_extract_addresses() {
        let wallets = vec![
            WalletEntry {
                address: "0xaaa".into(),
                score: 1.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
            WalletEntry {
                address: "0xbbb".into(),
                score: 2.0,
                sharpe: 0.0,
                pnl: 0.0,
                tags: vec![],
                positions: vec![],
                decaying: false,
            },
        ];
        let addrs = extract_addresses(&wallets);
        assert_eq!(addrs, vec!["0xaaa", "0xbbb"]);
    }
}
