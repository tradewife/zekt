//! Market data provider abstraction layer.
//!
//! Defines the `MarketDataProvider` trait for abstracting data sources
//! (Hyperliquid and Flash Trade), enabling strategy code and paper trading
//! engines to operate against different venues without code changes.
//!
//! Two implementations are provided:
//! - `HlDataProvider` — wraps `HlInfoClient`, uses HL fee model (0.035% taker,
//!   0.01%/hr borrow).
//! - `FlashDataProvider` — wraps `FlashClient` for backward compatibility
//!   with existing Flash Trade paper trading.

use anyhow::Result;
use std::sync::Arc;

use crate::flash_api::FlashClient;
use crate::funding_capture::FundingSnapshot;
use crate::hl_info::{HlFundingRate, HlInfoClient};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Hyperliquid taker fee rate: 0.035% of notional per side.
pub const HL_TAKER_FEE_RATE: f64 = 0.00035;

/// Hyperliquid maker fee rate: 0.01% of notional per side.
pub const HL_MAKER_FEE_RATE: f64 = 0.0001;

/// Hyperliquid hourly borrow rate: 0.01% of notional per hour.
pub const HL_BORROW_RATE_PER_HOUR: f64 = 0.0001;

/// Flash Trade default fee rate: 0.1% per side (from backtest default).
pub const FLASH_FEE_RATE: f64 = 0.001;

// ---------------------------------------------------------------------------
// Side enum
// ---------------------------------------------------------------------------

/// Trade side for fee estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    /// Parse from a human-readable string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "long" | "buy" | "b" => Some(Side::Long),
            "short" | "sell" | "a" => Some(Side::Short),
            _ => None,
        }
    }

    /// Returns the opposite side.
    pub fn invert(self) -> Self {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }

    /// String representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Long => "long",
            Side::Short => "short",
        }
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over market data sources (Hyperliquid, Flash Trade, mock).
///
/// Provides price data, funding rates, fee estimates, and borrow rates
/// needed by paper trading engines and strategy execution loops.
///
/// All async methods return `anyhow::Result` for consistent error handling.
///
/// Note: This trait uses `async fn` and is therefore NOT dyn-compatible.
/// Use generics or concrete types instead of `dyn MarketDataProvider`.
pub trait MarketDataProvider {
    /// Fetch the current mark price for a given market (e.g., "BTC").
    async fn get_price(&self, market: &str) -> Result<f64>;

    /// Fetch funding rates for all tracked markets.
    ///
    /// Each `FundingSnapshot` contains the annualized rate, raw rate,
    /// mark price, open interest, and a timestamp.
    async fn get_funding_rates(&self) -> Result<Vec<FundingSnapshot>>;

    /// Estimate the trading fee for a position of `notional` USD on the given side.
    ///
    /// Returns the fee amount in USD (e.g., $0.07 for a $200 notional at 0.035%).
    fn estimate_fee(&self, notional: f64, side: Side) -> f64;

    /// Return the current hourly borrow rate as a decimal fraction of notional.
    ///
    /// For Hyperliquid this is 0.0001 (0.01%/hr). For Flash Trade this is 0.0
    /// (FLash Trade doesn't charge borrow fees on perps).
    fn borrow_rate_per_hour(&self) -> f64;
}

// ---------------------------------------------------------------------------
// HlDataProvider
// ---------------------------------------------------------------------------

/// Hyperliquid data provider wrapping the existing `HlInfoClient`.
///
/// Uses HL fee schedule:
/// - Taker fee: 0.035% of notional
/// - Maker fee: 0.01% of notional
/// - Borrow rate: 0.01% of notional per hour
///
/// `get_price()` and `get_funding_rates()` both call `HlInfoClient::get_funding_rates()`
/// which returns the `metaAndAssetCtxs` endpoint data (includes `mark_px` for all markets).
#[derive(Debug, Clone)]
pub struct HlDataProvider {
    client: HlInfoClient,
}

impl HlDataProvider {
    /// Create a new HL data provider with the default HL Info API endpoint.
    pub fn new() -> Self {
        Self {
            client: HlInfoClient::default_client(),
        }
    }

    /// Create a new HL data provider with a custom base URL.
    pub fn with_url(base_url: &str) -> Self {
        Self {
            client: HlInfoClient::new(base_url),
        }
    }

    /// Returns a reference to the underlying `HlInfoClient`.
    pub fn client(&self) -> &HlInfoClient {
        &self.client
    }

