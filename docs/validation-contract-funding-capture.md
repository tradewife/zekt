# Validation Contract: Funding Rate Capture

Feature area of the "Make Zekt Profitable on Hyperliquid" mission.
Covers: HL Data Provider, HL Paper Engine, Funding Capture Wiring, CLI, and Output.

---

## HL Data Provider

### VAL-FC-001: HlInfoClient fetches real funding rates from Hyperliquid
Given a live internet connection, `HlInfoClient::get_funding_rates()` returns a non-empty `Vec<HlFundingRate>`. Each entry has `coin` matching a known HL perp (e.g. "BTC"), `funding` as a raw float (e.g. 0.0001), and `mark_px > 0.0`. The call completes without error within 30 seconds.
Tool: cargo-test
Evidence: Unit test calling `HlInfoClient::default_client().get_funding_rates().await` asserts `!rates.is_empty()`, `rates.iter().all(|r| r.mark_px > 0.0)`, and at least one entry has `coin == "BTC"`.

### VAL-FC-002: Funding rate annualization is correct
When `HlInfoClient` parses the `metaAndAssetCtxs` response, each `HlFundingRate.annualized_funding` equals `funding * 3 * 365` (HL uses 8-hour funding periods → 3 periods/day × 365 days). For a raw rate of 0.0001, the annualized value is `0.1095` (10.95%). Pass if the computed value is within 0.001 of the expected result for known test vectors.
Tool: cargo-test
Evidence: Synthetic `metaAndAssetCtxs` JSON response parsed through `parse_meta_and_asset_ctxs()`. Assert `annualized_funding` matches `raw_funding * 3.0 * 365.0` within tolerance `±0.001`.

### VAL-FC-003: HlInfoClient fetches real mid prices
`HlInfoClient::get_market_contexts()` returns `HlMarketContexts` where each `HlAssetContext` contains a parseable `markPx` string that converts to a positive f64. For BTC, the mark price must be > $10,000.
Tool: cargo-test
Evidence: Live integration test calling `get_market_contexts()`, asserting BTC mark price > 10,000.0 and ETH mark price > 100.0.

### VAL-FC-004: HL taker fee is 0.035% of notional
The HL Paper Engine uses a constant `HL_TAKER_FEE_RATE = 0.00035` (0.035%). Opening a $200 position accrues exactly $0.07 in entry fees. Closing the same position accrues another $0.07. Total taker fees for a round-trip = $0.14.
Tool: cargo-test
Evidence: Open a simulated $200 short, assert `entry_fee == 200.0 * 0.00035`. Close the position, assert `exit_fee == 200.0 * 0.00035`. Assert `total_fees == 0.14`.

---

## HL Paper Engine

### VAL-FC-005: HL Paper Engine opens a simulated short position
Given the funding capture strategy fires `MomentumShort` with funding at 30% annualized, the HL Paper Engine creates a `PaperPosition` with `is_long == false`, `size_usd == clip_size_usd * leverage`, and `entry_price` set from the current mark price. The simulated balance decreases by `clip_size_usd + entry_fee`.
Tool: cargo-test
Evidence: Construct `HlPaperEngine` with $1000 balance. Inject funding snapshot at 30% annualized for 2+ confirmation ticks. Call `detect_entry()` → verify position opened. Assert `sim_balance == 1000.0 - clip_size_usd - entry_fee`.

### VAL-FC-006: HL Paper Engine accrues borrow fees per tick
After opening a $200 short position and running 3 ticks at 10-second intervals (with `BORROW_FEE_HOURLY = 0.0001`), the position's `accrued_borrow_fee` equals `notional * 0.0001 * (10.0 / 3600.0) * 3`. For $200 notional: `200 * 0.0001 * 0.002778 * 3 ≈ $0.000167`.
Tool: cargo-test
Evidence: Open position, advance 3 ticks, assert `accrued_borrow_fee` is within `±0.000001` of the expected value `200.0 * 0.0001 * (10.0/3600.0) * 3.0`.

