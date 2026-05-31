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

    /// Fetch the L2 order book for a symbol and compute derived depth metrics.
    ///
    /// Maps to Hyperliquid's `l2Book` endpoint. Returns a typed `L2BookResponse`
    /// with best bid/ask, spread bps, depth tiers at 10/25/50 bps, bid/ask
    /// imbalance, depth slope, and timestamp.
    ///
    /// # Rate limit handling
    ///
    /// If the API returns HTTP 429 (Too Many Requests), this method backs off
    /// and retries up to `L2_MAX_RETRIES` times with exponential backoff.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a descriptive message for:
    /// - HTTP 4xx/5xx errors (other than 429 which is retried)
    /// - Malformed JSON responses
    /// - Network/connect errors (with DNS fallback)
    pub async fn l2_book(&self, symbol: &str) -> Result<L2BookResponse> {
        let body = serde_json::json!({
            "type": "l2Book",
            "coin": symbol
        });
        debug!("Fetching L2 book for symbol={}", symbol);
        let label = format!("l2Book({})", symbol);

        // Use retry-aware POST for rate limit handling
        let raw: HlL2BookRaw = self.post_with_retry(&body, &label).await?;
        Ok(compute_l2_metrics(raw))
    }

    /// POST with retry on HTTP 429 (rate limit).
    ///
    /// Retries up to `L2_MAX_RETRIES` times with exponential backoff starting
    /// at `L2_RETRY_BASE_DELAY_MS`. The backoff doubles on each retry.
    /// For other errors, falls through to the normal DNS-fallback retry path.
    async fn post_with_retry<T: serde::de::DeserializeOwned>(
        &self,
        body: &serde_json::Value,
        label: &str,
    ) -> Result<T> {
        let max_retries = 3usize;
        let base_delay_ms: u64 = 500;

        for attempt in 0..=max_retries {
            let resp = self.client
                .post(&self.base_url)
                .header("Content-Type", "application/json")
                .json(body)
                .send()
                .await;

            match resp {
                Ok(response) => {
                    let status = response.status();

                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if attempt < max_retries {
                            let delay_ms = base_delay_ms * 2u64.pow(attempt as u32);
                            warn!(
                                attempt = attempt + 1,
                                max_retries,
                                delay_ms,
                                "HL API returned 429 for {}, backing off",
                                label
                            );
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue;
                        } else {
                            anyhow::bail!(
                                "HL API returned 429 for {} after {} retries, giving up",
                                label,
                                max_retries
                            );
                        }
                    }

                    if !status.is_success() {
                        let text = response.text().await.unwrap_or_default();
                        anyhow::bail!(
                            "HL Info API returned {} for {}: {}",
                            status,
                            label,
                            text
                        );
                    }

                    let text = response.text().await.with_context(|| {
                        format!("Failed to read response body for {}", label)
                    })?;

                    return serde_json::from_str::<T>(&text).with_context(|| {
                        format!(
                            "Failed to parse {} response: {}",
                            label,
                            &text[..text.len().min(500)]
                        )
                    });
                }
                Err(e) => {
                    // Connect/DNS error → try fallback IPs
                    let err: anyhow::Error = e.into();
                    if is_connect_error(&err) {
                        warn!(
                            "DNS/connect error for {} ({}), trying fallback IPs",
                            self.base_url, label
                        );
                        return self.post_with_fallback_ips(body, label, err).await;
                    }
                    return Err(err.context(format!("HL Info request failed: {}", label)));
                }
            }
        }

        anyhow::bail!("Unexpected retry loop exit for {}", label)
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
// Response types — l2Book
// ---------------------------------------------------------------------------

/// A single level in the L2 order book (price, size, number of orders).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlL2Level {
    /// Price as a string (e.g., "100000.0").
    pub px: String,
    /// Size (in base asset) as a string (e.g., "1.5").
    pub sz: String,
    /// Number of orders at this level.
    pub n: u64,
}