    /// Convert an `HlFundingRate` to a `FundingSnapshot`.
    pub fn funding_rate_to_snapshot(rate: &HlFundingRate) -> FundingSnapshot {
        // HlFundingRate.annualized_funding is funding * 24 * 365 (hourly funding annualized)
        // Convert to percentage for FundingSnapshot
        let annualized_pct = rate.annualized_funding * 100.0;
        FundingSnapshot {
            coin: rate.coin.clone(),
            annualized_rate_pct: annualized_pct,
            raw_funding_rate: rate.funding,
            mark_px: rate.mark_px,
            open_interest_usd: rate.open_interest_usd,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            prev_day_px: rate.prev_day_px,
        }
    }
}

impl Default for HlDataProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketDataProvider for HlDataProvider {
    async fn get_price(&self, market: &str) -> Result<f64> {
        let rates = self.client.get_funding_rates().await?;
        let market_upper = market.to_uppercase();
        rates
            .iter()
            .find(|r| r.coin == market_upper)
            .map(|r| r.mark_px)
            .ok_or_else(|| anyhow::anyhow!("Market '{}' not found in HL funding rates", market_upper))
    }

    async fn get_funding_rates(&self) -> Result<Vec<FundingSnapshot>> {
        let rates = self.client.get_funding_rates().await?;
        Ok(rates
            .iter()
            .map(Self::funding_rate_to_snapshot)
            .collect())
    }

    fn estimate_fee(&self, notional: f64, _side: Side) -> f64 {
        // HL taker fee: 0.035% of notional
        notional * HL_TAKER_FEE_RATE
    }

    fn borrow_rate_per_hour(&self) -> f64 {
        HL_BORROW_RATE_PER_HOUR
    }
}

// ---------------------------------------------------------------------------
// FlashDataProvider
// ---------------------------------------------------------------------------

/// Flash Trade data provider wrapping the existing `FlashClient`.
///
/// Preserves existing behavior and provides backward compatibility for
/// paper trading against Flash Trade's API.
///
/// Fee model: Flash Trade charges ~0.1% per side (same as backtest default).
/// No hourly borrow fee (Flash Trade does not charge borrow on perps).
#[derive(Debug, Clone)]
pub struct FlashDataProvider {
    client: Arc<FlashClient>,
}

impl FlashDataProvider {
    /// Create a new Flash Trade data provider.
    pub fn new(client: Arc<FlashClient>) -> Self {
        Self { client }
    }
}

impl MarketDataProvider for FlashDataProvider {
    async fn get_price(&self, market: &str) -> Result<f64> {
        self.client.get_price(market).await
    }

    async fn get_funding_rates(&self) -> Result<Vec<FundingSnapshot>> {
        // Flash Trade doesn't have native funding rates. Return an empty vec
        // since funding capture is not applicable to Flash Trade.
        Ok(vec![])
    }

    fn estimate_fee(&self, notional: f64, _side: Side) -> f64 {
        // Flash Trade default fee: 0.1% per side
        notional * FLASH_FEE_RATE
    }

    fn borrow_rate_per_hour(&self) -> f64 {
        // Flash Trade does not charge borrow fees on perps
        0.0
    }
}

// ---------------------------------------------------------------------------
// MockDataProvider (for testing)
// ---------------------------------------------------------------------------

/// Mock data provider for unit testing.
///
/// Allows tests to inject pre-configured price and funding rate data
/// without requiring network access or real API calls.
#[derive(Debug, Clone)]
pub struct MockDataProvider {
    prices: std::collections::HashMap<String, f64>,
    funding_rates: Vec<FundingSnapshot>,
    fee_rate: f64,
    borrow_rate: f64,
}

impl MockDataProvider {
    /// Create a new mock data provider with the given price map and funding rates.
    pub fn new(prices: std::collections::HashMap<String, f64>, funding_rates: Vec<FundingSnapshot>) -> Self {
        Self {
            prices,
            funding_rates,
            fee_rate: HL_TAKER_FEE_RATE,
            borrow_rate: HL_BORROW_RATE_PER_HOUR,
        }
    }

    /// Create with custom fee and borrow rates.
    pub fn with_fees(mut self, fee_rate: f64, borrow_rate: f64) -> Self {
        self.fee_rate = fee_rate;
        self.borrow_rate = borrow_rate;
        self
    }
}

impl MarketDataProvider for MockDataProvider {
    async fn get_price(&self, market: &str) -> Result<f64> {
        self.prices
            .get(&market.to_uppercase())
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Market '{}' not found in mock data", market))
    }

