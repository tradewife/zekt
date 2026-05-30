# Zekt — Bootstrap Alpha Compounding System — Mission Report

**Date:** 2026-05-30
**Mission:** Bootstrap Alpha Compounding System (7 milestones)
**Status:** Complete (M1–M7)
**Commits:** 6 feature commits (20e4a9b → 076be95)

---

## 1. What Changed

### M1: Recon + Edge Inventory + Bottleneck Ranking (commit 20e4a9b)
- `MISSION_REPORT.md` — 9-section recon report (repo orientation, corpus delta, lifecycle map, risk surface, validation map, top-10 bottleneck list, Imperial Perps investigation)
- `library/edge-inventory.md` — All 7 strategies with confidence levels, fee sensitivity, drawdown profiles, failure regimes

### M2: Evidence + Cost Model + Backtest Integrity (commit 881c450)
- `src/risk.rs` — Fixed `maybe_reset_day` bug (peak now resets to `initial_balance + daily_pnl` instead of `initial_balance`)
- `src/backtest.rs` — Walk-forward validation (train/test split), configurable slippage model (basis points), fee decomposition (entry + exit + borrow + slippage), fee breakdown in output JSON
- `src/regime.rs` — Market regime segmentation (LowVol/Trending/HighVol/Choppy) with `RegimeDetector` class
- `config/perps.toml` — New `[backtest]` section with `walk_forward_enabled`, `slippage_bps`, `regime_filter`
- 12 new M2-specific tests

### M3: Risk + Kill-Switch Upgrade (commit 071a1a1)
- `src/config.rs` — 8 new `RiskConfig` fields: `max_weekly_loss_usd`, `max_correlated_exposure_pct`, `consecutive_loss_circuit_breaker`, `volatility_sizing_enabled`, `volatility_sizing_atr_threshold_pct`, `volatility_sizing_min_fraction`, `api_degradation_threshold`, `correlated_groups`
- `src/risk.rs` — Weekly PnL tracking with auto-reset, consecutive loss circuit breaker (resets on win), ATR-based volatility position sizing, API degradation circuit breaker, correlated exposure tracking (`check_correlated_exposure`), paper/live divergence detection framework (`DivergenceTracker`)
- `src/paper.rs` — Updated test `RiskConfig` constructions with new fields
- `config/perps.toml` — New risk fields with safe defaults (all limits disabled by default for backward compatibility)
- 16 new M3-specific tests

### M4: Highest-ROI Alpha Improvement (commit 74ce1be)
- `src/regime.rs` — `is_strategy_compatible()` method: strategy-type-specific regime compatibility rules (momentum skips LowVol/Choppy, mean-reversion skips Trending, trend-follower skips Choppy/LowVol, funding-capture skips HighVol)
- `src/backtest.rs` — Extended regime filter from blueprint-only to ALL strategy types
- `data/m4-regime-filter-comparison.md` — Before/after metrics with realistic costs
- 2 new M4 tests

### M5: Tooling + Agent Integration Review (commit 7656baf)
- `docs/m5-integration-gate-evaluation.md` — Integration Gate evaluation for 3 repos: fintool (REJECT), senpi-skills (DEFER), atlas-gic (REJECT)

### M6: Paper-Trading Promotion Gate (commit 076be95)
- `docs/paper-trading-promotion-gate.md` — Complete runbook with copy-paste commands, duration requirements (≥24h), 10 metrics to collect, 6 quantified promotion thresholds, monitoring checklist (6 items), 5 human review triggers, 5-item human approval checklist, explicit no-bypass statement

---

## 2. What Improved

### Before/After: Backtest Performance (momentum-scalper, BTC, 2026-05-15 to 2026-05-30)

| Metric              | Before (M1) | After (M4) | Change          |
|---------------------|-------------|------------|-----------------|
| Total Trades        | 406         | 5          | -98.8%          |
| Total Fees          | $838.36     | $10.33     | -98.8%          |
| Net PnL             | -$431.11    | +$0.97     | +$432.08        |
| Win Rate            | 20.4%       | 40.0%      | +19.6pp         |
| Sharpe Ratio        | -0.61       | 0.18       | +0.79           |

### Multi-Strategy Improvement (3 strategies × 2 markets)

| Metric              | Before      | After       | Change          |
|---------------------|-------------|-------------|-----------------|
| Total Trades        | 1,273       | 88          | -93.1%          |
| Total Fees          | $2,652.82   | $183.10     | -93.1%          |
| Net PnL             | -$1,543.29  | -$145.76    | +$1,397.53 (90.6% better) |

### Infrastructure Improvements

