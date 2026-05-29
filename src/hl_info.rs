//! Hyperliquid Info API client module.
//!
//! REST client for the Hyperliquid Info API (`POST https://api.hyperliquid.xyz/info`).
//! Provides methods for querying wallet positions, funding rates, fill history,
//! and market contexts. Follows the `reqwest` patterns established in
//! `flash_api.rs` and the `HlCandleFetcher` in `backtest.rs`.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default base URL for the Hyperliquid Info API.
pub const HL_INFO_URL: &str = "https://api.hyperliquid.xyz/info";

/// Hardcoded IP addresses for `api.hyperliquid.xyz` used as DNS fallback.
///
/// When DNS resolution fails intermittently, the client retries each IP in
/// order via `reqwest::ClientBuilder::resolve()`, which pins the hostname to
/// the IP while preserving TLS SNI (no certificate mismatch).
///
/// Last verified: 2026-05-30.
const HL_API_FALLBACK_IPS: &[&str] = &[
    "108.158.20.109",
    "108.158.20.70",
    "108.158.20.106",
    "108.158.20.67",
];

/// Check if an `anyhow::Error` chain contains a reqwest connect/DNS error.
///
/// Connect errors include DNS resolution failures, TCP connection refused,
/// and TLS handshake failures. Retrying via a fallback IP is appropriate for
/// all of these.
fn is_connect_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .map(|r| r.is_connect())
            .unwrap_or(false)
    })
}

/// Extract the hostname from a URL string (e.g. `"https://api.hyperliquid.xyz/info"`
/// → `"api.hyperliquid.xyz"`).
fn extract_host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split(':').next())
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// REST client for the Hyperliquid Info API.
///
/// All methods issue `POST` requests with a JSON body `{"type": "...", ...}`.
/// A 30-second timeout is applied to every request. Uses `tracing` for logging
/// and `anyhow::Result` for error propagation.
#[derive(Debug, Clone)]
pub struct HlInfoClient {
    client: Client,
    base_url: String,
}