    async fn get_funding_rates(&self) -> Result<Vec<FundingSnapshot>> {
        Ok(self.funding_rates.clone())
    }

    fn estimate_fee(&self, notional: f64, _side: Side) -> f64 {
        notional * self.fee_rate
    }

    fn borrow_rate_per_hour(&self) -> f64 {
        self.borrow_rate
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // =======================================================================
    // Side parsing
    // =======================================================================

    #[test]
    fn test_side_from_str_long_variants() {
        assert_eq!(Side::from_str("long"), Some(Side::Long));
        assert_eq!(Side::from_str("LONG"), Some(Side::Long));
        assert_eq!(Side::from_str("buy"), Some(Side::Long));
        assert_eq!(Side::from_str("B"), Some(Side::Long));
    }

    #[test]
    fn test_side_from_str_short_variants() {
        assert_eq!(Side::from_str("short"), Some(Side::Short));
        assert_eq!(Side::from_str("SHORT"), Some(Side::Short));
        assert_eq!(Side::from_str("sell"), Some(Side::Short));
        assert_eq!(Side::from_str("A"), Some(Side::Short));
    }

    #[test]
    fn test_side_from_str_invalid() {
        assert_eq!(Side::from_str("unknown"), None);
        assert_eq!(Side::from_str(""), None);
    }

    #[test]
    fn test_side_invert() {
        assert_eq!(Side::Long.invert(), Side::Short);
        assert_eq!(Side::Short.invert(), Side::Long);
    }

    #[test]
    fn test_side_as_str() {
        assert_eq!(Side::Long.as_str(), "long");
        assert_eq!(Side::Short.as_str(), "short");
    }

    // =======================================================================
    // HlDataProvider — fee constants
    // =======================================================================

    #[test]
    fn test_hl_taker_fee_calculation() {
        // Opening a $200 position accrues $0.07 entry fee at 0.035%
        let expected_fee: f64 = 200.0 * 0.00035; // = 0.07
        assert!(
            (expected_fee - 0.07_f64).abs() < 1e-10,
            "Expected fee $0.07, got ${}", expected_fee
        );
    }

    #[test]
    fn test_hl_taker_fee_round_trip() {
        // Round-trip: $200 * 0.00035 on entry + $200 * 0.00035 on exit = $0.14
        let notional = 200.0;
        let entry_fee = notional * HL_TAKER_FEE_RATE;
        let exit_fee = notional * HL_TAKER_FEE_RATE;
        let total = entry_fee + exit_fee;
        assert!((entry_fee - 0.07).abs() < 1e-10);
        assert!((exit_fee - 0.07).abs() < 1e-10);
        assert!((total - 0.14).abs() < 1e-10);
    }

    #[test]
    fn test_hl_borrow_rate_per_hour() {
        // 0.01%/hr = 0.0001
        assert!((HL_BORROW_RATE_PER_HOUR - 0.0001).abs() < 1e-15);
    }

    #[test]
    fn test_hl_borrow_fee_accrual_3_hours() {
        // $200 notional * 0.0001/hr * 3 hours = $0.06
        let notional = 200.0;
        let hours = 3.0;
        let borrow = notional * HL_BORROW_RATE_PER_HOUR * hours;
        assert!((borrow - 0.06).abs() < 1e-10);
    }

    // =======================================================================
    // HlDataProvider — funding_rate_to_snapshot conversion
    // =======================================================================

    #[test]
    fn test_funding_rate_to_snapshot_conversion() {
        let rate = HlFundingRate {
            coin: "BTC".to_string(),
            mark_px: 60000.0,
            funding: 0.0001,
            annualized_funding: 0.0001 * 24.0 * 365.0, // = 0.876
            open_interest_usd: 100_000_000.0,
            volume_24h_usd: 500_000_000.0,
            prev_day_funding: 0.00008,
            prev_day_px: 59500.0,
        };

        let snapshot = HlDataProvider::funding_rate_to_snapshot(&rate);
        assert_eq!(snapshot.coin, "BTC");
        assert!((snapshot.mark_px - 60000.0).abs() < 0.01);
        assert!((snapshot.raw_funding_rate - 0.0001).abs() < 1e-10);
        // annualized_funding is 0.876 as fraction, * 100 = 87.6%
        assert!((snapshot.annualized_rate_pct - 87.6).abs() < 0.01);
        assert!((snapshot.open_interest_usd - 100_000_000.0).abs() < 0.01);
        // timestamp should be recent
        let now = chrono::Utc::now().timestamp_millis();
        assert!(snapshot.timestamp_ms > 0);
        assert!(snapshot.timestamp_ms <= now);
    }

    #[test]
    fn test_funding_rate_to_snapshot_negative_funding() {
        let rate = HlFundingRate {
            coin: "ETH".to_string(),
            mark_px: 3000.0,
            funding: -0.00005,
            annualized_funding: -0.00005 * 24.0 * 365.0, // = -0.438
            open_interest_usd: 50_000_000.0,
            volume_24h_usd: 0.0,
            prev_day_funding: 0.0,
            prev_day_px: 0.0,
        };

        let snapshot = HlDataProvider::funding_rate_to_snapshot(&rate);
        assert_eq!(snapshot.coin, "ETH");
        assert!((snapshot.raw_funding_rate - (-0.00005)).abs() < 1e-10);
        // annualized_pct = -0.438 * 100 = -43.8%
        assert!((snapshot.annualized_rate_pct - (-43.8)).abs() < 0.01);
    }

    // =======================================================================
    // HlDataProvider — estimate_fee and borrow_rate_per_hour
    // =======================================================================

    #[test]
    fn test_hl_provider_estimate_fee() {
        let provider = HlDataProvider::new();
        let fee = provider.estimate_fee(1000.0, Side::Long);
        assert!((fee - 0.35).abs() < 1e-10); // 1000 * 0.00035 = 0.35

        let fee = provider.estimate_fee(1000.0, Side::Short);
        assert!((fee - 0.35).abs() < 1e-10); // same for short
    }

    #[test]
    fn test_hl_provider_estimate_fee_large_notional() {
        let provider = HlDataProvider::new();
        let fee = provider.estimate_fee(1_000_000.0, Side::Long);
        assert!((fee - 350.0).abs() < 1e-10); // $350 fee on $1M
    }

    #[test]
    fn test_hl_provider_borrow_rate() {
        let provider = HlDataProvider::new();
        assert!((provider.borrow_rate_per_hour() - 0.0001).abs() < 1e-15);
    }

    // =======================================================================
    // FlashDataProvider — estimate_fee and borrow_rate_per_hour
    // =======================================================================

    #[test]
    fn test_flash_provider_estimate_fee() {
        // We can't easily construct FlashClient without network, but we can
        // test the constants directly.
        let notional = 500.0;
        let fee = notional * FLASH_FEE_RATE;
        assert!((fee - 0.5).abs() < 1e-10); // 500 * 0.001 = 0.5
    }

    #[test]
    fn test_flash_fee_rate_vs_hl_fee_rate() {
        // Flash Trade fee (0.1%) is significantly higher than HL fee (0.035%)
        assert!(FLASH_FEE_RATE > HL_TAKER_FEE_RATE);
        assert!((FLASH_FEE_RATE - HL_TAKER_FEE_RATE - 0.00065).abs() < 1e-15);
    }

    // =======================================================================
    // MockDataProvider
    // =======================================================================

    #[test]
    fn test_mock_provider_get_price_found() {
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);
        prices.insert("ETH".to_string(), 3000.0);

        let provider = MockDataProvider::new(prices, vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let btc_price = rt.block_on(provider.get_price("BTC")).unwrap();
        assert!((btc_price - 60000.0).abs() < 0.01);

        let eth_price = rt.block_on(provider.get_price("ETH")).unwrap();
        assert!((eth_price - 3000.0).abs() < 0.01);
    }

    #[test]
    fn test_mock_provider_get_price_case_insensitive() {
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = MockDataProvider::new(prices, vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let price = rt.block_on(provider.get_price("btc")).unwrap();
        assert!((price - 60000.0).abs() < 0.01);
    }

    #[test]
    fn test_mock_provider_get_price_not_found() {
        let mut prices = HashMap::new();
        prices.insert("BTC".to_string(), 60000.0);

        let provider = MockDataProvider::new(prices, vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.get_price("SOL"));
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_provider_get_funding_rates() {
        let snapshots = vec![
            FundingSnapshot {
                coin: "BTC".to_string(),
                annualized_rate_pct: 25.0,
                raw_funding_rate: 0.0001,
                mark_px: 60000.0,
                open_interest_usd: 1_000_000.0,
                timestamp_ms: 1000,
                prev_day_px: 0.0,
            },
            FundingSnapshot {
                coin: "ETH".to_string(),
                annualized_rate_pct: 15.0,
                raw_funding_rate: 0.00005,
                mark_px: 3000.0,
                open_interest_usd: 500_000.0,
                timestamp_ms: 1000,
                prev_day_px: 0.0,
            },
        ];

        let provider = MockDataProvider::new(HashMap::new(), snapshots.clone());

        let rt = tokio::runtime::Runtime::new().unwrap();
        let rates = rt.block_on(provider.get_funding_rates()).unwrap();
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0].coin, "BTC");
        assert!((rates[0].annualized_rate_pct - 25.0).abs() < 0.01);
        assert_eq!(rates[1].coin, "ETH");
        assert!((rates[1].annualized_rate_pct - 15.0).abs() < 0.01);
    }

    #[test]
    fn test_mock_provider_get_funding_rates_empty() {
        let provider = MockDataProvider::new(HashMap::new(), vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let rates = rt.block_on(provider.get_funding_rates()).unwrap();
        assert!(rates.is_empty());
    }

    #[test]
    fn test_mock_provider_estimate_fee() {
        let provider = MockDataProvider::new(HashMap::new(), vec![]);
        let fee = provider.estimate_fee(200.0, Side::Short);
        assert!((fee - 0.07).abs() < 1e-10); // default HL fee of 0.00035
    }

    #[test]
    fn test_mock_provider_custom_fee_rate() {
        let provider = MockDataProvider::new(HashMap::new(), vec![])
            .with_fees(0.002, 0.00005);
        let fee = provider.estimate_fee(200.0, Side::Long);
        assert!((fee - 0.4).abs() < 1e-10); // 200 * 0.002 = 0.4
        assert!((provider.borrow_rate_per_hour() - 0.00005).abs() < 1e-15);
    }

    #[test]
    fn test_mock_provider_borrow_rate() {
        let provider = MockDataProvider::new(HashMap::new(), vec![]);
        assert!((provider.borrow_rate_per_hour() - 0.0001).abs() < 1e-15);
    }

    // =======================================================================
    // HlDataProvider construction and defaults
    // =======================================================================

    #[test]
    fn test_hl_provider_default() {
        let provider = HlDataProvider::default();
        // Just assert it constructs without panicking
        let _ = provider;
    }

    #[test]
    fn test_hl_provider_with_url() {
        let provider = HlDataProvider::with_url("https://api.hyperliquid.xyz/info");
        // Just assert it constructs without panicking
        let _ = provider;
    }

    // =======================================================================
    // Trait dispatch (verify trait methods work through concrete types)
    // =======================================================================

    #[test]
    fn test_mock_provider_trait_dispatch() {
        let mut prices = HashMap::new();
        prices.insert("SOL".to_string(), 150.0);

        let provider = MockDataProvider::new(prices, vec![]);

        let rt = tokio::runtime::Runtime::new().unwrap();

        // Use through trait methods directly on concrete type
        let price = rt.block_on(provider.get_price("SOL")).unwrap();
        assert!((price - 150.0).abs() < 0.01);

        let fee = provider.estimate_fee(100.0, Side::Long);
        assert!((fee - 0.035).abs() < 1e-10);

        let borrow = provider.borrow_rate_per_hour();
        assert!((borrow - 0.0001).abs() < 1e-15);
    }

    #[test]
    fn test_hl_provider_trait_dispatch() {
        let provider = HlDataProvider::default();
        // Verify trait methods work on concrete type
        let fee = provider.estimate_fee(200.0, Side::Short);
        assert!((fee - 0.07).abs() < 1e-10);
    }

    // =======================================================================
    // Edge cases: zero notional, negative side, etc.
    // =======================================================================

    #[test]
    fn test_estimate_fee_zero_notional() {
        let provider = HlDataProvider::new();
        let fee = provider.estimate_fee(0.0, Side::Long);
        assert!((fee - 0.0).abs() < 1e-15);
    }

    #[test]
    fn test_estimate_fee_very_small_notional() {
        let provider = HlDataProvider::new();
        let fee = provider.estimate_fee(1.0, Side::Long);
        assert!((fee - 0.00035).abs() < 1e-10);
    }

    #[test]
    fn test_borrow_fee_per_day() {
        // 0.01%/hr * 24h = 0.24%/day
        let daily_rate = HL_BORROW_RATE_PER_HOUR * 24.0;
        assert!((daily_rate - 0.0024).abs() < 1e-10);
        // $10,000 notional * 0.0024/day = $24/day
        let daily_cost = 10_000.0 * daily_rate;
        assert!((daily_cost - 24.0).abs() < 1e-10);
    }
}
