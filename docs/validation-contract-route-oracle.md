# Validation Contract — Route Cost Oracle + Blueprint Revalidation

**Mission:** Imperial Route Oracle + Liquidation-Zone Alpha Validation  
**Scope:** Workstream 2 (Route Cost Model) + Workstream 3 (Blueprint Revalidation)  
**Feature area IDs:** VAL-ROUTE-001 … VAL-ROUTE-055  
**Date:** 2026-05-30  

---

## 1. Route Cost Oracle — Core Construction

### VAL-ROUTE-001: RouteCostOracle Compiles and Exposes Correct Public Interface
`RouteCostOracle` in `src/route_cost.rs` must compile without errors and expose at minimum: `new(config)`, `best_route(market, side, size_usd)`, `is_stale()`, `fallback_cost(market, size_usd)`, and `last_refresh_time()`. Each method has a documented return type. The struct is `Send + Sync`.
Tool: cargo-build
Evidence: `cargo build --release` succeeds with no errors. `grep -c "pub fn" src/route_cost.rs` ≥ 5. Trait bound `Send + Sync` confirmed via `static_assertions::assert_impl_all!(RouteCostOracle: Send, Sync)` or manual inspection.

### VAL-ROUTE-002: RouteCostOracle Identifies Cheapest Venue for a Trade
Given route responses from multiple Solana perps venues (Flash, Phoenix, GMX, others via Imperial), `best_route("SOL", "long", 1000.0)` must return the venue with the lowest total cost (taker fee + borrow/funding + priority fee + liquidation-risk cost). The returned `RouteResult` must contain `venue_name`, `total_cost_usd`, `fee_breakdown`, and `confidence`.
Tool: cargo-test
Evidence: Unit test with mocked Imperial responses where venue A costs $0.50 and venue B costs $0.30. Assert `best_route()` returns venue B. Assert `total_cost_usd` equals mocked venue B total within ±0.001.

### VAL-ROUTE-003: Route Cost Breakdown Contains All Required Components
Every `RouteResult` returned by `best_route()` must include non-negative values for: `taker_open_fee_usd`, `taker_close_fee_usd`, `borrow_funding_usd` (estimated over expected hold), `priority_fee_usd`, `liquidation_risk_cost_usd`, and `total_cost_usd`. `total_cost_usd` must equal the sum of all component costs within floating-point tolerance (±0.0001).
Tool: cargo-test
Evidence: Call `best_route()` with mocked data. Assert each component ≥ 0.0. Assert `abs(total_cost_usd - (taker_open + taker_close + borrow + priority + liq_risk)) < 0.0001`.

### VAL-ROUTE-004: Route Cost Correctly Excludes Unsupported Markets
When `best_route()` is called for a market not available on any Imperial-connected venue, the oracle returns a structured `NoRouteAvailable` result (not a panic, not an empty result). The result includes `market` and `reason = "no_venue_support"`.
Tool: cargo-test
Evidence: Call `best_route("OBSCURE_TOKEN", "long", 100.0)` with mocked data where no venue lists this market. Assert result is `Err(NoRouteAvailable)` or equivalent. Assert no panic.

### VAL-ROUTE-005: Route Cost Respects Max Leverage Per Venue
If a venue caps leverage at 3x but the strategy requests 5x, the route must either adjust the position size down to fit within venue limits or exclude that venue from comparison. The returned `RouteResult` must indicate `leverage_adjusted: true` if a downward adjustment occurred, and the resulting effective size must respect the venue's max.
Tool: cargo-test
Evidence: Mock venue A with max_leverage=3, venue B with max_leverage=10. Request route for size=$1000 at 5x leverage. Assert venue A either returns `leverage_adjusted: true` with effective_size ≤ 3x collateral, or is excluded. Assert venue B is selected without adjustment if cheaper.

---

## 2. Cost Comparison Logic — Improvement Threshold

### VAL-ROUTE-006: route_improved Flag When Imperial Cost Is Lower by Threshold
When the Imperial route's total cost for a trade is lower than the current Flash-only model cost by ≥ the configured `improvement_threshold_bps` (e.g., 5 bps), the `RouteResult.route_improved` flag must be `true`. The improvement is computed as `((flash_cost - imperial_cost) / flash_cost) * 10000`.
Tool: cargo-test
Evidence: Flash cost = $1.00, Imperial cost = $0.995 (5 bps cheaper), threshold = 5 bps. Assert `route_improved == true`. If Imperial cost = $0.996 (4 bps cheaper), assert `route_improved == false`.