impl HlInfoClient {
    /// Create a new client pointed at the given base URL.
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client for HlInfoClient");
        Self {
            client,
            base_url: base_url.to_string(),
        }
    }

    /// Create a client using the default production URL.
    pub fn default_client() -> Self {
        Self::new(HL_INFO_URL)
    }

    // -- Internal POST helpers -----------------------------------------------

    /// Send a POST request and parse the JSON response.
    ///
    /// This is the core HTTP method used by both the primary path and the
    /// DNS-fallback retry path.
    async fn send_post<T: serde::de::DeserializeOwned>(
        &self,
        body: &serde_json::Value,
        label: &str,
        client: &Client,
    ) -> Result<T> {
        let resp = client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("HL Info request failed: {}", label))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("HL Info API returned {} for {}: {}", status, label, text);
        }

        let text = resp
            .text()
            .await
            .with_context(|| format!("Failed to read response body for {}", label))?;

        serde_json::from_str::<T>(&text)
            .with_context(|| format!("Failed to parse {} response: {}", label, &text[..text.len().min(500)]))
    }

    /// POST with automatic DNS-fallback retry.
    ///
    /// 1. Try the default client (normal DNS).
    /// 2. On connect/DNS error, retry each hardcoded IP in sequence by
    ///    building a temporary `reqwest::Client` with `resolve()` pinned to
    ///    the fallback IP. TLS SNI still uses the correct hostname, so
    ///    certificate validation passes.
    /// 3. If all fallback IPs fail, return the original error.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        body: &serde_json::Value,
        label: &str,
    ) -> Result<T> {
        debug!("HL Info POST {} — {}", self.base_url, label);

        // 1. Try the default client first
        match self.send_post(body, label, &self.client).await {
            Ok(result) => Ok(result),
            Err(e) if is_connect_error(&e) => {
                warn!(
                    "DNS/connect error for {} ({}), trying {} fallback IPs",
                    self.base_url,
                    label,
                    HL_API_FALLBACK_IPS.len()
                );
                self.post_with_fallback_ips(body, label, e).await
            }
            Err(e) => Err(e),
        }
    }

    /// Iterate through `HL_API_FALLBACK_IPS` and retry the POST on each.
    async fn post_with_fallback_ips<T: serde::de::DeserializeOwned>(
        &self,
        body: &serde_json::Value,
        label: &str,
        original_err: anyhow::Error,
    ) -> Result<T> {
        let host = match extract_host(&self.base_url) {
            Some(h) => h.to_string(),
            None => return Err(original_err),
        };

        for ip_str in HL_API_FALLBACK_IPS {
            let ip: std::net::IpAddr = match ip_str.parse() {
                Ok(ip) => ip,
                Err(_) => continue,
            };
            let addr = std::net::SocketAddr::new(ip, 443);

            info!("Retrying HL Info POST ({}) via fallback IP {}", label, ip_str);

            let fallback_client = match Client::builder()
                .timeout(Duration::from_secs(30))
                .resolve(&host, addr)
                .build()
            {
                Ok(c) => c,
                Err(build_err) => {
                    warn!(
                        "Failed to build fallback client for IP {}: {}",
                        ip_str, build_err
                    );
                    continue;
                }
            };

            match self.send_post(body, label, &fallback_client).await {
                Ok(result) => {
                    info!(
                        "HL Info POST ({}) succeeded via fallback IP {}",
                        label, ip_str
                    );
                    return Ok(result);
                }
                Err(e) => {
                    warn!("Fallback IP {} failed for {}: {}", ip_str, label, e);
                    continue;
                }
            }
        }

        Err(original_err.context(format!(
            "All {} DNS fallback IPs exhausted for {}",
            HL_API_FALLBACK_IPS.len(),
            label
        )))
    }

    // -- Public API methods --------------------------------------------------

    /// Fetch the clearinghouse state for a wallet (positions, margin, account value).
    ///
    /// Maps to Hyperliquid's `clearinghouseState` endpoint.
    /// Returns `HlPositions` with `asset_positions`, `margin_summary`, and `account_value`.
    pub async fn get_positions(&self, wallet: &str) -> Result<HlPositions> {
        let body = serde_json::json!({
            "type": "clearinghouseState",
            "user": wallet
        });
        debug!("Fetching positions for wallet={}", wallet);
        self.post(&body, &format!("clearinghouseState({})", &wallet[..wallet.len().min(10)])
        ).await
    }

    /// Fetch funding rates and market contexts for all perpetual markets.
    ///
    /// Maps to Hyperliquid's `metaAndAssetCtxs` endpoint.
    /// Returns a vector of `HlFundingRate` — one per listed perpetual market.
    /// The response format is `[{"universe":[...]}, [{funding, markPx, ...}, ...]]`.
    pub async fn get_funding_rates(&self) -> Result<Vec<HlFundingRate>> {
        let body = serde_json::json!({
            "type": "metaAndAssetCtxs"
        });
        debug!("Fetching funding rates for all markets");
        let raw: serde_json::Value = self.post(&body, "metaAndAssetCtxs").await?;
        parse_meta_and_asset_ctxs(&raw)
    }

    /// Fetch up to 2000 recent fills for a wallet.
    ///
    /// Maps to Hyperliquid's `userFills` endpoint.
    /// Returns a vector of `HlUserFill` sorted by time (ascending in API response).
    pub async fn get_user_fills(&self, wallet: &str) -> Result<Vec<HlUserFill>> {
        let body = serde_json::json!({
            "type": "userFills",
            "user": wallet
        });
        debug!("Fetching user fills for wallet={}", wallet);
        self.post(&body, &format!("userFills({})", &wallet[..wallet.len().min(10)])
        ).await
    }

    /// Fetch fills for a wallet starting from a given timestamp (milliseconds).
    ///
    /// Maps to Hyperliquid's `userFillsByTime` endpoint.
    /// Returns up to 10 000 fills with `startTime` filter applied.
    pub async fn get_user_fills_by_time(
        &self,
        wallet: &str,
        start_time_ms: i64,
    ) -> Result<Vec<HlUserFill>> {
        let body = serde_json::json!({
            "type": "userFillsByTime",
            "user": wallet,
            "startTime": start_time_ms
        });
        debug!(
            "Fetching user fills by time for wallet={}, startTime={}",
            wallet, start_time_ms
        );
        self.post(
            &body,
            &format!("userFillsByTime({})", &wallet[..wallet.len().min(10)]),
        )
        .await
    }

    /// Fetch market metadata (universe + asset contexts).
    ///
    /// Returns the full `HlMarketContexts` with both the universe metadata
    /// and per-asset context data (mark price, funding, open interest, etc.).
    pub async fn get_market_contexts(&self) -> Result<HlMarketContexts> {
        let body = serde_json::json!({
            "type": "metaAndAssetCtxs"
        });
        debug!("Fetching market contexts");
        let raw: serde_json::Value = self.post(&body, "metaAndAssetCtxs").await?;
        parse_market_contexts(&raw)
    }
}

// ---------------------------------------------------------------------------
// Response types — clearinghouseState
// ---------------------------------------------------------------------------

/// Top-level response from Hyperliquid `clearinghouseState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlPositions {
    /// Margin summary (account value, total margin used, etc.).
    pub margin_summary: HlMarginSummary,
    /// Current open positions.
    pub asset_positions: Vec<HlAssetPosition>,
    /// Cross-margin account value (string, e.g. "1234.56").
    #[serde(default)]
    pub cross_margin_account_value: Option<String>,
    /// Withdrawable collateral.
    #[serde(default)]
    pub withdrawable: Option<String>,
    /// Timestamp of the snapshot.
    #[serde(default)]
    pub time: Option<i64>,
}

impl HlPositions {
    /// Parse the account value (f64) from margin_summary.
    pub fn account_value(&self) -> f64 {
        parse_f64_safe(&self.margin_summary.account_value, "accountValue")
            .unwrap_or(0.0)
    }

