# Validation Contract: Imperial Read-Only Client (Milestone 1)

Feature area of the "Imperial Route Oracle + Liquidation-Zone Alpha Validation" mission.
Covers: `ImperialClient` struct in `src/imperial.rs`, all 10 public read-only endpoints, configuration, error handling, and auth boundary enforcement.

Base URL: `https://api.imperial.space`
All endpoints are unauthenticated GET requests. No JWT. No trading. No `/mobile/*`. No `/deposit/*`.

---

## 1. Client Construction & Configuration

### VAL-IMP-001: ImperialClient compiles and constructs with default base URL
`ImperialClient::new("https://api.imperial.space")` compiles without error and produces a usable client instance. The client stores the base URL internally and is `Clone + Send + Sync`.
Tool: cargo-test
Evidence: `let client = ImperialClient::new("https://api.imperial.space");` compiles. `assert!(!client.base_url().is_empty());` passes.

### VAL-IMP-002: ImperialClient provides a default_client() convenience constructor
`ImperialClient::default_client()` returns a client pre-configured with `base_url = "https://api.imperial.space"` and the standard timeout. Functionally identical to `ImperialClient::new("https://api.imperial.space")`.
Tool: cargo-test
Evidence: `let c = ImperialClient::default_client(); assert_eq!(c.base_url(), "https://api.imperial.space");`

### VAL-IMP-003: HTTP client timeout is configurable
The underlying `reqwest::Client` is built with a configurable timeout. The default is 30 seconds (matching the `FlashClient` and `HlInfoClient` patterns). A custom timeout (e.g., 10s) can be set via `ImperialClient::builder().timeout(Duration::from_secs(10)).build()`.
Tool: cargo-test
Evidence: Construct with a 1ms timeout and call any endpoint. Assert the call returns a timeout error (`reqwest::Error::is_timeout()`), proving the timeout is applied.

### VAL-IMP-004: Base URL is configurable for testing
`ImperialClient::new()` accepts any base URL string, enabling mock server testing. Passing `"http://localhost:0"` or a wiremock URL should not panic—only fail at request time.
Tool: cargo-test
Evidence: `ImperialClient::new("http://127.0.0.1:1")` succeeds in construction. A subsequent GET returns a connection error, not a panic.

### VAL-IMP-005: ImperialClient uses reqwest::Client internally (no blocking)
The `ImperialClient` holds a `reqwest::Client` (async), not a `reqwest::blocking::Client`. All public methods are `async fn` returning `anyhow::Result<T>`.
Tool: cargo-test
Evidence: Verify the struct contains `reqwest::Client` and all methods are `async`. Compile-time check: no `reqwest::blocking` import.

---

## 2. Route Endpoint (`/api/v1/route`)

### VAL-IMP-006: get_route() returns a recommended venue with full cost breakdown for SOL long
Calling `client.get_route("SOL", "long", 1000.0, 5.0).await` returns an `ImperialRouteResponse` containing: `venue` (non-empty string), `reason` (non-empty string), `expected_cost_usd` (positive f64), `cost_breakdown` with all 8 fields present (`open_fee`, `close_fee`, `open_slip`, `close_slip`, `borrow`, `expected_liq_cost`, `p_liq`, `total`), `candidates` (non-empty Vec), and `markets_version` (u64). The recommended venue is one of: `"gmtrade"`, `"phoenix"`, `"flash_trade"`, `"jupiter"`.
Tool: cargo-test + live smoke
Evidence: Parse live JSON response. Assert `venue` is one of the 4 known venues. Assert `cost_breakdown.total` is within ±0.01 of `expected_cost_usd`. Assert `candidates.len() >= 2`.

### VAL-IMP-007: get_route() returns multiple venue candidates sorted by cost
The `candidates` array contains at least 2 entries. Each candidate has a `venue` name, `expected_cost_usd`, `cost_breakdown`, and `max_leverage`. Candidates are sorted by `expected_cost_usd` ascending—the first candidate matches the top-level `venue` and `expected_cost_usd`.
Tool: cargo-test + live smoke
Evidence: Assert `candidates[0].venue == venue`. Assert `candidates[0].expected_cost_usd == expected_cost_usd`. Assert `candidates` is sorted ascending by `expected_cost_usd` (each `candidates[i].expected_cost_usd <= candidates[i+1].expected_cost_usd`).

### VAL-IMP-008: get_route() cost_breakdown fields are all non-negative
Every field in `cost_breakdown` (`open_fee`, `close_fee`, `open_slip`, `close_slip`, `borrow`, `expected_liq_cost`, `p_liq`, `total`) is >= 0.0. `total` equals the sum of the other 7 fields within floating-point tolerance (±0.0001).
Tool: cargo-test + live smoke
Evidence: Assert all 7 component fields >= 0.0. Assert `(total - sum_of_components).abs() < 0.0001`.

### VAL-IMP-009: get_route() works for BTC long with large notional
Calling `client.get_route("BTC", "long", 50000.0, 3.0).await` returns a valid response with `expected_cost_usd > 0`. BTC is supported on at least `flash_trade` and `gmtrade` venues.
Tool: cargo-test + live smoke
Evidence: Assert response is `Ok`. Assert `candidates.iter().any(|c| c.venue == "flash_trade" || c.venue == "gmtrade")`.

