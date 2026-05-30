# Validation Contract — Copy Trading & Combined Validation

**Mission:** Make Zekt Profitable on Hyperliquid  
**Scope:** Copy Trading (VAL-CT-*) and Combined Validation (VAL-CV-*) feature areas  
**Date:** 2026-05-28  

---

## Copy Trading Area

### VAL-CT-001: Alpha Scanner Produces Valid Watchlist
Alpha-scanner (`cargo run --bin alpha-scanner -- --once`) must produce a `data/watchlist.json` file. The file must be valid JSON containing a top-level `wallets` array with at least 1 entry. Each entry must have a non-empty `address` field that matches an Ethereum-style hex address pattern (`0x` prefix, 40+ hex chars). The file must be parseable by `copy-trader`'s `load_watchlist()` function.
Tool: cargo-test + shell
Evidence: `cat data/watchlist.json | jq '.wallets | length'` ≥ 1; `jq '.wallets[0].address'` starts with `0x`; cargo test `test_load_watchlist_alpha_format` passes.

### VAL-CT-002: Alpha Scanner Scores and Ranks Wallets
Every wallet entry in the output watchlist must contain a `composite_score` field (float ≥ 0). The wallets array must be sorted in descending order by `composite_score` — i.e., `wallets[0].composite_score ≥ wallets[1].composite_score`. The composite score is computed as `sharpe * log(|pnl| + 1) * consistency_factor` per `compute_composite_score()`.
Tool: cargo-test + shell
Evidence: `jq '.wallets | map(.composite_score) | sort_by(-.) == .' data/watchlist.json` returns `true`; unit test `test_compute_composite_score_positive` passes.

### VAL-CT-003: Alpha Scanner Enforces Watchlist Size
The output watchlist must contain at most `watchlist_size` entries (CLI flag, default 50). If fewer qualifying wallets are discovered, the list may be shorter but must never exceed the cap. A value of 0 must be rejected at argument parsing.
Tool: cargo-test
Evidence: `jq '.wallets | length' data/watchlist.json` ≤ watchlist_size; unit tests `test_validate_watchlist_size_zero_rejected` and `test_truncate_to_watchlist_size` pass.

### VAL-CT-004: Alpha Scanner Deduplicates Wallets
No address may appear more than once in the watchlist. If the same address appears in multiple data sources (Dextrabot, Hypurrscan), only the highest-scored entry is kept.
Tool: cargo-test
Evidence: `jq '.wallets | map(.address) | length == (map(.address) | unique | length)' data/watchlist.json` returns `true`.

### VAL-CT-005: Copy Trader Stop-Loss Bug — mark_prices Populated from HL Data
In the copy-trader main loop (around line 900 of `copy-trader.rs`), `mark_prices` must be populated from the HL `clearinghouseState` response. The `mark_px` field from each position's JSON must be parsed into the `HashMap<String, f64>`. The stop-loss check (`check_stop_losses`) must receive a non-empty HashMap — the second call (outside the per-wallet loop) must reuse or aggregate the collected prices, not create an empty `HashMap::new()`.
Tool: cargo-test
Evidence: Unit test `test_check_stop_losses_triggered` passes with non-zero mark_prices; grep for `HashMap::new()` after `check_stop_losses` call in main loop returns 0 occurrences where it replaces real data.

### VAL-CT-006: Stop-Loss Fires on Threshold Breach
`is_stop_loss_triggered(entry_price, current_price, is_long, stop_loss_pct)` must return `true` when unrealized loss exceeds the configured `stop_loss_pct`. For a long position at entry 60000 with `stop_loss_pct=5.0` and current price 57000 (−5%), SL must fire. For a short at entry 60000 with price 63000 (+5%), SL must fire. The function must return `false` when entry_price ≤ 0.0.
Tool: cargo-test
Evidence: Unit tests `test_is_stop_loss_triggered_long`, `test_is_stop_loss_triggered_short`, `test_engine_stop_loss_closes_position` pass.

### VAL-CT-007: Copy Trader Mirrors Positions in Paper Mode
When `--paper` flag is passed, `copy-trader` must open a paper trade for each new position detected on a followed wallet. The paper trade must record: `wallet_address`, `market`, `direction`, `size_usd` (scaled by `sizing_multiplier` and capped at `max_position_pct` of paper balance), `entry_price`, and `status=Open`. No real transactions are submitted.
Tool: cargo-test
Evidence: Unit test `test_engine_new_position` passes — engine's `process_wallet` returns count=1, trade log contains an open trade with correct fields; no Solana RPC calls in logs.