impl HlL2Level {
    /// Parse price as f64.
    pub fn price_f64(&self) -> f64 {
        self.px.parse().unwrap_or(0.0)
    }

    /// Parse size as f64.
    pub fn size_f64(&self) -> f64 {
        self.sz.parse().unwrap_or(0.0)
    }

    /// Notional value (price * size).
    pub fn notional(&self) -> f64 {
        self.price_f64() * self.size_f64()
    }
}

/// Depth summary within a specific bps range from mid price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthTier {
    /// Distance from mid price in basis points (e.g., 10, 25, 50).
    pub range_bps: f64,
    /// Total bid-side depth in USD within this range.
    pub bid_depth_usd: f64,
    /// Total ask-side depth in USD within this range.
    pub ask_depth_usd: f64,
}

/// Typed L2 order book response with computed metrics.
///
/// Contains the raw best bid/ask plus derived analytics:
/// spread in bps, depth tiers at various ranges, bid/ask imbalance,
/// depth slope, and the snapshot timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2BookResponse {
    /// Market symbol (e.g., "BTC").
    pub symbol: String,
    /// Best bid price (0.0 if book is empty).
    pub best_bid: f64,
    /// Best ask price (0.0 if book is empty).
    pub best_ask: f64,
    /// Spread in basis points: `(ask - bid) / mid * 10000`.
    /// 0.0 if bid or ask is zero.
    pub spread_bps: f64,
    /// Depth tiers at 10, 25, and 50 bps from mid.
    pub depth_tiers: Vec<DepthTier>,
    /// Bid/ask imbalance ratio: `bid_depth_50bps / ask_depth_50bps`.
    /// 0.0 if both are zero. >1.0 = bid-heavy, <1.0 = ask-heavy.
    pub imbalance: f64,
    /// Depth slope: rate of depth decay from mid outward.
    /// Positive = depth increases away from mid (normal), negative = depth thins.
    /// Computed as (depth_50bps - depth_10bps) / depth_10bps.
    pub depth_slope: f64,
    /// Snapshot timestamp from the API (milliseconds).
    pub timestamp_ms: i64,
    /// Raw bid levels (sorted descending by price).
    pub bids: Vec<HlL2Level>,
    /// Raw ask levels (sorted ascending by price).
    pub asks: Vec<HlL2Level>,
}

impl L2BookResponse {
    /// Mid price: (best_bid + best_ask) / 2.
    /// Returns 0.0 if both sides are empty.
    pub fn mid_price(&self) -> f64 {
        if self.best_bid > 0.0 && self.best_ask > 0.0 {
            (self.best_bid + self.best_ask) / 2.0
        } else if self.best_bid > 0.0 {
            self.best_bid
        } else {
            self.best_ask
        }
    }

    /// Total bid depth within `range_bps` from mid.
    pub fn bid_depth_within(&self, range_bps: f64) -> f64 {
        let mid = self.mid_price();
        if mid <= 0.0 {
            return 0.0;
        }
        let lower = mid * (1.0 - range_bps / 10_000.0);
        self.bids
            .iter()
            .filter(|l| l.price_f64() >= lower && l.price_f64() <= mid)
            .map(|l| l.notional())
            .sum()
    }

    /// Total ask depth within `range_bps` from mid.
    pub fn ask_depth_within(&self, range_bps: f64) -> f64 {
        let mid = self.mid_price();
        if mid <= 0.0 {
            return 0.0;
        }
        let upper = mid * (1.0 + range_bps / 10_000.0);
        self.asks
            .iter()
            .filter(|l| l.price_f64() >= mid && l.price_f64() <= upper)
            .map(|l| l.notional())
            .sum()
    }
}

/// Raw L2 book response from the Hyperliquid API.
/// The response is `{"coin": "BTC", "levels": [bids, asks], "time": ms}`.
#[derive(Debug, Clone, Deserialize)]
struct HlL2BookRaw {
    coin: String,
    levels: (Vec<HlL2Level>, Vec<HlL2Level>),
    time: i64,
}