    /// Parse total margin used (f64).
    pub fn total_margin_used(&self) -> f64 {
        parse_f64_safe(&self.margin_summary.total_margin_used, "totalMarginUsed")
            .unwrap_or(0.0)
    }

    /// Get the number of open positions.
    pub fn open_position_count(&self) -> usize {
        self.asset_positions
            .iter()
            .filter(|ap| {
                let sz: f64 = ap
                    .position
                    .size
                    .parse()
                    .unwrap_or(0.0);
                sz.abs() > 0.0
            })
            .count()
    }
}

/// Margin summary embedded in `clearinghouseState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlMarginSummary {
    pub account_value: String,
    pub total_ntl_pos: String,
    pub total_raw_usd: String,
    pub total_margin_used: String,
}

/// A single asset position entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlAssetPosition {
    /// The position details (coin, size, entry price, etc.).
    pub position: HlPositionData,
    /// Type label, e.g. "oneWay".
    #[serde(default)]
    pub r#type: Option<String>,
}

/// Position data inside an `HlAssetPosition`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlPositionData {
    /// Market coin (e.g. "BTC", "ETH").
    pub coin: String,
    /// Position size as string (negative for shorts).
    pub size: String,
    /// Entry price as string.
    #[serde(default)]
    pub entry_px: Option<String>,
    /// Mark price as string.
    #[serde(default)]
    pub mark_px: Option<String>,
    /// Position value (mark) as string.
    #[serde(default)]
    pub position_value: Option<String>,
    /// Unrealized PnL as string.
    #[serde(default)]
    pub unrealized_pnl: Option<String>,
    /// Leverage (e.g. "5").
    #[serde(default)]
    pub leverage: Option<HlLeverage>,
    /// Liquidation price as string.
    #[serde(default)]
    pub liquidation_px: Option<String>,
    /// Margin used as string.
    #[serde(default)]
    pub margin_used: Option<String>,
    /// Return on equity as string.
    #[serde(default)]
    pub return_on_equity: Option<String>,
}

/// Leverage descriptor embedded in position data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlLeverage {
    pub r#type: String,
    pub value: String,
    pub cross_margin_leverage: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types — metaAndAssetCtxs
// ---------------------------------------------------------------------------

/// Parsed funding rate data for a single market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlFundingRate {
    /// Market coin (e.g. "BTC").
    pub coin: String,
    /// Mark price (f64).
    pub mark_px: f64,
    /// Current funding rate (f64, e.g. 0.0001 = 0.01%).
    pub funding: f64,
    /// Annualized funding rate (f64, e.g. 0.365 = 36.5%).
    pub annualized_funding: f64,
    /// Open interest in USD (f64).
    pub open_interest_usd: f64,
    /// 24h volume in USD (f64), if available.
    #[serde(default)]
    pub volume_24h_usd: f64,
    /// Previous day's funding (f64), if available.
    #[serde(default)]
    pub prev_day_funding: f64,
    /// Previous day's mark price (f64), used for 24h volatility calculation.
    #[serde(default)]
    pub prev_day_px: f64,
}

/// Full market context — universe entry + asset context combined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlMarketContext {
    /// Market coin (e.g. "BTC").
    pub coin: String,
    /// Full name (e.g. "Bitcoin").
    #[serde(default)]
    pub name: String,
    /// Sz decimals for the base asset.
    #[serde(default)]
    pub sz_decimals: u32,
    /// Max leverage.
    #[serde(default)]
    pub max_leverage: f64,
    /// Mark price.
    pub mark_px: f64,
    /// Current funding rate.
    pub funding: f64,
    /// Annualized funding rate.
    pub annualized_funding: f64,
    /// Open interest in USD.
    pub open_interest_usd: f64,
    /// 24h volume in USD.
    #[serde(default)]
    pub volume_24h_usd: f64,
    /// Previous day's funding.
    #[serde(default)]
    pub prev_day_funding: f64,
}

/// Container for full market context data.
#[derive(Debug, Clone, Serialize)]
pub struct HlMarketContexts {
    /// Market metadata (universe).
    pub universe: Vec<HlUniverseEntry>,
    /// Per-asset context data.
    pub contexts: Vec<HlAssetContext>,
}

/// Universe metadata for a single perpetual market.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlUniverseEntry {
    pub name: String,
    pub sz_decimals: Option<u32>,
    pub max_leverage: Option<f64>,
    /// Whether the market is only mode.
    #[serde(default)]
    pub only_isolated: Option<bool>,
}

/// Per-asset context data (funding, mark price, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlAssetContext {
    pub funding: String,
    pub open_interest: Option<String>,
    pub prev_day_px: Option<String>,
    pub day_ntl_vlm: Option<String>,
    pub premium: Option<String>,
    pub oracle_px: Option<String>,
    pub mark_px: Option<String>,
    pub mid_px: Option<String>,
    pub impact_pxs: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Response types — userFills / userFillsByTime
// ---------------------------------------------------------------------------

