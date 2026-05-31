//! Route Cost Oracle — multi-venue trade cost comparison across Solana perps venues.
//!
//! Uses `ImperialClient` to fetch venue routing recommendations from the Imperial API,
//! then compares costs against the existing Flash Trade flat-fee model.
//!
//! **Decision logic for each candidate trade:**
//! 1. Query Imperial `/api/v1/route` for the symbol/side/notional/leverage
//! 2. Compare total expected cost against the current Flash Trade model
//! 3. If Imperial cost < Flash model by ≥ configured bps threshold → `route_improved = true`
//! 4. If route cost > expected edge budget → `vetoed = true`
//! 5. If route source stale/missing → fall back to Flash assumptions, log degradation
//!
//! **Oracle is disabled by default** — when `route-oracle.enabled = false`, no
//! Imperial API calls are made and the existing Flash Trade cost model is used everywhere.

use crate::config::RouteCostConfig;
use crate::imperial::{ImperialClient, ImperialRouteResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Result Types
// ---------------------------------------------------------------------------

/// Detailed fee breakdown for a routed trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteFeeBreakdown {
    /// Taker fee for opening the position (USD).
    pub taker_open_fee_usd: f64,
    /// Taker fee for closing the position (USD).
    pub taker_close_fee_usd: f64,
    /// Estimated borrow/funding cost over expected hold duration (USD).
    pub borrow_funding_usd: f64,
    /// Solana priority fee estimate (USD).
    pub priority_fee_usd: f64,
    /// Expected liquidation risk cost (USD).
    pub liquidation_risk_cost_usd: f64,
    /// Total cost (sum of all components).
    pub total_cost_usd: f64,
}

impl RouteFeeBreakdown {
    /// Flash-only cost breakdown using flat fee rate.
    pub fn flash_only(size_usd: f64, fee_rate: f64) -> Self {
        let entry_fee = size_usd * fee_rate;
        let exit_fee = size_usd * fee_rate;
        let total = entry_fee + exit_fee;
        Self {
            taker_open_fee_usd: entry_fee,
            taker_close_fee_usd: exit_fee,
            borrow_funding_usd: 0.0,
            priority_fee_usd: 0.0,
            liquidation_risk_cost_usd: 0.0,
            total_cost_usd: total,
        }
    }

    /// Validate all components are non-negative and total equals sum.
    pub fn validate(&self) -> bool {
        self.taker_open_fee_usd >= 0.0
            && self.taker_close_fee_usd >= 0.0
            && self.borrow_funding_usd >= 0.0
            && self.priority_fee_usd >= 0.0
            && self.liquidation_risk_cost_usd >= 0.0
            && self.total_cost_usd >= 0.0
            && (self.total_cost_usd
                - (self.taker_open_fee_usd
                    + self.taker_close_fee_usd
                    + self.borrow_funding_usd
                    + self.priority_fee_usd
                    + self.liquidation_risk_cost_usd))
            .abs()
                < 0.0001
    }
}

/// Result of a route cost query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteResult {
    /// Selected venue name (e.g., "flash_trade", "phoenix", "gmtrade", "flash-fallback").
    pub venue_name: String,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Detailed fee breakdown.
    pub fee_breakdown: RouteFeeBreakdown,
    /// Confidence score (0.0–1.0). 1.0 for fresh Imperial data, lower for stale/fallback.
    pub confidence: f64,
    /// True when Imperial route cost is cheaper than Flash by >= improvement_threshold_bps.
    pub route_improved: bool,
    /// True when route cost exceeds the configured edge budget (trade should be vetoed).
    pub vetoed: bool,
    /// True when the oracle fell back to Flash-only costs (stale/missing Imperial data).
    pub fallback: bool,
    /// True when degradation was logged during this query.
    pub degradation_logged: bool,
    /// True when leverage was adjusted downward to fit within venue limits.
    pub leverage_adjusted: bool,
    /// Maximum leverage available at the selected venue.
    pub max_leverage: f64,
    /// Reason for veto or fallback (human-readable).
    pub reason: String,
}

/// Cache key: (market, side, size_bucket).
type CacheKey = (String, String, u64);

/// A single cache entry.
struct CacheEntry {
    response: ImperialRouteResponse,
    timestamp: Instant,
}

// ---------------------------------------------------------------------------
// RouteCostOracle
// ---------------------------------------------------------------------------

/// Multi-venue cost estimation oracle that compares trade costs across Solana perps venues.
///
/// Uses the Imperial API to fetch routing recommendations with full cost breakdowns.
/// Falls back to Flash Trade flat-fee model when Imperial data is stale or unavailable.
///
/// # Thread Safety
///
/// `RouteCostOracle` is `Send + Sync`. The internal cache uses `Mutex` and veto/degradation
/// counters use atomic operations.
pub struct RouteCostOracle {
    client: ImperialClient,
    config: RouteCostConfig,
    cache: Mutex<HashMap<CacheKey, CacheEntry>>,
    /// Time of the last successful Imperial API response.
    last_refresh: Mutex<Option<Instant>>,
    /// Number of vetoed trades.
    veto_count: AtomicUsize,
    /// Number of consecutive fallback calls (resets on successful non-fallback).
    degradation_count: AtomicUsize,
    /// Total number of API calls made (for testing cache effectiveness).
    api_call_count: AtomicUsize,
    /// Number of route_improved calls.
    improved_count: AtomicUsize,
}

impl RouteCostOracle {
    /// Create a new `RouteCostOracle` with the given config and Imperial client.
    pub fn new(config: RouteCostConfig, client: ImperialClient) -> Self {
        Self {
            client,
            config,
            cache: Mutex::new(HashMap::new()),
            last_refresh: Mutex::new(None),
            veto_count: AtomicUsize::new(0),
            degradation_count: AtomicUsize::new(0),
            api_call_count: AtomicUsize::new(0),
            improved_count: AtomicUsize::new(0),
        }
    }

    /// Return the oracle's configuration.
    pub fn config(&self) -> &RouteCostConfig {
        &self.config
    }