### VAL-ROUTE-007: Improvement Threshold Is Configurable via TOML
The `improvement_threshold_bps` value is read from `[backtest]` or `[route-oracle]` section in `config/perps.toml`. Default is 5 bps if not specified. Changing the value in config changes the behavior of `route_improved` without recompilation.
Tool: cargo-test + file-read
Evidence: Set `improvement_threshold_bps = 10` in config. Run comparison with 8 bps improvement. Assert `route_improved == false`. Set to 5. Assert `route_improved == true`. Assert `RouteCostOracle::new()` reads the value without error.

### VAL-ROUTE-008: No False Positives on route_improved When Costs Are Equal
When Imperial route cost equals Flash cost exactly (within ±0.001 USD), `route_improved` must be `false`. The improvement must strictly exceed the threshold.
Tool: cargo-test
Evidence: Flash cost = $1.000, Imperial cost = $1.000, threshold = 5 bps. Assert `route_improved == false`.

---

## 3. Cost Comparison Logic — Edge Budget Veto

### VAL-ROUTE-009: Trade Vetoed When Route Cost Exceeds Edge Budget
A trade has an expected edge (expected profit from strategy signal). If the total route cost exceeds the `edge_budget_usd` (or `edge_budget_pct` of expected profit), the `RouteResult.vetoed` flag must be `true` and the oracle must return a `Vetoed` status with `reason = "route_cost_exceeds_edge"`.
Tool: cargo-test
Evidence: Expected edge = $2.00, edge budget = 80%, so max allowable route cost = $1.60. Route cost = $2.50. Assert `vetoed == true`. Route cost = $1.00. Assert `vetoed == false`.

### VAL-ROUTE-010: Edge Budget Veto Is Configurable
The `edge_budget_pct` or `edge_budget_usd` is read from config. Default allows all trades (budget = 100% or a very high USD value). Setting `edge_budget_pct = 50` means route cost must be < 50% of expected edge for the trade to proceed.
Tool: cargo-test + file-read
Evidence: Set `edge_budget_pct = 50`. Run comparison with edge = $2.00, cost = $0.90 (45%). Assert `vetoed == false`. Cost = $1.10 (55%). Assert `vetoed == true`.

### VAL-ROUTE-011: Veto Count Is Tracked and Reported
The oracle maintains a counter of vetoed trades. After N calls where `vetoed == true`, `oracle.veto_count()` returns N. This counter is included in backtest output metrics.
Tool: cargo-test
Evidence: Call `best_route()` 10 times with 3 vetoes. Assert `oracle.veto_count() == 3`. Assert veto count appears in serialized backtest output.

---

## 4. Fallback Behavior — Stale or Missing Route Data

### VAL-ROUTE-012: Fallback to Flash Assumptions When Route Source Is Stale
When `is_stale()` returns `true` (route data older than `staleness_threshold_secs`), `best_route()` must return a result using Flash-only cost assumptions with `fallback = true` and `degradation_logged = true`. No Imperial-specific venue data is used for the decision.
Tool: cargo-test
Evidence: Set `last_refresh_time` to 10 minutes ago with `staleness_threshold_secs = 300`. Call `best_route()`. Assert `result.fallback == true`. Assert `result.venue_name == "flash-fallback"`. Assert a warn-level log was emitted containing "degradation" or "stale".

### VAL-ROUTE-013: Fallback to Flash Assumptions When Imperial API Returns Error
When the Imperial API call fails (network error, timeout, 5xx), the oracle must not propagate the error to the caller. Instead, it returns Flash-only cost with `fallback = true` and logs the error at warn level. The oracle continues to function for subsequent calls.
Tool: cargo-test
Evidence: Mock Imperial API to return a timeout error. Call `best_route()`. Assert result is `Ok(RouteResult { fallback: true, .. })`. Assert log contains "Imperial API error". Call again with working mock — assert normal operation resumes.

### VAL-ROUTE-014: Staleness Threshold Is Configurable
`staleness_threshold_secs` is read from config. Default is 60 seconds. The oracle tracks `last_refresh_time()` and compares against this threshold to determine staleness.
Tool: cargo-test
Evidence: Set `staleness_threshold_secs = 300`. Don't refresh for 200 seconds. Assert `is_stale() == false`. Wait 301 seconds (or mock time). Assert `is_stale() == true`.

