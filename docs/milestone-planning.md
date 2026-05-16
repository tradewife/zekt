# Alpha Hunter — Milestone Decomposition

## Overview

The Alpha Hunter mission is decomposed into **4 milestones**. Phase 5 (Market Scanner) is deferred as stated in the mission spec. Each milestone is a vertical slice that produces a testable, independently validatable artifact. The milestones are ordered so that each one builds on a stable foundation from the previous.

| # | Milestone | Mission Phases | Estimated Effort | Risk |
|---|-----------|---------------|-----------------|------|
| M1 | Wallet Intelligence Pipeline | 1 + 2 | Medium | Low |
| M2 | Strategy Trait Refactor | 3 (partial) | Medium | **High** |
| M3 | Strategy Implementation | 3 (remainder) | Medium | Medium |
| M4 | Paper Trading Validation | 4 | Medium + 24h wait | Low |

---

## Milestone 1: Wallet Intelligence Pipeline

**Mission Phases:** Phase 1 (Leaderboard Scraping) + Phase 2 (Wallet Analysis)

**Rationale:** These two phases form a natural data pipeline — scrape wallets, then analyze them. Both are purely additive (new `src/bin/` binaries), touching zero existing trading code. Combining them avoids an artificial handoff boundary where scraped data has no analysis tooling to consume it.

### Features

- `src/bin/scrape-leaderboards.rs` — CLI tool that scrapes wallet addresses + summary stats from perp DEX leaderboards (Flash Trade via fstats.io, Jupiter Perps, Hyperliquid). Outputs `data/wallets.json`.
- `src/bin/analyze-wallet.rs` — CLI tool that takes a wallet address (or batch file), fetches full trade history from the relevant DEX API, computes the 12-metric classification suite, and outputs per-wallet reports + aggregated strategy blueprints.
- Strategy classification engine — clip size consistency, hold time distribution, direction bias, fee-adjusted PnL, counterparty concentration, etc. (as specified in Phase 2).
- Strategy blueprint output — quantified parameter sets in `data/strategy-blueprints/{strategy-name}.json` suitable for direct Rust implementation.

### What Gets Validated

- `scrape-leaderboards --source flash --output data/wallets.json` produces valid JSON with 20+ wallets from 2+ sources.
- `analyze-wallet --wallets data/wallets.json --output data/reports/` produces per-wallet reports with all 12 metrics computed.
- At least 3 distinct strategy types classified (e.g., Momentum Scalper, Mean Reversion, LP Consumer).
- Fee-adjusted PnL computed for every wallet — wallets with net-negative fee-adjusted PnL are flagged and excluded from blueprint generation.
- Rate limiting works (no 429s on normal runs).

### Exit Criteria

- [ ] Both CLI binaries compile without warnings (`cargo build --release`)
- [ ] 50+ wallets scraped from 2+ sources
- [ ] 3+ strategy types classified with quantified blueprints
- [ ] Every blueprint has concrete `parameters.entry`, `parameters.exit`, and `parameters.risk` sections
- [ ] No changes to existing trading code (`engine.rs`, `paper.rs`, `signal.rs`, `risk.rs`)

### Risk Assessment: **Low**

- Purely additive — new binaries, no modification to existing code.
- External API availability is the main risk. Mitigated by: (a) fstats.io and Hyperliquid API verified working, (b) Solana RPC fallback always available for on-chain transaction parsing.
- New dependency (`scraper` crate for HTML parsing) is the only Cargo.toml change.
- If a leaderboard API is down, we can still proceed with whatever wallets we have — partial results are still useful.

### Key Decisions

- **Scrape + analyze as one milestone** rather than two: The analysis tooling is what makes the scraped data valuable. Separating them creates a milestone that produces raw data with no way to interpret it.
- **No strategy implementation in M1**: Blueprints are the handoff artifact. Implementation requires the trait refactor (M2) first.

---

## Milestone 2: Strategy Trait Refactor

**Mission Phases:** Phase 3 (partial — architecture only)

**Rationale:** This is the highest-risk change in the entire mission. It modifies `signal.rs`, `engine.rs`, `paper.rs`, and `config.rs` — all currently working code. Making it its own milestone ensures we can validate regression before layering new strategies on top. If the refactor breaks something subtle (e.g., trailing stop behavior for shorts), we catch it here rather than debugging it alongside new strategy implementations.

