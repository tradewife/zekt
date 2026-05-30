# Zekt — Imperial Route Oracle + Liquidation-Zone Alpha Validation — Mission Report

**Date:** 2026-05-31
**Mission:** Imperial Route Oracle + Liquidation-Zone Alpha Validation
**Status:** Complete (M1–M4)
**Commits:** 5 feature commits (imperial client → route oracle → liquidation capture → strategy → replay)

---

## 1. What Changed

### M1: Imperial Read-Only Client (commit 61ff7ed)
- `src/imperial.rs` — `ImperialClient`: 10 read-only GET endpoints for `https://api.imperial.space`
  - Perpetual markets, orderbook depth, recent trades, funding rates, open interest, liquidations, ticker, mark price, fee schedule, server health
- `src/config.rs` — `[imperial]` config section with safe defaults (base URL, timeout, enabled flag)
- No auth, no POST/PUT/DELETE, no `/mobile/*`, no `/deposit/*`
- Live smoke tests marked `#[ignore]` (require network)
- **472 total Rust tests** after M1

### M2: Route Cost Oracle + Blueprint Revalidation (commit d23dddf)
- `src/route_cost.rs` — `RouteCostOracle`: multi-venue cost estimation using Imperial route data
  - Compares Imperial execution costs vs Flash-only baseline across fee, slippage, and borrow dimensions
- `src/backtest.rs` — `imperial-route-oracle` cost mode integrated into `BacktestEngine`
  - Backward compatible: `cost_mode = "flash-only"` (default) produces identical results
- `data/imperial-route-comparison.md` — Before/after comparison table for all 10 blueprint strategies
- Config: `[route-oracle]` section in `config/perps.toml`
- **512 total Rust tests** after M2

### M3: Liquidation Zone Intelligence Capture (commit 68ae963)
- `src/liquidation.rs` — Full data model, multi-source fusion, confidence scoring, snapshot persistence
  - 4 sources: HL positions, HL fills, Imperial OI imbalance, Imperial depth fragility
  - Confidence scoring based on source agreement and data freshness
  - Atomic write snapshots to `data/liquidation-zones/` with configurable retention
- `src/config.rs` — `[liquidation]` config section with `enabled = false` default
- **619 total Rust tests** after M3

### M4: Liquidation Strategy + Replay Validation Pipeline (commit 05ceb0d / c30d90b)
- `src/strategy.rs` — `liquidation-cascade-hunter` strategy (paper-only)
  - Two setup types: cascade continuation and exhaustion reversal
  - Configurable parameters: confidence threshold, entry/exit rules, hold time limits
- `src/replay.rs` — `ReplayPipeline`: loads captured data, replays through strategy, evaluates promotion gate criteria
  - 45 new replay tests covering VAL-STRAT-046 through VAL-STRAT-080, VAL-CROSS-005 through VAL-CROSS-008
- Promotion gate checks: net expectancy, max drawdown, stale-data trades, duplicate pendings, signal count, Sharpe ratio
- **736 total Rust tests** after M4

---

## 2. What Improved

### Infrastructure Additions

| Dimension | Before | After |
|-----------|--------|-------|
| Multi-venue visibility | Flash-only pricing | Imperial + Flash route comparison with cost oracle |
| Cost estimation | Single-venue taker fee | Multi-venue fee/slippage/borrow estimation |
| Liquidation intelligence | None | 4-source fusion with confidence scoring and persistence |
| Strategy library | 5 strategies + 10 blueprint | 16 strategies (added liquidation-cascade-hunter) |
| Validation pipeline | Backtest-only | Full replay pipeline with promotion gate |
| Test coverage | 414 tests | 736 tests (+322 new tests, +78% growth) |

### New Modules

| Module | Purpose |
|--------|---------|
| `src/imperial.rs` | Imperial Solana perps aggregator read-only client |
| `src/route_cost.rs` | Multi-venue route cost oracle |
| `src/liquidation.rs` | Liquidation zone data model, fusion, persistence |
| `src/replay.rs` | Replay validation pipeline with promotion gate |

### Config Additions

| Section | Key Fields |
|---------|------------|
| `[imperial]` | base_url, timeout_secs, enabled |
| `[route-oracle]` | enabled, min_improvement_bps, edge_budget_pct |
| `[liquidation]` | enabled, snapshot_dir, retention_hours, confidence_threshold |

---

## 3. What Failed / Did Not Work