/// A single fill record from Hyperliquid `userFills` or `userFillsByTime`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlUserFill {
    /// Market coin (e.g. "BTC").
    pub coin: String,
    /// Side: "B" (buy) or "A" (sell).
    pub side: String,
    /// Fill price as string.
    pub px: String,
    /// Fill size as string.
    pub sz: String,
    /// Fee paid as string.
    pub fee: String,
    /// Realized PnL for this fill as string (can be empty).
    #[serde(default)]
    pub closed_pnl: Option<String>,
    /// Timestamp in milliseconds.
    pub time: i64,
    /// Direction: "Open Long", "Close Long", "Open Short", "Close Short".
    pub dir: String,
    /// Transaction hash.
    pub hash: String,
    /// Position before this fill (string, "0" means new position).
    #[serde(default)]
    pub start_position: Option<String>,
}

impl HlUserFill {
    /// Parse fill price as f64.
    pub fn price_f64(&self) -> f64 {
        self.px.parse().unwrap_or(0.0)
    }

    /// Parse fill size as f64.
    pub fn size_f64(&self) -> f64 {
        self.sz.parse().unwrap_or(0.0)
    }

    /// Parse fee as f64.
    pub fn fee_f64(&self) -> f64 {
        self.fee.parse().unwrap_or(0.0)
    }

    /// Parse closed PnL as f64 (0.0 if absent).
    pub fn closed_pnl_f64(&self) -> f64 {
        self.closed_pnl
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    }

    /// Parse start position as f64 (0.0 if absent).
    pub fn start_position_f64(&self) -> f64 {
        self.start_position
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    }

    /// Is this fill opening a brand-new position?
    ///
    /// A fill opens a new position when `dir` starts with "Open" and
    /// `startPosition` is zero (or absent).
    pub fn is_new_position(&self) -> bool {
        self.dir.starts_with("Open") && self.start_position_f64() == 0.0
    }

    /// Notional value of this fill (price * size).
    pub fn notional(&self) -> f64 {
        self.price_f64() * self.size_f64()
    }
}

// ---------------------------------------------------------------------------
// Position diff — detect new vs existing positions
// ---------------------------------------------------------------------------

/// Result of comparing two position snapshots.
#[derive(Debug, Clone, Serialize)]
pub struct PositionDiff {
    /// Positions that exist in `new` but not in `old` (fresh entries).
    pub new_positions: Vec<HlAssetPosition>,
    /// Positions that exist in both but their size changed.
    pub modified_positions: Vec<PositionChange>,
    /// Positions that were in `old` but are missing from `new` (closed).
    pub closed_positions: Vec<HlAssetPosition>,
}

/// A position whose size changed between snapshots.
#[derive(Debug, Clone, Serialize)]
pub struct PositionChange {
    pub coin: String,
    pub old_size: f64,
    pub new_size: f64,
    pub delta: f64,
}