### VAL-ROUTE-015: Degradation Counter Tracked for Monitoring
The oracle tracks the number of consecutive fallback calls. After `N` consecutive fallbacks (configurable, default 10), the oracle logs an error-level "sustained degradation" message. The counter resets to 0 on the next successful non-fallback call. The count is available via `oracle.degradation_count()`.
Tool: cargo-test
Evidence: Force 10 consecutive fallbacks. Assert `oracle.degradation_count() == 10`. Assert error-level log emitted. Perform a successful refresh. Assert `oracle.degradation_count() == 0`.

### VAL-ROUTE-016: Route Cache Prevents Redundant API Calls
For identical `(market, side, size_usd)` queries within the cache TTL, the oracle returns the cached result without making a new Imperial API call. Cache key is `(market, side, floor(size_usd / cache_bucket_usd))` where `cache_bucket_usd` groups similar sizes (default $100).
Tool: cargo-test
Evidence: Call `best_route("SOL", "long", 1050.0)` twice within TTL. Assert only 1 Imperial API call was made (mock call counter = 1). Call with `best_route("SOL", "long", 1150.0)` — same cache bucket → still 1 call. Call with `best_route("SOL", "long", 1250.0)` — new bucket → 2 calls.

---

## 5. Backtest Integration — Cost Mode Switching

### VAL-ROUTE-017: BacktestEngine Supports cost_mode = "flash-only"
When `BacktestConfig.cost_mode` is `"flash-only"` (default), the backtest engine uses the existing fee model: `fee_rate`, `borrow_rate_hourly`, `slippage_bps` from config. This is the current behavior and must remain unchanged.
Tool: cargo-test
Evidence: Run backtest with `cost_mode = "flash-only"`. Assert fee calculation matches existing formula: `entry_fee = size_usd * fee_rate`, `exit_fee = size_usd * fee_rate`, `slippage = size_usd * (slippage_bps / 10000)`. Results must be bit-identical to pre-oracle backtest for same inputs.

### VAL-ROUTE-018: BacktestEngine Supports cost_mode = "imperial-route-oracle"
When `BacktestConfig.cost_mode` is `"imperial-route-oracle"`, the backtest engine creates a `RouteCostOracle` and uses it for every trade's cost estimation instead of the flat `fee_rate`. Each trade's `entry_fee`, `exit_fee`, `borrow_fee`, and `slippage` are sourced from the oracle's `RouteResult` for the trade's market, side, and size.
Tool: cargo-test
Evidence: Run backtest with `cost_mode = "imperial-route-oracle"` and mocked oracle. Assert `BacktestCellStats.entry_fees_total` equals sum of oracle-provided entry fees across all trades. Assert same for exit_fees_total, borrow_fees_total.

### VAL-ROUTE-019: BacktestEngine Rejects Unknown cost_mode Values
If `cost_mode` is set to an unrecognized string (e.g., `"drift"` or `"jupiter"`), `BacktestEngine::new()` must return an error listing valid options: `"flash-only"`, `"imperial-route-oracle"`.
Tool: cargo-test
Evidence: Construct `BacktestConfig { cost_mode: "invalid".into(), .. }`. Assert `BacktestEngine::new()` returns `Err` containing "invalid cost_mode" and listing valid values.

### VAL-ROUTE-020: cost_mode Is Read from Config TOML
`cost_mode` is read from `[backtest]` section in `config/perps.toml`. Default is `"flash-only"` if absent. Valid values: `"flash-only"`, `"imperial-route-oracle"`.
Tool: cargo-test + file-read
Evidence: Add `cost_mode = "imperial-route-oracle"` to `[backtest]` in config. Parse config. Assert `bt_config.cost_mode == "imperial-route-oracle"`. Remove the key. Assert `bt_config.cost_mode == "flash-only"`.

---

## 6. Backtest Integration — Fee Accounting with Route Costs

### VAL-ROUTE-021: Entry Fee Uses Route Cost When Oracle Mode Active
In `imperial-route-oracle` mode, when a position is opened, `BtPosition.entry_fee` must be set to the oracle-returned `taker_open_fee_usd` (not `size_usd * flat_fee_rate`). This replaces the existing flat-fee calculation.
Tool: cargo-test
Evidence: Run backtest with oracle returning `taker_open_fee_usd = 0.35` for a $500 position. Assert `BtPosition.entry_fee == 0.35`. Compare with flash-only mode where `entry_fee = 500 * 0.001 = 0.50`. Assert they differ.