1. **No live validation of Imperial route savings.** The route oracle shows potential cost differences in backtests, but real execution savings are unconfirmed until live paper trading with both venues.

2. **Liquidation zone capture requires sustained runtime.** The capture loop needs 24–72 hours of continuous operation to accumulate meaningful zone data. Short test runs produce sparse snapshots with limited fusion value.

3. **Imperial API coverage gaps.** The Imperial API does not expose historical liquidation events or per-user fill data, limiting the fusion sources for liquidation zone intelligence to aggregate metrics only.

---

## 4. What Remains Unknown

1. **Do Imperial routes actually produce better execution?** The oracle estimates potential savings, but real execution depends on liquidity depth, latency, and dynamic fee changes. Need live paper testing to confirm.

2. **How many liquidation cascade setups occur in practice?** The strategy is paper-only and needs sustained capture runs to estimate signal frequency. Zero signals in a 24h period would indicate the approach needs rethinking.

3. **What is the optimal confidence threshold for liquidation zone fusion?** Current default is 0.5. This is a guess — needs sensitivity analysis against captured data.

4. **Can replay validation predict live performance?** The replay pipeline uses historical captured data. Its correlation with live trading performance is unproven.

---

## 5. Reference Corpus Delta

No new external repos analyzed during this mission. Built on infrastructure from prior missions:

| Resource | Role in This Mission |
|----------|---------------------|
| Flash Trade API | Execution target, fee baseline for route comparison |
| Imperial API | New venue for route oracle and liquidation OI data |
| Hyperliquid API | Fill data and position data for liquidation zone fusion |
| QuickNode HyperCore | Batch wallet scanning for HL position data |

---

## 6. Next Missions (Ranked by Expected Impact)

### Mission A: 90-Day Backtest with Imperial Routing
**Rank:** 1 (Highest Impact)
**Description:** Run extended 90-day backtests using the `imperial-route-oracle` cost mode across all strategies and markets. Compare route-aware costs vs Flash-only baseline. Identify which strategies benefit most from Imperial routing.
**Impact:** Determines whether multi-venue routing provides real cost savings. 15-day tests are too short for statistical significance.
**Prerequisites:** M2 route oracle (complete).

### Mission B: Liquidation Capture 24–72h Run
**Rank:** 2
**Description:** Run the liquidation zone capture loop for 24–72 hours on live data. Analyze captured zones for signal frequency, source agreement, and confidence distribution. Feed results into replay validation.
**Impact:** Provides real data to evaluate whether liquidation-cascade-hunter has practical edge.
**Prerequisites:** M3 liquidation capture (complete), M4 replay pipeline (complete).

### Mission C: Parameter Optimization via Walk-Forward Search
**Rank:** 3
**Description:** Run automated parameter sweeps for each strategy using the walk-forward backtest engine. Test parameter stability across train/test windows. Identify parameter sets that pass Sharpe ≥ 0.5 out-of-sample.
**Impact:** Addresses parameter fragility — current values are defaults or single-point estimates.
**Prerequisites:** Walk-forward engine (from prior mission), regime filter (from prior mission).

### Mission D: Liquidation Strategy Parameter Sensitivity
**Rank:** 4
**Description:** Sweep confidence threshold, hold time, and exit parameters for liquidation-cascade-hunter using captured replay data. Find the parameter region with best risk-adjusted returns.
**Impact:** The liquidation strategy has untested parameter sensitivity. Could be zero-signal or high-signal depending on threshold choices.
**Prerequisites:** Mission B (captured zone data), replay pipeline (complete).

---

## 7. Test Results

| Suite | Count | Status |
|-------|-------|--------|
| Rust tests | 736 passed | ✅ All pass |
| Python tests | 132 passed | ✅ All pass |
| `cargo build --release` | 0 errors, 0 warnings | ✅ Clean |
| `cargo clippy` | Clean | ✅ Clean |
| Scrutiny validators | 4/4 passed | ✅ All pass |

**Test growth during Imperial mission:** 414 → 736 (+322 new tests, +78% growth)

### Test Breakdown by Milestone

| Milestone | Cumulative Tests | New Tests Added |
|-----------|-----------------|-----------------|
| Pre-mission baseline | 414 | — |
| M1: Imperial Client | 472 | +58 |
| M2: Route Oracle | 512 | +40 |
| M3: Liquidation Capture | 619 | +107 |
| M4: Strategy + Replay | 736 | +117 |

---

## 8. Open Risks