### VAL-FC-007: HL Paper Engine closes position and realizes net PnL
A short position entered at $100,000 BTC with $200 notional is closed at $99,000. The price moved -1% (favorable for short). Unrealized PnL = `200 * 0.01 = $2.00`. Net PnL = `2.00 - entry_fee - exit_fee - accrued_borrow_fee`. The engine records this in the trade log and updates `sim_balance` by `clip_size_usd + net_pnl`.
Tool: cargo-test
Evidence: Open short at 100,000, push price to 99,000, trigger exit. Assert `trade_log.last().net_pnl == 2.00 - total_fees`. Assert `sim_balance == initial - clip_size_usd + clip_size_usd + net_pnl`.

### VAL-FC-008: HL Paper Engine rejects trades when balance insufficient
When `sim_balance < clip_size_usd`, the engine does not open a position even if the strategy signals entry. No position is created, no fee is deducted.
Tool: cargo-test
Evidence: Set balance to $50, fire entry signal with `clip_size_usd = 200.0`. Assert `position.is_none()` and `sim_balance == 50.0`.

### VAL-FC-009: HL Paper Engine handles multiple concurrent markets
When run with `--markets BTC,ETH,SOL`, the engine maintains independent position state per market. A short on BTC does not interfere with an entry signal on ETH. Each market tracks its own price feed and fee accrual.
Tool: cargo-test
Evidence: Construct engine with 3 markets. Fire entry for BTC only. Assert BTC has position, ETH and SOL have `None`. Fire entry for ETH. Assert both BTC and ETH have positions independently.

---

## Funding Capture Wiring

### VAL-FC-010: push_funding() is called on every tick for active markets
On each engine tick, for each market in the active set, the HL Paper Engine fetches the current funding rate via `HlInfoClient::get_funding_rates()`, filters to the target market, constructs a `FundingSnapshot`, and calls `strategy.push_funding(snapshot)`. The strategy's `current_rate()` reflects the latest pushed value.
Tool: cargo-test
Evidence: Mock the data provider to return a fixed funding rate of 25%. Run 3 ticks. Assert `strategy.current_rate() == 25.0`. Assert `strategy.funding_history.len() == 3`.

### VAL-FC-011: Strategy opens short when funding > 20% for confirmation_ticks
With default parameters (`min_annualized_rate_pct = 20.0`, `confirmation_ticks = 2`): pushing two consecutive funding snapshots at 25% annualized causes `detect_entry()` to return `Signal::MomentumShort { strength, velocity_pct }` where `strength >= 60.0`. After firing, the consecutive counter resets to 0.
Tool: cargo-test
Evidence: Already covered by `test_entry_signal_after_confirmation` and `test_entry_signal_resets_consecutive_counter` in `funding_capture.rs`. Run `cargo test funding_capture` and assert all 40 tests pass.

### VAL-FC-012: Strategy closes short when funding < 5%
A short position is open. `push_funding()` receives a snapshot with `annualized_rate_pct = 3.0` (< `exit_annualized_rate_pct = 5.0`). `detect_exit()` returns `Some(Signal::ExitShort { reason: ReversalDetected })`.
Tool: cargo-test
Evidence: Already covered by `test_exit_funding_below_threshold`. Verify the test passes. Assert `reason == ExitReason::ReversalDetected`.

### VAL-FC-013: Strategy does not enter when funding is between exit and entry thresholds
Funding at 12% (above 5% exit, below 20% entry): `push_funding()` is called but `consecutive_above_threshold` stays at 0. `detect_entry()` returns `Signal::NoSignal`.
Tool: cargo-test
Evidence: Already covered by `test_no_signal_below_threshold`. Verify test passes. Assert no position opened after 5 ticks at 12% funding.

### VAL-FC-014: Exit priority is StopLoss > TimeStop > FundingDrop
A short position with entry at 100, current price at 106 (6% adverse = stop-loss triggered), hold time at 73 hours (time-stop triggered), and funding at 2% (funding-exit triggered). `detect_exit()` returns `ExitShort(StopLoss)` — stop-loss takes priority over all other exits.
Tool: cargo-test
Evidence: Already covered by `test_exit_priority_stop_loss_over_time_stop`. Run `cargo test test_exit_priority` and assert it passes with `reason == StopLoss`.

---

## CLI

### VAL-FC-015: --hl-paper flag is accepted by main.rs
Running `cargo run -- --hl-paper --paper-balance 1000 --strategies funding-capture --markets BTC` starts the HL Paper Engine in paper-trading mode. The process does not exit with a CLI parse error. The log output includes "HL PAPER Trading Mode" (or equivalent).
Tool: tuistory
Evidence: Launch `cargo run -- --hl-paper --paper-balance 1000 --strategies funding-capture --markets BTC`. Capture TUI output. Assert exit code 0 (or still running after 5 seconds). Assert log line contains "funding-capture" and "BTC".