### VAL-ROUTE-022: Exit Fee Uses Route Cost When Oracle Mode Active
When a position is closed in `imperial-route-oracle` mode, the exit fee is sourced from the oracle's `taker_close_fee_usd` for the current market, side, and remaining size. The `BtTrade.exit_fee` field reflects this value.
Tool: cargo-test
Evidence: Close a position where oracle returns `taker_close_fee_usd = 0.28`. Assert `BtTrade.exit_fee == 0.28`. Assert `BacktestCellStats.exit_fees_total` includes this value.

### VAL-ROUTE-023: Borrow Fee Uses Route-Sourced Rate When Oracle Mode Active
In `imperial-route-oracle` mode, the borrow rate per hour for fee accrual is sourced from the oracle's `borrow_funding_rate_hourly` for the trade's market and side, replacing the config's flat `borrow_rate_hourly`. If the oracle doesn't provide a borrow rate, fall back to config value.
Tool: cargo-test
Evidence: Oracle returns `borrow_funding_rate_hourly = 0.0002` for SOL. Open position, run 10 ticks at 5-minute intervals. Assert `accrued_borrow_fee ≈ size_usd * 0.0002 * (5/3600) * 10`. Compare with flash-only mode using config `borrow_rate_hourly = 0.0001` — assert different values.

### VAL-ROUTE-024: Vetoed Trades Are Skipped in Backtest
When the oracle vetoes a trade (cost exceeds edge budget), the backtest must not open the position. The signal is logged as `vetoed` and the trade is excluded from the backtest's trade count and PnL calculations. The veto count is tracked per cell.
Tool: cargo-test
Evidence: Run backtest with 10 entry signals where oracle vetoes 3. Assert `cell.trade_count == 7`. Assert `cell.veto_count == 3` (new field on `BacktestCellStats`). Assert vetoed trades are not in the trade log.

### VAL-ROUTE-025: Fallback Trades Use Flash Costs in Backtest
When the oracle falls back to Flash assumptions (stale/missing data) during a backtest, the trade's cost is computed using Flash-only rates. The backtest tracks `fallback_count` per cell. The trade is included in results normally but flagged.
Tool: cargo-test
Evidence: Mock oracle to return stale for 2 out of 5 trades. Assert `cell.fallback_count == 2`. Assert the 2 fallback trades use Flash fee rates. Assert all 5 trades are in the trade log.

### VAL-ROUTE-026: route_improved Trades Are Tagged in Trade Log
Each `BtTrade` in `imperial-route-oracle` mode includes a `route_venue` field (string) and `route_improved` field (bool). When `route_improved == true`, the venue is the Imperial-recommended venue. When `false`, venue may still differ from Flash.
Tool: cargo-test
Evidence: Run backtest with mixed oracle results. Assert `BtTrade` records contain `route_venue` and `route_improved` fields. Assert values match oracle responses.

---

## 7. Backtest Integration — Backward Compatibility

### VAL-ROUTE-027: Flash-Only Mode Produces Identical Results to Pre-Oracle Build
Running the same backtest (same strategy, market, time range, config) with `cost_mode = "flash-only"` must produce numerically identical results to the same run on a pre-oracle build. Net PnL, trade count, fees, and Sharpe must match within floating-point tolerance (±0.0001).
Tool: cargo-test
Evidence: Capture results from existing backtest (pre-oracle). Run same backtest with `cost_mode = "flash-only"`. Assert `abs(pre_oracle_net_pnl - current_net_pnl) < 0.0001`. Assert `trade_count` is identical. Assert all `BtTrade` records match.

### VAL-ROUTE-028: All Existing Backtest Tests Pass Without Modification
All 17+ existing backtest unit tests (candle parsing, position PnL, fee accrual, synthetic replay, walk-forward, slippage, regime) must continue to pass without code changes. The new oracle code must not break any existing test.
Tool: cargo-test
Evidence: `cargo test backtest` passes all tests. `cargo test` passes all 711+ tests. No test regressions.