| Risk | Severity | Description | Next Step |
|------|----------|-------------|-----------|
| **Imperial route savings unconfirmed** | High | Cost oracle shows theoretical savings but no live validation. | 90-day backtest with imperial routing (Mission A) |
| **Liquidation signal frequency unknown** | High | Strategy may produce zero signals in practice if cascades are rare. | 24–72h capture run (Mission B) |
| **Capture loop reliability** | Medium | Zone capture requires sustained runtime. Process crashes or API gaps could produce sparse data. | Monitor first capture run closely |
| **Confidence threshold is a guess** | Medium | Default 0.5 fusion threshold has no empirical basis. | Parameter sensitivity sweep (Mission D) |
| **Replay vs live correlation unproven** | Medium | Replay pipeline uses historical captured data; may not predict live performance. | Compare replay predictions against live paper trades |

---

## 9. Cross-Milestone Assertions

- **No live trading enabled:** All runs used `--paper` or `--backtest` modes. No `--keypair` flag used. ✅
- **No secrets committed:** No private keys, API tokens, or wallet secrets in git history. ✅
- **No risk limits weakened:** Risk config changes were additive only. No existing limits were decreased. ✅
- **All tests pass at every milestone:** 736 Rust tests; 132 Python tests throughout. ✅
- **No Imperial trading:** All Imperial API calls are read-only GET. No JWT, no auth headers. ✅
- **Liquidation strategy is paper-only:** `liquidation-cascade-hunter` gated behind promotion gate. Cannot run live without passing all replay criteria. ✅
- **All 4 scrutiny validators passed:** Each milestone validated before proceeding. ✅

---

## 10. Key Terms for Agent Continuity

- **Zekt**: The project. Rust binary + Python analysis pipeline for Solana perps trading.
- **Flash Trade**: Execution target. Solana perps via oracle-based REST API. Public, no auth.
- **Hyperliquid (HL)**: Intelligence layer. Wallet discovery, fill analysis, historical candles.
- **Imperial**: Solana perps aggregator. Read-only API for route cost comparison and liquidation OI data.
- **ImperialClient**: Read-only HTTP client for Imperial API. 10 GET endpoints. No auth.
- **RouteCostOracle**: Multi-venue cost estimation comparing Imperial routes vs Flash-only costs.
- **LiquidationZoneSnapshot**: Captured liquidation zone data per symbol per timestamp. Persisted to `data/liquidation-zones/`.
- **LiquidationCascadeHunter**: Paper-only strategy exploiting liquidation cascades (continuation + reversal setups).
- **ReplayPipeline**: Loads captured liquidation zone data, replays through strategy, evaluates promotion gate criteria.
- **PromotionGate**: Checks ALL criteria before strategy promotion: net expectancy, max drawdown, stale-data trades, duplicates, signal count, Sharpe ratio.
- **Strategy trait**: `Strategy: Send + Sync` with `detect_entry()`, `detect_exit()`, `parameters()`, `push_price()`, `snapshot()`.
- **RegimeLabel**: `LowVol`, `Trending`, `HighVol`, `Choppy` — detected by `RegimeDetector` in `regime.rs`.
- **RiskManager**: Holds `RiskConfig` with 12 fields. `check_can_trade()` gates all entries. Thread-safe via `Mutex` + `AtomicBool`.
- **BacktestEngine**: Replays candles through `Strategy` trait. Supports walk-forward, slippage, regime filtering, fee decomposition, and imperial-route-oracle cost mode.
- **Blueprint strategies**: `blueprint-scalper` (cluster-001), `blueprint-mean-revert` (cluster-004) — parameters from actual profitable HL wallet clusters.

---

## 11. Deliverables

| Deliverable | Type | Location |
|-------------|------|----------|
| Imperial client module | Code | `src/imperial.rs` |
| Route cost oracle | Code | `src/route_cost.rs` |
| Liquidation zone module | Code | `src/liquidation.rs` |
| Replay validation pipeline | Code | `src/replay.rs` |
| Liquidation cascade hunter strategy | Code | `src/strategy.rs` (new variant) |
| Config additions | Config | `config/perps.toml` ([imperial], [route-oracle], [liquidation]) |
| Imperial integration gate doc | Doc | `docs/imperial-integration-gate.md` |
| Liquidation zone methodology | Doc | `docs/liquidation-zone-methodology.md` |
| Liquidation capture summary | Data | `data/liquidation-zone-capture-summary.md` |
| Imperial route comparison | Data | `data/imperial-route-comparison.md` |
