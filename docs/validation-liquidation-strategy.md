# Liquidation Strategy Prototype + Replay Validation — Behavioral Assertions

**Mission:** Imperial Route Oracle + Liquidation-Zone Alpha Validation  
**Milestone:** M4 — Liquidation Strategy Prototype + Replay Validation  
**Area:** `liquidation-cascade-hunter` strategy  
**Status:** DRAFT  

---

## 1. Strategy Registration & Configuration

### VAL-STRAT-001: Strategy name registered in available_strategies()
The string `"liquidation-cascade-hunter"` appears in the `available_strategies()` return slice. Calling `available_strategies().contains(&"liquidation-cascade-hunter")` returns `true`.
- **Tool:** `cargo test` — unit test in strategy.rs calling `available_strategies()`
- **Evidence:** Assert `available_strategies().contains(&"liquidation-cascade-hunter")` is true

### VAL-STRAT-002: Factory function creates correct concrete type
Calling `create_strategy_from_config("liquidation-cascade-hunter", sub_table, fallback_params)` returns `Ok(Box<dyn Strategy>)` where `strategy.name()` equals `"liquidation-cascade-hunter"`. Passing an unknown name returns an error listing `liquidation-cascade-hunter` in the available strategies message.
- **Tool:** `cargo test` — unit test in strategy.rs
- **Evidence:** Factory returns Ok, `.name()` matches, unknown name error includes the strategy

### VAL-STRAT-003: Config disabled by default
The default `LiquidationCascadeParams` struct (or equivalent TOML sub-table) has `enabled: false`. When no `[strategy.liquidation-cascade-hunter]` section exists in config, the strategy is not activated in any engine (paper, backtest, or live). Paper engine and backtest engine skip strategy creation when `enabled` is false.
- **Tool:** `cargo test` — unit test verifying default struct field; integration test with minimal config
- **Evidence:** Default params have `enabled == false`; engines log "strategy disabled" and return no signals

### VAL-STRAT-004: Config must be explicitly enabled to activate
Only when the TOML config contains `enabled = true` under `[strategy.liquidation-cascade-hunter]` does the strategy produce entry signals. When `enabled = false` or absent, `detect_entry()` always returns `Signal::NoSignal` regardless of input data.
- **Tool:** `cargo test` — unit test with enabled=true vs enabled=false
- **Evidence:** With enabled=false, detect_entry returns NoSignal for all snapshots; with enabled=true, valid snapshots produce signals

### VAL-STRAT-005: Strategy parameters validate correctly
All parameter fields have valid ranges. `validate()` rejects: negative confidence_min, confidence_min > 1.0, negative volume_z_score_threshold, negative max_distance_to_zone_pct, negative spread_multiplier, negative depth_threshold, zero or negative tp_pct, zero or negative sl_pct, negative stale_data_threshold_secs, zero or negative route_cost_max_bps. Valid parameters pass `validate()` without error.
- **Tool:** `cargo test` — unit test covering valid defaults + boundary invalid cases
- **Evidence:** Valid defaults → Ok; each out-of-range field → Err with descriptive message

---

## 2. Cascade Continuation Entry

### VAL-STRAT-006: Cascade continuation entry — all gates pass → Long signal
Given a MomentumSnapshot where: (1) confidence ≥ confidence_min, (2) volume z-score ≥ threshold, (3) price is within max_distance_to_zone_pct of a liquidation zone below current price (forcing shorts to cover → price pushes up), (4) VWAP confirms direction (price above VWAP for long), (5) spread ≤ max_spread, (6) depth ≥ depth_threshold, (7) regime is compatible, (8) route cost ≤ route_cost_max_bps, (9) no pending trade for this symbol/side — then `detect_entry()` returns `Signal::MomentumLong` with positive strength.
- **Tool:** `cargo test` — unit test constructing snapshot meeting all criteria
- **Evidence:** Signal is MomentumLong, strength > 0

### VAL-STRAT-007: Cascade continuation entry — all gates pass → Short signal
Symmetric to VAL-STRAT-006 but for shorts: liquidation zone above current price (forcing longs to liquidate → price pushes down), price below VWAP, spread/depth/regime/route cost all pass. `detect_entry()` returns `Signal::MomentumShort`.
- **Tool:** `cargo test` — unit test with short-direction snapshot
- **Evidence:** Signal is MomentumShort, strength > 0