### VAL-ROUTE-029: BacktestCellStats New Fields Are Optional/Serde-Safe
New fields added to `BacktestCellStats` (e.g., `route_venue_counts`, `veto_count`, `fallback_count`, `cost_mode`) must have `#[serde(default)]` or `#[serde(skip_serializing_if = ...)]` so that existing serialized JSON results remain parseable.
Tool: cargo-test
Evidence: Deserialize a pre-oracle `BacktestCellStats` JSON (without new fields). Assert no error. Assert new fields have default values (0, empty hashmap, etc.).

### VAL-ROUTE-030: CLI Accepts Both Old and New Backtest Flags
Existing CLI flags (`--backtest`, `--strategies`, `--markets`, `--backtest-start`, `--backtest-interval`) continue to work unchanged. New flag `--cost-mode` is optional and defaults to `"flash-only"`.
Tool: cargo-test + shell
Evidence: Run `./target/release/zekt --backtest --strategies momentum-scalper --markets BTC --backtest-start 2026-05-15` without `--cost-mode`. Assert success with flash-only costs. Run with `--cost-mode imperial-route-oracle`. Assert success with oracle costs.

---

## 8. Blueprint Strategy Revalidation — Coverage

### VAL-ROUTE-031: All 10 Blueprint Strategies Run Through Imperial Route Cost Model
Running backtest with `--strategies blueprint-scalper,blueprint-mean-revert,blueprint-cluster-002,blueprint-cluster-003,blueprint-cluster-005,blueprint-cluster-006,blueprint-cluster-007,blueprint-cluster-008,blueprint-cluster-009,blueprint-hft-market-maker --cost-mode imperial-route-oracle` produces results for all 10 strategies across all requested markets. No strategy is skipped or errors out.
Tool: cargo-test + shell
Evidence: Run backtest with all 10 strategies on BTC, SOL, ETH. Assert output `BacktestResult.cells.len() == 30` (10 strategies × 3 markets). Assert each cell has `strategy` matching one of the 10 names and `net_pnl` is a finite number.

### VAL-ROUTE-032: blueprint-hft-market-maker Uses Realistic Cost Assumptions
The `blueprint-hft-market-maker` strategy must only be included in the revalidation if the cost model can represent realistic HFT costs (tight spreads, maker rebates, high fill rate). If the oracle only provides taker fees, the strategy must be flagged with `cost_assumptions_warning = "taker-only"` and results must note this limitation.
Tool: cargo-test
Evidence: Run backtest with `blueprint-hft-market-maker`. If maker fee data is unavailable from oracle, assert `cell.cost_assumptions_warning == "taker-only"`. If available, assert normal operation with maker fees applied.

### VAL-ROUTE-033: Each Blueprint Strategy Has Both Flash-Only and Imperial Results
For each of the 10 blueprint strategies, the validation must produce a paired result: one run with `cost_mode = "flash-only"` and one with `cost_mode = "imperial-route-oracle"`. Both runs use identical strategy parameters, markets, and time ranges.
Tool: cargo-test
Evidence: For each of 10 strategies, assert two `BacktestCellStats` entries exist (one per cost mode). Assert `strategy` and `market` match. Assert `cost_mode` differs. Assert parameters are identical.

---

## 9. Blueprint Strategy Revalidation — Metrics and Comparison

### VAL-ROUTE-034: Before/After Comparison Table Is Generated
The validation output includes a markdown or JSON table comparing each strategy's performance under flash-only vs imperial-route-oracle cost modes. Columns: strategy, market, flash_net_pnl, imperial_net_pnl, pnl_delta, flash_total_fees, imperial_total_fees, fee_delta, flash_sharpe, imperial_sharpe, veto_count, route_improved_count, venue_distribution.
Tool: file-read
Evidence: Output file `data/imperial-route-comparison.md` (or `.json`) exists. Assert all 10 strategies × markets have entries. Assert `pnl_delta = imperial_net_pnl - flash_net_pnl` for each row.

### VAL-ROUTE-035: Ranked Strategy Table Is Generated
A ranked table sorts all strategy-market combinations by `imperial_net_pnl` (descending). The top entry is the most profitable under the new cost model. The table includes: rank, strategy, market, net_pnl, total_fees, fee_bps, sharpe, max_drawdown, trade_count, veto_count.
Tool: file-read
Evidence: Output file contains a table with `rows.len() >= 10` (at least 10 strategy-market combos). Assert `rows[0].net_pnl >= rows[1].net_pnl`. Assert `fee_bps = (total_fees / abs(gross_pnl)) * 10000` for each row.

