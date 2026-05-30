//! Imperial read-only API client for Solana perps venue aggregation.
//!
//! Connects to `https://api.imperial.space` (public, no auth) to fetch
//! venue routing, funding rates, mark prices, order book depth, market configs,
//! and statistics across Flash Trade, Phoenix, GMTrade, and Jupiter.
//!
//! **Constraints enforced at the type level:**
//! - HTTP GET only — no mutation methods exist on this client.
//! - No auth storage, no auth headers, no credentials.
//! - No gated or restricted endpoint methods.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Default base URL for the Imperial API.
pub const IMPERIAL_BASE_URL: &str = "https://api.imperial.space";

/// Default HTTP request timeout in seconds.
pub const IMPERIAL_DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Threshold (seconds) above which API calls are logged as slow.
pub const IMPERIAL_SLOW_THRESHOLD_SECS: f64 = 5.0;

// ---------------------------------------------------------------------------
// Response type structs
// ---------------------------------------------------------------------------

/// Route recommendation with full cost breakdown and all candidate venues.
/// Returned by `GET /api/v1/route`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialRouteResponse {
    pub venue: String,
    pub reason: String,
    pub max_leverage: f64,
    pub expected_cost_usd: f64,
    pub cost_breakdown: ImperialCostBreakdown,
    pub clamped: bool,
    pub candidates: Vec<ImperialRouteCandidate>,
    pub markets_version: u64,
}

/// Detailed cost breakdown for a single route.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialCostBreakdown {
    pub open_fee: f64,
    pub close_fee: f64,
    pub open_slip: f64,
    pub close_slip: f64,
    pub borrow: f64,
    pub expected_liq_cost: f64,
    pub p_liq: f64,
    pub total: f64,
}

/// A single venue candidate in a route response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialRouteCandidate {
    pub venue: String,
    pub expected_cost_usd: f64,
    pub cost_breakdown: ImperialCostBreakdown,
    pub max_leverage: f64,
}

/// Per-symbol, per-venue funding and borrow rates.
/// Returned by `GET /api/v1/funding-rates`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialFundingRatesResponse {
    pub rows: Vec<ImperialFundingRateRow>,
}

/// A single row in the funding rates response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialFundingRateRow {
    pub symbol: String,
    #[serde(default)]
    pub flash: Option<ImperialVenueFundingRate>,
    #[serde(default)]
    pub gmtrade: Option<ImperialVenueFundingRate>,
    #[serde(default)]
    pub phoenix: Option<ImperialVenueFundingRate>,
    #[serde(default)]
    pub jupiter: Option<ImperialVenueFundingRate>,
}

/// Venue-specific funding/borrow rate data.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialVenueFundingRate {
    pub source: String,
    #[serde(default)]
    pub long_funding_rate_per_hour_percent: Option<f64>,
    #[serde(default)]
    pub short_funding_rate_per_hour_percent: Option<f64>,
    #[serde(default)]
    pub long_borrow_rate_per_hour_percent: Option<f64>,
    #[serde(default)]
    pub short_borrow_rate_per_hour_percent: Option<f64>,
}

/// Per-symbol, per-venue mark prices with timestamps.
/// Returned by `GET /api/v1/mark-prices`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialMarkPricesResponse {
    pub rows: Vec<ImperialMarkPriceRow>,
}

/// A single row in the mark prices response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialMarkPriceRow {
    pub symbol: String,
    #[serde(default)]
    pub flash: Option<ImperialVenuePrice>,
    #[serde(default)]
    pub gmtrade: Option<ImperialVenuePrice>,
    #[serde(default)]
    pub phoenix: Option<ImperialVenuePrice>,
    #[serde(default)]
    pub jupiter: Option<ImperialVenuePrice>,
}

/// Venue-specific mark price with source and timestamp.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialVenuePrice {
    pub source: String,
    pub price: f64,
    pub fetched_at_unix_ms: u64,
}

/// Phoenix order book depth.
/// Returned by `GET /api/v1/phoenix/depth`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialPhoenixDepthResponse {
    #[serde(default)]
    pub snapshots: std::collections::HashMap<String, ImperialPhoenixDepthSnapshot>,
}

/// A single market's order book snapshot.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialPhoenixDepthSnapshot {
    pub symbol: String,
    pub mid: f64,
    #[serde(default)]
    pub bids: Vec<ImperialDepthLevel>,
    #[serde(default)]
    pub asks: Vec<ImperialDepthLevel>,
}

/// A single price level in an order book.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialDepthLevel {
    pub price: f64,
    pub size_base: f64,
}