### VAL-STRAT-008: Cascade entry blocked — confidence below minimum
When all cascade continuation gates pass EXCEPT confidence < confidence_min, `detect_entry()` returns `Signal::NoSignal`. The strategy does not enter even if all other indicators are favorable.
- **Tool:** `cargo test` — unit test with confidence = confidence_min - epsilon
- **Evidence:** Returns NoSignal

### VAL-STRAT-009: Cascade entry blocked — volume z-score below threshold
When volume z-score < volume_z_score_threshold, no signal is generated regardless of other conditions. This prevents entering on low-conviction moves.
- **Tool:** `cargo test` — unit test with volume_z_score below threshold
- **Evidence:** Returns NoSignal

### VAL-STRAT-010: Cascade entry blocked — price too far from liquidation zone
When price distance to nearest liquidation zone exceeds max_distance_to_zone_pct, `detect_entry()` returns NoSignal. This ensures the strategy only trades when price is close enough to be affected by cascade flows.
- **Tool:** `cargo test` — unit test with price > max_distance_to_zone_pct away
- **Evidence:** Returns NoSignal

### VAL-STRAT-011: Cascade entry blocked — VWAP filter fails
For a potential long entry, if price is below VWAP, no signal is generated. For a potential short entry, if price is above VWAP, no signal is generated. The VWAP filter confirms institutional flow direction.
- **Tool:** `cargo test` — unit test with price on wrong side of VWAP
- **Evidence:** Returns NoSignal

### VAL-STRAT-012: Cascade entry blocked — spread too wide
When bid-ask spread exceeds max_spread (after multiplying by spread_multiplier), `detect_entry()` returns NoSignal. Wide spreads indicate low liquidity or high uncertainty, unsuitable for cascade entries.
- **Tool:** `cargo test` — unit test with spread > threshold
- **Evidence:** Returns NoSignal

### VAL-STRAT-013: Cascade entry blocked — depth too thin
When order book depth at the liquidation zone is below depth_threshold, no signal is generated. Thin depth means insufficient forced-flow magnitude to sustain a cascade.
- **Tool:** `cargo test` — unit test with depth < threshold
- **Evidence:** Returns NoSignal

### VAL-STRAT-014: Cascade entry blocked — regime incompatible
When the regime detector labels current conditions as incompatible with cascade hunting (e.g., LowVol or Choppy for a trend-dependent cascade), `detect_entry()` returns NoSignal. Regime compatibility must be explicitly checked.
- **Tool:** `cargo test` — unit test with incompatible regime label
- **Evidence:** Returns NoSignal

### VAL-STRAT-015: Cascade entry blocked — route cost exceeds veto threshold
When estimated route cost (in basis points) exceeds route_cost_max_bps, `detect_entry()` returns NoSignal even if all other gates pass. The Imperial Route Oracle cost is factored into the entry decision.
- **Tool:** `cargo test` — unit test with route_cost > route_cost_max_bps
- **Evidence:** Returns NoSignal

---

## 3. Exhaustion Reversal Entry

### VAL-STRAT-016: Exhaustion reversal entry — all gates pass → Long signal
After a liquidation cascade burst to the downside: (1) liquidation burst detected (forced-flow spike), (2) price reclaims VWAP (closes above VWAP), (3) order book depth refills at support, (4) forced-flow velocity decays toward zero, (5) spread normalizes to within threshold, (6) confidence ≥ minimum, (7) regime compatible, (8) route cost OK — then `detect_entry()` returns `Signal::MomentumLong` (buy the reversal).
- **Tool:** `cargo test` — unit test constructing exhaustion reversal snapshot
- **Evidence:** Signal is MomentumLong, strength > 0

### VAL-STRAT-017: Exhaustion reversal entry — all gates pass → Short signal
Symmetric to VAL-STRAT-016 for upside exhaustion: cascade burst to the upside, price falls back below VWAP, depth refills at resistance, velocity decays, spread normalizes. `detect_entry()` returns `Signal::MomentumShort`.
- **Tool:** `cargo test` — unit test with short-direction exhaustion snapshot
- **Evidence:** Signal is MomentumShort, strength > 0

### VAL-STRAT-018: Exhaustion reversal blocked — no liquidation burst detected
If no recent liquidation burst is detected (forced-flow spike absent), no exhaustion signal is generated. This prevents "knife catching" without confirmation that a cascade actually occurred.
- **Tool:** `cargo test` — unit test with no burst in recent data
- **Evidence:** Returns NoSignal