### Features

- Define `Strategy` trait in new `src/strategy.rs`:
  ```rust
  pub trait Strategy: Send + Sync {
      fn name(&self) -> &str;
      fn detect_entry(&self, snapshot: &PriceSnapshot) -> Option<EntrySignal>;
      fn detect_exit(&self, position: &Position, snapshot: &PriceSnapshot) -> Option<ExitSignal>;
      fn parameters(&self) -> &StrategyParams;
  }
  ```
- Refactor `MomentumDetector` from `signal.rs` into `MomentumScalperStrategy` implementing `Strategy`.
- Update `engine.rs` (`ScalperEngine`) to hold a `Box<dyn Strategy>` instead of `MomentumDetector` directly.
- Update `paper.rs` (`PaperEngine`) to hold a `Box<dyn Strategy>` instead of `MomentumDetector` directly.
- Add `--strategy` CLI flag and `[strategy] active = "momentum-scalper"` config field.
- Config TOML restructured: existing `[strategy]` fields moved to `[strategy.momentum-scalper]` sub-table.
- `signal.rs` retains `PricePoint`, `MomentumSnapshot`, `TradeDirection`, and signal/exit types — these become shared types used by the trait interface.

### What Gets Validated

- **Regression test**: Existing momentum scalper behavior is identical after refactor.
  - Run `--paper --market SOL` for 10+ minutes before and after refactor — same signal generation, same exit logic, same fee accounting.
  - Dry-run mode still works identically.
- Strategy selection via `--strategy momentum-scalper` produces same behavior as the pre-refactor code.
- Invalid strategy name produces a clear error, not a panic.
- `cargo build --release` succeeds with no warnings.

### Exit Criteria

- [ ] `Strategy` trait defined in `src/strategy.rs`
- [ ] Existing `MomentumDetector` logic fully encapsulated in `MomentumScalperStrategy`
- [ ] Both `ScalperEngine` and `PaperEngine` use `Box<dyn Strategy>`
- [ ] `--strategy` CLI flag and config field work
- [ ] Paper trading produces identical signals/behavior to pre-refactor baseline
- [ ] `cargo build --release` succeeds with no warnings
- [ ] No new strategies implemented yet — this milestone is purely architectural

### Risk Assessment: **High**

- **Highest-risk milestone in the project.** Modifies 4 core files that currently work correctly.
- Config format change (`[strategy]` → `[strategy.momentum-scalper]`) requires migration of existing `config/perps.toml`.
- The `detect_exit` interface needs to thread through 8+ parameters (entry_price, peak_price, hold_secs, tp_pct, sl_pct, trail_pct, trail_act_pct, max_hold_secs). The trait signature must handle this cleanly — likely via a `PositionContext` struct rather than individual parameters.
- `MomentumDetector` has mutable state (`prices: VecDeque`) — the trait design must accommodate stateful strategies. The `detect_entry` method likely needs `&mut self`, not just `&self`.
- **Mitigation**: This is why it's its own milestone. Any regression is caught immediately, not hidden under new strategy code.

### Key Decisions

- **Separate milestone from strategy implementation**: If we bundle the refactor + new strategies together, a regression could be caused by either the refactor OR the new strategy code. Separating them isolates the risk.
- **No unit tests yet** (per CLAUDE.md): Validation is via manual paper trading regression check. The 10+ minute paper run is the "test suite."
- **`strategy.rs` is a new file**, not a rename of `signal.rs`. `signal.rs` keeps shared types; `strategy.rs` has the trait + implementations.

---

## Milestone 3: Strategy Implementation

**Mission Phases:** Phase 3 (remainder — new strategies)

**Rationale:** Now that the trait framework is stable and validated, implementing new strategies is additive and low-risk. Each strategy is an isolated `impl Strategy` block.

### Features

- Implement 2-3 new strategies based on M1 blueprints (likely candidates):
  - **LP Consumption Detector** — the real edge from Bulk.Trade analysis. Detects when a single LP's depth is being consumed in one direction. Entry on consumption velocity, exit on momentum stall.
  - **Mean Reversion Scalper** — fades momentum spikes. Entry after sharp move + reversal signal, exit on return to mean.
  - **Trend Follower** (if M1 data supports it) — confirmed momentum breakouts, wider stops, trailing exits.