### VAL-ROUTE-036: Fee BPS Metric Is Computed Correctly
`fee_bps` is defined as `(total_fees / abs(gross_pnl)) * 10000` when `gross_pnl != 0`, else 0. For a strategy with `total_fees = $50` and `gross_pnl = $200`: `fee_bps = 2500`. This must be computed per cell and included in the comparison table.
Tool: cargo-test
Evidence: Construct `BacktestCellStats` with known fees and gross PnL. Assert computed `fee_bps` matches expected value. Assert `fee_bps` appears in serialized output.

### VAL-ROUTE-037: Venue Count Distribution Is Recorded
The comparison table includes a `venue_distribution` column showing how many trades were routed to each venue (e.g., `{"flash": 45, "phoenix": 12, "gmx": 3}`). This is aggregated from `BtTrade.route_venue` across all trades in the cell.
Tool: cargo-test
Evidence: Run backtest with mocked oracle returning varied venues. Assert `BacktestCellStats.venue_counts` hashmap sums to `trade_count`. Assert values appear in comparison table.

### VAL-ROUTE-038: Sharpe Ratio Computed Consistently Across Cost Modes
Sharpe ratio uses the same annualization formula in both cost modes: `(mean_return / std_dev_returns) * sqrt(periods_per_year)` where periods_per_year depends on candle interval. The only difference between modes should be the per-trade returns (which change due to different fee accounting).
Tool: cargo-test
Evidence: Run same strategy on same data with both cost modes. Assert Sharpe formula is identical — only inputs differ. If Flash fees are higher, assert `imperial_sharpe >= flash_sharpe` (better cost → higher Sharpe) for at least one strategy where fee savings are significant.

### VAL-ROUTE-039: Net PnL Difference Attributable to Route Cost Savings
For any strategy where `imperial_net_pnl > flash_net_pnl`, the difference must be explainable by route cost savings. Verify: `abs(pnl_delta - (flash_total_fees - imperial_total_fees)) < 0.01` — the PnL improvement equals the fee reduction (within rounding).
Tool: cargo-test
Evidence: For each strategy cell, compute `pnl_delta = imperial_net_pnl - flash_net_pnl` and `fee_delta = flash_total_fees - imperial_total_fees`. Assert `abs(pnl_delta - fee_delta) < 0.01`. This confirms the only change is cost, not strategy logic.

---

## 10. Walk-Forward Validation with Route Costs

### VAL-ROUTE-040: Walk-Forward Train/Test Split Works with Imperial Cost Mode
When `walk_forward_enabled = true` and `cost_mode = "imperial-route-oracle"`, the backtest splits candles into train/test at `walk_forward_train_ratio` and runs both phases using the oracle for cost estimation. Train and test results appear in separate `BacktestCellStats` entries.
Tool: cargo-test
Evidence: Run walk-forward backtest with 70/30 split. Assert `result.cells` contains train entries and `result.walk_forward_test_cells` contains test entries. Assert both use oracle costs (non-zero `venue_counts`). Assert train and test periods do not overlap.

### VAL-ROUTE-041: Walk-Forward Test Metrics Use Oracle Costs, Not Train Costs
The out-of-sample (test) walk-forward results must use the oracle's cost estimates for the test period, not carry over or average costs from the train period. Each test-period trade gets its own route cost lookup.
Tool: cargo-test
Evidence: Mock oracle to return different costs for train vs test timestamps. Assert test-period trades use test-period costs. Assert train-period costs don't leak into test results.

### VAL-ROUTE-042: Walk-Forward Results Include Cost Mode Label
Both train and test `BacktestCellStats` must include `cost_mode = "imperial-route-oracle"` so results are unambiguous when comparing across modes.
Tool: cargo-test
Evidence: Assert `result.cells[0].cost_mode == "imperial-route-oracle"`. Assert `result.walk_forward_test_cells[0].cost_mode == "imperial-route-oracle"`.

---

## 11. Regime Filter Comparison

### VAL-ROUTE-043: Regime Filter ON vs OFF Comparison Exists for Each Strategy
For each of the 10 blueprint strategies, the validation produces four result sets: (flash-only + regime ON), (flash-only + regime OFF), (imperial + regime ON), (imperial + regime OFF). This creates a 2×2 matrix per strategy per market.
Tool: cargo-test + shell
Evidence: Run 4 backtests per strategy-market. Assert all 4 combinations produce results. Assert `regime_filter` and `cost_mode` labels are correct in each.