### VAL-STRAT-019: Exhaustion reversal blocked — price fails to reclaim VWAP
For a long reversal, if price remains below VWAP, no signal is generated. VWAP reclamation is the primary confirmation that the cascade has exhausted and smart money is re-entering.
- **Tool:** `cargo test` — unit test with price still below VWAP
- **Evidence:** Returns NoSignal

### VAL-STRAT-020: Exhaustion reversal blocked — depth not refilled
If order book depth at the key level has not recovered to at least depth_threshold, no signal is generated. Unfilled depth means the cascade may not be exhausted.
- **Tool:** `cargo test` — unit test with depth still below threshold
- **Evidence:** Returns NoSignal

### VAL-STRAT-021: Exhaustion reversal blocked — velocity not decaying
If forced-flow velocity is still elevated (not decaying), no signal is generated. Continued high velocity means the cascade is still active, not exhausted.
- **Tool:** `cargo test` — unit test with velocity still above decay threshold
- **Evidence:** Returns NoSignal

### VAL-STRAT-022: Exhaustion reversal blocked — spread still elevated
If bid-ask spread has not normalized (still above max_spread), no exhaustion reversal signal is generated. Elevated spread indicates continued uncertainty.
- **Tool:** `cargo test` — unit test with spread still wide
- **Evidence:** Returns NoSignal

---

## 4. Duplicate & Concurrency Guards

### VAL-STRAT-023: Max one pending trade per symbol/side
After generating a signal for SOL/Long, the strategy must not generate another SOL/Long signal while the first is still pending (not filled, not expired, not cancelled). A second call to `detect_entry()` with the same symbol/side combination returns NoSignal. A signal for SOL/Short or BTC/Long is not blocked.
- **Tool:** `cargo test` — unit test calling detect_entry twice with same symbol/side
- **Evidence:** Second call returns NoSignal; different symbol/side returns signal

### VAL-STRAT-024: Pending state cleared after position opens
Once a pending signal transitions to an open position (filled), the pending counter for that symbol/side is decremented. A new signal for the same symbol/side can then be generated.
- **Tool:** `cargo test` — unit test: signal → fill → detect_entry again
- **Evidence:** Second detect_entry after fill returns a signal

### VAL-STRAT-025: Pending state cleared after signal expires
If a pending signal expires (time-based expiry or next tick without fill), the pending state is cleared and a new signal may be generated for the same symbol/side.
- **Tool:** `cargo test` — unit test: signal → expiry → detect_entry again
- **Evidence:** After expiry, detect_entry returns a signal for same symbol/side

---

## 5. Mandatory TP + SL Enforcement

### VAL-STRAT-026: Every entry signal has TP and SL defined
Every `Signal::MomentumLong` or `Signal::MomentumShort` produced by the strategy must have associated take-profit and stop-loss levels. The strategy never produces a "naked" entry without risk parameters. TP and SL are computed from the strategy's parameters and embedded in the signal or position context.
- **Tool:** `cargo test` — unit test verifying all output signals have TP > 0 and SL > 0
- **Evidence:** Assert TP and SL fields are populated for every non-NoSignal output

### VAL-STRAT-027: TP is positive (profit target above entry for longs, below for shorts)
For a long signal, TP price > entry price. For a short signal, TP price < entry price. The absolute TP percentage equals take_profit_pct from strategy parameters.
- **Tool:** `cargo test` — unit test checking TP direction and magnitude
- **Evidence:** Long TP > entry, Short TP < entry, |TP% - take_profit_pct| < epsilon

### VAL-STRAT-028: SL is positive (stop loss below entry for longs, above for shorts)
For a long signal, SL price < entry price. For a short signal, SL price > entry price. The absolute SL percentage equals stop_loss_pct from strategy parameters.
- **Tool:** `cargo test` — unit test checking SL direction and magnitude
- **Evidence:** Long SL < entry, Short SL > entry, |SL% - stop_loss_pct| < epsilon

---

## 6. Exit Logic

### VAL-STRAT-029: Take-profit exit triggers correctly
When current price reaches or exceeds the TP level for an open position, `detect_exit()` returns `Some(Signal::ExitLong { reason: TakeProfit })` (or ExitShort). This fires before any other exit check.
- **Tool:** `cargo test` — unit test with price at TP level
- **Evidence:** Returns ExitLong/ExitShort with reason TakeProfit