- Each strategy gets its own config sub-table: `[strategy.lp-consumption]`, `[strategy.mean-reversion]`, etc.
- Parameters pulled directly from M1 blueprints — no invention, no guessing.
- `--strategy <name>` flag activates the chosen strategy for both live and paper modes.

### What Gets Validated

- Each strategy compiles, instantiates, and generates entry/exit signals in paper mode.
- Strategy parameters match the blueprints from M1 (traceable: strategy → blueprint → source wallets).
- `--dry-run --strategy lp-consumption` produces a valid preview against live API.
- Paper trading with each strategy for 30+ minutes produces plausible trade logs (reasonable entry/exit timing, no stuck positions, no infinite loops).

### Exit Criteria

- [ ] 2+ new strategies implemented as `impl Strategy` in `src/strategy.rs`
- [ ] Each strategy has its own config sub-table in `config/perps.toml`
- [ ] `--strategy <name>` flag works for all strategies
- [ ] All strategies work with both `--paper` and `--dry-run` modes
- [ ] `cargo build --release` succeeds with no warnings
- [ ] Parameters match M1 blueprints (documented in code comments referencing blueprint files)

### Risk Assessment: **Medium**

- The trait framework is stable from M2, so strategy implementation is additive.
- Risk is in strategy quality — a poorly implemented strategy won't generate signals, or will generate too many. This is mitigated by strict adherence to M1 blueprints.
- LP Consumption Detector requires pool data that may not be available via current Flash Trade API endpoints — may need additional API methods in `flash_api.rs`.
- Mean Reversion requires a "mean" calculation (VWAP or moving average) that doesn't exist in the current codebase. This is straightforward but adds complexity.

### Key Decisions

- **Implement strategies from blueprints, not from theory**: The whole point of M1 is to extract real parameters from real profitable wallets. Using invented parameters defeats the purpose.
- **2-3 strategies, not more**: Each strategy needs 24h of paper validation (M4). More strategies = more validation time. Start with the highest-confidence blueprints.

---

## Milestone 4: Paper Trading Validation

**Mission Phases:** Phase 4 (Paper Trading)

**Rationale:** This is the truth-telling milestone. No strategy ships without positive net PnL after fees in 24+ hours of paper trading. The 24h run is a blocking operation, but the infrastructure to run it is lightweight — most of the work is in M3.

### Features

**Infrastructure (pre-run, ~1 hour):**
- Enhance `PaperEngine` to support multi-strategy execution: run multiple strategy instances simultaneously, each on its own set of markets.
- Add multi-market support: single paper engine instance monitors 3+ markets (SOL, BTC, ETH, ZEC, etc.) and generates signals per market per strategy.
- Result collection: each trade logged with strategy name, market, entry/exit prices, fees breakdown (entry + exit + borrow), gross PnL, net PnL.
- Summary report generator: after the run, produce `data/paper-results/summary.json` with the head-to-head comparison table (strategy × market ranked by net PnL).

**The 24h Run:**
- Start all strategy × market combinations simultaneously.
- Let them run for 24+ hours against live Flash Trade prices.
- No human intervention needed — ctrlc for graceful shutdown, results persist.
- Periodic status logs (every hour): per-strategy trade count, net PnL, win rate.

**Post-Run Analysis:**
- Generate ranked summary table: all strategy × market combinations sorted by net PnL (after fees).
- Flag which strategies are net positive, which are net negative.
- Identify best market for each strategy.
- Compute fee ratio (fees as % of gross PnL) — strategies with >100% fee ratio are net negative after fees.

### What Gets Validated

- Multi-strategy paper engine runs for 24+ hours without crashing, memory leaking, or getting stuck.
- Fee accounting is accurate: entry fee from live preview, exit fee from live preview, borrow fee accrued per hour.
- At least 2 strategies tested across 3+ markets each.
- Head-to-head comparison table produced.
- At least one strategy shows positive net PnL after fees (mission success criterion).

### Exit Criteria

- [ ] Paper engine runs 24+ hours without crash or memory issues
- [ ] All fee components tracked: entry fee, exit fee, borrow fee
- [ ] 2+ strategies × 3+ markets = 6+ strategy-market combinations tested
- [ ] Head-to-head summary table generated at `data/paper-results/summary.json`
- [ ] Fee ratio computed for every strategy-market pair
- [ ] **At least one strategy shows positive net PnL after fees over 24h** (mission success)
- [ ] If no strategy is net positive, the mission still completes with documented negative results and analysis of why