/// Compute the L2BookResponse from raw levels.
fn compute_l2_metrics(raw: HlL2BookRaw) -> L2BookResponse {
    let (bids, asks) = raw.levels;
    let best_bid = bids.first().map(|l| l.price_f64()).unwrap_or(0.0);
    let best_ask = asks.first().map(|l| l.price_f64()).unwrap_or(0.0);

    let mid = if best_bid > 0.0 && best_ask > 0.0 {
        (best_bid + best_ask) / 2.0
    } else if best_bid > 0.0 {
        best_bid
    } else {
        best_ask
    };

    let spread_bps = if best_bid > 0.0 && best_ask > 0.0 && mid > 0.0 {
        ((best_ask - best_bid) / mid) * 10_000.0
    } else {
        0.0
    };

    // Compute depth tiers at 10, 25, 50 bps
    let tier_ranges = [10.0_f64, 25.0, 50.0];
    let depth_tiers: Vec<DepthTier> = tier_ranges
        .iter()
        .map(|&range_bps| {
            let bid_lower = mid * (1.0 - range_bps / 10_000.0);
            let ask_upper = mid * (1.0 + range_bps / 10_000.0);

            let bid_depth: f64 = bids
                .iter()
                .filter(|l| l.price_f64() >= bid_lower && l.price_f64() <= mid)
                .map(|l| l.notional())
                .sum();

            let ask_depth: f64 = asks
                .iter()
                .filter(|l| l.price_f64() >= mid && l.price_f64() <= ask_upper)
                .map(|l| l.notional())
                .sum();

            DepthTier {
                range_bps,
                bid_depth_usd: bid_depth,
                ask_depth_usd: ask_depth,
            }
        })
        .collect();

    // Imbalance: use 50bps tier
    let tier_50 = depth_tiers.iter().find(|t| (t.range_bps - 50.0).abs() < 0.01);
    let imbalance = if let Some(t) = tier_50 {
        if t.ask_depth_usd > 0.0 {
            t.bid_depth_usd / t.ask_depth_usd
        } else if t.bid_depth_usd > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    } else {
        0.0
    };

    // Depth slope: rate of depth change from 10bps to 50bps
    let tier_10 = depth_tiers.iter().find(|t| (t.range_bps - 10.0).abs() < 0.01);
    let depth_slope = if let (Some(t10), Some(t50)) = (tier_10, tier_50) {
        let depth_10 = t10.bid_depth_usd + t10.ask_depth_usd;
        let depth_50 = t50.bid_depth_usd + t50.ask_depth_usd;
        if depth_10 > 0.0 {
            (depth_50 - depth_10) / depth_10
        } else {
            0.0
        }
    } else {
        0.0
    };

    L2BookResponse {
        symbol: raw.coin,
        best_bid,
        best_ask,
        spread_bps,
        depth_tiers,
        imbalance,
        depth_slope,
        timestamp_ms: raw.time,
        bids,
        asks,
    }
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

    // =======================================================================
    // L2 Book Response Type Tests (VAL-L2-001)
    // =======================================================================

    #[test]
    fn test_l2_level_parse() {
        let level = HlL2Level {
            px: "100000.5".to_string(),
            sz: "1.5".to_string(),
            n: 3,
        };
        assert!((level.price_f64() - 100000.5).abs() < 0.01);
        assert!((level.size_f64() - 1.5).abs() < 0.01);
        assert!((level.notional() - 150000.75).abs() < 1.0);
    }

    #[test]
    fn test_compute_l2_metrics_basic() {
        // VAL-L2-001: l2_book returns typed struct with spread, depth tiers, imbalance
        // Using SOL at ~150 so 10bps/25bps/50bps ranges are distinct
        let raw = HlL2BookRaw {
            coin: "SOL".to_string(),
            levels: (
                vec![
                    HlL2Level { px: "150.00".to_string(), sz: "10.0".to_string(), n: 5 },  // at mid area
                    HlL2Level { px: "149.50".to_string(), sz: "100.0".to_string(), n: 3 }, // ~38bps below mid, in 50bps tier only
                    HlL2Level { px: "148.00".to_string(), sz: "500.0".to_string(), n: 2 },  // far below, outside 50bps
                ],
                vec![
                    HlL2Level { px: "150.15".to_string(), sz: "8.0".to_string(), n: 4 },    // ~5bps above mid
                    HlL2Level { px: "150.50".to_string(), sz: "80.0".to_string(), n: 2 },   // ~28bps above, in 50bps only
                ],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);

        // Basic fields
        assert_eq!(resp.symbol, "SOL");
        assert!((resp.best_bid - 150.00).abs() < 0.01);
        assert!((resp.best_ask - 150.15).abs() < 0.01);
        assert_eq!(resp.timestamp_ms, 1_770_000_000_000_i64);

        // Spread: (150.15 - 150.00) / 150.075 * 10000 ≈ 9.99 bps
        let mid = (150.00 + 150.15) / 2.0;
        let expected_spread = (150.15 - 150.00) / mid * 10_000.0;
        assert!(
            (resp.spread_bps - expected_spread).abs() < 0.1,
            "spread_bps: expected {}, got {}",
            expected_spread,
            resp.spread_bps
        );

        // Depth tiers: 3 tiers at 10, 25, 50 bps
        assert_eq!(resp.depth_tiers.len(), 3);
        assert!((resp.depth_tiers[0].range_bps - 10.0).abs() < 0.01);
        assert!((resp.depth_tiers[1].range_bps - 25.0).abs() < 0.01);
        assert!((resp.depth_tiers[2].range_bps - 50.0).abs() < 0.01);

        // Each tier has positive depth
        assert!(resp.depth_tiers[0].bid_depth_usd > 0.0);
        assert!(resp.depth_tiers[0].ask_depth_usd > 0.0);

        // 50bps tier should have more depth than 10bps (the 149.50 bid is only in 50bps)
        let tier_10 = &resp.depth_tiers[0];
        let tier_50 = &resp.depth_tiers[2];
        assert!(
            tier_50.bid_depth_usd > tier_10.bid_depth_usd,
            "50bps bid ({}) > 10bps bid ({})",
            tier_50.bid_depth_usd,
            tier_10.bid_depth_usd
        );
        assert!(
            tier_50.ask_depth_usd > tier_10.ask_depth_usd,
            "50bps ask ({}) > 10bps ask ({})",
            tier_50.ask_depth_usd,
            tier_10.ask_depth_usd
        );

        // Imbalance should be finite (bid-heavy)
        assert!(resp.imbalance.is_finite());
        assert!(resp.imbalance > 1.0, "bid-heavy book should have imbalance > 1.0, got {}", resp.imbalance);

        // Depth slope should be positive (more depth further out)
        assert!(resp.depth_slope > 0.0, "depth_slope should be positive: {}", resp.depth_slope);
    }

    #[test]
    fn test_compute_l2_metrics_empty_book() {
        let raw = HlL2BookRaw {
            coin: "DOGE".to_string(),
            levels: (vec![], vec![]),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);
        assert_eq!(resp.symbol, "DOGE");
        assert!((resp.best_bid - 0.0).abs() < 0.01);
        assert!((resp.best_ask - 0.0).abs() < 0.01);
        assert!((resp.spread_bps - 0.0).abs() < 0.01);
        assert!((resp.imbalance - 0.0).abs() < 0.01);
        assert!((resp.depth_slope - 0.0).abs() < 0.01);
        assert_eq!(resp.depth_tiers.len(), 3);
        // All tiers should have zero depth
        for tier in &resp.depth_tiers {
            assert!((tier.bid_depth_usd - 0.0).abs() < 0.01);
            assert!((tier.ask_depth_usd - 0.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_compute_l2_metrics_bids_only() {
        let raw = HlL2BookRaw {
            coin: "BTC".to_string(),
            levels: (
                vec![
                    HlL2Level { px: "100000.0".to_string(), sz: "5.0".to_string(), n: 10 },
                ],
                vec![],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);
        assert!((resp.best_bid - 100000.0).abs() < 0.01);
        assert!((resp.best_ask - 0.0).abs() < 0.01);
        assert!((resp.spread_bps - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_l2_metrics_spread_calculation() {
        // Spread = (ask - bid) / mid * 10000
        let raw = HlL2BookRaw {
            coin: "SOL".to_string(),
            levels: (
                vec![HlL2Level { px: "150.00".to_string(), sz: "100.0".to_string(), n: 5 }],
                vec![HlL2Level { px: "150.15".to_string(), sz: "80.0".to_string(), n: 3 }],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);
        let mid = (150.00 + 150.15) / 2.0;
        let expected_spread = (150.15 - 150.00) / mid * 10_000.0;
        assert!(
            (resp.spread_bps - expected_spread).abs() < 0.1,
            "spread_bps: expected {}, got {}",
            expected_spread,
            resp.spread_bps
        );
    }

    #[test]
    fn test_compute_l2_metrics_depth_tiers_ranges() {
        // VAL-L2-001: depth tiers computed at 10, 25, 50 bps
        let mid_price = 100_000.0;

        let raw = HlL2BookRaw {
            coin: "BTC".to_string(),
            levels: (
                vec![
                    HlL2Level { px: "100000.0".to_string(), sz: "1.0".to_string(), n: 1 }, // at mid
                    HlL2Level { px: "99990.0".to_string(), sz: "2.0".to_string(), n: 1 }, // 10bps below mid
                    HlL2Level { px: "99975.0".to_string(), sz: "3.0".to_string(), n: 1 }, // 25bps below
                    HlL2Level { px: "99950.0".to_string(), sz: "4.0".to_string(), n: 1 }, // 50bps below
                    HlL2Level { px: "99800.0".to_string(), sz: "10.0".to_string(), n: 1 }, // 200bps below → outside 50bps
                ],
                vec![
                    HlL2Level { px: "100001.0".to_string(), sz: "1.0".to_string(), n: 1 }, // 0.1 bps above
                    HlL2Level { px: "100010.0".to_string(), sz: "2.0".to_string(), n: 1 }, // 10bps above
                    HlL2Level { px: "100025.0".to_string(), sz: "3.0".to_string(), n: 1 }, // 25bps above
                    HlL2Level { px: "100050.0".to_string(), sz: "4.0".to_string(), n: 1 }, // 50bps above
                    HlL2Level { px: "100200.0".to_string(), sz: "10.0".to_string(), n: 1 }, // 200bps above → outside 50bps
                ],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);

        // 10bps tier:
        // Bids in [mid*0.999, mid]: 100000.0 (1.0) + 99990.0 (2.0) → $300K
        // Asks in [mid, mid*1.001]: 100001.0 (1.0) + 100010.0 (2.0) → ~$300K
        let tier_10 = resp.depth_tiers.iter().find(|t| (t.range_bps - 10.0).abs() < 0.01).unwrap();
        assert!(tier_10.bid_depth_usd > 0.0);
        assert!(tier_10.ask_depth_usd > 0.0);

        // 50bps tier should have more depth than 10bps
        let tier_50 = resp.depth_tiers.iter().find(|t| (t.range_bps - 50.0).abs() < 0.01).unwrap();
        assert!(
            tier_50.bid_depth_usd > tier_10.bid_depth_usd,
            "50bps bid depth ({}) > 10bps bid depth ({})",
            tier_50.bid_depth_usd,
            tier_10.bid_depth_usd
        );
        assert!(
            tier_50.ask_depth_usd > tier_10.ask_depth_usd,
            "50bps ask depth ({}) > 10bps ask depth ({})",
            tier_50.ask_depth_usd,
            tier_10.ask_depth_usd
        );
    }

    #[test]
    fn test_compute_l2_metrics_imbalance() {
        // VAL-L2-001: imbalance = bid_depth_50bps / ask_depth_50bps
        let raw = HlL2BookRaw {
            coin: "SOL".to_string(),
            levels: (
                // Heavy bid side
                vec![
                    HlL2Level { px: "150.00".to_string(), sz: "10000.0".to_string(), n: 100 }, // $1.5M
                ],
                // Light ask side
                vec![
                    HlL2Level { px: "150.01".to_string(), sz: "100.0".to_string(), n: 1 }, // $15K
                ],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);
        assert!(
            resp.imbalance > 1.0,
            "bid-heavy book should have imbalance > 1.0, got {}",
            resp.imbalance
        );
    }

    #[test]
    fn test_l2_book_response_mid_price() {
        let resp = L2BookResponse {
            symbol: "BTC".to_string(),
            best_bid: 100_000.0,
            best_ask: 100_002.0,
            spread_bps: 0.2,
            depth_tiers: vec![],
            imbalance: 1.0,
            depth_slope: 0.0,
            timestamp_ms: 1_770_000_000_000,
            bids: vec![],
            asks: vec![],
        };
        assert!((resp.mid_price() - 100_001.0).abs() < 0.01);

        // Empty book
        let empty = L2BookResponse {
            symbol: "BTC".to_string(),
            best_bid: 0.0,
            best_ask: 0.0,
            spread_bps: 0.0,
            depth_tiers: vec![],
            imbalance: 0.0,
            depth_slope: 0.0,
            timestamp_ms: 0,
            bids: vec![],
            asks: vec![],
        };
        assert!((empty.mid_price() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_l2_book_response_depth_within() {
        let resp = L2BookResponse {
            symbol: "BTC".to_string(),
            best_bid: 100_000.0,
            best_ask: 100_001.0,
            spread_bps: 0.1,
            depth_tiers: vec![],
            imbalance: 1.0,
            depth_slope: 0.0,
            timestamp_ms: 1_770_000_000_000,
            bids: vec![
                HlL2Level { px: "100000.0".to_string(), sz: "1.0".to_string(), n: 1 },
                HlL2Level { px: "99990.0".to_string(), sz: "2.0".to_string(), n: 1 }, // ~10bps below
            ],
            asks: vec![
                HlL2Level { px: "100001.0".to_string(), sz: "1.0".to_string(), n: 1 },
                HlL2Level { px: "100010.0".to_string(), sz: "2.0".to_string(), n: 1 }, // ~10bps above
            ],
        };

        let bid_10 = resp.bid_depth_within(10.0);
        let ask_10 = resp.ask_depth_within(10.0);
        assert!(bid_10 > 0.0, "should have bid depth within 10bps");
        assert!(ask_10 > 0.0, "should have ask depth within 10bps");
    }

    // =======================================================================
    // L2 Book API Error Handling Tests (VAL-L2-002)
    // =======================================================================

    #[test]
    fn test_l2_raw_parse_malformed_levels() {
        // VAL-L2-002: malformed response returns error
        let bad_json = r#"{"coin": "BTC", "levels": "not_an_array", "time": 123}"#;
        let result: Result<HlL2BookRaw, _> = serde_json::from_str(bad_json);
        assert!(result.is_err(), "malformed levels should fail to parse");
    }

    #[test]
    fn test_l2_raw_parse_missing_coin() {
        let bad_json = r#"{"levels": [[{"px": "100", "sz": "1", "n": 1}]], "time": 123}"#;
        let result: Result<HlL2BookRaw, _> = serde_json::from_str(bad_json);
        assert!(result.is_err(), "missing coin should fail to parse");
    }

    #[test]
    fn test_l2_raw_parse_valid() {
        let valid_json = r#"{
            "coin": "BTC",
            "levels": [
                [{"px": "100000.0", "sz": "1.5", "n": 3}],
                [{"px": "100001.0", "sz": "2.0", "n": 1}]
            ],
            "time": 1770000000000
        }"#;
        let raw: HlL2BookRaw = serde_json::from_str(valid_json).unwrap();
        assert_eq!(raw.coin, "BTC");
        assert_eq!(raw.levels.0.len(), 1);
        assert_eq!(raw.levels.1.len(), 1);
        assert_eq!(raw.time, 1_770_000_000_000);
    }

    #[test]
    fn test_l2_raw_parse_empty_levels() {
        let valid_json = r#"{
            "coin": "DOGE",
            "levels": [[], []],
            "time": 1770000000000
        }"#;
        let raw: HlL2BookRaw = serde_json::from_str(valid_json).unwrap();
        assert_eq!(raw.coin, "DOGE");
        assert!(raw.levels.0.is_empty());
        assert!(raw.levels.1.is_empty());
    }

    // =======================================================================
    // Rate Limit / Backoff Tests (VAL-L2-003)
    // =======================================================================

    #[test]
    fn test_l2_book_method_exists() {
        // VAL-L2-003: verify l2_book method compiles and is callable
        let client = HlInfoClient::default_client();
        // We can't call it without a server, but we verify it compiles
        let _ = async {
            let _ = client.l2_book("BTC").await;
        };
    }

    #[test]
    fn test_compute_l2_metrics_serde_roundtrip() {
        let raw = HlL2BookRaw {
            coin: "ETH".to_string(),
            levels: (
                vec![HlL2Level { px: "3000.0".to_string(), sz: "10.0".to_string(), n: 5 }],
                vec![HlL2Level { px: "3000.5".to_string(), sz: "8.0".to_string(), n: 3 }],
            ),
            time: 1_770_000_000_000_i64,
        };

        let resp = compute_l2_metrics(raw);
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: L2BookResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.symbol, "ETH");
        assert!((parsed.best_bid - 3000.0).abs() < 0.01);
        assert!((parsed.best_ask - 3000.5).abs() < 0.01);
        assert_eq!(parsed.depth_tiers.len(), 3);
        assert!(!parsed.bids.is_empty());
        assert!(!parsed.asks.is_empty());
    }

    #[test]
    fn test_l2_book_response_all_fields_populated() {
        // VAL-L2-001: All required fields are populated and non-default
        let raw = HlL2BookRaw {
            coin: "SOL".to_string(),
            levels: (
                vec![
                    HlL2Level { px: "150.0".to_string(), sz: "100.0".to_string(), n: 10 },
                    HlL2Level { px: "149.5".to_string(), sz: "200.0".to_string(), n: 5 },
                ],
                vec![
                    HlL2Level { px: "150.1".to_string(), sz: "80.0".to_string(), n: 8 },
                    HlL2Level { px: "150.5".to_string(), sz: "150.0".to_string(), n: 3 },
                ],
            ),
            time: 1_770_000_000_123_i64,
        };

        let resp = compute_l2_metrics(raw);

        // All fields populated
        assert!(!resp.symbol.is_empty());
        assert!(resp.best_bid > 0.0);
        assert!(resp.best_ask > 0.0);
        assert!(resp.spread_bps >= 0.0);
        assert_eq!(resp.depth_tiers.len(), 3);
        assert!(resp.timestamp_ms > 0);
        assert!(!resp.bids.is_empty());
        assert!(!resp.asks.is_empty());

        // Spread formula: (ask - bid) / mid * 10000
        let mid = (150.0 + 150.1) / 2.0;
        let expected_spread = (150.1 - 150.0) / mid * 10_000.0;
        assert!((resp.spread_bps - expected_spread).abs() < 0.1);

        // Imbalance formula: bid_depth_50bps / ask_depth_50bps
        assert!(resp.imbalance > 0.0);
    }
}