### VAL-STRAT-030: Stop-loss exit triggers correctly
When current price reaches or breaches the SL level for an open position, `detect_exit()` returns `Some(Signal::ExitLong { reason: StopLoss })`. SL check fires after TP check (TP takes priority if both hit simultaneously).
- **Tool:** `cargo test` — unit test with price at SL level
- **Evidence:** Returns ExitLong/ExitShort with reason StopLoss

### VAL-STRAT-031: Trailing stop exit triggers correctly
For a long position where price has moved favorably beyond trailing_activation_pct from entry: peak_price is tracked. When current_price drops by trailing_stop_pct from peak_price, `detect_exit()` returns `Some(Signal::ExitLong { reason: TrailingStop })`. For shorts, peak_price is the lowest price, and trailing triggers when price rises by trailing_stop_pct from peak.
- **Tool:** `cargo test` — unit test simulating price rise then reversal
- **Evidence:** Returns ExitLong with TrailingStop for longs; ExitShort with TrailingStop for shorts

### VAL-STRAT-032: Trailing stop does not trigger before activation
When price has moved less than trailing_activation_pct from entry, trailing stop is not active. `detect_exit()` returns None even if a small reversal occurs (as long as SL is not breached).
- **Tool:** `cargo test` — unit test with price below activation threshold
- **Evidence:** Returns None

### VAL-STRAT-033: Time stop exit triggers correctly
When a position has been held for longer than max_hold_secs, `detect_exit()` returns `Some(Signal::ExitLong { reason: TimeStop })` regardless of PnL state. This prevents capital from being locked in stale positions.
- **Tool:** `cargo test` — unit test with hold_secs > max_hold_secs
- **Evidence:** Returns ExitLong/ExitShort with reason TimeStop

### VAL-STRAT-034: Exit priority order: TP > SL > Trailing > TimeStop
When multiple exit conditions are simultaneously met, the exit with highest priority is returned: (1) TakeProfit, (2) StopLoss, (3) TrailingStop, (4) TimeStop. This ordering ensures profit-taking takes precedence over risk-reduction.
- **Tool:** `cargo test` — unit test with multiple exit conditions simultaneously true
- **Evidence:** Highest-priority reason is returned

### VAL-STRAT-035: No exit when position is healthy
When price is between SL and TP, trailing stop is not triggered, and hold time is within limits, `detect_exit()` returns None. The position is held.
- **Tool:** `cargo test` — unit test with price in healthy range
- **Evidence:** Returns None

---

## 7. Stale Data Detection & Forced Exit

### VAL-STRAT-036: Stale zone data forces exit
When the liquidation zone data used for the entry decision becomes older than stale_data_threshold_secs, `detect_exit()` returns `Some(Signal::ExitLong { reason: ReversalDetected })` (or equivalent stale-data reason). The strategy does not hold positions based on outdated liquidation zone intelligence.
- **Tool:** `cargo test` — unit test with zone_data_age > stale_data_threshold_secs
- **Evidence:** Returns exit signal with stale-data reason

### VAL-STRAT-037: Fresh data does not trigger stale exit
When zone data age is less than stale_data_threshold_secs, no stale-data exit is triggered. The position continues to be held (assuming no other exit condition is met).
- **Tool:** `cargo test` — unit test with zone_data_age < threshold
- **Evidence:** Returns None

### VAL-STRAT-038: Stale exit is independent of PnL
A stale-data exit is forced even if the position is in profit. The rationale is that the original thesis (liquidation zone proximity) is no longer valid, and holding is pure speculation.
- **Tool:** `cargo test` — unit test with stale data + profitable position
- **Evidence:** Returns exit signal despite positive PnL

### VAL-STRAT-039: Stale data prevents new entries
When the most recent liquidation zone data is older than stale_data_threshold_secs, no new entry signals are generated for either cascade continuation or exhaustion reversal. The strategy effectively pauses until fresh zone data arrives.
- **Tool:** `cargo test` — unit test with stale zone data + otherwise valid snapshot
- **Evidence:** Returns NoSignal for both cascade and exhaustion setups

---

## 8. Paper-Only Enforcement

### VAL-STRAT-040: Strategy cannot execute live trades
When `liquidation-cascade-hunter` is used with the live engine (`engine.rs`), the engine must refuse to create or use this strategy. The live engine either skips it or returns an error. Alternatively, the strategy's config contains a `paper_only: true` flag that the live engine checks.
- **Tool:** `cargo test` — unit test in engine.rs attempting to create live strategy
- **Evidence:** Engine returns error or skips strategy; log message indicates paper-only restriction