### VAL-CT-008: Copy Trader Closes Positions When Followed Wallet Closes
When a followed wallet's position disappears between poll cycles (present in previous snapshot, absent in current), `detect_positions_diff` must return it in `closed_positions`. The engine must close the corresponding paper trade with the current mark price as exit_price, record `pnl_usd` (including direction), and set `status=Closed`.
Tool: cargo-test
Evidence: Unit tests `test_detect_positions_diff_closed` and `test_engine_closed_position_pnl` pass; trade log shows closed trade with non-None `exit_price` and `pnl_usd`.

### VAL-CT-009: Copy Trader Tracks PnL with HL Fee Estimates
Each closed paper trade must include fee deductions. The trade record must contain `entry_fee` and `exit_fee` fields (or equivalent). The `net_pnl` must equal `gross_pnl - entry_fee - exit_fee - borrow_fee`. Fees are estimated from HL fee schedule (0.035% taker for market orders).
Tool: cargo-test
Evidence: Unit test `test_engine_closed_position_pnl` asserts `pnl_usd` accounts for fees; `test_engine_fee_deduction` (if exists) passes.

### VAL-CT-010: Position Size Capped by Max Position Percentage
`calculate_position_size(whale_size, sizing_multiplier, account_balance, max_position_pct)` must return `whale_size * sizing_multiplier` when that value is ≤ `account_balance * max_position_pct / 100`. If it exceeds the cap, the capped value is used. The engine must never open a position larger than this limit.
Tool: cargo-test
Evidence: Unit test `test_calculate_position_size_capped` and `test_engine_position_size_cap` pass.

### VAL-CT-011: Max Positions Limit Enforced
`can_open_position(open_count, max_positions)` must return `false` when `open_count >= max_positions`. The engine must skip mirroring new positions when the limit is reached, logging "skipped: max positions reached".
Tool: cargo-test
Evidence: Unit test `test_engine_max_positions_limit` passes; log output contains "max positions reached" when limit is hit.

### VAL-CT-012: Position Diff Detection — New, Closed, and Unchanged
`detect_positions_diff(old_snapshot, current_snapshot)` must correctly classify: (a) coins in current but not old → `new_positions`, (b) coins in old but not current → `closed_positions`, (c) coins in both → neither list. Positions with zero size are excluded from comparison.
Tool: cargo-test
Evidence: Unit tests `test_detect_positions_diff_new`, `test_detect_positions_diff_closed`, `test_detect_positions_diff_no_change`, `test_detect_positions_diff_multiple` all pass.

### VAL-CT-013: Copy Trader Handles API Failures Gracefully
If `fetch_wallet_positions` returns an error for a specific wallet, the copy-trader must log a warning and continue to the next wallet. The engine must not crash, and previously opened paper trades must remain intact. The total error count is logged per poll cycle.
Tool: cargo-test + manual
Evidence: Log output shows "API failure fetching positions — will retry next cycle" for failed wallets; engine does not panic; subsequent successful polls continue mirroring.

---

## Combined Validation Area

### VAL-CV-001: 48h Pipeline Run Without Crash
`cargo run --bin pipeline -- --paper-balance 1000 --duration-hours 48` must run for approximately 48 hours without panicking, deadlocking, or OOM. The pipeline must launch alpha-scanner, copy-trader, whale-watcher, and paper trading subprocesses. On `duration_hours` expiry, all children are killed and a final report is generated. The exit code must be 0.
Tool: tuistory / shell
Evidence: Process runs for ≥47h; stdout shows "Max duration reached (48.0h)"; "Pipeline complete" logged; exit code 0.

### VAL-CV-002: Pipeline Produces Final PnL Report
After pipeline shutdown (either timeout or Ctrl+C), a `data/combined-pnl.json` file must exist. The file must be valid JSON containing: `generated_at` (ISO timestamp), `total_net_pnl` (float), `total_gross_pnl`, `total_fees`, `total_trades` (int ≥ 0), `strategies` (array), and `data_sources` (object). The report is generated by `PnlTracker::generate_report()`.
Tool: shell + jq
Evidence: `jq '.total_net_pnl' data/combined-pnl.json` returns a number (not null); `jq '.strategies | length'` ≥ 1; `jq '.generated_at'` is a valid ISO 8601 timestamp.

### VAL-CV-003: Combined Tracking Includes Funding Capture and Copy Trading
The `strategies` array in `combined-pnl.json` must contain entries for both `"funding-capture"` and `"copy-trader"` (or `"copy-trading"`). Each entry must have non-null `total_trades`, `closed_trades`, `gross_pnl`, `total_fees`, and `net_pnl`. Data sources (`data_sources` field) must list the paths that were successfully read.
Tool: shell + jq
Evidence: `jq '.strategies[] | select(.strategy == "funding-capture") | .net_pnl' data/combined-pnl.json` returns a number; same for copy-trader strategy.