    /// Return whether the oracle is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Return the last time a successful Imperial API response was received.
    pub fn last_refresh_time(&self) -> Option<Instant> {
        *self.last_refresh.lock().unwrap()
    }

    /// Check if route data is stale (older than `staleness_threshold_secs`).
    ///
    /// Returns `false` when never refreshed (allows the initial API call through).
    /// Returns `true` only when we've previously refreshed but the data is now too old.
    pub fn is_stale(&self) -> bool {
        let last = self.last_refresh.lock().unwrap();
        match *last {
            Some(t) => t.elapsed().as_secs() >= self.config.staleness_threshold_secs,
            None => false, // Never refreshed → not stale (initial call proceeds)
        }
    }

    /// Return the total number of vetoed trades.
    pub fn veto_count(&self) -> usize {
        self.veto_count.load(Ordering::Relaxed)
    }

    /// Return the number of consecutive fallback calls.
    pub fn degradation_count(&self) -> usize {
        self.degradation_count.load(Ordering::Relaxed)
    }

    /// Return the total number of Imperial API calls made.
    pub fn api_call_count(&self) -> usize {
        self.api_call_count.load(Ordering::Relaxed)
    }

    /// Return the number of route_improved calls.
    pub fn improved_count(&self) -> usize {
        self.improved_count.load(Ordering::Relaxed)
    }

    /// Compute the Flash-only fallback cost for a trade.
    ///
    /// Uses the flat fee rate (default 0.1% = 0.001 per side).
    pub fn fallback_cost(&self, _market: &str, size_usd: f64, fee_rate: f64) -> RouteResult {
        let breakdown = RouteFeeBreakdown::flash_only(size_usd, fee_rate);
        RouteResult {
            venue_name: "flash-fallback".to_string(),
            total_cost_usd: breakdown.total_cost_usd,
            fee_breakdown: breakdown,
            confidence: 0.5,
            route_improved: false,
            vetoed: false,
            fallback: true,
            degradation_logged: true,
            leverage_adjusted: false,
            max_leverage: f64::MAX,
            reason: "stale_or_missing_imperial_data".to_string(),
        }
    }

    /// Compute the cache bucket for a given size.
    fn cache_bucket(&self, size_usd: f64) -> u64 {
        if self.config.cache_bucket_usd <= 0.0 {
            return 0;
        }
        (size_usd / self.config.cache_bucket_usd).floor() as u64
    }

    /// Look up the cache for a previous route response.
    fn cache_get(&self, market: &str, side: &str, size_usd: f64) -> Option<ImperialRouteResponse> {
        let bucket = self.cache_bucket(size_usd);
        let key = (market.to_string(), side.to_string(), bucket);
        let cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(&key)
            && entry.timestamp.elapsed().as_secs() < self.config.cache_ttl_secs
        {
            debug!(
                "Route cache hit: {} {} bucket={} (age={}s)",
                market,
                side,
                bucket,
                entry.timestamp.elapsed().as_secs()
            );
            return Some(entry.response.clone());
        }
        None
    }

    /// Store a route response in the cache.
    fn cache_put(
        &self,
        market: &str,
        side: &str,
        size_usd: f64,
        response: &ImperialRouteResponse,
    ) {
        let bucket = self.cache_bucket(size_usd);
        let key = (market.to_string(), side.to_string(), bucket);
        let mut cache = self.cache.lock().unwrap();
        cache.insert(
            key,
            CacheEntry {
                response: response.clone(),
                timestamp: Instant::now(),
            },
        );
    }

    /// Compute the improvement in basis points of Imperial cost vs Flash cost.
    ///
    /// Returns `((flash_cost - imperial_cost) / flash_cost) * 10000`.
    fn improvement_bps(imperial_cost: f64, flash_cost: f64) -> f64 {
        if flash_cost <= 0.0 {
            return 0.0;
        }
        ((flash_cost - imperial_cost) / flash_cost) * 10_000.0
    }