### VAL-STRAT-041: Paper engine executes normally
When `liquidation-cascade-hunter` is used with the paper engine (`paper.rs`), all signals, entries, and exits function normally. The paper engine creates the strategy and processes signals without restriction.
- **Tool:** `cargo test` — unit test in paper.rs with the strategy
- **Evidence:** Paper engine creates strategy, processes signals, tracks virtual PnL

### VAL-STRAT-042: Backtest engine executes normally
When `liquidation-cascade-hunter` is used with the backtest engine (`backtest.rs`), all signals function normally. Backtesting is allowed because it is offline and uses historical data.
- **Tool:** `cargo test` — unit test in backtest.rs with the strategy
- **Evidence:** Backtest engine creates strategy, produces BacktestResult

---

## 9. Route Cost Veto Integration

### VAL-STRAT-043: Route cost is checked before signal emission
Route cost estimation happens inside `detect_entry()` before returning a signal. It is not a post-hoc filter. When route cost exceeds the veto threshold, the signal is suppressed at the source.
- **Tool:** `cargo test` — unit test verifying route cost check happens during detect_entry
- **Evidence:** With high route cost, detect_entry returns NoSignal (not a signal that gets filtered later)

### VAL-STRAT-044: Route cost zero or missing does not veto
When route cost data is unavailable (None or 0.0), the strategy does NOT block entry. Route cost veto only activates when a positive cost estimate exceeds the threshold. Missing data should not prevent trading — it degrades gracefully.
- **Tool:** `cargo test` — unit test with None/0.0 route cost
- **Evidence:** Signal is generated when all other gates pass and route cost is absent

### VAL-STRAT-045: Route cost below threshold allows entry
When route cost is positive but below route_cost_max_bps, entry is allowed (assuming all other gates pass). The strategy does not block entries for reasonable route costs.
- **Tool:** `cargo test` — unit test with route_cost = route_cost_max_bps - epsilon
- **Evidence:** Signal is generated

---

## 10. Replay Validation Pipeline

### VAL-STRAT-046: Replay pipeline loads captured data
The replay pipeline reads captured liquidation zone data (from a JSON or binary file) and constructs MomentumSnapshots with the exact same fields as live data. The replayed snapshots include: price, volume, VWAP, spread, depth, liquidation zone distance, forced-flow velocity, regime label, and route cost.
- **Tool:** `cargo test` — unit test loading a fixture file and verifying snapshot construction
- **Evidence:** N snapshots loaded; each field matches fixture data within tolerance

### VAL-STRAT-047: Replay produces deterministic results
Given the same captured data file, the replay pipeline produces identical results on every run. No randomness, no time-dependent state. This is verified by running the replay twice and comparing trade logs signal-by-signal.
- **Tool:** `cargo test` — integration test running replay twice
- **Evidence:** Trade logs are byte-identical between runs

### VAL-STRAT-048: Replay compares against no-trade baseline
The replay pipeline computes two results: (1) strategy trades applied to the captured data, (2) a no-trade baseline (hold or flat). The comparison shows net PnL, max drawdown, trade count, and Sharpe ratio for both scenarios. The strategy must beat the baseline to be considered for promotion.
- **Tool:** `cargo test` — integration test running both paths
- **Evidence:** Both result objects exist; comparison function returns delta metrics

### VAL-STRAT-049: Replay respects all entry gates
During replay, the strategy's entry gates (confidence, volume, VWAP, spread, depth, regime, route cost, duplicate blocking) are all enforced. No phantom trades appear that would have been blocked in live mode. This is verified by checking that every trade in the replay log has a corresponding snapshot that passes all gate checks.
- **Tool:** `cargo test` — integration test verifying each replay trade's gate snapshot
- **Evidence:** 100% of replay trades have passing gate snapshots; no trades occur where gates fail

### VAL-STRAT-050: Replay respects all exit conditions
During replay, TP, SL, trailing stop, time stop, and stale-data exits are all enforced. Every closed trade has a valid exit reason. No position is left open at the end of replay without a final forced exit.
- **Tool:** `cargo test` — integration test verifying exit reasons
- **Evidence:** All trades have exit reasons; no orphan positions at end

---

## 11. Promotion Metrics & Thresholds