### VAL-CV-004: Periodic Report Generation During Run
During the 48h run, a combined PnL report must be generated every `report_interval` seconds (default 300s / 5 min). Each report overwrites the previous `data/combined-pnl.json`. Log output must show "Report generated" or equivalent at each interval.
Tool: shell
Evidence: `grep -c "report" pipeline.log` shows periodic entries at ~5min intervals; `stat data/combined-pnl.json` shows mtime updated within last 5 minutes of a running pipeline.

### VAL-CV-005: Profitability — Net PnL Positive After Fees
**This is the mission's primary success criterion.** After a 48h paper run, `total_net_pnl` in `combined-pnl.json` must be > 0.00. This means gross trading profits exceed all simulated fees (entry, exit, borrow, funding). Both funding-capture and copy-trading strategies contribute. If the result is negative, the mission fails validation.
Tool: shell + jq
Evidence: `jq '.total_net_pnl' data/combined-pnl.json` > 0.00; also check `jq '.strategies[] | "\(.strategy): \(.net_pnl)"'` to see per-strategy breakdown.

### VAL-CV-006: Fee Accounting — Total Fees Match Sum of Strategy Fees
`total_fees` in the combined report must equal the sum of all `strategies[].total_fees`. No fee is double-counted or omitted. Each strategy's `net_pnl` must equal `gross_pnl - total_fees`.
Tool: shell + jq
Evidence: `jq '.total_fees == (.strategies | map(.total_fees) | add)' data/combined-pnl.json` returns `true`; `jq '.strategies[] | .net_pnl == (.gross_pnl - .total_fees)' data/combined-pnl.json` returns `true` for every strategy.

### VAL-CV-007: Copy Trade Log Persists Across Restarts
`data/copy-trades.json` must survive pipeline restarts. If the pipeline is stopped (Ctrl+C) and restarted with `--skip-scanner`, the copy-trader must load existing trades from the log and not re-open positions for already-tracked wallets. Trade IDs must be unique across restarts (timestamp-based).
Tool: shell + cargo-test
Evidence: After restart, `jq '[.[] | select(.status == "Open")] | length' data/copy-trades.json` equals the count before restart; no duplicate trade IDs.

### VAL-CV-008: Account Balance Consistency
The `account_balance` tracked by `CopyTraderEngine` must equal `initial_balance + sum(all closed trade pnl_usd)`. After each trade close, the balance is updated by adding `pnl_usd`. The final balance must match what appears in the combined report's net PnL calculation.
Tool: cargo-test
Evidence: Unit test `test_engine_balance_tracks_pnl` passes; `jq '.total_net_pnl' data/combined-pnl.json` is consistent with copy-trader's final logged account balance minus initial balance.

### VAL-CV-009: Watchlist Refresh Cycle
During a multi-hour pipeline run, the alpha-scanner must re-execute every 6 hours (`scanner_interval = Duration::from_secs(21600)`). The watchlist must be refreshed in-place at `data/watchlist.json`. Copy-trader and whale-watcher pick up changes on their next poll cycle. New wallets are mirrored; removed wallets' open positions are left to close naturally.
Tool: shell
Evidence: Log shows "Periodic alpha-scanner refresh" at ~6h intervals; `stat data/watchlist.json` shows mtime updated; copy-trader logs new wallet processing after refresh.

### VAL-CV-010: Graceful Shutdown Produces Clean State
Sending SIGINT (Ctrl+C) to the pipeline must: (1) set `running` to false, (2) finish the current monitoring tick, (3) kill all child processes, (4) generate a final combined PnL report, (5) exit with code 0. No orphan processes should remain. All trade logs must be valid JSON (not truncated).
Tool: tuistory
Evidence: After Ctrl+C, `ps aux | grep zekt` shows no running instances; `jq . data/combined-pnl.json` succeeds (valid JSON); `jq . data/copy-trades.json` succeeds; exit code is 0.

---

## Summary

| Area | ID Range | Count |
|------|----------|-------|
| Copy Trading | VAL-CT-001 … VAL-CT-013 | 13 |
| Combined Validation | VAL-CV-001 … VAL-CV-010 | 10 |
| **Total** | | **23** |

**Primary gate:** VAL-CV-005 (net PnL > 0 after 48h) is the mission-level pass/fail criterion.  
**Bug-fix gate:** VAL-CT-005 + VAL-CT-006 confirm the stop-loss `mark_prices` bug is resolved.  
**Data-flow gate:** VAL-CT-001 + VAL-CT-007 + VAL-CV-003 confirm the full pipeline from wallet discovery → position mirroring → PnL tracking works end-to-end.