### Risk Assessment: **Low**

- Paper engine already works (M2 regression validated it). Multi-strategy is additive.
- The 24h run is I/O bound (API polls every 5s) — minimal CPU/memory risk.
- Main risk is that no strategy is net positive after fees. This is a valid outcome — the mission succeeds by identifying this truth. But we should optimize fee parameters before concluding.
- If Flash Trade API has downtime during the 24h run, the engine already handles errors with retry + sleep.

### Key Decisions

- **24h is a blocking wall-clock wait, not agent work**: The agent implements the infrastructure, starts the run, and then waits. Total agent effort is ~1 hour for infrastructure. The 24h is just waiting.
- **Multi-market simultaneous execution**: Rather than running strategies sequentially (24h × N strategies), run them all in parallel on different markets. This is more realistic (strategies compete for the same capital) and faster (total wall time ≈ 24h, not 24h × N).
- **Positive net PnL is the exit criterion, not gross PnL**: This is the lesson from the Bulk.Trade analysis. Three wallets were net negative after fees despite looking profitable.

---

## Deferred: Market Scanner (Phase 5)

Implemented only if M3/M4 results indicate strategies need market-specific targeting (e.g., LP Consumption only works on illiquid markets, but we don't know which ones). If strategies perform well across all tested markets, the scanner is unnecessary for mission success.

If needed, it would become **Milestone 5** as a thin CLI tool (`src/bin/scan-markets.rs`) that queries `GET /raw/markets` + `GET /pool-data` and ranks markets by LP concentration, liquidity thinness, and volatility.

---

## Dependency Graph

```
M1 (Wallet Intelligence)
 │
 ├── produces: strategy blueprints
 │
 ▼
M2 (Strategy Trait Refactor)  ← no dependency on M1, but M1 should run first for context
 │
 ├── produces: stable Strategy trait framework
 │
 ▼
M3 (Strategy Implementation)  ← depends on M1 blueprints + M2 trait
 │
 ├── produces: 2-3 new strategy implementations
 │
 ▼
M4 (Paper Trading Validation)  ← depends on M3 strategies
 │
 └── produces: 24h validated results, ranked strategy comparison
```

Note: M1 and M2 are technically independent (M1 produces data, M2 produces architecture). However, running M1 first is recommended because:
1. The M1 blueprints inform whether the Strategy trait design in M2 needs any accommodations (e.g., LP Consumption may need pool data in the snapshot).
2. M1 results may reveal that certain strategies require additional trait methods.

---

## Answer to Key Questions

### 1. Natural milestone boundaries where the product is testable?

- After M1: We have a database of wallets + classified strategies. Testable by examining blueprints.
- After M2: The trading engine works identically but is now extensible. Testable by paper trading regression.
- After M3: We have new strategies. Testable by short paper runs (30 min).
- After M4: We have validated results. Testable by reading the summary table.

### 2. Each milestone as a vertical slice?

- M1: scrape → analyze → blueprint (full data pipeline)
- M2: trait → refactor → validate regression (full architecture change)
- M3: blueprint → code → short test (full strategy implementation)
- M4: infrastructure → 24h run → analysis (full validation cycle)

### 3. Why 4 milestones?

- 3 would combine M2+M3 (risky — refactor + new code together)
- 5 would split M1 (scraping vs analysis) or M4 (infrastructure vs run)
- 4 is the natural granularity where each milestone has a distinct purpose and can be validated independently

### 4. Strategy trait refactor — own milestone or part of larger?

**Own milestone (M2).** The refactor touches 4 core files and changes the fundamental architecture. If bundled with new strategy implementation (M3), any regression could come from either the refactor or the new code, making debugging harder. Isolating it ensures the refactor is clean before building on it.

### 5. How to structure 24h paper trading?

- **Infrastructure** (M4 start): ~1 hour of agent work. Multi-strategy runner, multi-market support, result collection, summary generator.
- **The run**: Agent starts the process, then waits. The binary runs autonomously for 24h with periodic status logs. Ctrlc for graceful shutdown.
- **Post-run**: Agent analyzes results, generates summary table, identifies winning strategies.
- **Total agent effort for M4**: ~2 hours of active work + 24h of waiting. The waiting is not billable agent time — it's just the binary running.