### VAL-STRAT-051: Positive net expectancy after route costs
The replay result must show positive net expectancy: (win_rate × avg_win) - (loss_rate × avg_loss) - avg_route_cost > 0. This is the primary promotion criterion. If net expectancy is negative, promotion is blocked.
- **Tool:** `cargo test` — unit test on BacktestResult/replay metrics
- **Evidence:** net_expectancy > 0 for passing data; ≤ 0 for failing data

### VAL-STRAT-052: Max drawdown within policy limit
Max drawdown during replay must be within the policy limit (configurable, e.g., 10% of starting balance). If max drawdown exceeds the limit, promotion is blocked regardless of expectancy.
- **Tool:** `cargo test` — unit test with drawdown at limit vs above limit
- **Evidence:** drawdown ≤ limit passes; drawdown > limit fails

### VAL-STRAT-053: No stale-data trades in promotion run
For promotion eligibility, zero trades may be triggered by stale data. If any trade has a stale-data exit reason or was entered while zone data was stale, promotion is blocked. This ensures the strategy trades only on fresh intelligence.
- **Tool:** `cargo test` — unit test on replay trade log filtering for stale-data exits
- **Evidence:** stale_trade_count == 0 for passing runs; > 0 fails promotion

### VAL-STRAT-054: No duplicate pending trades in promotion run
The promotion replay must show zero instances of duplicate pending trades for the same symbol/side. If the duplicate guard fails and two pending trades exist for SOL/Long simultaneously, promotion is blocked.
- **Tool:** `cargo test` — unit test scanning replay trade log for duplicate symbol/side within pending window
- **Evidence:** duplicate_count == 0

### VAL-STRAT-055: Minimum 30 signal events for statistical validity
The replay must contain at least 30 signal events (entry signals that resulted in trades) to be statistically meaningful. Fewer than 30 trades is insufficient for promotion, even if all other metrics pass.
- **Tool:** `cargo test` — unit test with 29 vs 30 trades
- **Evidence:** 30+ trades passes; 29 or fewer fails

### VAL-STRAT-056: Sharpe ratio ≥ 1.0 threshold
Consistent with the existing backtest Sharpe threshold (see `BacktestResult::sharpe_pass`), the liquidation strategy replay must achieve Sharpe ratio ≥ 1.0. Below this threshold, the strategy is not promoted.
- **Tool:** `cargo test` — unit test on replay Sharpe calculation
- **Evidence:** sharpe ≥ 1.0 passes; sharpe < 1.0 fails

---

## 12. Strategy Trait Compliance

### VAL-STRAT-057: Implements Strategy trait (Send + Sync)
The `LiquidationCascadeHunter` struct implements `Strategy: Send + Sync`. It can be boxed as `Box<dyn Strategy>` and used in async contexts. This is verified by attempting to create and use the strategy through the trait object.
- **Tool:** `cargo test` — compile-time check + unit test using Box<dyn Strategy>
- **Evidence:** Code compiles; strategy functions through trait object

### VAL-STRAT-058: name() returns correct string
`strategy.name()` returns `"liquidation-cascade-hunter"` exactly. Not "liquidation_cascade_hunter", not "cascade-hunter", not any other variant.
- **Tool:** `cargo test` — unit test asserting string equality
- **Evidence:** assert_eq!(strategy.name(), "liquidation-cascade-hunter")

### VAL-STRAT-059: parameters() returns StrategyParams with expected defaults
`strategy.parameters()` returns a `StrategyParams` struct where fields match the documented defaults. Specifically: take_profit_pct, stop_loss_pct, trailing_stop_pct, trailing_activation_pct, max_hold_secs, clip_size_usd are all set to the strategy-specific defaults.
- **Tool:** `cargo test` — unit test checking parameter values
- **Evidence:** Each parameter matches expected default

### VAL-STRAT-060: push_price() updates internal state
Calling `push_price(price, timestamp_ms)` updates the strategy's internal price buffer. Subsequent calls to `detect_entry()` or `snapshot()` reflect the new price data. At least `lookback_count` prices must be pushed before any signal can be generated.
- **Tool:** `cargo test` — unit test pushing prices then checking snapshot
- **Evidence:** snapshot().price_count increases; snapshot reflects latest price

### VAL-STRAT-061: snapshot() returns valid MomentumSnapshot
`strategy.snapshot()` returns a MomentumSnapshot with: price_count > 0 (after pushing prices), current_price matching last pushed price, and pool_data reflecting liquidation zone state.
- **Tool:** `cargo test` — unit test after pushing prices
- **Evidence:** snapshot fields are populated and consistent