/// Phoenix market configuration.
/// Returned by `GET /api/v1/phoenix/markets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialPhoenixMarket {
    pub symbol: String,
    pub underwriter: String,
    pub orderbook: String,
    pub perp_asset_map: String,
    pub asset_id: u64,
    pub subaccount_index: u64,
    pub base_lots_decimals: u32,
    pub tick_size_in_quote_lots_per_base_lot: u64,
    pub maker_fee_micro: u64,
    pub taker_fee_micro: u64,
    pub max_leverage: f64,
    pub max_size_base_lots: u64,
}

/// Flash Trade market configuration.
/// Returned by `GET /api/v1/flash/markets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialFlashMarket {
    pub symbol: String,
    pub side: String,
    pub underwriter: String,
    pub market_address: String,
    pub pool_address: String,
    pub pool_name: String,
    pub target_custody: String,
    pub target_mint: String,
    pub target_oracle: String,
    pub collateral_custody: String,
    pub collateral_mint: String,
    pub collateral_oracle: String,
    pub price_exponent: i32,
    pub token_decimals: u32,
    pub allow_open_position: bool,
    pub allow_close_position: bool,
    pub max_leverage: f64,
    pub open_position_fee_rate: f64,
    pub volatility_fee_rate: f64,
    pub max_conf_bps: u64,
}

/// GMTrade market configuration.
/// Returned by `GET /api/v1/gmtrade/markets`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialGmtradeMarket {
    pub symbol: String,
    pub underwriter: String,
    pub market: String,
    pub market_token_mint: String,
    pub index_token_mint: String,
    pub long_token_mint: String,
    pub short_token_mint: String,
    pub long_token_vault: String,
    pub short_token_vault: String,
    pub oracle: String,
    pub index_token_decimals: u32,
    pub closed: bool,
}

/// GMTrade available liquidity per symbol.
/// Returned by `GET /api/v1/gmtrade/liquidity`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialGmtradeLiquidity {
    pub symbol: String,
    pub long_available_usd: f64,
    pub short_available_usd: f64,
}

/// Solana priority fee recommendation.
/// Returned by `GET /api/v1/priority-fee`.
///
/// Note: This endpoint uses snake_case (`priority_fee`) in JSON, not camelCase.
#[derive(Debug, Clone, Deserialize)]
pub struct ImperialPriorityFee {
    pub priority_fee: u64,
}

/// Market statistics across venues.
/// Returned by `GET /api/v1/stats/markets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialStatsMarketsResponse {
    pub period: String,
    pub rows: Vec<ImperialStatsRow>,
}

/// A single symbol's market statistics.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialStatsRow {
    pub symbol: String,
    /// Volume in USD — string-quoted to preserve precision.
    pub volume_usd: String,
    /// Open interest in USD — string-quoted to preserve precision.
    pub open_interest_usd: String,
    /// Long open interest in USD — string-quoted.
    pub long_oi_usd: String,
    /// Short open interest in USD — string-quoted.
    pub short_oi_usd: String,
    pub trader_count: u64,
    pub position_count: u64,
    pub by_venue: ImperialVenueBreakdown,
}

/// Per-venue volume breakdown.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImperialVenueBreakdown {
    /// Jupiter volume in USD — string-quoted.
    pub jupiter_usd: String,
    /// Flash Trade volume in USD — string-quoted.
    pub flash_usd: String,
    /// Phoenix volume in USD — string-quoted.
    pub phoenix_usd: String,
    /// GMTrade volume in USD — string-quoted.
    pub gmtrade_usd: String,
}

// ---------------------------------------------------------------------------
// ImperialClient
// ---------------------------------------------------------------------------

/// Read-only HTTP client for the Imperial Solana perps aggregator API.
///
/// All methods are `async` returning `anyhow::Result<T>`.
/// Only HTTP GET is used — no POST, PUT, DELETE, or auth headers.
///
/// # Construction
///
/// ```ignore
/// use zekt::imperial::ImperialClient;
///
/// // Default client
/// let client = ImperialClient::default_client();
///
/// // Custom base URL
/// let client = ImperialClient::new("http://localhost:8080");
///
/// // Builder pattern with custom timeout
/// let client = ImperialClient::builder()
///     .base_url("http://localhost:8080".to_string())
///     .timeout(std::time::Duration::from_secs(10))
///     .build();
/// ```
#[derive(Clone)]
pub struct ImperialClient {
    client: Client,
    base_url: String,
}

/// Builder for constructing an `ImperialClient` with custom settings.
pub struct ImperialClientBuilder {
    base_url: String,
    timeout: Duration,
}

impl ImperialClientBuilder {
    /// Set the base URL for the Imperial API.
    pub fn base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Set the HTTP request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build the `ImperialClient`.
    pub fn build(self) -> Result<ImperialClient> {
        let client = Client::builder()
            .timeout(self.timeout)
            .build()
            .context("failed to build reqwest client for ImperialClient")?;
        Ok(ImperialClient {
            client,
            base_url: self.base_url,
        })
    }
}