### VAL-FC-016: --hl-paper accepts --strategies with multiple strategies
`--hl-paper --strategies funding-capture,momentum-scalper` is parsed without error. The engine initializes both strategies. Log output lists both strategy names.
Tool: tuistory
Evidence: Launch with multi-strategy flag. Assert both "funding-capture" and "momentum-scalper" appear in startup logs.

### VAL-FC-017: --hl-paper accepts --markets with multiple markets
`--hl-paper --markets BTC,ETH,SOL` is parsed and the engine initializes market feeds for all three. Log output lists all three markets.
Tool: tuistory
Evidence: Launch with multi-market flag. Assert "BTC", "ETH", and "SOL" all appear in initialization output.

### VAL-FC-018: --hl-paper accepts --paper-balance override
`--hl-paper --paper-balance 5000` sets the simulated starting balance to $5000. The log output shows "Simulated balance: $5000.00".
Tool: tuistory
Evidence: Launch with `--paper-balance 5000`. Assert log output contains "5000" in the balance line.

### VAL-FC-019: Missing required flags produce a helpful error
Running `--hl-paper` without `--markets` either uses the config default or prints a clear error. No panic or stack trace.
Tool: tuistory
Evidence: Launch `cargo run -- --hl-paper` (no markets). Assert output is a clean error message, not a panic.

---

## Output

### VAL-FC-020: JSON PnL report is written on shutdown
After the HL Paper Engine stops (natural exit or SIGINT), a JSON file is written to the configured output directory containing: `start_time`, `end_time`, `initial_balance`, `final_balance`, `total_trades`, `win_rate`, `net_pnl`, `total_fees`.
Tool: cargo-test + file-read
Evidence: Run engine for a single trade cycle (open + close). After shutdown, read the output JSON file. Assert all top-level keys exist. Assert `total_trades >= 1`.

### VAL-FC-021: PnL report includes per-market breakdown
The JSON report contains a `markets` array where each entry has: `market` (string), `trades` (count), `pnl` (f64), `fees` (f64), `funding_captured` (f64). When trading on BTC and ETH, the report has two entries in `markets`.
Tool: cargo-test
Evidence: Run engine with `--markets BTC,ETH`. Generate report. Parse JSON. Assert `markets.len() == 2`. Assert each entry has all required fields. Assert `markets[].pnl + markets[].fees` sums match the top-level totals.

### VAL-FC-022: PnL report includes fee breakdown
The `total_fees` field decomposes into `taker_fees`, `borrow_fees`, and `slippage_est`. `total_fees == taker_fees + borrow_fees + slippage_est`. For a funding capture trade (1x leverage, $200 clip, 4-hour hold): taker_fees ≈ $0.14 (round-trip), borrow_fees ≈ $0.08 (4h × $200 × 0.01%/hr).
Tool: cargo-test
Evidence: Run a single round-trip trade with known parameters. Parse output JSON. Assert `abs(total_fees - (taker_fees + borrow_fees + slippage_est)) < 0.001`.

### VAL-FC-023: Sharpe ratio is computed and included in the report
The JSON report contains a `sharpe_ratio` field computed as `(mean_return / std_dev_returns) * sqrt(365 * 3)` (annualized, assuming 8-hour sampling). When there are fewer than 5 trades, `sharpe_ratio` is `null` or absent (insufficient data).
Tool: cargo-test
Evidence: Run engine producing 10 trades. Parse report. Assert `sharpe_ratio` is a finite f64 (not NaN, not Inf). Run engine producing 2 trades. Assert `sharpe_ratio` is null/absent.

### VAL-FC-024: Report uses atomic writes (no corrupt partial files)
The output JSON file is written to a `.tmp` file first, then atomically renamed to the final path. If the process is killed mid-write, the final path either contains the complete previous report or the complete new report — never a partial file.
Tool: cargo-test
Evidence: Inspect the write path in the output module. Assert it uses `write_to_tmp_then_rename` pattern (write to `<path>.tmp`, then `std::fs::rename`). Verify no `<path>.tmp` file exists after clean shutdown.