### VAL-STRAT-062: as_any() and as_any_mut() support downcasting
`strategy.as_any()` returns a `&dyn Any` that can be downcast to `LiquidationCascadeHunter`. Similarly, `as_any_mut()` returns a mutable reference. This allows external code to access strategy-specific methods (e.g., zone data inspection).
- **Tool:** `cargo test` — unit test downcasting
- **Evidence:** downcast_ref::<LiquidationCascadeHunter>() returns Some

---

## 13. No Knife-Catching Without Confirmation

### VAL-STRAT-063: No entry on pure price drop without confirmation
A raw price drop toward a liquidation zone does NOT trigger a long entry by itself. The strategy requires at least one confirmation filter to pass: volume z-score, VWAP reclamation, or forced-flow velocity spike. Without any confirmation, the strategy treats the move as unconfirmed and returns NoSignal.
- **Tool:** `cargo test` — unit test with price near zone but no confirmations
- **Evidence:** Returns NoSignal when all confirmation filters are off/failed

### VAL-STRAT-064: No entry on cascade without velocity confirmation
For cascade continuation, forced-flow velocity must be elevated (above a configurable threshold). A price approaching a liquidation zone without corresponding velocity increase does not trigger entry. This filters out false setups where price happens to be near a zone but no forced liquidation is occurring.
- **Tool:** `cargo test` — unit test with price near zone but low velocity
- **Evidence:** Returns NoSignal

### VAL-STRAT-065: Exhaustion requires velocity decay, not just price stabilization
For exhaustion reversal, it is not sufficient for price to stabilize near a zone. Forced-flow velocity must show clear decay (monotonically decreasing or below decay threshold). A price that merely stops falling without velocity decay is not an exhaustion signal.
- **Tool:** `cargo test` — unit test with stabilized price but sustained velocity
- **Evidence:** Returns NoSignal

---

## 14. Edge Cases & Robustness

### VAL-STRAT-066: No signal before sufficient price history
Before `lookback_count` prices have been pushed, `detect_entry()` returns NoSignal regardless of snapshot content. The strategy needs a minimum price history to compute velocity, VWAP, and other indicators.
- **Tool:** `cargo test` — unit test with fewer than lookback_count prices
- **Evidence:** Returns NoSignal; after lookback_count prices, signal may appear

### VAL-STRAT-067: Handles zero AUM gracefully
When pool AUM is 0.0 (e.g., API returned empty data), the strategy does not panic or divide by zero. It returns NoSignal and logs a warning about missing pool data.
- **Tool:** `cargo test` — unit test with AUM = 0.0
- **Evidence:** No panic; returns NoSignal; warning logged

### VAL-STRAT-068: Handles missing pool data gracefully
When `MomentumSnapshot.pool_data` is `None`, the strategy returns NoSignal for cascade continuation entries (which require pool data). Exhaustion reversal entries that don't strictly require pool data may still proceed if all other gates pass.
- **Tool:** `cargo test` — unit test with pool_data = None
- **Evidence:** Returns NoSignal for cascade; behavior defined for exhaustion

### VAL-STRAT-069: Handles extreme volatility without panic
During extreme volatility (e.g., 50% price swing in one candle), the strategy does not panic, overflow, or produce NaN in any computed metric. Returns NoSignal or a valid signal with finite numeric values.
- **Tool:** `cargo test` — unit test with extreme price input
- **Evidence:** No panic; all output values are finite (not NaN, not Inf)

### VAL-STRAT-070: Config from TOML sub-table parses correctly
The `[strategy.liquidation-cascade-hunter]` TOML sub-table is parsed into the strategy's parameter struct. All fields are deserialized correctly: enabled, confidence_min, volume_z_score_threshold, max_distance_to_zone_pct, spread_multiplier, depth_threshold, route_cost_max_bps, stale_data_threshold_secs, take_profit_pct, stop_loss_pct, trailing_stop_pct, trailing_activation_pct, max_hold_secs, clip_size_usd, direction_bias, cooldown_after_loss_secs.
- **Tool:** `cargo test` — unit test parsing a TOML string into params
- **Evidence:** All fields match the TOML values

### VAL-STRAT-071: Missing optional config fields use sensible defaults
When the TOML sub-table omits optional fields (e.g., route_cost_max_bps), the strategy uses sensible defaults. All fields have `#[serde(default = ...)]` annotations so the strategy works with a minimal config.
- **Tool:** `cargo test` — unit test with minimal TOML config
- **Evidence:** Strategy creates successfully; missing fields have expected defaults