impl ImperialClient {
    /// Create a new client with the given base URL and default timeout (30s).
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(IMPERIAL_DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            base_url: base_url.to_string(),
        }
    }

    /// Create a client pre-configured with the default Imperial API URL.
    pub fn default_client() -> Self {
        Self::new(IMPERIAL_BASE_URL)
    }

    /// Return a builder for custom client configuration.
    pub fn builder() -> ImperialClientBuilder {
        ImperialClientBuilder {
            base_url: IMPERIAL_BASE_URL.to_string(),
            timeout: Duration::from_secs(IMPERIAL_DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Return the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Internal: perform an HTTP GET, check status, and deserialize the JSON response.
    ///
    /// This is the single entry point for all API calls. It handles:
    /// - HTTP GET only (no POST/PUT/DELETE possible)
    /// - No auth headers (no Authorization, Cookie, etc.)
    /// - Status code checking (non-2xx → Err with context)
    /// - JSON deserialization errors → Err with context
    /// - Timeout errors → Err with timeout indication
    /// - Network errors → Err with endpoint context
    /// - Logging: debug on success, warn on slow, warn/error on failure
    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let start = Instant::now();

        debug!("GET {}", url);

        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    warn!("Imperial API timeout: GET {} — {}", url, e);
                } else {
                    warn!("Imperial API network error: GET {} — {}", url, e);
                }
                return Err(e)
                    .with_context(|| format!("Imperial API request failed: GET {}", path));
            }
        };

        let status = resp.status();
        let elapsed = start.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();

        if !status.is_success() {
            warn!(
                "Imperial API HTTP {}: GET {} ({:.1}s)",
                status, url, elapsed_secs
            );
            anyhow::bail!(
                "Imperial API returned HTTP {} for GET {}",
                status,
                path
            );
        }

        // Read raw text first for better error context
        let body_text = resp.text().await.with_context(|| {
            format!("Imperial API: failed to read response body for GET {}", path)
        })?;

        if body_text.is_empty() {
            anyhow::bail!(
                "Imperial API returned empty response body for GET {}",
                path
            );
        }

        let parsed: T = serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Imperial API: failed to parse JSON response for GET {} ({} bytes)",
                path,
                body_text.len()
            )
        })?;

        if elapsed_secs > IMPERIAL_SLOW_THRESHOLD_SECS {
            warn!(
                "Imperial API slow response: GET {} ({:.1}s)",
                path, elapsed_secs
            );
        } else {
            debug!(
                "Imperial API OK: GET {} ({:.0}ms, {} bytes)",
                path,
                elapsed.as_millis(),
                body_text.len()
            );
        }

        Ok(parsed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Client Construction ────────────────────────────────────────────────

    #[test]
    fn test_client_construction_with_base_url() {
        let client = ImperialClient::new("https://api.imperial.space");
        assert_eq!(client.base_url(), "https://api.imperial.space");
    }

    #[test]
    fn test_client_construction_with_custom_url() {
        let client = ImperialClient::new("http://localhost:9999");
        assert_eq!(client.base_url(), "http://localhost:9999");
    }

    #[test]
    fn test_client_construction_with_empty_url() {
        let client = ImperialClient::new("");
        assert_eq!(client.base_url(), "");
    }

    #[test]
    fn test_default_client() {
        let client = ImperialClient::default_client();
        assert_eq!(client.base_url(), "https://api.imperial.space");
    }

    #[test]
    fn test_builder_default() {
        let client = ImperialClient::builder().build().expect("build should succeed");
        assert_eq!(client.base_url(), "https://api.imperial.space");
    }

    #[test]
    fn test_builder_custom_url() {
        let client = ImperialClient::builder()
            .base_url("http://test.example.com".to_string())
            .build()
            .expect("build should succeed");
        assert_eq!(client.base_url(), "http://test.example.com");
    }

    #[test]
    fn test_builder_custom_timeout() {
        // Build with a very short timeout and verify it doesn't panic
        let client = ImperialClient::builder()
            .timeout(Duration::from_millis(1))
            .build()
            .expect("build should succeed");
        assert_eq!(client.base_url(), "https://api.imperial.space");
    }

    #[test]
    fn test_client_is_clone() {
        let client = ImperialClient::default_client();
        let cloned = client.clone();
        assert_eq!(cloned.base_url(), client.base_url());
    }

    // Compile-time check: ImperialClient is Send + Sync
    // (reqwest::Client is Send + Sync, and String is Send + Sync)
    fn _assert_send_sync<T: Send + Sync>() {}
    #[test]
    fn test_client_is_send_sync() {
        _assert_send_sync::<ImperialClient>();
    }

    // ── Error Handling: Network Failure ────────────────────────────────────

    #[tokio::test]
    async fn test_network_failure_returns_err_with_context() {
        // Point at a port that nothing listens on
        let client = ImperialClient::new("http://127.0.0.1:1");
        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/priority-fee").await;
        let err = result.expect_err("should return Err for unreachable host");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("priority-fee") || msg.contains("api/v1"),
            "error message should reference the endpoint, got: {}",
            msg
        );
    }

    // ── Error Handling: Timeout ────────────────────────────────────────────

    #[tokio::test]
    async fn test_timeout_returns_err() {
        // Use wiremock with a delayed response that exceeds the 1ms timeout
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_json(serde_json::json!({"priority_fee": 500})),
            )
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .timeout(Duration::from_millis(1))
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should timeout");
        let msg = format!("{:?}", err).to_lowercase();
        // The error chain should indicate timeout
        assert!(
            msg.contains("timeout") || msg.contains("timed out"),
            "error should indicate timeout, got: {}",
            msg
        );
    }

    // ── Error Handling: HTTP Non-200 ───────────────────────────────────────

    #[tokio::test]
    async fn test_http_500_returns_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for HTTP 500");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("500"),
            "error should contain HTTP status 500, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_http_429_returns_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for HTTP 429");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("429"),
            "error should contain HTTP status 429, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_http_404_returns_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for HTTP 404");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("404"),
            "error should contain HTTP status 404, got: {}",
            msg
        );
    }

    // ── Error Handling: Malformed JSON ─────────────────────────────────────

    #[tokio::test]
    async fn test_malformed_json_returns_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{{{invalid json"))
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for malformed JSON");
        let msg = format!("{:#}", err).to_lowercase();
        assert!(
            msg.contains("parse") || msg.contains("deserialize") || msg.contains("json"),
            "error should mention parsing/deserialization, got: {}",
            msg
        );
    }

    // ── Error Handling: Empty Body ─────────────────────────────────────────

    #[tokio::test]
    async fn test_empty_body_returns_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for empty body");
        let msg = format!("{:#}", err).to_lowercase();
        assert!(
            msg.contains("empty"),
            "error should mention empty body, got: {}",
            msg
        );
    }

    // ── Error Handling: Unexpected JSON Shape ──────────────────────────────

    #[tokio::test]
    async fn test_unexpected_json_shape_returns_err() {
        let server = MockServer::start().await;
        // Return an object when an array is expected
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"unexpected": "object"})),
            )
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<Vec<ImperialPhoenixMarket>> = client.get_json("/api/v1/test").await;
        let err = result.expect_err("should return Err for unexpected JSON shape");
        let msg = format!("{:#}", err).to_lowercase();
        assert!(
            msg.contains("parse") || msg.contains("deserialize") || msg.contains("json"),
            "error should mention parsing, got: {}",
            msg
        );
    }

    // ── Auth Boundary Enforcement ──────────────────────────────────────────

    /// Helper: extract the source code of the impl block for ImperialClient,
    /// excluding doc comments and test module to avoid false positives.
    fn impl_source() -> String {
        let source = include_str!("imperial.rs");
        let mut in_impl = false;
        let mut brace_depth: i32 = 0;
        let mut result = String::new();
        for line in source.lines() {
            if line.trim().starts_with("impl ImperialClient") {
                in_impl = true;
                brace_depth = 0;
            }
            if in_impl {
                result.push_str(line);
                result.push('\n');
                brace_depth += line.matches('{').count() as i32;
                brace_depth -= line.matches('}').count() as i32;
                if brace_depth <= 0 && result.contains('{') {
                    break;
                }
            }
        }
        result
    }

    #[test]
    fn test_no_post_put_delete_methods() {
        let impl_code = impl_source();
        assert!(
            !impl_code.contains(".post("),
            "ImperialClient must not use HTTP POST"
        );
        assert!(
            !impl_code.contains(".put("),
            "ImperialClient must not use HTTP PUT"
        );
        assert!(
            !impl_code.contains(".delete("),
            "ImperialClient must not use HTTP DELETE"
        );
    }

    #[test]
    fn test_no_mobile_endpoints() {
        let impl_code = impl_source();
        assert!(
            !impl_code.contains("/mobile/"),
            "No mobile endpoints in ImperialClient impl"
        );
    }

    #[test]
    fn test_no_deposit_endpoints() {
        let impl_code = impl_source();
        assert!(
            !impl_code.contains("/deposit/"),
            "No deposit endpoints in ImperialClient impl"
        );
    }

    #[test]
    fn test_no_jwt_or_auth_fields() {
        // Check the struct definition only (between "pub struct ImperialClient {" and "}")
        let source = include_str!("imperial.rs");
        let struct_start = source.find("pub struct ImperialClient {")
            .expect("ImperialClient struct not found");
        let struct_end = source[struct_start..].find('}').expect("struct closing brace") + struct_start;
        let struct_def = &source[struct_start..struct_end];

        assert!(
            !struct_def.contains("token"),
            "ImperialClient must not have token fields"
        );
        assert!(
            !struct_def.contains("credential"),
            "ImperialClient must not have credential fields"
        );

        // Check the impl block for Authorization header usage
        let impl_code = impl_source();
        assert!(
            !impl_code.contains("Authorization"),
            "ImperialClient must not send Authorization headers"
        );
    }

    #[tokio::test]
    async fn test_no_auth_headers_in_requests() {
        let server = MockServer::start().await;

        // Set up a mock that will verify the request
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            // This should NOT match if Authorization header is present
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"priority_fee": 500})),
            )
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;
        assert!(result.is_ok(), "request should succeed");

        // Verify the received request has no Authorization header
        let requests = server.received_requests().await.expect("get requests");
        assert_eq!(requests.len(), 1, "should have received exactly 1 request");
        let req = &requests[0];
        assert!(
            !req.headers.contains_key("authorization"),
            "request must not contain Authorization header"
        );
        assert!(
            !req.headers.contains_key("cookie"),
            "request must not contain Cookie header"
        );
    }

    // ── Response Type Parsing ──────────────────────────────────────────────

    #[test]
    fn test_route_response_parsing() {
        let json = r#"{
            "venue": "flash_trade",
            "reason": "Lowest total cost",
            "maxLeverage": 113.0,
            "expectedCostUsd": 1.234,
            "costBreakdown": {
                "openFee": 0.1,
                "closeFee": 0.1,
                "openSlip": 0.05,
                "closeSlip": 0.05,
                "borrow": 0.5,
                "expectedLiqCost": 0.1,
                "pLiq": 0.334,
                "total": 1.234
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "flash_trade",
                    "expectedCostUsd": 1.234,
                    "costBreakdown": {
                        "openFee": 0.1,
                        "closeFee": 0.1,
                        "openSlip": 0.05,
                        "closeSlip": 0.05,
                        "borrow": 0.5,
                        "expectedLiqCost": 0.1,
                        "pLiq": 0.334,
                        "total": 1.234
                    },
                    "maxLeverage": 113.0
                },
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 2.5,
                    "costBreakdown": {
                        "openFee": 0.2,
                        "closeFee": 0.2,
                        "openSlip": 0.1,
                        "closeSlip": 0.1,
                        "borrow": 1.5,
                        "expectedLiqCost": 0.2,
                        "pLiq": 0.2,
                        "total": 2.5
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 42
        }"#;

        let resp: ImperialRouteResponse = serde_json::from_str(json).expect("parse route response");
        assert_eq!(resp.venue, "flash_trade");
        assert_eq!(resp.reason, "Lowest total cost");
        assert!((resp.max_leverage - 113.0).abs() < 0.001);
        assert!((resp.expected_cost_usd - 1.234).abs() < 0.001);
        assert!(!resp.clamped);
        assert_eq!(resp.markets_version, 42);
        assert_eq!(resp.candidates.len(), 2);

        // Cost breakdown fields
        let cb = &resp.cost_breakdown;
        assert!((cb.open_fee - 0.1).abs() < 0.001);
        assert!((cb.close_fee - 0.1).abs() < 0.001);
        assert!((cb.borrow - 0.5).abs() < 0.001);
        assert!((cb.total - 1.234).abs() < 0.001);
        // total should equal sum of components
        let sum = cb.open_fee + cb.close_fee + cb.open_slip + cb.close_slip
            + cb.borrow + cb.expected_liq_cost + cb.p_liq;
        assert!(
            (cb.total - sum).abs() < 0.0001,
            "total should equal sum of components: total={}, sum={}",
            cb.total,
            sum
        );

        // Candidates
        assert_eq!(resp.candidates[0].venue, "flash_trade");
        assert_eq!(resp.candidates[1].venue, "phoenix");
    }

    #[test]
    fn test_funding_rate_null_deserialization() {
        let json = r#"{
            "rows": [{
                "symbol": "SOL",
                "flash": {
                    "source": "flash_custody_oracle",
                    "longFundingRatePerHourPercent": null,
                    "shortFundingRatePerHourPercent": null
                },
                "gmtrade": {
                    "source": "gmtrade_ws",
                    "longFundingRatePerHourPercent": 0.01,
                    "shortBorrowRatePerHourPercent": 0.005
                }
            }]
        }"#;

        let resp: ImperialFundingRatesResponse =
            serde_json::from_str(json).expect("parse funding rates");
        assert_eq!(resp.rows.len(), 1);

        let sol = &resp.rows[0];
        assert_eq!(sol.symbol, "SOL");

        // Flash funding rates are null → None
        let flash = sol.flash.as_ref().expect("flash should be present");
        assert_eq!(flash.source, "flash_custody_oracle");
        assert!(flash.long_funding_rate_per_hour_percent.is_none());
        assert!(flash.short_funding_rate_per_hour_percent.is_none());

        // GMTrade funding rates are present
        let gmtrade = sol.gmtrade.as_ref().expect("gmtrade should be present");
        assert_eq!(gmtrade.source, "gmtrade_ws");
        assert!((gmtrade.long_funding_rate_per_hour_percent.unwrap() - 0.01).abs() < 0.001);
    }

    #[test]
    fn test_funding_rate_missing_venue_deserializes_to_none() {
        let json = r#"{
            "rows": [{
                "symbol": "BTC",
                "flash": {
                    "source": "flash_custody_oracle"
                }
            }]
        }"#;

        let resp: ImperialFundingRatesResponse =
            serde_json::from_str(json).expect("parse funding rates with missing venues");
        let btc = &resp.rows[0];
        assert_eq!(btc.symbol, "BTC");
        assert!(btc.gmtrade.is_none(), "missing venue should be None");
        assert!(btc.phoenix.is_none(), "missing venue should be None");
        assert!(btc.jupiter.is_none(), "missing venue should be None");

        // Flash exists but all rate fields are missing → None due to #[serde(default)]
        let flash = btc.flash.as_ref().expect("flash should be present");
        assert!(flash.long_funding_rate_per_hour_percent.is_none());
    }

    #[test]
    fn test_mark_price_parsing() {
        let json = r#"{
            "rows": [{
                "symbol": "SOL",
                "flash": {
                    "source": "flash_custody_oracle",
                    "price": 150.25,
                    "fetchedAtUnixMs": 1780099198000
                },
                "gmtrade": {
                    "source": "gmtrade_ws",
                    "price": 150.30,
                    "fetchedAtUnixMs": 1780099197000
                }
            }]
        }"#;

        let resp: ImperialMarkPricesResponse =
            serde_json::from_str(json).expect("parse mark prices");
        assert_eq!(resp.rows.len(), 1);

        let sol = &resp.rows[0];
        assert_eq!(sol.symbol, "SOL");

        let flash = sol.flash.as_ref().expect("flash should be present");
        assert!((flash.price - 150.25).abs() < 0.01);
        assert_eq!(flash.fetched_at_unix_ms, 1780099198000);
        assert_eq!(flash.source, "flash_custody_oracle");
    }

    #[test]
    fn test_phoenix_depth_parsing() {
        let json = r#"{
            "snapshots": {
                "SOL": {
                    "symbol": "SOL",
                    "mid": 150.25,
                    "bids": [
                        {"price": 150.20, "sizeBase": 5.0},
                        {"price": 150.10, "sizeBase": 10.0}
                    ],
                    "asks": [
                        {"price": 150.30, "sizeBase": 3.2},
                        {"price": 150.40, "sizeBase": 7.5}
                    ]
                }
            }
        }"#;

        let resp: ImperialPhoenixDepthResponse =
            serde_json::from_str(json).expect("parse phoenix depth");
        let sol = resp.snapshots.get("SOL").expect("SOL snapshot should exist");
        assert!((sol.mid - 150.25).abs() < 0.01);
        assert_eq!(sol.bids.len(), 2);
        assert_eq!(sol.asks.len(), 2);
        assert!((sol.bids[0].price - 150.20).abs() < 0.01);
        assert!((sol.bids[0].size_base - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_phoenix_market_parsing() {
        let json = r#"{
            "symbol": "SOL",
            "underwriter": "phoenix",
            "orderbook": "test-orderbook",
            "perpAssetMap": "SOL-PERP",
            "assetId": 1,
            "subaccountIndex": 0,
            "baseLotsDecimals": 4,
            "tickSizeInQuoteLotsPerBaseLot": 100,
            "makerFeeMicro": 100,
            "takerFeeMicro": 200,
            "maxLeverage": 15.0,
            "maxSizeBaseLots": 10000
        }"#;

        let market: ImperialPhoenixMarket = serde_json::from_str(json).expect("parse phoenix market");
        assert_eq!(market.symbol, "SOL");
        assert_eq!(market.underwriter, "phoenix");
        assert!((market.max_leverage - 15.0).abs() < 0.1);
        assert!(market.taker_fee_micro >= market.maker_fee_micro);
    }

    #[test]
    fn test_flash_market_parsing() {
        let json = serde_json::json!({
            "symbol": "SOL",
            "side": "long",
            "underwriter": "flash_trade",
            "marketAddress": "test-addr-1",
            "poolAddress": "test-addr-2",
            "poolName": "Crypto.1",
            "targetCustody": "test-addr-3",
            "targetMint": "test-addr-4",
            "targetOracle": "test-addr-5",
            "collateralCustody": "test-addr-6",
            "collateralMint": "test-addr-7",
            "collateralOracle": "test-addr-8",
            "priceExponent": -8,
            "tokenDecimals": 9,
            "allowOpenPosition": true,
            "allowClosePosition": true,
            "maxLeverage": 120.0,
            "openPositionFeeRate": 0.001,
            "volatilityFeeRate": 0.0,
            "maxConfBps": 200
        });
        let market: ImperialFlashMarket =
            serde_json::from_value(json).expect("parse flash market");
        assert_eq!(market.symbol, "SOL");
        assert_eq!(market.side, "long");
        assert_eq!(market.underwriter, "flash_trade");
        assert!((market.max_leverage - 120.0).abs() < 1.0);
        assert!(market.open_position_fee_rate >= 0.0);
    }

    #[test]
    fn test_gmtrade_market_parsing() {
        // Test GMTrade market deserialization via serde round-trip
        let original = ImperialGmtradeMarket {
            symbol: "WIF".to_string(),
            underwriter: "gmtrade".to_string(),
            market: "test-addr-1".to_string(),
            market_token_mint: "test-addr-2".to_string(),
            index_token_mint: "test-addr-3".to_string(),
            long_token_mint: "test-addr-4".to_string(),
            short_token_mint: "test-addr-5".to_string(),
            long_token_vault: "test-addr-6".to_string(),
            short_token_vault: "test-addr-7".to_string(),
            oracle: "test-addr-8".to_string(),
            index_token_decimals: 6,
            closed: false,
        };
        // Serialize to JSON (produces camelCase keys), then deserialize
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: ImperialGmtradeMarket =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.symbol, "WIF");
        assert_eq!(parsed.underwriter, "gmtrade");
        assert_eq!(parsed.market, "test-addr-1");
        assert!(!parsed.closed);
        assert_eq!(parsed.index_token_decimals, 6);
    }

    #[test]
    fn test_gmtrade_liquidity_parsing() {
        let json = r#"{
            "symbol": "BTC",
            "longAvailableUsd": 50000.0,
            "shortAvailableUsd": 30000.0
        }"#;

        let liq: ImperialGmtradeLiquidity = serde_json::from_str(json).expect("parse liquidity");
        assert_eq!(liq.symbol, "BTC");
        assert!((liq.long_available_usd - 50000.0).abs() < 0.01);
        assert!((liq.short_available_usd - 30000.0).abs() < 0.01);
    }

    #[test]
    fn test_priority_fee_parsing() {
        let json = r#"{"priority_fee": 500000}"#;
        let fee: ImperialPriorityFee = serde_json::from_str(json).expect("parse priority fee");
        assert_eq!(fee.priority_fee, 500000);
    }

    #[test]
    fn test_stats_markets_parsing() {
        let json = r#"{
            "period": "24h",
            "rows": [{
                "symbol": "BTC",
                "volumeUsd": "281137.198699",
                "openInterestUsd": "29993.11",
                "longOiUsd": "15956.70",
                "shortOiUsd": "14036.41",
                "traderCount": 20,
                "positionCount": 20,
                "byVenue": {
                    "jupiterUsd": "0",
                    "flashUsd": "19761.9",
                    "phoenixUsd": "255601.7",
                    "gmtradeUsd": "5773.6"
                }
            }]
        }"#;

        let resp: ImperialStatsMarketsResponse =
            serde_json::from_str(json).expect("parse stats markets");
        assert_eq!(resp.period, "24h");
        assert_eq!(resp.rows.len(), 1);

        let btc = &resp.rows[0];
        assert_eq!(btc.symbol, "BTC");
        // Volume is a string, not parsed to f64
        assert_eq!(btc.volume_usd, "281137.198699");
        assert_eq!(btc.open_interest_usd, "29993.11");
        assert_eq!(btc.trader_count, 20);

        // OI consistency: long + short ≈ total
        let long_oi: f64 = btc.long_oi_usd.parse().unwrap();
        let short_oi: f64 = btc.short_oi_usd.parse().unwrap();
        let total_oi: f64 = btc.open_interest_usd.parse().unwrap();
        assert!((long_oi + short_oi - total_oi).abs() < 1.0, "OI should be consistent");

        // Venue breakdown
        let venues = &btc.by_venue;
        assert_eq!(venues.jupiter_usd, "0");
        assert_eq!(venues.flash_usd, "19761.9");
    }

    #[test]
    fn test_stats_volume_preserved_as_string() {
        // Verify that string-quoted numbers round-trip exactly
        let json = r#"{
            "period": "24h",
            "rows": [{
                "symbol": "ETH",
                "volumeUsd": "281137.198699",
                "openInterestUsd": "0",
                "longOiUsd": "0",
                "shortOiUsd": "0",
                "traderCount": 0,
                "positionCount": 0,
                "byVenue": {
                    "jupiterUsd": "0",
                    "flashUsd": "0",
                    "phoenixUsd": "0",
                    "gmtradeUsd": "0"
                }
            }]
        }"#;

        let resp: ImperialStatsMarketsResponse =
            serde_json::from_str(json).expect("parse stats");
        let eth = &resp.rows[0];
        assert_eq!(eth.volume_usd, "281137.198699", "string-quoted number must round-trip exactly");
    }

    // ── Response Type Derive Checks ────────────────────────────────────────

    #[test]
    fn test_response_types_are_clone() {
        // Compile-time check: all types implement Clone
        fn check_clone<T: Clone>() {}

        check_clone::<ImperialRouteResponse>();
        check_clone::<ImperialCostBreakdown>();
        check_clone::<ImperialRouteCandidate>();
        check_clone::<ImperialFundingRatesResponse>();
        check_clone::<ImperialFundingRateRow>();
        check_clone::<ImperialVenueFundingRate>();
        check_clone::<ImperialMarkPricesResponse>();
        check_clone::<ImperialMarkPriceRow>();
        check_clone::<ImperialVenuePrice>();
        check_clone::<ImperialPhoenixDepthResponse>();
        check_clone::<ImperialPhoenixDepthSnapshot>();
        check_clone::<ImperialDepthLevel>();
        check_clone::<ImperialPhoenixMarket>();
        check_clone::<ImperialFlashMarket>();
        check_clone::<ImperialGmtradeMarket>();
        check_clone::<ImperialGmtradeLiquidity>();
        check_clone::<ImperialPriorityFee>();
        check_clone::<ImperialStatsMarketsResponse>();
        check_clone::<ImperialStatsRow>();
        check_clone::<ImperialVenueBreakdown>();
    }

    #[test]
    fn test_response_types_are_debug() {
        // Compile-time check: all types implement Debug
        fn check_debug<T: std::fmt::Debug>() {}

        check_debug::<ImperialRouteResponse>();
        check_debug::<ImperialCostBreakdown>();
        check_debug::<ImperialRouteCandidate>();
        check_debug::<ImperialFundingRatesResponse>();
        check_debug::<ImperialFundingRateRow>();
        check_debug::<ImperialVenueFundingRate>();
        check_debug::<ImperialMarkPricesResponse>();
        check_debug::<ImperialMarkPriceRow>();
        check_debug::<ImperialVenuePrice>();
        check_debug::<ImperialPhoenixDepthResponse>();
        check_debug::<ImperialPhoenixDepthSnapshot>();
        check_debug::<ImperialDepthLevel>();
        check_debug::<ImperialPhoenixMarket>();
        check_debug::<ImperialFlashMarket>();
        check_debug::<ImperialGmtradeMarket>();
        check_debug::<ImperialGmtradeLiquidity>();
        check_debug::<ImperialPriorityFee>();
        check_debug::<ImperialStatsMarketsResponse>();
        check_debug::<ImperialStatsRow>();
        check_debug::<ImperialVenueBreakdown>();
    }

    #[test]
    fn test_response_types_debug_output() {
        // Verify Debug formatting works (not just a compile check)
        let fee = ImperialPriorityFee { priority_fee: 500 };
        let debug_str = format!("{:?}", fee);
        assert!(debug_str.contains("priority_fee"), "Debug output should show field names");
    }

    // ── Successful GET via wiremock ────────────────────────────────────────

    #[tokio::test]
    async fn test_successful_get_returns_parsed_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/priority-fee"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "priority_fee": 500000
                })),
            )
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let result: Result<ImperialPriorityFee> = client.get_json("/api/v1/priority-fee").await;
        let fee = result.expect("should parse successfully");
        assert_eq!(fee.priority_fee, 500000);
    }

    #[tokio::test]
    async fn test_successful_get_uses_get_method() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"priority_fee": 1})),
            )
            .mount(&server)
            .await;

        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let _: Result<ImperialPriorityFee> = client.get_json("/api/v1/test").await;

        // Verify GET was used (not POST, PUT, DELETE)
        let requests = server.received_requests().await.expect("get requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, wiremock::http::Method::GET);
    }
}