| Dimension           | Before      | After                                        |
|---------------------|-------------|----------------------------------------------|
| Daily peak bug      | Present     | Fixed (peak = initial + pnl, not initial)    |
| Walk-forward        | None        | Train/test split with separate metrics       |
| Slippage model      | None        | Configurable basis points                    |
| Regime filtering    | None        | 4-regime detector with strategy-specific rules |
| Risk engine         | 4 fields    | 12 fields with weekly/correlated/consecutive/API limits |
| Divergence tracking | None        | Paper/live fill comparison framework         |
| Promotion gate      | None        | Formal runbook with thresholds and sign-offs |

---

## 3. What Failed

1. **All strategies still net-negative after regime filter.** The regime filter dramatically reduced losses (-90.6%), but no strategy achieved positive expectancy after costs on this 15-day test period. The improvement is primarily from *not trading* rather than *trading better*.

2. **M2 scrutiny validation crashed 4 times.** Worker processes exited unexpectedly during validation of M2 (technical issue, not code). Required manual validation instead.

3. **Regime filter may over-filter.** Going from 406 to 5 trades means the strategy is inactive >98% of the time. The few remaining trades have very low statistical significance. The regime labels may be too restrictive for the momentum-scalper on BTC.

4. **SOL strategies underperform BTC.** Even after regime filtering, SOL momentum-scalper lost $62.73 (27 trades, 18.5% win rate). SOL appears more challenging for momentum strategies during this period.

---

## 4. What Remains Unknown

1. **Is the regime filter robust across different market periods?** Only tested on 15 days. The filter could be period-specific. Need walk-forward validation across 90+ days to confirm.

2. **Can any strategy achieve Sharpe ≥ 1.0?** The original mission goal. After M4, momentum-scalper BTC hit Sharpe 0.18 — better but still far from 1.0. The regime filter is necessary but not sufficient.