### VAL-STRAT-072: Direction bias filters are respected
When `direction_bias = "long"`, the strategy only generates MomentumLong signals (no shorts). When `"short"`, only MomentumShort. When `"neutral"`, both directions are allowed. Direction bias is checked before signal emission.
- **Tool:** `cargo test` — unit test for each bias value
- **Evidence:** Long bias blocks short signals; short bias blocks long signals; neutral allows both

---

## 15. Integration with Existing Infrastructure

### VAL-STRAT-073: MultiPaperEngine supports liquidation-cascade-hunter
The `MultiPaperEngine` (see `paper.rs`) can run `liquidation-cascade-hunter` across multiple markets (e.g., SOL, BTC, ETH) simultaneously. Each market gets its own strategy instance with independent state. Signals from one market do not interfere with another.
- **Tool:** `cargo test` — integration test in paper.rs
- **Evidence:** MultiPaperEngine creates strategy per market; signals are independent

### VAL-STRAT-074: Risk manager circuit breaker applies to liquidation strategy
The `RiskManager` circuit breaker (daily loss, weekly loss, max drawdown, consecutive losses) applies to liquidation-cascade-hunter trades. If the circuit breaker is tripped, the paper engine stops generating new entry signals for this strategy, same as all others.
- **Tool:** `cargo test` — integration test with risk manager
- **Evidence:** After circuit breaker trips, detect_entry is not called / returns NoSignal

### VAL-STRAT-075: Correlated exposure limit enforced
If the strategy opens a SOL/Long position and the correlated exposure limit for the "SOL-correlated" group (e.g., SOL, mSOL, JitoSOL) is reached, no additional correlated positions are opened. The risk manager's `open_exposures` map is updated.
- **Tool:** `cargo test` — integration test with correlated exposure
- **Evidence:** Second correlated position is blocked

### VAL-STRAT-076: Backtest engine regime filter applies
When `backtest.regime_filter = true`, the backtest engine applies regime compatibility checking to liquidation-cascade-hunter. Trades are blocked during incompatible regimes (e.g., Choppy for cascade continuation). The `regime_blocked_count` is incremented and included in `BacktestCellStats`.
- **Tool:** `cargo test` — integration test in backtest.rs
- **Evidence:** regime_blocked_count > 0 when regime is incompatible

### VAL-STRAT-077: PnL tracker records liquidation strategy trades
The `pnl_tracker.rs` module records all liquidation-cascade-hunter trades with the strategy name, symbol, side, entry/exit prices, PnL, fees, and timestamps. The combined PnL report includes this strategy's contributions.
- **Tool:** `cargo test` — unit test in pnl_tracker.rs with liquidation trade data
- **Evidence:** Trade appears in report; PnL totals are correct

### VAL-STRAT-078: Trade journal atomic write includes strategy name
When the paper engine closes a liquidation-cascade-hunter position, the trade is written to `perks-trades.json` via atomic write (`.tmp` → rename). The entry includes `strategy: "liquidation-cascade-hunter"` for traceability.
- **Tool:** `cargo test` — integration test checking journal output
- **Evidence:** JSON entry has correct strategy field; file was atomically written

---

## 16. Replay Promotion Gate

### VAL-STRAT-079: Promotion gate aggregates all criteria
The promotion gate function checks ALL criteria simultaneously: positive expectancy (VAL-STRAT-051), max drawdown within limit (VAL-STRAT-052), zero stale-data trades (VAL-STRAT-053), zero duplicate pendings (VAL-STRAT-054), ≥30 trades (VAL-STRAT-055), Sharpe ≥ 1.0 (VAL-STRAT-056). ALL must pass for promotion to be approved. If any single criterion fails, promotion is denied with a report explaining which criteria failed.
- **Tool:** `cargo test` — unit test with all-passing and each-individual-failure scenario
- **Evidence:** All-pass → approved; any single fail → denied with failure reason

### VAL-STRAT-080: Promotion gate produces human-readable report
The promotion gate outputs a structured report (JSON or Markdown) showing: each criterion name, pass/fail status, actual value, threshold value, and a summary verdict. This report is suitable for human review before live deployment.
- **Tool:** `cargo test` — unit test on report generation
- **Evidence:** Report contains all criteria; each has status/value/threshold; summary is correct