### VAL-ROUTE-044: Regime Filter Interaction with Route Cost Is Consistent
The regime filter must not interfere with route cost estimation. Whether a trade is blocked by regime incompatibility or not, the route cost lookup for non-blocked trades must behave identically. Blocked trades are not sent to the oracle (no wasted API calls).
Tool: cargo-test
Evidence: Run backtest with regime filter ON and 10 entry signals, 4 blocked by regime. Assert oracle was called exactly 6 times (only for non-blocked signals). Assert veto count applies only to the 6 non-blocked trades.

### VAL-ROUTE-045: Regime-Blocked Count Is Preserved in Oracle Mode
The `regime_blocked_count` field in `BacktestCellStats` must be populated correctly in `imperial-route-oracle` mode, just as it is in `flash-only` mode. The count must be identical between modes for the same data and strategy (regime decisions don't depend on cost model).
Tool: cargo-test
Evidence: Run same backtest with both cost modes. Assert `cell.regime_blocked_count` is identical in both modes for each strategy-market cell.

---

## 12. Validation Window and Market Coverage

### VAL-ROUTE-046: Backtest Window Is at Least 90 Days
The revalidation runs on a minimum 90-day window where candle data is available. `end_time_ms - start_time_ms >= 90 * 86400 * 1000`. If data is unavailable for the full window, the actual window is logged and the shortfall is noted.
Tool: cargo-test
Evidence: Assert `bt_config.end_time_ms - bt_config.start_time_ms >= 90 * 86400 * 1000`. Log output includes "Backtest window: N days". If N < 90, assert warning is logged.

### VAL-ROUTE-047: Multiple Markets Validated (BTC, SOL, ETH Minimum)
The revalidation covers at least BTC, SOL, and ETH. Additional markets (e.g., high-scoring Flash/Imperial-supported markets from `scan-markets`) may be included. Each market produces independent backtest results.
Tool: shell + file-read
Evidence: Output comparison table has entries for "BTC", "SOL", "ETH" for each strategy. Assert at least 30 rows (10 strategies × 3 markets).

### VAL-ROUTE-048: No Strategy Promotion Without Positive Net Expectancy
No blueprint strategy is promoted (recommended for paper/live trading) unless its `imperial_net_pnl > 0.0` in the out-of-sample walk-forward test. Strategies with positive train PnL but negative test PnL are explicitly flagged as `"overfit"`.
Tool: file-read
Evidence: The ranked strategy table includes a `promotable` column (bool). Assert `promotable == true` only when `test_net_pnl > 0`. Assert `promotable == false` for all overfit strategies. Assert a "NOT PROMOTED" or "REJECTED" section lists strategies failing the threshold.

### VAL-ROUTE-049: Near-Break-Even Strategies Highlighted for Imperial Routing Impact
Strategies with flash-only net PnL in range `[-$50, +$50]` (near break-even) are highlighted in the comparison table. The report explicitly states whether Imperial routing turns any of these strategies positive.
Tool: file-read
Evidence: Comparison table includes a `near_break_even` flag. For highlighted strategies, `imperial_net_pnl` is compared to `flash_net_pnl`. If sign flips from negative to positive, the row is marked `imperial_routing_turned_positive = true`.

---

## 13. Config and Integration

### VAL-ROUTE-050: Route Oracle Config Section Exists in TOML
`config/perps.toml` includes a `[route-oracle]` section (or equivalent) with: `enabled` (bool), `improvement_threshold_bps` (f64), `edge_budget_pct` (f64), `staleness_threshold_secs` (u64), `cache_ttl_secs` (u64), `cache_bucket_usd` (f64), `excluded_venues` (array of strings).
Tool: file-read
Evidence: Parse `config/perps.toml`. Assert `[route-oracle]` section exists. Assert all listed keys are present with valid types. Assert defaults are documented.

### VAL-ROUTE-051: Route Oracle Disabled by Default
When `[route-oracle] enabled = false` (default), the system behaves exactly as before — no Imperial API calls, no route cost lookups, flash-only cost model everywhere. This ensures zero risk to existing behavior.
Tool: cargo-test
Evidence: Run full test suite with `enabled = false`. Assert no Imperial API calls attempted (mock call counter = 0). Assert backtest results match flash-only results exactly.

### VAL-ROUTE-052: Route Oracle Gracefully Handles Empty Imperial Response
If Imperial API returns a valid but empty route response (no venues for the requested market), the oracle falls back to Flash assumptions. No panic, no error propagated. Degradation is logged.
Tool: cargo-test
Evidence: Mock Imperial to return `{"routes": []}`. Call `best_route()`. Assert result uses Flash costs. Assert `fallback == true`. Assert degradation log emitted.

---

## 14. Output and Reporting

### VAL-ROUTE-053: BacktestResult Includes Route Oracle Summary Fields
`BacktestResult` (top-level) includes: `cost_mode` (string), `total_veto_count` (usize), `total_fallback_count` (usize), `venue_distribution` (HashMap<String, usize>), and `route_improved_count` (usize). These aggregate across all cells.
Tool: cargo-test
Evidence: Serialize `BacktestResult` from imperial-route-oracle backtest. Assert all fields present. Assert `total_veto_count == sum(cells[].veto_count)`. Assert `venue_distribution` sums to `total_trades`.

### VAL-ROUTE-054: Trade Log File Includes Route Information
`data/backtest-trades.json` (or equivalent) includes per-trade fields: `route_venue`, `route_cost_usd`, `route_improved`, `vetoed`, `fallback`. When `cost_mode = "flash-only"`, these fields are absent or set to default values.
Tool: cargo-test + file-read
Evidence: Run imperial backtest. Read trade log. Assert each trade has route fields. Run flash-only backtest. Assert route fields are absent or default.

### VAL-ROUTE-055: Atomic Writes for All New Output Files
All new output files (`imperial-route-comparison.md/json`, updated backtest results) use atomic writes (write to `.tmp` then rename), consistent with the existing trade journal pattern. No partial/corrupt files on crash.
Tool: code-review
Evidence: Grep for `rename` or `atomic_write` in the route comparison output code path. Assert pattern: write to `<path>.tmp`, then `std::fs::rename(tmp, final)`. Verify no `.tmp` files remain after clean run.

---

## Summary

| Area | ID Range | Count |
|------|----------|-------|
| Route Oracle — Core Construction | VAL-ROUTE-001 … VAL-ROUTE-005 | 5 |
| Cost Comparison — Improvement Threshold | VAL-ROUTE-006 … VAL-ROUTE-008 | 3 |
| Cost Comparison — Edge Budget Veto | VAL-ROUTE-009 … VAL-ROUTE-011 | 3 |
| Fallback — Stale/Missing Data | VAL-ROUTE-012 … VAL-ROUTE-016 | 5 |
| Backtest — Cost Mode Switching | VAL-ROUTE-017 … VAL-ROUTE-020 | 4 |
| Backtest — Fee Accounting with Route Costs | VAL-ROUTE-021 … VAL-ROUTE-026 | 6 |
| Backward Compatibility | VAL-ROUTE-027 … VAL-ROUTE-030 | 4 |
| Blueprint Revalidation — Coverage | VAL-ROUTE-031 … VAL-ROUTE-033 | 3 |
| Blueprint Revalidation — Metrics | VAL-ROUTE-034 … VAL-ROUTE-039 | 6 |
| Walk-Forward with Route Costs | VAL-ROUTE-040 … VAL-ROUTE-042 | 3 |
| Regime Filter Comparison | VAL-ROUTE-043 … VAL-ROUTE-045 | 3 |
| Validation Window & Markets | VAL-ROUTE-046 … VAL-ROUTE-049 | 4 |
| Config & Integration | VAL-ROUTE-050 … VAL-ROUTE-052 | 3 |
| Output & Reporting | VAL-ROUTE-053 … VAL-ROUTE-055 | 3 |
| **Total** | | **55** |

**Critical gates:**
- **VAL-ROUTE-028**: All existing tests pass — no regressions (backward compatibility).
- **VAL-ROUTE-031**: All 10 blueprint strategies run through the new cost model (coverage).
- **VAL-ROUTE-039**: PnL difference is fully explained by fee savings (correctness).
- **VAL-ROUTE-048**: No promotion without positive out-of-sample expectancy (safety).
- **VAL-ROUTE-049**: Near-break-even strategies evaluated for Imperial routing impact (mission success criterion).