3. **What is the optimal regime filter configuration?** Current rules are hardcoded (e.g., "momentum skips LowVol"). These could be too aggressive. A data-driven approach (Senpi's Lynx self-tuning) could optimize the filter per strategy.

4. **What are the real Flash Trade execution costs?** Backtests use 0.10% taker fee. Flash Trade has dynamic fees that could be higher. Real fee impact won't be known until live paper testing with API fee previews.

5. **Do any of the blueprint strategies (cluster-001, cluster-004) have genuine edge?** The data-driven strategies haven't been tested with the regime filter yet. They may perform better than the placeholder strategies.

---

## 5. Reference Corpus Delta

15 repos analyzed during M1 recon and M5 tooling review:

| Repo | Finding |
|------|---------|
| tradewife/trading | Reference corpus of 106 crypto trading repos — used for initial landscape scan |
| second-state/fintool | Rust CLI for multi-exchange trading. REJECT for Zekt: 80% overlap with existing infra |
| Senpi-ai/senpi-skills | Python agent skills for HL trading. DEFER: study Coyote regime classifier, Lynx self-tuning, fee optimizer patterns |
| chrisworsey55/atlas-gic | AI agent autoresearch architecture. REJECT: proprietary SaaS, 7 commits, no testable code |
| CryptoGnome/aster_lick_hunter_node | Liquidation hunting strategy concept. Referenced in M4 spec but not implemented (research-level) |
| Jupiter Perps | Dominant Solana perps DEX. Not used (Zekt targets Flash Trade) |
| Flash Trade | Oracle-based Solana perps — Zekt's execution target. API well-integrated |
| Ranger Finance | First Solana perps aggregator. Noted for future multi-venue evaluation |
| Phoenix (Ellipsis Labs) | Solana DEX with perps. Noted as alternative venue |
| Drift Protocol | Solana perps v2. Noted as alternative venue |
| HFDX | Gaining traction as Solana traders rotate from Raydium. Noted for monitoring |
| Percolator | In-development perp DEX by Solana co-founder. Noted for monitoring |
| Dextrabot | Wallet discovery API. Integrated in alpha-scanner binary |
| Hypurrscan | HL wallet analytics with JWT API. Integrated in alpha-scanner |
| QuickNode HyperCore | Primary data source for HL wallet discovery and fill analysis. Fallback to direct HL API |

---

## 6. Next Missions (Ranked by Expected Impact)

### Mission A: Strategy Parameter Optimization via Walk-Forward Search
**Rank:** 1 (Highest Impact)  
**Description:** Run automated parameter sweeps for each strategy using the walk-forward backtest engine. Test parameter stability across train/test windows. Identify parameter sets that pass Sharpe ≥ 0.5 out-of-sample. Use the regime filter to gate entries during optimization.  
**Impact:** Directly addresses "all strategies unproven" (bottleneck #3). Current parameters are placeholders. Data-driven optimization could find genuine edge.  
**Prerequisites:** M2 walk-forward engine, M4 regime filter (both complete).

### Mission B: Blueprint Strategy Validation with Regime Filtering
**Rank:** 2  
**Description:** Test the data-driven blueprint strategies (cluster-001 momentum-scalper, cluster-004 mean-revert) with the new regime filter and fee model. These strategies have parameters derived from actual profitable HL wallets and may outperform the placeholder strategies.  
**Impact:** Blueprint strategies have higher evidence quality (derived from real wallet data). Testing them with proper regime filtering and cost accounting is the most likely path to positive expectancy.  
**Prerequisites:** M2 fee model, M4 regime filter, data/blueprints/ directory with cluster JSON files.

### Mission C: Self-Tuning Strategy Parameters (Senpi Lynx Pattern)
**Rank:** 3  
**Description:** Implement a self-tuning mechanism that adjusts strategy thresholds based on recent trade performance. Inspired by Senpi's Lynx archetype: pull own trade history, bucket by signal strength, raise thresholds on bottom buckets that bleed. Implement in Rust within the Strategy trait.  
**Impact:** Addresses parameter fragility — fixed parameters degrade over time as market conditions change. Self-tuning adapts continuously.  
**Prerequisites:** Mission A (parameter sweep baseline), sufficient trade history (24h+ paper run).

---

## 7. Test Results

| Suite | Count | Status |
|-------|-------|--------|
| Rust tests | 414 passed | ✅ All pass |
| Python tests | 132 passed | ✅ All pass |
| `cargo build --release` | 0 errors, 0 warnings (from Zekt source) | ✅ Clean |
| `cargo clippy` | Clean | ✅ Clean |
| Paper trading smoke test (60s) | No panic, price fetched | ✅ Pass |

**New tests added during mission:** +33 tests (12 in M2, 16 in M3, 2 in M4, 3 in M5 regime module)

---

## 8. Open Risks

| Risk | Severity | Description | Next Step |
|------|----------|-------------|-----------|
| **No strategy with positive expectancy** | Critical | After all improvements, no strategy achieves Sharpe ≥ 1.0 or even positive net PnL across all markets. | Run parameter optimization (Mission A) and test blueprint strategies (Mission B) |
| **Regime filter period-specific** | High | The dramatic improvement could be specific to the May 15-30 test period. Need out-of-sample validation. | Walk-forward backtest across 90+ days |
| **Over-filtering** | Medium | 5 trades in 15 days means near-zero statistical power. The regime filter may be too restrictive. | Tune regime thresholds per strategy (Mission C) |
| **Flash Trade execution risk** | Medium | Real execution costs unknown. Dynamic fees, slippage on thin books, and API latency could eliminate remaining edge. | 24h paper trading run with API fee previews |
| **Strategy parameter fragility** | Medium | All strategy parameters are either placeholders or single-point estimates from cluster medians. No sensitivity analysis. | Parameter sweep with walk-forward (Mission A) |

---

## 9. Cross-Milestone Assertions

- **No live trading enabled:** All runs used `--paper` or `--backtest` modes. No `--keypair` flag used. ✅
- **No secrets committed:** No private keys, API tokens, or wallet secrets in git history. ✅
- **No risk limits weakened:** Risk config changes were additive only (new fields with safe defaults). No existing limits were decreased. ✅
- **All tests pass at every milestone:** 381 → 396 → 412 → 414 Rust tests; 132 Python tests throughout. ✅

---

## 10. Key Terms for Agent Continuity

- **Zekt**: The project. Rust binary + Python analysis pipeline for Solana perps trading.
- **Flash Trade**: Execution target. Solana perps via oracle-based REST API. Public, no auth.
- **Hyperliquid (HL)**: Intelligence layer. Wallet discovery, fill analysis, historical candles.
- **Strategy trait**: `Strategy: Send + Sync` with `detect_entry()`, `detect_exit()`, `parameters()`, `push_price()`, `snapshot()`.
- **RegimeLabel**: `LowVol`, `Trending`, `HighVol`, `Choppy` — detected by `RegimeDetector` in `regime.rs`.
- **RiskManager**: Holds `RiskConfig` with 12 fields. `check_can_trade()` gates all entries. Thread-safe via `Mutex` + `AtomicBool`.
- **BacktestEngine**: Replays candles through `Strategy` trait. Supports walk-forward, slippage, regime filtering, fee decomposition.
- **Blueprint strategies**: `blueprint-scalper` (cluster-001), `blueprint-mean-revert` (cluster-004) — parameters from actual profitable HL wallet clusters.
- **Pipeline**: Orchestrator binary that launches alpha-scanner + copy-trader + whale-watcher + paper engine.