    /// Find the best (cheapest) venue for a trade.
    ///
    /// This is the main entry point for route cost comparison.
    ///
    /// # Arguments
    /// * `market` - Symbol (e.g., "SOL", "BTC")
    /// * `side` - Trade side ("long" or "short")
    /// * `size_usd` - Position size in USD
    /// * `leverage` - Requested leverage
    /// * `flash_cost_usd` - Cost using the current Flash Trade flat-fee model
    /// * `expected_edge_usd` - Expected profit from the strategy signal (for veto check)
    ///
    /// # Returns
    /// A `RouteResult` with the selected venue, cost breakdown, and flags.
    #[allow(clippy::too_many_arguments)]
    pub async fn best_route(
        &self,
        market: &str,
        side: &str,
        size_usd: f64,
        leverage: f64,
        flash_cost_usd: f64,
        expected_edge_usd: f64,
    ) -> RouteResult {
        // Check staleness first
        if self.is_stale() {
            let mut deg = self.degradation_count.load(Ordering::Relaxed);
            deg += 1;
            self.degradation_count.store(deg, Ordering::Relaxed);

            warn!(
                "Route oracle stale (last refresh: {:?}, threshold: {}s). \
                 Falling back to Flash costs for {} {} ${:.0}. Degradation count: {}",
                self.last_refresh.lock().unwrap().map(|t| format!("{:.0}s ago", t.elapsed().as_secs())),
                self.config.staleness_threshold_secs,
                market,
                side,
                size_usd,
                deg,
            );

            // Sustained degradation warning
            if deg == 10 {
                tracing::error!(
                    "Route oracle sustained degradation: {} consecutive fallbacks",
                    deg
                );
            }

            // Compute flash fallback cost using the provided flash_cost_usd
            let fee_rate = if size_usd > 0.0 {
                flash_cost_usd / (2.0 * size_usd) // entry + exit = 2 * size * rate
            } else {
                0.001
            };
            return self.fallback_cost(market, size_usd, fee_rate);
        }

        // Try cache first
        if let Some(cached) = self.cache_get(market, side, size_usd) {
            return self.process_route_response(
                &cached,
                market,
                side,
                size_usd,
                leverage,
                flash_cost_usd,
                expected_edge_usd,
            );
        }

        // Try file-based cache for cross-run persistence (only in backtest mode with cache dir)
        let bucket = self.cache_bucket(size_usd);
        let file_cache_dir = "data/route-cache";
        let file_cache_key = format!("{}_{}_{}_{}", market, side, bucket, leverage as u64);
        let file_cache_path = std::path::Path::new(file_cache_dir)
            .join(format!("{}.json", file_cache_key));
        if self.config.enabled && std::env::var("ZEKT_FILE_CACHE").is_ok() && file_cache_path.exists()
            && let Ok(data) = std::fs::read_to_string(&file_cache_path)
            && let Ok(cached) = serde_json::from_str::<crate::imperial::ImperialRouteResponse>(&data)
        {
            // Store in memory cache too for subsequent lookups
            self.cache_put(market, side, size_usd, &cached);
            return self.process_route_response(
                &cached,
                market,
                side,
                size_usd,
                leverage,
                flash_cost_usd,
                expected_edge_usd,
            );
        }

        // Make Imperial API call
        self.api_call_count.fetch_add(1, Ordering::Relaxed);

        match self.client.get_route(market, side, size_usd, leverage).await {
            Ok(response) => {
                // Update last refresh time
                *self.last_refresh.lock().unwrap() = Some(Instant::now());
                // Reset degradation counter on success
                self.degradation_count.store(0, Ordering::Relaxed);

                // Cache the response
                self.cache_put(market, side, size_usd, &response);

                // Save to file cache for cross-run persistence
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = std::fs::create_dir_all("data/route-cache");
                    let tmp_path = file_cache_path.with_extension("json.tmp");
                    if std::fs::write(&tmp_path, &json).is_ok() {
                        let _ = std::fs::rename(&tmp_path, &file_cache_path);
                    }
                }

                debug!(
                    "Imperial route for {} {} ${:.0}: venue={}, cost=${:.4}",
                    market, side, size_usd, response.venue, response.expected_cost_usd
                );

                self.process_route_response(
                    &response,
                    market,
                    side,
                    size_usd,
                    leverage,
                    flash_cost_usd,
                    expected_edge_usd,
                )
            }
            Err(e) => {
                // API error → fallback to Flash costs
                let mut deg = self.degradation_count.load(Ordering::Relaxed);
                deg += 1;
                self.degradation_count.store(deg, Ordering::Relaxed);

                warn!(
                    "Imperial API error for {} {} ${:.0}: {}. \
                     Falling back to Flash costs. Degradation count: {}",
                    market, side, size_usd, e, deg,
                );

                if deg == 10 {
                    tracing::error!(
                        "Route oracle sustained degradation: {} consecutive fallbacks",
                        deg
                    );
                }

                let fee_rate = if size_usd > 0.0 {
                    flash_cost_usd / (2.0 * size_usd)
                } else {
                    0.001
                };
                self.fallback_cost(market, size_usd, fee_rate)
            }
        }
    }

    /// Process an Imperial route response into a RouteResult.
    #[allow(clippy::too_many_arguments)]
    fn process_route_response(
        &self,
        response: &ImperialRouteResponse,
        market: &str,
        side: &str,
        size_usd: f64,
        leverage: f64,
        flash_cost_usd: f64,
        expected_edge_usd: f64,
    ) -> RouteResult {
        // Filter out excluded venues
        let mut candidates: Vec<_> = response
            .candidates
            .iter()
            .filter(|c| {
                !self.config.excluded_venues.contains(&c.venue)
                    && c.expected_cost_usd > 0.0
                    && c.filtered_reason.is_none()
            })
            .collect();

        // Sort by cost ascending (should already be sorted, but ensure)
        candidates.sort_by(|a, b| {
            a.expected_cost_usd
                .partial_cmp(&b.expected_cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Pick the best candidate
        let best = if let Some(candidate) = candidates.first() {
            candidate
        } else {
            // No valid candidates → fallback
            warn!(
                "No valid venue candidates for {} {} ${:.0}. Falling back to Flash.",
                market, side, size_usd
            );
            let fee_rate = if size_usd > 0.0 && flash_cost_usd > 0.0 {
                flash_cost_usd / (2.0 * size_usd)
            } else {
                0.001
            };
            return self.fallback_cost(market, size_usd, fee_rate);
        };

        let imperial_cost = best.expected_cost_usd;
        let venue_name = best.venue.clone();

        // Check leverage adjustment
        let leverage_adjusted = leverage > best.max_leverage && best.max_leverage > 0.0;

        // Build fee breakdown from Imperial cost breakdown
        // Imperial has: open_fee, close_fee, open_slip, close_slip, borrow, expected_liq_cost, p_liq, total
        // We map to: taker_open (open_fee + open_slip), taker_close (close_fee + close_slip),
        //   borrow_funding, priority_fee (0, not broken out), liquidation_risk (expected_liq_cost + p_liq)
        let cb = &best.cost_breakdown;
        let breakdown = RouteFeeBreakdown {
            taker_open_fee_usd: cb.open_fee + cb.open_slip,
            taker_close_fee_usd: cb.close_fee + cb.close_slip,
            borrow_funding_usd: cb.borrow,
            priority_fee_usd: 0.0, // Imperial doesn't break this out separately in route
            liquidation_risk_cost_usd: cb.expected_liq_cost + cb.p_liq,
            total_cost_usd: imperial_cost,
        };

        // Compute improvement vs Flash
        let improvement = Self::improvement_bps(imperial_cost, flash_cost_usd);
        let route_improved = improvement >= self.config.improvement_threshold_bps;

        if route_improved {
            self.improved_count.fetch_add(1, Ordering::Relaxed);
            info!(
                "Route improved for {} {}: Imperial ${:.4} vs Flash ${:.4} ({} bps better) via {}",
                market, side, imperial_cost, flash_cost_usd, improvement as i64, venue_name
            );
        }

        // Check edge budget veto
        let vetoed = if self.config.edge_budget_pct < 100.0 && expected_edge_usd > 0.0 {
            let max_cost = expected_edge_usd * (self.config.edge_budget_pct / 100.0);
            imperial_cost > max_cost
        } else if self.config.edge_budget_pct < 100.0 {
            // No expected edge provided but budget < 100% → veto if cost > 0
            imperial_cost > 0.0 && expected_edge_usd <= 0.0 && self.config.edge_budget_pct <= 0.0
        } else {
            false
        };

        if vetoed {
            self.veto_count.fetch_add(1, Ordering::Relaxed);
            warn!(
                "Trade vetoed for {} {}: cost ${:.4} exceeds edge budget ({:.0}% of ${:.4} = ${:.4})",
                market,
                side,
                imperial_cost,
                self.config.edge_budget_pct,
                expected_edge_usd,
                expected_edge_usd * self.config.edge_budget_pct / 100.0,
            );
        }

        RouteResult {
            venue_name,
            total_cost_usd: imperial_cost,
            fee_breakdown: breakdown,
            confidence: 1.0,
            route_improved,
            vetoed,
            fallback: false,
            degradation_logged: false,
            leverage_adjusted,
            max_leverage: best.max_leverage,
            reason: if vetoed {
                "route_cost_exceeds_edge".to_string()
            } else {
                String::new()
            },
        }
    }

    /// Force-set the last refresh time (for testing).
    pub fn set_last_refresh(&self, instant: Instant) {
        *self.last_refresh.lock().unwrap() = Some(instant);
    }

    /// Clear the cache (for testing).
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }
}

// Compile-time check: RouteCostOracle is Send + Sync
fn _assert_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<RouteCostOracle>();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RouteCostConfig;
    use crate::imperial::ImperialClient;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: create a default RouteCostConfig with oracle enabled.
    fn test_config() -> RouteCostConfig {
        RouteCostConfig {
            enabled: true,
            improvement_threshold_bps: 5.0,
            edge_budget_pct: 100.0,
            staleness_threshold_secs: 60,
            cache_ttl_secs: 60,
            cache_bucket_usd: 100.0,
            excluded_venues: Vec::new(),
        }
    }

    /// Helper: create an oracle pointed at a mock server.
    fn test_oracle(server: &MockServer) -> RouteCostOracle {
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build client");
        RouteCostOracle::new(test_config(), client)
    }

    /// Helper: Imperial route JSON response with two candidates.
    fn route_json(venue: &str, cost: f64, venue2: &str, cost2: f64) -> serde_json::Value {
        serde_json::json!({
            "venue": venue,
            "reason": "Lowest total cost",
            "maxLeverage": 113.0,
            "expectedCostUsd": cost,
            "costBreakdown": {
                "openFee": cost * 0.2,
                "closeFee": cost * 0.2,
                "openSlip": cost * 0.1,
                "closeSlip": cost * 0.1,
                "borrow": cost * 0.15,
                "expectedLiqCost": cost * 0.1,
                "pLiq": cost * 0.15,
                "total": cost
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": venue,
                    "expectedCostUsd": cost,
                    "costBreakdown": {
                        "openFee": cost * 0.2,
                        "closeFee": cost * 0.2,
                        "openSlip": cost * 0.1,
                        "closeSlip": cost * 0.1,
                        "borrow": cost * 0.15,
                        "expectedLiqCost": cost * 0.1,
                        "pLiq": cost * 0.15,
                        "total": cost
                    },
                    "maxLeverage": 113.0
                },
                {
                    "venue": venue2,
                    "expectedCostUsd": cost2,
                    "costBreakdown": {
                        "openFee": cost2 * 0.2,
                        "closeFee": cost2 * 0.2,
                        "openSlip": cost2 * 0.1,
                        "closeSlip": cost2 * 0.1,
                        "borrow": cost2 * 0.15,
                        "expectedLiqCost": cost2 * 0.1,
                        "pLiq": cost2 * 0.15,
                        "total": cost2
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 42
        })
    }

    /// Helper: set up a mock route endpoint on the server.
    async fn mock_route(server: &MockServer, json: &serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json.clone()))
            .mount(server)
            .await;
    }

    // ── VAL-ROUTE-001: Compiles and exposes correct public interface ───────

    #[test]
    fn test_oracle_is_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<RouteCostOracle>();
    }

    #[test]
    fn test_oracle_exposes_public_methods() {
        let client = ImperialClient::new("http://localhost:0");
        let config = test_config();
        let oracle = RouteCostOracle::new(config, client);

        // Verify public interface exists (compile-time check)
        let _ = oracle.is_enabled();
        let _ = oracle.is_stale();
        let _ = oracle.last_refresh_time();
        let _ = oracle.veto_count();
        let _ = oracle.degradation_count();
        let _ = oracle.api_call_count();
        let _ = oracle.improved_count();
        let _ = oracle.config();
    }

    // ── VAL-ROUTE-002: Identifies cheapest venue ───────────────────────────

    #[tokio::test]
    async fn test_best_route_identifies_cheapest_venue() {
        let server = MockServer::start().await;
        // Phoenix costs $0.30, flash_trade costs $0.50
        let json = serde_json::json!({
            "venue": "phoenix",
            "reason": "Lowest total cost",
            "maxLeverage": 15.0,
            "expectedCostUsd": 0.30,
            "costBreakdown": {
                "openFee": 0.06, "closeFee": 0.06,
                "openSlip": 0.03, "closeSlip": 0.03,
                "borrow": 0.045, "expectedLiqCost": 0.03,
                "pLiq": 0.045, "total": 0.30
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.30,
                    "costBreakdown": {
                        "openFee": 0.06, "closeFee": 0.06,
                        "openSlip": 0.03, "closeSlip": 0.03,
                        "borrow": 0.045, "expectedLiqCost": 0.03,
                        "pLiq": 0.045, "total": 0.30
                    },
                    "maxLeverage": 15.0
                },
                {
                    "venue": "flash_trade",
                    "expectedCostUsd": 0.50,
                    "costBreakdown": {
                        "openFee": 0.10, "closeFee": 0.10,
                        "openSlip": 0.05, "closeSlip": 0.05,
                        "borrow": 0.075, "expectedLiqCost": 0.05,
                        "pLiq": 0.075, "total": 0.50
                    },
                    "maxLeverage": 113.0
                }
            ],
            "marketsVersion": 42
        });
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let flash_cost = 1.0; // $1.00 flash cost
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, flash_cost, 5.0).await;

        assert_eq!(result.venue_name, "phoenix", "should select cheapest venue");
        assert!((result.total_cost_usd - 0.30).abs() < 0.001, "cost should be ${:.2}", result.total_cost_usd);
        assert!(!result.fallback, "should not be fallback");
        assert!(!result.vetoed, "should not be vetoed");
    }

    // ── VAL-ROUTE-003: Fee breakdown contains all required components ──────

    #[tokio::test]
    async fn test_fee_breakdown_components_non_negative() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 1.0, "phoenix", 2.0);
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;

        let bd = &result.fee_breakdown;
        assert!(bd.taker_open_fee_usd >= 0.0, "open fee >= 0");
        assert!(bd.taker_close_fee_usd >= 0.0, "close fee >= 0");
        assert!(bd.borrow_funding_usd >= 0.0, "borrow >= 0");
        assert!(bd.priority_fee_usd >= 0.0, "priority fee >= 0");
        assert!(bd.liquidation_risk_cost_usd >= 0.0, "liq risk >= 0");
        assert!(bd.total_cost_usd > 0.0, "total > 0");

        // total should equal sum of components
        let sum = bd.taker_open_fee_usd + bd.taker_close_fee_usd
            + bd.borrow_funding_usd + bd.priority_fee_usd
            + bd.liquidation_risk_cost_usd;
        assert!(
            (bd.total_cost_usd - sum).abs() < 0.0001,
            "total ({}) should equal sum ({})",
            bd.total_cost_usd, sum
        );
    }

    // ── VAL-ROUTE-004: Unsupported market → structured error ──────────────

    #[tokio::test]
    async fn test_unsupported_market_fallback() {
        let server = MockServer::start().await;
        // API returns error for unsupported asset
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"error": "No venue supports OBSCURE_TOKEN"}"#),
            )
            .mount(&server)
            .await;

        let oracle = test_oracle(&server);
        let result = oracle.best_route("OBSCURE_TOKEN", "long", 100.0, 5.0, 0.2, 1.0).await;

        // Should fallback gracefully (not panic)
        assert!(result.fallback, "should be fallback for unsupported market");
        assert_eq!(result.venue_name, "flash-fallback");
    }

    // ── VAL-ROUTE-005: Route respects max leverage per venue ──────────────

    #[tokio::test]
    async fn test_leverage_adjusted_when_exceeds_max() {
        let server = MockServer::start().await;
        let json = serde_json::json!({
            "venue": "phoenix",
            "reason": "Lowest total cost",
            "maxLeverage": 3.0,
            "expectedCostUsd": 0.50,
            "costBreakdown": {
                "openFee": 0.1, "closeFee": 0.1,
                "openSlip": 0.05, "closeSlip": 0.05,
                "borrow": 0.1, "expectedLiqCost": 0.05,
                "pLiq": 0.05, "total": 0.50
            },
            "clamped": true,
            "candidates": [
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.50,
                    "costBreakdown": {
                        "openFee": 0.1, "closeFee": 0.1,
                        "openSlip": 0.05, "closeSlip": 0.05,
                        "borrow": 0.1, "expectedLiqCost": 0.05,
                        "pLiq": 0.05, "total": 0.50
                    },
                    "maxLeverage": 3.0
                }
            ],
            "marketsVersion": 42
        });
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;

        assert!(result.leverage_adjusted, "leverage should be adjusted when requested > max");
        assert!((result.max_leverage - 3.0).abs() < 0.1, "max_leverage should be 3.0");
    }

    #[tokio::test]
    async fn test_no_leverage_adjusted_when_within_max() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.50, "phoenix", 0.80);
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let result = oracle.best_route("SOL", "long", 1000.0, 3.0, 2.0, 10.0).await;

        assert!(!result.leverage_adjusted, "no adjustment when leverage within max");
    }

    // ── VAL-ROUTE-006: route_improved flag ─────────────────────────────────

    #[tokio::test]
    async fn test_route_improved_when_cheaper_by_threshold() {
        let server = MockServer::start().await;
        // Imperial cost = $0.995 (5 bps cheaper than $1.00), threshold = 5 bps
        let json = serde_json::json!({
            "venue": "phoenix",
            "reason": "Lowest total cost",
            "maxLeverage": 15.0,
            "expectedCostUsd": 0.995,
            "costBreakdown": {
                "openFee": 0.199, "closeFee": 0.199,
                "openSlip": 0.0995, "closeSlip": 0.0995,
                "borrow": 0.14925, "expectedLiqCost": 0.0995,
                "pLiq": 0.14925, "total": 0.995
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.995,
                    "costBreakdown": {
                        "openFee": 0.199, "closeFee": 0.199,
                        "openSlip": 0.0995, "closeSlip": 0.0995,
                        "borrow": 0.14925, "expectedLiqCost": 0.0995,
                        "pLiq": 0.14925, "total": 0.995
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 1
        });
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let flash_cost = 1.0; // $1.00
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, flash_cost, 10.0).await;

        assert!(result.route_improved, "should be improved when 5 bps cheaper (exactly at threshold)");
    }

    #[tokio::test]
    async fn test_route_not_improved_when_below_threshold() {
        let server = MockServer::start().await;
        // Imperial cost = $0.9996 (4 bps cheaper than $1.00), threshold = 5 bps
        let json = serde_json::json!({
            "venue": "phoenix",
            "reason": "Lowest total cost",
            "maxLeverage": 15.0,
            "expectedCostUsd": 0.9996,
            "costBreakdown": {
                "openFee": 0.19992, "closeFee": 0.19992,
                "openSlip": 0.09996, "closeSlip": 0.09996,
                "borrow": 0.14994, "expectedLiqCost": 0.09996,
                "pLiq": 0.14994, "total": 0.9996
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.9996,
                    "costBreakdown": {
                        "openFee": 0.19992, "closeFee": 0.19992,
                        "openSlip": 0.09996, "closeSlip": 0.09996,
                        "borrow": 0.14994, "expectedLiqCost": 0.09996,
                        "pLiq": 0.14994, "total": 0.9996
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 1
        });
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let flash_cost = 1.0;
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, flash_cost, 10.0).await;

        assert!(!result.route_improved, "should NOT be improved when only 4 bps cheaper (below 5 threshold)");
    }

    // ── VAL-ROUTE-007: Threshold configurable ──────────────────────────────

    #[tokio::test]
    async fn test_improvement_threshold_configurable() {
        let server = MockServer::start().await;
        // Imperial cost = $0.9992 (8 bps cheaper than Flash $1.00)
        let json = serde_json::json!({
            "venue": "phoenix",
            "reason": "Lowest total cost",
            "maxLeverage": 15.0,
            "expectedCostUsd": 0.9992,
            "costBreakdown": {
                "openFee": 0.19984, "closeFee": 0.19984,
                "openSlip": 0.09992, "closeSlip": 0.09992,
                "borrow": 0.14988, "expectedLiqCost": 0.09992,
                "pLiq": 0.14988, "total": 0.9992
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.9992,
                    "costBreakdown": {
                        "openFee": 0.19984, "closeFee": 0.19984,
                        "openSlip": 0.09992, "closeSlip": 0.09992,
                        "borrow": 0.14988, "expectedLiqCost": 0.09992,
                        "pLiq": 0.14988, "total": 0.9992
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 1
        });

        // Mock for SOL asset
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .and(query_param("asset", "SOL"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json.clone()))
            .mount(&server)
            .await;
        // Mock for ETH asset
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .and(query_param("asset", "ETH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json.clone()))
            .mount(&server)
            .await;

        // Oracle 1: threshold = 10 bps → 8 bps improvement NOT enough
        let mut config1 = test_config();
        config1.improvement_threshold_bps = 10.0;
        let client1 = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle1 = RouteCostOracle::new(config1, client1);
        let result1 = oracle1.best_route("SOL", "long", 1000.0, 5.0, 1.0, 10.0).await;
        assert!(!result1.route_improved, "8 bps improvement should not meet 10 bps threshold");

        // Oracle 2: threshold = 5 bps → 8 bps improvement IS enough
        let mut config2 = test_config();
        config2.improvement_threshold_bps = 5.0;
        let client2 = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle2 = RouteCostOracle::new(config2, client2);
        let result2 = oracle2.best_route("ETH", "long", 1000.0, 5.0, 1.0, 10.0).await;
        assert!(result2.route_improved, "8 bps improvement should meet 5 bps threshold");
    }

    // ── VAL-ROUTE-008: No false positives when costs equal ─────────────────

    #[tokio::test]
    async fn test_no_improved_when_costs_equal() {
        let server = MockServer::start().await;
        // Imperial cost = $1.000 (same as Flash)
        let json = serde_json::json!({
            "venue": "flash_trade",
            "reason": "Lowest total cost",
            "maxLeverage": 113.0,
            "expectedCostUsd": 1.000,
            "costBreakdown": {
                "openFee": 0.2, "closeFee": 0.2,
                "openSlip": 0.1, "closeSlip": 0.1,
                "borrow": 0.15, "expectedLiqCost": 0.1,
                "pLiq": 0.15, "total": 1.0
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "flash_trade",
                    "expectedCostUsd": 1.0,
                    "costBreakdown": {
                        "openFee": 0.2, "closeFee": 0.2,
                        "openSlip": 0.1, "closeSlip": 0.1,
                        "borrow": 0.15, "expectedLiqCost": 0.1,
                        "pLiq": 0.15, "total": 1.0
                    },
                    "maxLeverage": 113.0
                }
            ],
            "marketsVersion": 1
        });
        mock_route(&server, &json).await;

        let oracle = test_oracle(&server);
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 1.0, 10.0).await;

        assert!(!result.route_improved, "should NOT be improved when costs are equal");
    }

    // ── VAL-ROUTE-009: Trade vetoed when cost exceeds edge budget ──────────

    #[tokio::test]
    async fn test_veto_when_cost_exceeds_edge_budget() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 2.50, "phoenix", 3.00);
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.edge_budget_pct = 80.0; // max cost = 80% of expected edge
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client.clone());

        // Expected edge = $2.00, budget = 80% → max cost = $1.60
        // Route cost = $2.50 > $1.60 → vetoed
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 3.0, 2.0).await;

        assert!(result.vetoed, "should be vetoed when cost exceeds edge budget");
        assert_eq!(result.reason, "route_cost_exceeds_edge");

        // Not vetoed with same cost but bigger budget
        let mut config2 = test_config();
        config2.edge_budget_pct = 100.0;
        let _oracle2 = RouteCostOracle::new(config2, client);
    }

    #[tokio::test]
    async fn test_no_veto_when_cost_within_edge_budget() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.50, "phoenix", 0.80);
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.edge_budget_pct = 80.0;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // Expected edge = $2.00, budget = 80% → max cost = $1.60
        // Route cost = $0.50 < $1.60 → not vetoed
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 1.0, 2.0).await;

        assert!(!result.vetoed, "should NOT be vetoed when cost within edge budget");
    }

    // ── VAL-ROUTE-010: Edge budget configurable ────────────────────────────

    #[tokio::test]
    async fn test_edge_budget_configurable() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.90, "phoenix", 1.10);
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json))
            .mount(&server)
            .await;

        // With edge_budget_pct = 50, expected_edge = $2.00 → max = $1.00
        // Cost = $0.90 < $1.00 → not vetoed
        let mut config = test_config();
        config.edge_budget_pct = 50.0;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client.clone());
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 2.0).await;
        assert!(!result.vetoed, "cost $0.90 < 50% of $2.00 = $1.00");

        // Now with edge_budget_pct = 40 → max = $0.80, cost $0.90 > $0.80 → vetoed
        let mut config2 = test_config();
        config2.edge_budget_pct = 40.0;
        let oracle2 = RouteCostOracle::new(config2, client);
        mock_route(&server, &route_json("flash_trade", 0.90, "phoenix", 1.10)).await;
        let result2 = oracle2.best_route("SOL", "long", 1000.0, 5.0, 2.0, 2.0).await;
        assert!(result2.vetoed, "cost $0.90 > 40% of $2.00 = $0.80");
    }

    // ── VAL-ROUTE-011: Veto count tracked ──────────────────────────────────

    #[tokio::test]
    async fn test_veto_count_tracked() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 2.50, "phoenix", 3.00);
        // Mount enough for multiple calls
        for _ in 0..10 {
            mock_route(&server, &json).await;
        }

        let mut config = test_config();
        config.edge_budget_pct = 80.0;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // Make 10 calls, all should be vetoed (cost $2.50 > 80% of edge $2.00 = $1.60)
        for _ in 0..10 {
            let _ = oracle.best_route("SOL", "long", 1000.0, 5.0, 3.0, 2.0).await;
        }

        assert_eq!(oracle.veto_count(), 10, "should track 10 vetoes");
    }

    // ── VAL-ROUTE-012: Fallback when stale ─────────────────────────────────

    #[tokio::test]
    async fn test_fallback_when_stale() {
        let server = MockServer::start().await;
        // No mock needed — oracle's data is stale so it won't make API calls

        let mut config = test_config();
        config.staleness_threshold_secs = 5; // 5 second threshold
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // Set last_refresh to 10 seconds ago (exceeds 5s threshold)
        oracle.set_last_refresh(Instant::now() - Duration::from_secs(10));

        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;

        assert!(result.fallback, "should fallback when stale");
        assert_eq!(result.venue_name, "flash-fallback");
        assert!(result.degradation_logged);
        assert!(oracle.degradation_count() > 0);
    }

    // ── VAL-ROUTE-013: Fallback when API error ─────────────────────────────

    #[tokio::test]
    async fn test_fallback_when_api_error() {
        let server = MockServer::start().await;

        // Set up mock: return 500 for ALL route requests
        Mock::given(method("GET"))
            .and(path("/api/v1/route"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let mut config = test_config();
        config.staleness_threshold_secs = 3600;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // API call fails → fallback to Flash costs
        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;
        assert!(result.fallback, "should fallback on API error");
        assert_eq!(result.venue_name, "flash-fallback");
        assert!(oracle.degradation_count() > 0, "degradation count should increment");

        // Second call should also fallback without panic
        let result2 = oracle.best_route("BTC", "long", 5000.0, 3.0, 4.0, 20.0).await;
        assert!(result2.fallback, "should fallback on second API error too");
    }

    // ── VAL-ROUTE-014: Staleness threshold configurable ────────────────────

    #[tokio::test]
    async fn test_staleness_threshold_configurable() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.50, "phoenix", 0.30);
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.staleness_threshold_secs = 300; // 5 minutes
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // Make a call to refresh
        let _ = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;
        assert!(!oracle.is_stale(), "should not be stale immediately after refresh");

        // Set last refresh to 200 seconds ago — within 300s threshold
        oracle.set_last_refresh(Instant::now() - Duration::from_secs(200));
        assert!(!oracle.is_stale(), "200s < 300s threshold → not stale");

        // Set last refresh to 301 seconds ago — exceeds 300s threshold
        oracle.set_last_refresh(Instant::now() - Duration::from_secs(301));
        assert!(oracle.is_stale(), "301s > 300s threshold → stale");
    }

    // ── VAL-ROUTE-015: Degradation counter ─────────────────────────────────

    #[tokio::test]
    async fn test_degradation_counter_tracked() {
        let server = MockServer::start().await;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");

        let mut config = test_config();
        config.staleness_threshold_secs = 0; // Always stale once refreshed
        let oracle = RouteCostOracle::new(config, client);

        // Set last_refresh to the past so oracle is stale
        oracle.set_last_refresh(Instant::now() - Duration::from_secs(100));

        // Force 10 consecutive fallbacks
        for _ in 0..10 {
            let _ = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;
        }

        assert_eq!(oracle.degradation_count(), 10, "should track 10 consecutive fallbacks");
    }

    #[tokio::test]
    async fn test_degradation_counter_resets_on_success() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.50, "phoenix", 0.30);
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.staleness_threshold_secs = 3600; // 1 hour
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // Force stale → 5 degradation calls
        oracle.set_last_refresh(Instant::now() - Duration::from_secs(7200));
        for _ in 0..5 {
            let _ = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;
        }
        assert_eq!(oracle.degradation_count(), 5);

        // Now succeed — set last refresh to now
        oracle.set_last_refresh(Instant::now());
        let _result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;
        // Should use cache since we're not stale, but cache was populated with fallback data
        // Actually cache was from the fallback. Let's clear cache and use a new market
        oracle.clear_cache();
        mock_route(&server, &json).await;
        let result = oracle.best_route("ETH", "long", 1000.0, 5.0, 2.0, 10.0).await;
        assert!(!result.fallback, "should succeed after refresh");

        // Degradation counter should reset
        assert_eq!(oracle.degradation_count(), 0, "degradation should reset on success");
    }

    // ── VAL-ROUTE-016: Cache prevents redundant API calls ──────────────────

    #[tokio::test]
    async fn test_cache_prevents_redundant_api_calls() {
        let server = MockServer::start().await;
        let json = route_json("flash_trade", 0.50, "phoenix", 0.30);
        // Mount only once — second call should hit cache
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.staleness_threshold_secs = 3600;
        config.cache_ttl_secs = 60;
        config.cache_bucket_usd = 100.0;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        // First call — should make API call
        let _r1 = oracle.best_route("SOL", "long", 1050.0, 5.0, 2.0, 10.0).await;
        assert_eq!(oracle.api_call_count(), 1, "first call should make 1 API call");

        // Second call with same bucket (1050 / 100 = bucket 10) — should use cache
        let _r2 = oracle.best_route("SOL", "long", 1099.0, 5.0, 2.0, 10.0).await;
        assert_eq!(oracle.api_call_count(), 1, "second call in same bucket should use cache");

        // Third call with different bucket (1150 / 100 = bucket 11) — new API call
        // Need to mount again since wiremock only serves once by default
        mock_route(&server, &json).await;
        let _r3 = oracle.best_route("SOL", "long", 1150.0, 5.0, 2.0, 10.0).await;
        assert_eq!(oracle.api_call_count(), 2, "third call in new bucket should make new API call");
    }

    // ── Excluded venues ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_excluded_venues_filtered() {
        let server = MockServer::start().await;
        // flash_trade is cheapest but will be excluded
        let json = serde_json::json!({
            "venue": "flash_trade",
            "reason": "Lowest total cost",
            "maxLeverage": 113.0,
            "expectedCostUsd": 0.10,
            "costBreakdown": {
                "openFee": 0.02, "closeFee": 0.02,
                "openSlip": 0.01, "closeSlip": 0.01,
                "borrow": 0.015, "expectedLiqCost": 0.01,
                "pLiq": 0.015, "total": 0.10
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "flash_trade",
                    "expectedCostUsd": 0.10,
                    "costBreakdown": {
                        "openFee": 0.02, "closeFee": 0.02,
                        "openSlip": 0.01, "closeSlip": 0.01,
                        "borrow": 0.015, "expectedLiqCost": 0.01,
                        "pLiq": 0.015, "total": 0.10
                    },
                    "maxLeverage": 113.0
                },
                {
                    "venue": "phoenix",
                    "expectedCostUsd": 0.50,
                    "costBreakdown": {
                        "openFee": 0.10, "closeFee": 0.10,
                        "openSlip": 0.05, "closeSlip": 0.05,
                        "borrow": 0.075, "expectedLiqCost": 0.05,
                        "pLiq": 0.075, "total": 0.50
                    },
                    "maxLeverage": 15.0
                }
            ],
            "marketsVersion": 42
        });
        mock_route(&server, &json).await;

        let mut config = test_config();
        config.excluded_venues = vec!["flash_trade".to_string()];
        config.staleness_threshold_secs = 3600;
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let oracle = RouteCostOracle::new(config, client);

        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;

        // flash_trade excluded → phoenix selected
        assert_eq!(result.venue_name, "phoenix", "excluded venue should be skipped");
        assert!((result.total_cost_usd - 0.50).abs() < 0.001);
    }

    // ── Oracle disabled by default ─────────────────────────────────────────

    #[test]
    fn test_oracle_disabled_by_default() {
        let config = RouteCostConfig::default();
        assert!(!config.enabled, "oracle should be disabled by default");
    }

    // ── Flash-only cost model ──────────────────────────────────────────────

    #[test]
    fn test_flash_only_cost_breakdown() {
        let bd = RouteFeeBreakdown::flash_only(1000.0, 0.001);
        assert!((bd.taker_open_fee_usd - 1.0).abs() < 0.001, "entry fee = 1000 * 0.001 = $1.00");
        assert!((bd.taker_close_fee_usd - 1.0).abs() < 0.001, "exit fee = 1000 * 0.001 = $1.00");
        assert!((bd.total_cost_usd - 2.0).abs() < 0.001, "total = $2.00");
        assert!(bd.validate(), "breakdown should validate");
    }

    // ── VAL-CROSS-001: ImperialClient feeds RouteCostOracle ────────────────

    #[tokio::test]
    async fn test_imperial_client_feeds_route_oracle() {
        let server = MockServer::start().await;
        let json = serde_json::json!({
            "venue": "gmtrade",
            "reason": "Lowest total cost",
            "maxLeverage": 250.0,
            "expectedCostUsd": 0.45,
            "costBreakdown": {
                "openFee": 0.09, "closeFee": 0.09,
                "openSlip": 0.045, "closeSlip": 0.045,
                "borrow": 0.0675, "expectedLiqCost": 0.045,
                "pLiq": 0.0675, "total": 0.45
            },
            "clamped": false,
            "candidates": [
                {
                    "venue": "gmtrade",
                    "expectedCostUsd": 0.45,
                    "costBreakdown": {
                        "openFee": 0.09, "closeFee": 0.09,
                        "openSlip": 0.045, "closeSlip": 0.045,
                        "borrow": 0.0675, "expectedLiqCost": 0.045,
                        "pLiq": 0.0675, "total": 0.45
                    },
                    "maxLeverage": 250.0
                },
                {
                    "venue": "flash_trade",
                    "expectedCostUsd": 1.0,
                    "costBreakdown": {
                        "openFee": 0.2, "closeFee": 0.2,
                        "openSlip": 0.1, "closeSlip": 0.1,
                        "borrow": 0.15, "expectedLiqCost": 0.1,
                        "pLiq": 0.15, "total": 1.0
                    },
                    "maxLeverage": 113.0
                }
            ],
            "marketsVersion": 42
        });
        mock_route(&server, &json).await;

        // Construct ImperialClient → RouteCostOracle → best_route
        let client = ImperialClient::builder()
            .base_url(server.uri())
            .build()
            .expect("build");
        let config = test_config();
        let oracle = RouteCostOracle::new(config, client);

        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 1.0, 10.0).await;

        // Result uses Imperial-sourced venue data
        assert_ne!(result.venue_name, "flash-fallback", "should use Imperial venue, not flash-fallback");
        assert_eq!(result.venue_name, "gmtrade");
        assert!(!result.fallback, "should not be fallback");
        assert!((result.total_cost_usd - 0.45).abs() < 0.001);
    }

    // ── Live smoke test (marked #[ignore]) ─────────────────────────────────

    #[tokio::test]
    #[ignore]
    async fn live_smoke_test_route_oracle() {
        let client = ImperialClient::default_client();
        let config = test_config();
        let oracle = RouteCostOracle::new(config, client);

        let result = oracle.best_route("SOL", "long", 1000.0, 5.0, 2.0, 10.0).await;

        assert!(!result.fallback, "live call should not be fallback");
        assert!(!result.venue_name.is_empty(), "should have a venue");
        assert!(result.total_cost_usd > 0.0, "cost should be positive");
        assert!(result.confidence > 0.0, "confidence should be positive");
        println!(
            "Live route: venue={}, cost=${:.4}, improved={}, vetoed={}",
            result.venue_name, result.total_cost_usd, result.route_improved, result.vetoed
        );
    }
}