/// Detect new, modified, and closed positions by comparing two snapshots.
///
/// - "New" positions: present in `new_positions` but not in `old_positions` (by coin).
/// - "Modified" positions: present in both but with different sizes.
/// - "Closed" positions: present in `old_positions` but not in `new_positions`.
pub fn detect_new_positions(
    old_positions: &[HlAssetPosition],
    new_positions: &[HlAssetPosition],
) -> PositionDiff {
    let old_map: std::collections::HashMap<&str, (&HlAssetPosition, f64)> = old_positions
        .iter()
        .filter_map(|ap| {
            let sz: f64 = ap.position.size.parse().unwrap_or(0.0);
            if sz.abs() > 0.0 {
                Some((ap.position.coin.as_str(), (ap, sz)))
            } else {
                None
            }
        })
        .collect();

    let new_map: std::collections::HashMap<&str, (&HlAssetPosition, f64)> = new_positions
        .iter()
        .filter_map(|ap| {
            let sz: f64 = ap.position.size.parse().unwrap_or(0.0);
            if sz.abs() > 0.0 {
                Some((ap.position.coin.as_str(), (ap, sz)))
            } else {
                None
            }
        })
        .collect();

    let old_coins: HashSet<&str> = old_map.keys().copied().collect();
    let new_coins: HashSet<&str> = new_map.keys().copied().collect();

    // New positions: in new but not in old
    let new_positions: Vec<HlAssetPosition> = new_coins
        .difference(&old_coins)
        .filter_map(|coin| new_map.get(coin).map(|(ap, _)| (*ap).clone()))
        .collect();

    // Closed positions: in old but not in new
    let closed_positions: Vec<HlAssetPosition> = old_coins
        .difference(&new_coins)
        .filter_map(|coin| old_map.get(coin).map(|(ap, _)| (*ap).clone()))
        .collect();

    // Modified positions: in both but size changed
    let modified_positions: Vec<PositionChange> = old_coins
        .intersection(&new_coins)
        .filter_map(|coin| {
            let (_, old_sz) = old_map.get(coin)?;
            let (_, new_sz) = new_map.get(coin)?;
            if (old_sz - new_sz).abs() > f64::EPSILON {
                Some(PositionChange {
                    coin: coin.to_string(),
                    old_size: *old_sz,
                    new_size: *new_sz,
                    delta: new_sz - old_sz,
                })
            } else {
                None
            }
        })
        .collect();

    PositionDiff {
        new_positions,
        modified_positions,
        closed_positions,
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse the `metaAndAssetCtxs` response into a flat vector of funding rates.
///
/// The raw response is `[{"universe": [...]}, [{funding, markPx, ...}, ...]]`.
/// Element [0] is a dict with key `universe` containing an array of market metadata.
/// Element [1] is an array of per-asset context objects.
fn parse_meta_and_asset_ctxs(raw: &serde_json::Value) -> Result<Vec<HlFundingRate>> {
    let arr = raw
        .as_array()
        .with_context(|| "metaAndAssetCtxs response is not an array")?;

    if arr.len() < 2 {
        anyhow::bail!("metaAndAssetCtxs response has fewer than 2 elements");
    }

    // Element 0: {"universe": [{name, szDecimals, maxLeverage, ...}, ...]}
    let universe = arr[0]
        .get("universe")
        .and_then(|v| v.as_array())
        .with_context(|| "metaAndAssetCtxs[0].universe is not an array")?;

    // Element 1: [{funding, markPx, openInterest, ...}, ...]
    let contexts = arr[1]
        .as_array()
        .with_context(|| "metaAndAssetCtxs[1] is not an array")?;

    let mut rates = Vec::with_capacity(universe.len().min(contexts.len()));
    for (i, (meta, ctx)) in universe.iter().zip(contexts.iter()).enumerate() {
        let coin = meta
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                warn!("metaAndAssetCtxs universe[{}] has no name", i);
                "UNKNOWN"
            })
            .to_string();

        let mark_px = ctx
            .get("markPx")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("markPx").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        let funding = ctx
            .get("funding")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("funding").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        // Annualize: funding is per-hour, so multiply by 24 * 365
        let annualized_funding = funding * 24.0 * 365.0;

        let open_interest_usd = ctx
            .get("openInterest")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("openInterest").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        let volume_24h_usd = ctx
            .get("dayNtlVlm")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("dayNtlVlm").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        let prev_day_funding = ctx
            .get("premium")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("premium").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        let prev_day_px = ctx
            .get("prevDayPx")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
            .or_else(|| ctx.get("prevDayPx").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        rates.push(HlFundingRate {
            coin,
            mark_px,
            funding,
            annualized_funding,
            open_interest_usd,
            volume_24h_usd,
            prev_day_funding,
            prev_day_px,
        });
    }

    info!("Parsed {} funding rate entries from metaAndAssetCtxs", rates.len());
    Ok(rates)
}

/// Parse the `metaAndAssetCtxs` response into `HlMarketContexts`.
fn parse_market_contexts(raw: &serde_json::Value) -> Result<HlMarketContexts> {
    let arr = raw
        .as_array()
        .with_context(|| "metaAndAssetCtxs response is not an array")?;

    if arr.len() < 2 {
        anyhow::bail!("metaAndAssetCtxs response has fewer than 2 elements");
    }

    let universe: Vec<HlUniverseEntry> = serde_json::from_value(
        arr[0].get("universe").cloned().unwrap_or(serde_json::Value::Array(vec![])),
    ).with_context(|| "Failed to parse universe from metaAndAssetCtxs")?;

    let contexts: Vec<HlAssetContext> = serde_json::from_value(arr[1].clone())
        .with_context(|| "Failed to parse asset contexts from metaAndAssetCtxs")?;

    Ok(HlMarketContexts { universe, contexts })
}

/// Safe string-to-f64 parsing with context on failure.
pub fn parse_f64_safe(s: &str, field_name: &str) -> Result<f64> {
    s.parse::<f64>()
        .with_context(|| format!("Failed to parse '{}' as f64 for field '{}'", s, field_name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =======================================================================
    // HlPositions parsing
    // =======================================================================

    #[test]
    fn test_hl_positions_parse_full() {
        let raw = json!({
            "marginSummary": {
                "accountValue": "12345.67",
                "totalNtlPos": "5000.0",
                "totalRawUsd": "12345.67",
                "totalMarginUsed": "2500.0"
            },
            "assetPositions": [
                {
                    "position": {
                        "coin": "BTC",
                        "size": "0.5",
                        "entryPx": "60000.0",
                        "markPx": "61000.0",
                        "positionValue": "30500.0",
                        "unrealizedPnl": "500.0",
                        "leverage": {
                            "type": "cross",
                            "value": "5"
                        },
                        "liquidationPx": "50000.0",
                        "marginUsed": "6000.0",
                        "returnOnEquity": "0.083"
                    },
                    "type": "oneWay"
                }
            ],
            "crossMarginAccountValue": "12345.67",
            "withdrawable": "10000.0",
            "time": 1700000000000_i64
        });

        let positions: HlPositions = serde_json::from_value(raw).unwrap();
        assert_eq!(positions.asset_positions.len(), 1);
        assert_eq!(positions.asset_positions[0].position.coin, "BTC");
        assert_eq!(positions.asset_positions[0].position.size, "0.5");

        assert_eq!(positions.account_value(), 12345.67);
        assert_eq!(positions.total_margin_used(), 2500.0);
        assert_eq!(positions.open_position_count(), 1);
    }

    #[test]
    fn test_hl_positions_empty_wallet() {
        let raw = json!({
            "marginSummary": {
                "accountValue": "0.0",
                "totalNtlPos": "0.0",
                "totalRawUsd": "0.0",
                "totalMarginUsed": "0.0"
            },
            "assetPositions": []
        });

        let positions: HlPositions = serde_json::from_value(raw).unwrap();
        assert_eq!(positions.open_position_count(), 0);
        assert_eq!(positions.account_value(), 0.0);
    }

    #[test]
    fn test_hl_positions_short_position() {
        let raw = json!({
            "marginSummary": {
                "accountValue": "5000.0",
                "totalNtlPos": "2000.0",
                "totalRawUsd": "5000.0",
                "totalMarginUsed": "1000.0"
            },
            "assetPositions": [
                {
                    "position": {
                        "coin": "ETH",
                        "size": "-2.0",
                        "entryPx": "3000.0",
                        "markPx": "2950.0"
                    }
                }
            ]
        });

        let positions: HlPositions = serde_json::from_value(raw).unwrap();
        assert_eq!(positions.open_position_count(), 1);
        let pos = &positions.asset_positions[0].position;
        assert_eq!(pos.coin, "ETH");
        assert_eq!(pos.size, "-2.0");
    }

    #[test]
    fn test_hl_positions_multiple_markets() {
        let raw = json!({
            "marginSummary": {
                "accountValue": "10000.0",
                "totalNtlPos": "8000.0",
                "totalRawUsd": "10000.0",
                "totalMarginUsed": "4000.0"
            },
            "assetPositions": [
                {
                    "position": {
                        "coin": "BTC",
                        "size": "0.1",
                        "entryPx": "60000.0"
                    }
                },
                {
                    "position": {
                        "coin": "SOL",
                        "size": "50.0",
                        "entryPx": "150.0"
                    }
                },
                {
                    "position": {
                        "coin": "ETH",
                        "size": "-3.0",
                        "entryPx": "3000.0"
                    }
                }
            ]
        });

        let positions: HlPositions = serde_json::from_value(raw).unwrap();
        assert_eq!(positions.open_position_count(), 3);
        assert_eq!(positions.asset_positions.len(), 3);
    }

    // =======================================================================
    // Funding rates parsing
    // =======================================================================

    #[test]
    fn test_parse_funding_rates_basic() {
        let raw = json!([
            {
                "universe": [
                    {"name": "BTC", "szDecimals": 5, "maxLeverage": 50},
                    {"name": "ETH", "szDecimals": 4, "maxLeverage": 50}
                ]
            },
            [
                {"funding": "0.0001", "markPx": "60000.0", "openInterest": "100000000.0", "dayNtlVlm": "500000000.0", "premium": "0.00008"},
                {"funding": "-0.00005", "markPx": "3000.0", "openInterest": "50000000.0", "dayNtlVlm": "200000000.0", "premium": "-0.00003"}
            ]
        ]);

        let rates = parse_meta_and_asset_ctxs(&raw).unwrap();
        assert_eq!(rates.len(), 2);

        assert_eq!(rates[0].coin, "BTC");
        assert!((rates[0].mark_px - 60000.0).abs() < 0.01);
        assert!((rates[0].funding - 0.0001).abs() < 1e-10);
        // Annualized = 0.0001 * 24 * 365 = 0.876
        assert!((rates[0].annualized_funding - 0.876).abs() < 0.001);
        assert!((rates[0].open_interest_usd - 100_000_000.0).abs() < 0.01);

        assert_eq!(rates[1].coin, "ETH");
        assert!((rates[1].funding - (-0.00005)).abs() < 1e-10);
    }

    #[test]
    fn test_parse_funding_rates_empty() {
        let raw = json!([
            {"universe": []},
            []
        ]);

        let rates = parse_meta_and_asset_ctxs(&raw).unwrap();
        assert!(rates.is_empty());
    }

    #[test]
    fn test_parse_funding_rates_numeric_fields() {
        // Some API versions return numeric instead of string
        let raw = json!([
            {"universe": [{"name": "SOL"}]},
            [{"funding": 0.0002, "markPx": 150.5, "openInterest": 30000000.0, "dayNtlVlm": 100000000.0}]
        ]);

        let rates = parse_meta_and_asset_ctxs(&raw).unwrap();
        assert_eq!(rates.len(), 1);
        assert!((rates[0].funding - 0.0002).abs() < 1e-10);
        assert!((rates[0].mark_px - 150.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_funding_rates_missing_fields() {
        let raw = json!([
            {"universe": [{"name": "DOGE"}]},
            [{"funding": "0.0001", "markPx": "0.15"}]
        ]);

        let rates = parse_meta_and_asset_ctxs(&raw).unwrap();
        assert_eq!(rates[0].coin, "DOGE");
        assert!((rates[0].funding - 0.0001).abs() < 1e-10);
        assert!((rates[0].open_interest_usd - 0.0).abs() < 0.01);
        assert!((rates[0].volume_24h_usd - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_funding_rates_invalid_format() {
        let raw = json!("not an array");
        assert!(parse_meta_and_asset_ctxs(&raw).is_err());
    }

    // =======================================================================
    // HlUserFill parsing
    // =======================================================================

    #[test]
    fn test_hl_user_fill_parse() {
        let raw = json!({
            "coin": "BTC",
            "side": "B",
            "px": "60000.5",
            "sz": "0.1",
            "fee": "0.06",
            "closedPnl": "150.25",
            "time": 1700000000000_i64,
            "dir": "Open Long",
            "hash": "0xabc123",
            "startPosition": "0"
        });

        let fill: HlUserFill = serde_json::from_value(raw).unwrap();
        assert_eq!(fill.coin, "BTC");
        assert_eq!(fill.side, "B");
        assert_eq!(fill.dir, "Open Long");
        assert!((fill.price_f64() - 60000.5).abs() < 0.01);
        assert!((fill.size_f64() - 0.1).abs() < 0.001);
        assert!((fill.fee_f64() - 0.06).abs() < 0.001);
        assert!((fill.closed_pnl_f64() - 150.25).abs() < 0.01);
        assert!((fill.start_position_f64() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_hl_user_fill_is_new_position() {
        let mut fill = HlUserFill {
            coin: "BTC".into(),
            side: "B".into(),
            px: "60000.0".into(),
            sz: "0.1".into(),
            fee: "0.06".into(),
            closed_pnl: None,
            time: 0,
            dir: "Open Long".into(),
            hash: "0xabc".into(),
            start_position: Some("0".into()),
        };
        assert!(fill.is_new_position());

        // Non-zero start position → not new
        fill.start_position = Some("0.5".into());
        assert!(!fill.is_new_position());

        // Close direction → not new
        fill.dir = "Close Long".into();
        fill.start_position = Some("0".into());
        assert!(!fill.is_new_position());
    }

    #[test]
    fn test_hl_user_fill_notional() {
        let fill = HlUserFill {
            coin: "ETH".into(),
            side: "B".into(),
            px: "3000.0".into(),
            sz: "2.5".into(),
            fee: "0.1".into(),
            closed_pnl: None,
            time: 0,
            dir: "Open Long".into(),
            hash: "0xdef".into(),
            start_position: None,
        };
        assert!((fill.notional() - 7500.0).abs() < 0.01);
    }

    #[test]
    fn test_hl_user_fill_missing_optional_fields() {
        let raw = json!({
            "coin": "SOL",
            "side": "A",
            "px": "150.0",
            "sz": "10.0",
            "fee": "0.02",
            "time": 1700000000000_i64,
            "dir": "Close Short",
            "hash": "0x123"
        });

        let fill: HlUserFill = serde_json::from_value(raw).unwrap();
        assert_eq!(fill.coin, "SOL");
        assert!((fill.closed_pnl_f64() - 0.0).abs() < 0.001);
        assert!((fill.start_position_f64() - 0.0).abs() < 0.001);
    }

    // =======================================================================
    // detect_new_positions
    // =======================================================================

    fn make_position(coin: &str, size: &str) -> HlAssetPosition {
        HlAssetPosition {
            position: HlPositionData {
                coin: coin.to_string(),
                size: size.to_string(),
                entry_px: Some("100.0".into()),
                mark_px: None,
                position_value: None,
                unrealized_pnl: None,
                leverage: None,
                liquidation_px: None,
                margin_used: None,
                return_on_equity: None,
            },
            r#type: Some("oneWay".into()),
        }
    }

    #[test]
    fn test_detect_new_position_from_empty() {
        let old: Vec<HlAssetPosition> = vec![];
        let new = vec![make_position("BTC", "0.5")];

        let diff = detect_new_positions(&old, &new);
        assert_eq!(diff.new_positions.len(), 1);
        assert_eq!(diff.new_positions[0].position.coin, "BTC");
        assert!(diff.modified_positions.is_empty());
        assert!(diff.closed_positions.is_empty());
    }

    #[test]
    fn test_detect_closed_position() {
        let old = vec![make_position("BTC", "0.5")];
        let new: Vec<HlAssetPosition> = vec![];

        let diff = detect_new_positions(&old, &new);
        assert!(diff.new_positions.is_empty());
        assert!(diff.modified_positions.is_empty());
        assert_eq!(diff.closed_positions.len(), 1);
        assert_eq!(diff.closed_positions[0].position.coin, "BTC");
    }

    #[test]
    fn test_detect_modified_position() {
        let old = vec![make_position("BTC", "0.5")];
        let new = vec![make_position("BTC", "1.0")];

        let diff = detect_new_positions(&old, &new);
        assert!(diff.new_positions.is_empty());
        assert!(diff.closed_positions.is_empty());
        assert_eq!(diff.modified_positions.len(), 1);
        assert_eq!(diff.modified_positions[0].coin, "BTC");
        assert!((diff.modified_positions[0].old_size - 0.5).abs() < 0.001);
        assert!((diff.modified_positions[0].new_size - 1.0).abs() < 0.001);
        assert!((diff.modified_positions[0].delta - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_mixed_changes() {
        let old = vec![
            make_position("BTC", "0.5"),
            make_position("ETH", "2.0"),
            make_position("SOL", "10.0"),
        ];
        let new = vec![
            make_position("BTC", "0.5"),     // unchanged
            make_position("ETH", "3.0"),     // modified
            make_position("DOGE", "1000.0"), // new
            // SOL removed → closed
        ];

        let diff = detect_new_positions(&old, &new);
        assert_eq!(diff.new_positions.len(), 1);
        assert_eq!(diff.new_positions[0].position.coin, "DOGE");
        assert_eq!(diff.modified_positions.len(), 1);
        assert_eq!(diff.modified_positions[0].coin, "ETH");
        assert_eq!(diff.closed_positions.len(), 1);
        assert_eq!(diff.closed_positions[0].position.coin, "SOL");
    }

    #[test]
    fn test_detect_no_changes() {
        let old = vec![make_position("BTC", "0.5")];
        let new = vec![make_position("BTC", "0.5")];

        let diff = detect_new_positions(&old, &new);
        assert!(diff.new_positions.is_empty());
        assert!(diff.modified_positions.is_empty());
        assert!(diff.closed_positions.is_empty());
    }

    #[test]
    fn test_detect_ignores_zero_size_positions() {
        let old = vec![make_position("BTC", "0.5")];
        let new = vec![
            make_position("BTC", "0.5"),
            make_position("ETH", "0.0"), // zero size, should be ignored
        ];

        let diff = detect_new_positions(&old, &new);
        assert!(diff.new_positions.is_empty()); // ETH with 0 size ignored
    }

    // =======================================================================
    // HlMarketContexts parsing
    // =======================================================================

    #[test]
    fn test_parse_market_contexts() {
        let raw = json!([
            {
                "universe": [
                    {"name": "BTC", "szDecimals": 5, "maxLeverage": 50},
                    {"name": "ETH", "szDecimals": 4, "maxLeverage": 50}
                ]
            },
            [
                {"funding": "0.0001", "markPx": "60000.0", "openInterest": "100000000.0"},
                {"funding": "-0.00005", "markPx": "3000.0", "openInterest": "50000000.0"}
            ]
        ]);

        let ctx = parse_market_contexts(&raw).unwrap();
        assert_eq!(ctx.universe.len(), 2);
        assert_eq!(ctx.contexts.len(), 2);
        assert_eq!(ctx.universe[0].name, "BTC");
        assert_eq!(ctx.contexts[0].funding, "0.0001");
    }

    // =======================================================================
    // parse_f64_safe
    // =======================================================================

    #[test]
    fn test_parse_f64_safe_valid() {
        assert!((parse_f64_safe("123.45", "test").unwrap() - 123.45).abs() < 0.001);
        assert!((parse_f64_safe("0.0", "test").unwrap()).abs() < 0.001);
        assert!((parse_f64_safe("-50.5", "test").unwrap() - (-50.5)).abs() < 0.001);
    }

    #[test]
    fn test_parse_f64_safe_invalid() {
        assert!(parse_f64_safe("not_a_number", "test_field").is_err());
    }

    // =======================================================================
    // HlInfoClient construction
    // =======================================================================

    #[test]
    fn test_hl_info_client_new() {
        let client = HlInfoClient::new("https://api.hyperliquid.xyz/info");
        assert_eq!(client.base_url, "https://api.hyperliquid.xyz/info");
    }

    #[test]
    fn test_hl_info_client_default() {
        let client = HlInfoClient::default_client();
        assert_eq!(client.base_url, HL_INFO_URL);
    }

    // =======================================================================
    // DNS fallback
    // =======================================================================

    #[test]
    fn test_fallback_ips_parse_correctly() {
        for ip_str in HL_API_FALLBACK_IPS {
            let ip: std::net::IpAddr = ip_str
                .parse()
                .unwrap_or_else(|_| panic!("{} should be a valid IP address", ip_str));
            let addr = std::net::SocketAddr::new(ip, 443);
            assert!(addr.is_ipv4(), "{} should be IPv4", ip_str);
            assert_eq!(addr.port(), 443);
        }
        assert_eq!(HL_API_FALLBACK_IPS.len(), 4);
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://api.hyperliquid.xyz/info"),
            Some("api.hyperliquid.xyz")
        );
        assert_eq!(
            extract_host("https://api.hyperliquid.xyz:443/info"),
            Some("api.hyperliquid.xyz")
        );
        assert_eq!(
            extract_host("http://localhost:8080/test"),
            Some("localhost")
        );
        assert_eq!(extract_host("not-a-url"), None);
        assert_eq!(extract_host(""), None);
    }
}