### VAL-IMP-010: get_route() works for ETH long
Calling `client.get_route("ETH", "long", 5000.0, 5.0).await` returns a valid response with a recommended venue and cost breakdown. ETH is supported on at least `phoenix` venue.
Tool: cargo-test + live smoke
Evidence: Assert response is `Ok`. Assert `expected_cost_usd > 0.0`. Assert at least one candidate exists.

### VAL-IMP-011: get_route() works for SOL short side
Calling `client.get_route("SOL", "short", 50000.0, 10.0).await` returns a valid response. The `venue` field is a known venue string. Short-side routing is supported and returns distinct cost estimates from the long side.
Tool: cargo-test + live smoke
Evidence: Assert `venue` is one of the 4 known venues. Assert `expected_cost_usd > 0.0`. Assert `candidates.len() >= 2`.

### VAL-IMP-012: get_route() returns max_leverage per candidate
Each candidate in the response has a `max_leverage` field (f64 > 0). The top-level `max_leverage` equals `candidates[0].max_leverage`. Leverage values vary by venue (e.g., Phoenix ~15, Jupiter ~100, Flash ~113, GMTrade ~250).
Tool: cargo-test + live smoke
Evidence: Assert `candidates.iter().all(|c| c.max_leverage > 0.0)`. Assert top-level `max_leverage == candidates[0].max_leverage`.

### VAL-IMP-013: get_route() returns clamped field
The response includes a `clamped` boolean field. When `desiredLeverage` exceeds a venue's max, `clamped` may be `true`. With a low leverage request (e.g., 1x), `clamped` is typically `false`.
Tool: cargo-test + live smoke
Evidence: Assert the `clamped` field is present and is a boolean. For a 1x leverage request, assert `clamped == false`.

### VAL-IMP-014: get_route() returns error for unsupported asset
Calling `client.get_route("INVALIDCOIN123", "long", 1000.0, 5.0).await` returns `Err` containing the message `"No venue supports INVALIDCOIN123"` or equivalent. The error is propagated via `anyhow::Result`.
Tool: cargo-test + live smoke
Evidence: Match on `Err(e)` and assert error message contains `"INVALIDCOIN123"` or `"No venue supports"`.

### VAL-IMP-015: get_route() returns error for missing required parameters
Calling the route endpoint with missing query parameters (e.g., no `asset`) returns an error. The API responds with `"Failed to deserialize query string: missing field asset"`. The client returns this as `Err`.
Tool: cargo-test + live smoke
Evidence: Directly call the endpoint without required params (or with empty strings). Assert `Err` is returned with message containing `"missing"`.

### VAL-IMP-016: get_route() sends no authentication headers
The HTTP GET request to `/api/v1/route` includes no `Authorization` header, no `Cookie` header, and no JWT. The request is a plain GET with query parameters only.
Tool: cargo-test
Evidence: Use a wiremock/mockito server to capture the request. Assert no `Authorization` header is present. Assert no `Cookie` header is present.

### VAL-IMP-017: get_route() response types are correctly deserialized
The `ImperialRouteResponse` struct uses serde to deserialize the JSON. Fields are correctly typed: `venue: String`, `reason: String`, `max_leverage: f64`, `expected_cost_usd: f64`, `cost_breakdown: ImperialCostBreakdown`, `clamped: bool`, `candidates: Vec<ImperialRouteCandidate>`, `markets_version: u64`.
Tool: cargo-test
Evidence: Parse a known JSON fixture into `ImperialRouteResponse` via `serde_json::from_str()`. Assert all fields match expected values. No deserialization error.

### VAL-IMP-018: ImperialCostBreakdown contains all 8 numeric fields
The `ImperialCostBreakdown` struct has fields: `open_fee: f64`, `close_fee: f64`, `open_slip: f64`, `close_slip: f64`, `borrow: f64`, `expected_liq_cost: f64`, `p_liq: f64`, `total: f64`. All are deserialized from camelCase JSON (`openFee`, `closeFee`, `openSlip`, `closeSlip`, `borrow`, `expectedLiqCost`, `pLiq`, `total`).
Tool: cargo-test
Evidence: Parse JSON fixture with known values. Assert each field matches. Use `#[serde(rename_all = "camelCase")]` or explicit `#[serde(rename = "...")]`.

---

## 3. Funding Rates Endpoint (`/api/v1/funding-rates`)

### VAL-IMP-019: get_funding_rates() returns non-empty array
Calling `client.get_funding_rates().await` returns `Ok(Vec<ImperialFundingRateRow>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `rows.len() > 0`.

### VAL-IMP-020: Funding rate rows contain symbol and venue-specific rate data
Each `ImperialFundingRateRow` has a `symbol: String` and one or more venue-specific rate objects (e.g., `flash: Option<ImperialVenueFundingRate>`, `gmtrade: Option<ImperialVenueFundingRate>`, `phoenix: Option<ImperialVenueFundingRate>`). At least one venue field is `Some` per row.
Tool: cargo-test + live smoke
Evidence: Assert each row has `symbol.is_empty() == false`. Assert at least one venue field is `Some` per row for the first 10 rows.

### VAL-IMP-021: ImperialVenueFundingRate has all required fields
Each `ImperialVenueFundingRate` contains: `source: String` (e.g., `"gmtrade_ws"`, `"fallback_constant"`, `"phoenix_market_ws"`), `long_funding_rate_per_hour_percent: Option<f64>`, `short_funding_rate_per_hour_percent: Option<f64>`, `long_borrow_rate_per_hour_percent: Option<f64>`, `short_borrow_rate_per_hour_percent: Option<f64>`. Fields may be `null` for some venues (e.g., Flash `longFundingRatePerHourPercent` is `null`).
Tool: cargo-test
Evidence: Parse JSON fixture. Assert `source` is non-empty. Assert `Option<f64>` fields deserialize correctly (some `Some`, some `None`).

### VAL-IMP-022: Funding rates for SOL are present
The response contains at least one row with `symbol == "SOL"` with funding/borrow rate data from at least one venue.
Tool: cargo-test + live smoke
Evidence: Assert `rows.iter().any(|r| r.symbol == "SOL")`. Assert the SOL row has at least one venue with `Some` rate data.

### VAL-IMP-023: Funding rates for BTC are present
The response contains at least one row with `symbol == "BTC"` with funding/borrow rate data from at least one venue.
Tool: cargo-test + live smoke
Evidence: Assert `rows.iter().any(|r| r.symbol == "BTC")`.

### VAL-IMP-024: Funding rate null values deserialize to None
When the API returns `null` for `longFundingRatePerHourPercent` (as Flash does), the Rust `Option<f64>` field deserializes to `None` rather than `0.0` or causing an error.
Tool: cargo-test
Evidence: Parse a JSON fixture containing `"longFundingRatePerHourPercent": null`. Assert the corresponding field is `None`.

---

## 4. Mark Prices Endpoint (`/api/v1/mark-prices`)

### VAL-IMP-025: get_mark_prices() returns non-empty array
Calling `client.get_mark_prices().await` returns `Ok(Vec<ImperialMarkPriceRow>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `rows.len() > 0`.

### VAL-IMP-026: Mark price rows contain symbol and venue-specific price data
Each `ImperialMarkPriceRow` has a `symbol: String` and one or more venue-specific price objects (e.g., `flash: Option<ImperialVenuePrice>`, `gmtrade: Option<ImperialVenuePrice>`, `phoenix: Option<ImperialVenuePrice>`).
Tool: cargo-test + live smoke
Evidence: Assert `symbol` is non-empty. Assert at least one venue field is `Some`.

### VAL-IMP-027: ImperialVenuePrice has all required fields
Each `ImperialVenuePrice` contains: `source: String`, `price: f64`, `fetched_at_unix_ms: u64`. The `price` is always positive for known assets.
Tool: cargo-test
Evidence: Parse fixture. Assert `source.is_empty() == false`. Assert `price > 0.0`. Assert `fetched_at_unix_ms > 0`.

### VAL-IMP-028: Mark prices for SOL and BTC are present and reasonable
The response contains rows for SOL and BTC. SOL price is > $50. BTC price is > $10,000.
Tool: cargo-test + live smoke
Evidence: Find SOL row, assert price > 50.0. Find BTC row, assert price > 10000.0.

### VAL-IMP-029: Mark price fetched_at_unix_ms is a recent timestamp
The `fetched_at_unix_ms` value for any venue is within 5 minutes of the current time (i.e., `now_ms - fetched_at < 300_000`). This indicates live data freshness.
Tool: cargo-test + live smoke
Evidence: Get response, find SOL row. Assert `fetched_at_unix_ms` is within 5 minutes of `SystemTime::now()`.

### VAL-IMP-030: Mark price source strings are recognized
The `source` field in `ImperialVenuePrice` is one of: `"flash_custody_oracle"`, `"gmtrade_ws"`, `"phoenix_orderbook_ws"`, `"jupiter_oracle"`, or another known source string. Unknown sources should log a warning but not error.
Tool: cargo-test
Evidence: Assert source matches a known pattern or is logged with `tracing::warn!`.

---

## 5. Phoenix Depth Endpoint (`/api/v1/phoenix/depth`)

### VAL-IMP-031: get_phoenix_depth() returns order book data for SOL-PERP
Calling `client.get_phoenix_depth("SOL-PERP").await` returns `Ok(ImperialPhoenixDepth)` with a `snapshots` field containing at least one market key (e.g., `"SOL"`).
Tool: cargo-test + live smoke
Evidence: Assert `snapshots.contains_key("SOL")` or equivalent.

### VAL-IMP-032: Phoenix depth snapshot contains bids and asks
Each snapshot entry has `symbol: String`, `mid: f64`, `bids: Vec<ImperialDepthLevel>`, and `asks: Vec<ImperialDepthLevel>`. Both `bids` and `asks` are non-empty arrays.
Tool: cargo-test + live smoke
Evidence: Assert `bids.len() > 0 && asks.len() > 0`.

### VAL-IMP-033: ImperialDepthLevel has price and size fields
Each depth level has `price: f64` and `size_base: f64`. For bids, `price < mid`. For asks, `price > mid` (if present). `size_base > 0.0`.
Tool: cargo-test + live smoke
Evidence: Assert bid prices < mid. Assert `size_base > 0.0` for all levels.

### VAL-IMP-034: Phoenix depth for BTC-PERP returns BTC snapshot
Calling `client.get_phoenix_depth("BTC-PERP").await` returns a response with a BTC snapshot containing bids and asks.
Tool: cargo-test + live smoke
Evidence: Assert `snapshots` contains `"BTC"` key. Assert bids and asks are non-empty.

### VAL-IMP-035: Phoenix depth for invalid market returns empty or error
Calling `client.get_phoenix_depth("INVALID-MARKET").await` returns either an empty `snapshots` map or an `Err`. No panic.
Tool: cargo-test + live smoke
Evidence: Assert result is `Ok` with empty snapshots or `Err`. No panic.

---

## 6. Phoenix Markets Endpoint (`/api/v1/phoenix/markets`)

### VAL-IMP-036: get_phoenix_markets() returns non-empty array
Calling `client.get_phoenix_markets().await` returns `Ok(Vec<ImperialPhoenixMarket>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `markets.len() > 0`.

### VAL-IMP-037: Phoenix market has all required fields
Each `ImperialPhoenixMarket` contains: `symbol: String`, `underwriter: String` (always `"phoenix"`), `orderbook: String` (Solana pubkey), `perp_asset_map: String`, `asset_id: u64`, `subaccount_index: u64`, `base_lots_decimals: u32`, `tick_size_in_quote_lots_per_base_lot: u64`, `maker_fee_micro: u64`, `taker_fee_micro: u64`, `max_leverage: f64`, `max_size_base_lots: u64`.
Tool: cargo-test
Evidence: Parse fixture. Assert all fields are present and non-default (e.g., `symbol` non-empty, `max_leverage > 0.0`, `orderbook` is a valid base58 string).

### VAL-IMP-038: Phoenix markets include SOL
The response contains at least one market with `symbol == "SOL"`. SOL has `max_leverage` approximately 15.0 (±2.0).
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().any(|m| m.symbol == "SOL")`. Assert SOL market's `max_leverage` is in range [10.0, 20.0].

### VAL-IMP-039: Phoenix market underwriter is always "phoenix"
All entries in the phoenix markets response have `underwriter == "phoenix"`.
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().all(|m| m.underwriter == "phoenix")`.

### VAL-IMP-040: Phoenix market fee fields are positive
For every market, `maker_fee_micro > 0` and `taker_fee_micro > 0`. `taker_fee_micro >= maker_fee_micro`.
Tool: cargo-test + live smoke
Evidence: Assert all markets have positive fee fields and taker >= maker.

---

## 7. Flash Markets Endpoint (`/api/v1/flash/markets`)

### VAL-IMP-041: get_flash_markets() returns non-empty array
Calling `client.get_flash_markets().await` returns `Ok(Vec<ImperialFlashMarket>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `markets.len() > 0`.

### VAL-IMP-042: Flash market has all required fields
Each `ImperialFlashMarket` contains: `symbol: String`, `side: String` (`"long"` or `"short"`), `underwriter: String` (`"flash_trade"`), `market_address: String`, `pool_address: String`, `pool_name: String`, `target_custody: String`, `target_mint: String`, `target_oracle: String`, `collateral_custody: String`, `collateral_mint: String`, `collateral_oracle: String`, `price_exponent: i32`, `token_decimals: u32`, `allow_open_position: bool`, `allow_close_position: bool`, `max_leverage: f64`, `open_position_fee_rate: f64`, `volatility_fee_rate: f64`, `max_conf_bps: u64`.
Tool: cargo-test
Evidence: Parse fixture. Assert all fields present. Assert `symbol` is non-empty. Assert `max_leverage > 0.0`.

### VAL-IMP-043: Flash markets include SOL long and short
The response contains at least one market with `symbol == "SOL"` and `side == "long"`, and at least one with `symbol == "SOL"` and `side == "short"`. SOL long has `max_leverage` approximately 120.0 (±30.0).
Tool: cargo-test + live smoke
Evidence: Assert SOL long exists. Assert SOL short exists. Assert SOL long `max_leverage > 50.0`.

### VAL-IMP-044: Flash market underwriter is always "flash_trade"
All entries have `underwriter == "flash_trade"`.
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().all(|m| m.underwriter == "flash_trade")`.

### VAL-IMP-045: Flash market fee rates are non-negative
`open_position_fee_rate >= 0.0` and `volatility_fee_rate >= 0.0` for all markets.
Tool: cargo-test + live smoke
Evidence: Assert both fee fields are non-negative for every market.

### VAL-IMP-046: Flash markets include BTC
The response contains at least one market with `symbol == "BTC"`.
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().any(|m| m.symbol == "BTC")`.

---

## 8. GMTrade Markets Endpoint (`/api/v1/gmtrade/markets`)

### VAL-IMP-047: get_gmtrade_markets() returns non-empty array
Calling `client.get_gmtrade_markets().await` returns `Ok(Vec<ImperialGmtradeMarket>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `markets.len() > 0`.

### VAL-IMP-048: GMTrade market has all required fields
Each `ImperialGmtradeMarket` contains: `symbol: String`, `underwriter: String` (`"gmtrade"`), `market: String` (Solana pubkey), `market_token_mint: String`, `index_token_mint: String`, `long_token_mint: String`, `short_token_mint: String`, `long_token_vault: String`, `short_token_vault: String`, `oracle: String`, `index_token_decimals: u32`, `closed: bool`.
Tool: cargo-test
Evidence: Parse fixture. Assert all fields present. Assert `symbol` non-empty. Assert `market` is a valid base58 string.

### VAL-IMP-049: GMTrade market underwriter is always "gmtrade"
All entries have `underwriter == "gmtrade"`.
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().all(|m| m.underwriter == "gmtrade")`.

### VAL-IMP-050: GMTrade markets include closed == false entries
At least some markets have `closed == false` (active markets).
Tool: cargo-test + live smoke
Evidence: Assert `markets.iter().any(|m| m.closed == false)`.

### VAL-IMP-051: GMTrade markets can have duplicate symbols with different market addresses
The same symbol (e.g., `"WIF"`) may appear multiple times with different `market` addresses. The client preserves all entries without deduplication.
Tool: cargo-test + live smoke
Evidence: Count entries per symbol. Assert at least one symbol has multiple entries (e.g., WIF typically has 3+).

---

## 9. GMTrade Liquidity Endpoint (`/api/v1/gmtrade/liquidity`)

### VAL-IMP-052: get_gmtrade_liquidity() returns non-empty array
Calling `client.get_gmtrade_liquidity().await` returns `Ok(Vec<ImperialGmtradeLiquidity>)` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `liquidity.len() > 0`.

### VAL-IMP-053: GMTrade liquidity row has all required fields
Each `ImperialGmtradeLiquidity` contains: `symbol: String`, `long_available_usd: f64`, `short_available_usd: f64`.
Tool: cargo-test
Evidence: Parse fixture. Assert `symbol` is non-empty. Assert both USD fields are present.

### VAL-IMP-054: GMTrade liquidity values are non-negative
`long_available_usd >= 0.0` and `short_available_usd >= 0.0` for all entries. Zero is valid (means no liquidity on that side).
Tool: cargo-test + live smoke
Evidence: Assert both fields >= 0.0 for every entry.

### VAL-IMP-055: GMTrade liquidity for BTC shows non-zero available on at least one side
The entry with `symbol == "BTC"` has `long_available_usd > 0.0` or `short_available_usd > 0.0`.
Tool: cargo-test + live smoke
Evidence: Find BTC entry. Assert `(long_available_usd + short_available_usd) > 0.0`.

### VAL-IMP-056: GMTrade liquidity for SOL is present
The response contains at least one entry with `symbol == "SOL"`.
Tool: cargo-test + live smoke
Evidence: Assert `liquidity.iter().any(|l| l.symbol == "SOL")`.

---

## 10. Priority Fee Endpoint (`/api/v1/priority-fee`)

### VAL-IMP-057: get_priority_fee() returns a priority fee value
Calling `client.get_priority_fee().await` returns `Ok(ImperialPriorityFee)` with `priority_fee: u64 > 0`.
Tool: cargo-test + live smoke
Evidence: Assert `priority_fee > 0`.

### VAL-IMP-058: Priority fee is a reasonable value
The returned `priority_fee` is typically between 1,000 and 10,000,000 micro-lamports. Values outside this range are logged with `tracing::warn!` but not rejected.
Tool: cargo-test + live smoke
Evidence: Assert `priority_fee > 0`. Assert `priority_fee < 100_000_000` (sanity bound). Log warning if outside [1_000, 10_000_000].

### VAL-IMP-059: Priority fee response has exactly one field
The JSON response is `{"priority_fee": <number>}`. No extra top-level fields.
Tool: cargo-test
Evidence: Parse minimal JSON `{"priority_fee": 500000}`. Assert successful parse with `priority_fee == 500000`.

---

## 11. Stats Markets Endpoint (`/api/v1/stats/markets`)

### VAL-IMP-060: get_stats_markets() returns valid stats response
Calling `client.get_stats_markets().await` returns `Ok(ImperialStatsMarkets)` with `period: String` (e.g., `"24h"`) and `rows: Vec<ImperialStatsRow>` with at least 1 entry.
Tool: cargo-test + live smoke
Evidence: Assert `period == "24h"`. Assert `rows.len() > 0`.

### VAL-IMP-061: Stats row has all required fields
Each `ImperialStatsRow` contains: `symbol: String`, `volume_usd: String` (numeric string), `open_interest_usd: String`, `long_oi_usd: String`, `short_oi_usd: String`, `trader_count: u64`, `position_count: u64`, `by_venue: ImperialVenueBreakdown` (with `jupiter_usd`, `flash_usd`, `phoenix_usd`, `gmtrade_usd` as Strings).
Tool: cargo-test
Evidence: Parse fixture. Assert all fields present. Assert `symbol` non-empty. Assert `volume_usd` parses to f64 >= 0.0.

### VAL-IMP-062: Stats include SOL and BTC
The response contains rows for SOL and BTC with non-zero volume or OI.
Tool: cargo-test + live smoke
Evidence: Find SOL row: assert `volume_usd.parse::<f64>().unwrap() > 0.0 || open_interest_usd.parse::<f64>().unwrap() > 0.0`. Same for BTC.

### VAL-IMP-063: Stats by_venue breakdown sums to approximately total volume
For each stats row, `jupiter_usd + flash_usd + phoenix_usd + gmtrade_usd` (parsed as f64) approximately equals `volume_usd` (within ±1.0 to account for rounding).
Tool: cargo-test + live smoke
Evidence: Assert `(venue_sum - volume).abs() < 1.0` for rows with volume > 0.

### VAL-IMP-064: Stats long_oi_usd + short_oi_usd approximately equals open_interest_usd
For each row with non-zero OI, `(long_oi_usd.parse::<f64>() + short_oi_usd.parse::<f64>() - open_interest_usd.parse::<f64>()).abs() < 1.0`.
Tool: cargo-test + live smoke
Evidence: Assert OI components sum correctly.

### VAL-IMP-065: Stats trader_count and position_count are non-negative integers
For every row, `trader_count >= 0` and `position_count >= 0`.
Tool: cargo-test + live smoke
Evidence: Assert both counts >= 0 for all rows.

---

## 12. Error Handling & Resilience

### VAL-IMP-066: Network connectivity failure returns Err with context
When the Imperial API is unreachable (e.g., DNS failure or connection refused), the client returns `Err` with an `anyhow::Error` containing the reqwest error. The error message includes sufficient context to identify the failed endpoint and URL.
Tool: cargo-test
Evidence: Point client at `"http://127.0.0.1:1"`. Call `get_route()`. Assert `Err`. Assert error message contains `"route"` or the URL.

### VAL-IMP-067: HTTP non-200 status code returns Err
If the API returns a non-200 status (e.g., 500, 429), the client returns `Err` with the HTTP status code in the error message.
Tool: cargo-test
Evidence: Use wiremock to return 500. Assert `Err` with message containing `"500"` or `"Internal Server Error"`.

### VAL-IMP-068: Malformed JSON response returns Err with parse context
If the API returns invalid JSON (e.g., `{{{`), the client returns `Err` with a serde parse error. The error message identifies the endpoint and that parsing failed.
Tool: cargo-test
Evidence: Use wiremock to return `{{{`. Assert `Err`. Assert error message contains `"parse"` or `"deserialize"`.

### VAL-IMP-069: Timeout returns Err with timeout indication
When a request exceeds the configured timeout, the client returns `Err` with a `reqwest::Error` where `is_timeout() == true`.
Tool: cargo-test
Evidence: Set timeout to 1ms. Call any endpoint. Assert `Err`. Assert `err.is_timeout()` or error chain contains timeout.

### VAL-IMP-070: Empty response body returns Err
If the API returns a 200 with an empty body, the client returns `Err` with a parse error (EOF while parsing).
Tool: cargo-test
Evidence: Use wiremock to return 200 with empty body. Assert `Err`.

### VAL-IMP-071: Unexpected JSON shape returns Err
If the API returns valid JSON but with unexpected structure (e.g., an object instead of an array for a list endpoint), the client returns `Err` with a type mismatch error.
Tool: cargo-test
Evidence: Use wiremock to return `{}` when an array is expected. Assert `Err`.

### VAL-IMP-072: All errors are logged with tracing::warn! or tracing::error!
When any API call fails, the client logs the failure using `tracing::warn!` or `tracing::error!` (consistent with the `flash_api.rs` and `hl_info.rs` patterns). The log includes the endpoint name and error message.
Tool: cargo-test
Evidence: Use `tracing_subscriber` to capture log output. Trigger a failure. Assert log line contains endpoint name and `"error"` or `"warn"`.

---

## 13. Auth & Security Boundary

### VAL-IMP-073: No method exists for /mobile/connect endpoint
The `ImperialClient` has no method that calls `/mobile/connect` or any `/mobile/*` endpoint. Compilation fails if such a method is attempted.
Tool: cargo-test (compile-time)
Evidence: Grep `ImperialClient` impl block. Assert no method references `/mobile/`.

### VAL-IMP-074: No method exists for /mobile/exchange endpoint
The `ImperialClient` has no method that calls `/mobile/exchange`.
Tool: cargo-test (compile-time)
Evidence: Grep for `/mobile/exchange` in `imperial.rs`. Assert 0 matches.

### VAL-IMP-075: No method exists for /mobile/orders endpoint
The `ImperialClient` has no method that calls `/mobile/orders`.
Tool: cargo-test (compile-time)
Evidence: Grep for `/mobile/orders` in `imperial.rs`. Assert 0 matches.

### VAL-IMP-076: No method exists for /deposit/build-tx endpoint
The `ImperialClient` has no method that calls `/deposit/build-tx` or any `/deposit/*` endpoint.
Tool: cargo-test (compile-time)
Evidence: Grep for `/deposit/` in `imperial.rs`. Assert 0 matches.

### VAL-IMP-077: No JWT token storage or generation
The `ImperialClient` struct has no field for storing JWT tokens, API keys, or auth credentials. No method accepts or generates JWTs.
Tool: cargo-test (compile-time)
Evidence: Inspect `ImperialClient` struct fields. Assert no `String` or `Vec<u8>` fields that could store tokens. Assert no method signature includes `token`, `jwt`, `auth`, or `credential`.

### VAL-IMP-078: No POST/PUT/DELETE methods on ImperialClient
All methods on `ImperialClient` use HTTP GET. No POST, PUT, or DELETE methods exist. This enforces the read-only constraint at the type level.
Tool: cargo-test (compile-time)
Evidence: Grep for `.post(`, `.put(`, `.delete(` in `imperial.rs`. Assert 0 matches.

### VAL-IMP-079: No Authorization header sent on any request
Across all 10 endpoints, no request includes an `Authorization` header. Verified by inspecting request construction (no `.header("Authorization", ...)` call).
Tool: cargo-test
Evidence: Use wiremock to capture all requests. Assert no `Authorization` header on any of the 10 endpoint calls.

---

## 14. Integration & Live Smoke Tests

### VAL-IMP-080: Live smoke test: all 10 endpoints return Ok
With a live internet connection, calling all 10 public endpoints sequentially returns `Ok` for each one. This validates DNS resolution, TLS, HTTP routing, and basic response parsing end-to-end.
Tool: cargo-test (marked `#[ignore]` for CI, run with `--ignored` flag)
Evidence: Call all 10 methods. Assert all return `Ok`. Log response sizes.

### VAL-IMP-081: Live smoke test: route endpoint for SOL returns flash_trade as a candidate
Calling `get_route("SOL", "long", 1000.0, 5.0)` returns a response where `candidates` includes `"flash_trade"` as one of the venue options.
Tool: cargo-test (marked `#[ignore]`)
Evidence: Assert `candidates.iter().any(|c| c.venue == "flash_trade")`.

### VAL-IMP-082: Live smoke test: mark prices for SOL are within reasonable range
Calling `get_mark_prices()` returns SOL mark price between $50 and $500 (generous range to avoid flakiness).
Tool: cargo-test (marked `#[ignore]`)
Evidence: Find SOL row, find any venue price, assert `50.0 < price < 500.0`.

### VAL-IMP-083: Live smoke test: funding rates for SOL include at least one venue
Calling `get_funding_rates()` returns a SOL row with at least one venue having `Some` funding/borrow rate data.
Tool: cargo-test (marked `#[ignore]`)
Evidence: Assert SOL row has at least one `Some` venue.

### VAL-IMP-084: Live smoke test: priority fee is a positive integer
Calling `get_priority_fee()` returns a value > 0.
Tool: cargo-test (marked `#[ignore]`)
Evidence: Assert `priority_fee > 0`.

### VAL-IMP-085: Live smoke test: stats markets includes multiple symbols
Calling `get_stats_markets()` returns at least 3 distinct symbols.
Tool: cargo-test (marked `#[ignore]`)
Evidence: Collect unique symbols from rows. Assert `len() >= 3`.

### VAL-IMP-086: Live smoke test: phoenix depth returns bids below mid
Calling `get_phoenix_depth("SOL-PERP")` returns SOL bids where every bid price < mid price.
Tool: cargo-test (marked `#[ignore]`)
Evidence: Find SOL snapshot. Assert all bid prices < mid.

---

## 15. Logging & Observability

### VAL-IMP-087: All successful API calls are logged with tracing::debug!
Every successful API call logs the endpoint, URL, and response time at `debug` level using `tracing::debug!`. This matches the pattern in `flash_api.rs` (`debug!("GET {}", url)`).
Tool: cargo-test
Evidence: Capture tracing output during a successful call. Assert log line contains endpoint path and `"GET"`.

### VAL-IMP-088: Slow API calls are logged with tracing::warn!
If an API call takes longer than a configurable threshold (default: 5 seconds), a `tracing::warn!` is emitted with the endpoint name and elapsed time.
Tool: cargo-test
Evidence: Set threshold to 0ms. Assert warn log emitted with endpoint name and elapsed time.

### VAL-IMP-089: API response HTTP status is always checked
The client verifies `resp.status().is_success()` before parsing JSON. Non-success status codes are logged and returned as `Err`.
Tool: cargo-test
Evidence: Wiremock returns 404. Assert `Err`. Assert warn/error log emitted with status code.

---

## 16. Response Type Integrity

### VAL-IMP-090: All serde structs derive Debug, Clone, Deserialize
Every response struct (`ImperialRouteResponse`, `ImperialCostBreakdown`, `ImperialRouteCandidate`, `ImperialFundingRateRow`, `ImperialVenueFundingRate`, `ImperialMarkPriceRow`, `ImperialVenuePrice`, `ImperialPhoenixDepth`, `ImperialDepthLevel`, `ImperialPhoenixMarket`, `ImperialFlashMarket`, `ImperialGmtradeMarket`, `ImperialGmtradeLiquidity`, `ImperialPriorityFee`, `ImperialStatsMarkets`, `ImperialStatsRow`, `ImperialVenueBreakdown`) derives `Debug`, `Clone`, and `Deserialize`.
Tool: cargo-test (compile-time)
Evidence: Attempt to call `.clone()` and `format!("{:?}", ...)` on each struct. Assert compilation succeeds.

### VAL-IMP-091: Serde uses camelCase rename for API JSON fields
All struct field names use snake_case in Rust but are mapped from camelCase JSON keys via `#[serde(rename_all = "camelCase")]`. For example, `expected_cost_usd` ← `"expectedCostUsd"`, `open_fee` ← `"openFee"`, `fetched_at_unix_ms` ← `"fetchedAtUnixMs"`.
Tool: cargo-test
Evidence: Parse JSON with camelCase keys. Assert no error. Assert fields populated correctly.

### VAL-IMP-092: Optional fields use Option<T> with serde default
Fields that may be absent or null (e.g., `long_funding_rate_per_hour_percent`, `collateral_custody_token_account`) are typed as `Option<T>` with `#[serde(default)]` on the containing struct, so missing fields deserialize to `None` instead of causing an error.
Tool: cargo-test
Evidence: Parse JSON fixture with known-null fields removed. Assert `Ok`. Assert fields are `None`.

### VAL-IMP-093: String-quoted numbers in stats endpoints are preserved as Strings
The stats endpoint returns volume/OI as string-quoted numbers (e.g., `"volumeUsd": "281137.198699"`). The Rust struct preserves these as `String` (not parsed to f64 at the struct level), deferring parse to the caller. This avoids precision loss.
Tool: cargo-test
Evidence: Parse fixture. Assert `volume_usd` is a `String`. Assert `"281137.198699"` round-trips exactly.

---

## 17. Config Integration

### VAL-IMP-094: Imperial base URL can be set via config TOML
The `[imperial]` section in `config/perps.toml` supports `base_url = "https://api.imperial.space"`. The `ImperialClient` reads this from the loaded config at startup.
Tool: cargo-test
Evidence: Create a TOML snippet `[imperial] base_url = "http://test.example.com"`. Parse config. Assert `config.imperial.base_url == "http://test.example.com"`.

### VAL-IMP-095: Imperial timeout can be set via config TOML
The `[imperial]` section supports `timeout_secs = 15`. The `ImperialClient` uses this value when constructing the reqwest client.
Tool: cargo-test
Evidence: Create TOML `[imperial] timeout_secs = 15`. Parse config. Assert `config.imperial.timeout_secs == 15`.

### VAL-IMP-096: Imperial config section is optional with sensible defaults
If the `[imperial]` section is absent from config, the system uses defaults: `base_url = "https://api.imperial.space"`, `timeout_secs = 30`. No config parsing error occurs.
Tool: cargo-test
Evidence: Parse config without `[imperial]` section. Assert default values are used.

---

## Summary

| Category | Count | IDs |
|---|---|---|
| Client Construction & Config | 5 | VAL-IMP-001 to VAL-IMP-005 |
| Route Endpoint | 13 | VAL-IMP-006 to VAL-IMP-018 |
| Funding Rates | 6 | VAL-IMP-019 to VAL-IMP-024 |
| Mark Prices | 6 | VAL-IMP-025 to VAL-IMP-030 |
| Phoenix Depth | 5 | VAL-IMP-031 to VAL-IMP-035 |
| Phoenix Markets | 5 | VAL-IMP-036 to VAL-IMP-040 |
| Flash Markets | 6 | VAL-IMP-041 to VAL-IMP-046 |
| GMTrade Markets | 5 | VAL-IMP-047 to VAL-IMP-051 |
| GMTrade Liquidity | 5 | VAL-IMP-052 to VAL-IMP-056 |
| Priority Fee | 3 | VAL-IMP-057 to VAL-IMP-059 |
| Stats Markets | 6 | VAL-IMP-060 to VAL-IMP-065 |
| Error Handling | 7 | VAL-IMP-066 to VAL-IMP-072 |
| Auth & Security | 7 | VAL-IMP-073 to VAL-IMP-079 |
| Live Smoke Tests | 7 | VAL-IMP-080 to VAL-IMP-086 |
| Logging | 3 | VAL-IMP-087 to VAL-IMP-089 |
| Response Types | 4 | VAL-IMP-090 to VAL-IMP-093 |
| Config Integration | 3 | VAL-IMP-094 to VAL-IMP-096 |
| **Total** | **96** | **VAL-IMP-001 to VAL-IMP-096** |
