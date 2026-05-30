# Zekt — M1 Recon Report

**Date:** 2026-05-30
**Milestone:** m1-recon (Recon + Edge Inventory + Bottleneck Ranking)
**Status:** Complete

---

## 1. Repo Orientation

### What Zekt Is
Zekt is an autonomous strategy-poaching system that discovers profitable Hyperliquid wallets, reverse-engineers their strategies from fill-level data, and replicates them on Flash Trade (Solana perps). It consists of a Rust binary for trading infrastructure and a Python analysis pipeline for wallet intelligence.

### Architecture Summary
- **Rust binary** (single crate, `[[bin]]` entries): CLI routing → strategy detection → risk management → execution
- **Python pipeline** (6 modules in `analysis/`): position clustering → wallet metrics → strategy classification → blueprint generation
- **Data flow:** HL API → fill analysis → blueprints → backtest → paper → live

### Key Files (lines, role)
| File | Lines | Role |
|------|-------|------|
| `src/strategy.rs` | 6095 | Strategy trait + 7 implementations + factory function (63 tests) |
| `src/funding_capture.rs` | 1016 | Funding rate capture strategy (40 tests) |
| `src/paper.rs` | 2273 | Paper trading: single + MultiPaperEngine (14 tests) |
| `src/backtest.rs` | 1427 | HL candle fetcher + BacktestEngine (15 tests) |
| `src/risk.rs` | 306 | Risk manager: SL/TP/trailing, circuit breaker, daily reset |
| `src/engine.rs` | 617 | Live trading engine |
| `src/pnl_tracker.rs` | — | Combined PnL tracking (10 tests) |
| `src/flash_api.rs` | — | Flash Trade REST client |
| `src/hl_info.rs` | — | Hyperliquid Info API client |
| `src/signal.rs` | — | MomentumDetector, MomentumSnapshot, Signal types |
| `src/config.rs` | — | TOML config loader |
| `src/executor.rs` | — | Solana keypair loading + tx signing |
| `src/bin/pipeline.rs` | — | Orchestrator: alpha-scanner + copy-trader + whale-watcher (14 tests) |
| `src/bin/alpha-scanner.rs` | — | Wallet discovery daemon (64 tests) |
| `src/bin/copy-trader.rs` | — | Position mirroring engine (85 tests) |
| `src/bin/whale-watcher.rs` | — | WebSocket fill monitoring (41 tests) |
| `src/bin/scrape-leaderboards.rs` | — | QuickNode HyperCore wallet discovery (22 tests) |
| `src/bin/analyze-wallet.rs` | — | Strategy classification + blueprints (24 tests) |
| `src/bin/scan-markets.rs` | — | Flash Trade market ranking (18 tests) |
| `src/bin/scrape-dextrabot.rs` | — | Dextrabot API integration (8 tests) |

### Build & Test Baseline
- `cargo build --release`: ✅ Clean build, 0 errors, 0 warnings
- `cargo test`: ✅ 381 passed, 0 failed
- `python3 -m pytest analysis/tests/ -v`: ✅ 132 passed, 0 failed

### API Connectivity
- **Flash Trade** (`https://flashapi.trade/prices/BTC`): ✅ Returns valid JSON with Pyth oracle prices
- **Hyperliquid Info** (`POST https://api.hyperliquid.xyz/info` with `allMids`): ✅ Returns full market mid-price map

### Strategy Trait (Object-Safe)
All 7 strategies implement `Strategy: Send + Sync`:
```rust
fn name(&self) -> &str;
fn detect_entry(&mut self, snapshot: &MomentumSnapshot) -> Signal;
fn detect_exit(&self, snapshot: &MomentumSnapshot, context: &PositionContext) -> Option<Signal>;
fn parameters(&self) -> &StrategyParams;
fn push_price(&mut self, price: f64, timestamp_ms: i64);
fn snapshot(&self) -> MomentumSnapshot;
```

---

## 2. Reference Corpus Delta

The reference corpus is sourced from `github.com/stars/tradewife/lists/trading` (15 repositories). Each repo was assessed for relevance to Zekt's mission (strategy poaching, Solana perps, agentic trading).

| # | Repo | Stars | Language | One-Line Finding |
|---|------|-------|----------|------------------|
| 1 | **NoFxAiOS/nofx** | 12.5K | Go | AI trading terminal assistant for multi-asset (stocks, commodities, forex, crypto). Large community but not Solana-specific. No direct integration path for Zekt. |
| 2 | **TraderAlice/OpenAlice** | 4.6K | TypeScript | One-person Wall Street AI agent. Research → entry → management → exit. Architecture pattern (agent pipeline) is relevant but crypto execution layer differs from Flash Trade. |
| 3 | **Superior-Trade/superior-skills** | 217 | — | Trading skills/collection. Low-star, appears to be a skill pack. Minimal documentation for evaluation. |
| 4 | **Ellipsis-Labs/vulcan-cli** | 16 | Rust | Phoenix CLI for humans and agents. Directly relevant — Phoenix is a Solana DEX. Could inform Solana perp interaction patterns. |
| 5 | **cosmic-markets/cinder** | 27 | Rust | Terminal UI for Phoenix perpetuals on Solana. Live charts, order book, signed transactions. **High relevance** — Phoenix perps on Solana overlap with Flash Trade target market. Rust patterns for Solana perp trading are directly applicable. |
| 6 | **zostaff/ai-quant-researcher** | 90 | Python | AI quant research agent. Research-oriented, not execution. Analysis patterns could inform wallet analysis improvements. |
| 7 | **shiyu-coder/Kronos** | 27.4K | Python | Foundation model for financial markets language. Massive project but academic/research. No direct integration but represents the frontier of AI-financial modeling. |
| 8 | **Fincept-Corporation/FinceptTerminal** | 24.5K | C++ | Modern finance analytics terminal. General-purpose, not crypto-specific. No integration path for Zekt's Solana perps focus. |
| 9 | **second-state/fintool** | 295 | Rust | **High relevance** — Rust CLI tools for agentic trading across HL/Binance/Coinbase/OKX. Dedicated HL binary. Patterns for HL API integration, fee handling, and agent workflows are directly applicable. Scheduled for M5 Integration Gate evaluation. |
| 10 | **The-Swarm-Corporation/AutoHedge** | 2.9K | Python | Autonomous hedge fund using swarm intelligence. AI agent architecture patterns relevant to Zekt's multi-strategy pipeline design. |
| 11 | **virattt/ai-hedge-fund** | 59.5K | Python | AI Hedge Fund Team — most-starred in corpus. Multi-agent architecture (analyst, risk manager, trader). No Solana/crypto-specific code but architecture patterns are highly relevant. |
| 12 | **jup-ag/cli** | 51 | TypeScript | CLI for Jupiter products (Spot, Perps, Lend). **High relevance** — Jupiter Perps is a major Solana perps DEX competing with Flash Trade. Could inform alternative execution paths. |
| 13 | **chrisworsey55/atlas-gic** | 1.9K | Python | Self-improving AI trading agents using autoresearch. Karpathy-style approach. **High relevance** — scheduled for M5 Integration Gate evaluation. |
| 14 | **Nunchi-trade/agent-cli** | 520 | Python | Trading agent CLI. Agent execution patterns relevant. No Solana-specific code. |
| 15 | **Senpi-ai/senpi-skills** | 85 | Python | **High relevance** — Agent skills for autonomous crypto trading on Hyperliquid. Trailing stops, market scanning, position management. Directly overlaps with Zekt's HL intelligence layer. Scheduled for M5 Integration Gate evaluation. |

### Key Takeaways from Corpus
1. **Rust Solana perps tooling is sparse** — `cosmic-markets/cinder` and `Ellipsis-Labs/vulcan-cli` are the only Rust projects targeting Solana perps directly
2. **second-state/fintool** is the most relevant Rust project for HL API patterns (M5 target)
3. **Senpi-ai/senpi-skills** has the most direct HL agent skill overlap (M5 target)
4. **Jupiter CLI** provides TypeScript patterns for Solana perps that could inform Flash Trade integration
5. AI agent architectures (OpenAlice, ai-hedge-fund, AutoHedge) follow a pipeline pattern similar to Zekt's research → backtest → paper → live flow

---

## 3. Trading Lifecycle Map

The trading lifecycle in Zekt follows this pipeline:

```
1. DISCOVER    → Scrape HL leaderboards (QuickNode HyperCore API)
                  → Alpha-scanner daemon: Dextrabot + Hypurrscan enrichment
                  → Whale-watcher: WebSocket fill monitoring
                  
2. ANALYZE     → Python pipeline:
                  position_clustering.py → wallet_metrics.py → strategy_classifier.py
                  → entry_reconstruction.py → cluster_analysis.py → blueprint_generator.py
                  → Strategy blueprints (JSON) with data-derived parameters
                  
3. IMPLEMENT   → Rust Strategy trait implementation
                  → Factory function: create_strategy_from_config(name, sub_table, params)
                  → 7 strategies: momentum-scalper, lp-consumption, mean-reversion,
                     trend-follower, funding-capture, blueprint-scalper, blueprint-mean-revert
                  
4. BACKTEST    → BacktestEngine replays HL historical candles
                  → Strategy.detect_entry/exit called per candle
                  → Fee model: entry fee (0.1%) + exit fee (0.1%) + borrow fee (0.01%/hr)
                  → Output: summary JSON with PnL, trade count, fee breakdown
                  → Current gap: no walk-forward validation, no slippage model
                  
5. PAPER TRADE → MultiPaperEngine: strategy × market matrix
                  → Live Flash Trade prices via REST API
                  → Fee estimation: live API preview + 0.01%/hr borrow estimate
                  → Risk manager: daily loss, drawdown, position limits
                  → Output: paper-trades.json with full trade log
                  → Minimum duration gate: 24h (not yet enforced by code)
                  
6. LIVE        → ScalperEngine: real execution
                  → Flash Trade API → tx builder → sign with Solana keypair → submit via RPC
                  → Risk manager with circuit breaker
                  → Trade journal: perps-trades.json (atomic writes)
                  → REQUIRES: funded Solana wallet with USDC + human approval
```

### Strategy Detection Flow (per tick)
```
push_price(price, ts) → price buffer updated
                         ↓
detect_entry(snapshot) → MomentumDetector.analyze()
                         → Check velocity > threshold
                         → Check lookback count
                         → Return Signal::MomentumLong/Short/NoSignal
                         ↓ (if in position)
detect_exit(snapshot, ctx) → Priority order:
                              1. Stop-loss
                              2. Take-profit
                              3. Trailing stop (after activation)
                              4. Time stop
                              5. Strategy-specific exit (consumption stall, mean return, etc.)
```

### Fee Model (Current)
- **Entry fee:** 0.1% of notional (Flash Trade taker fee, from live API preview)
- **Exit fee:** 0.1% of notional (same, from live API preview)
- **Borrow fee:** 0.01%/hr on notional (conservative estimate for dynamic Flash fees)
- **Slippage:** NOT modeled (instant fill at candle close in backtest)
- **Funding:** Not charged in backtest; captured as yield in funding-capture strategy

---

## 4. Risk Surface Map

### Current Risk Controls
| Control | Location | Behavior |
|---------|----------|----------|
| Daily loss limit | `risk.rs` | Halts trading when daily PnL < -max_daily_loss_usd |
| Max drawdown | `risk.rs` | Halts when balance drops below peak * (1 - max_drawdown_pct) |
| Position size limit | `risk.rs` | Rejects positions exceeding max_position_notional_usd |
| Total notional limit | `paper.rs` | Sum of all cell positions ≤ max_total_notional_usd |
| Cooldown after loss | `risk.rs` | Prevents entry for cooldown_after_loss_secs after any loss |
| Circuit breaker | `risk.rs` | Sets `halted` flag; no trades until manual intervention |
| Time stop | `strategy.rs` | Force-closes positions exceeding max_hold_secs |
| Stop-loss/take-profit | `strategy.rs` | Per-strategy SL/TP via price monitoring |
| Trailing stop | `strategy.rs` | Per-strategy trailing stop after activation threshold |
| 2h min hold (SL) | `strategy.rs` | Stop-loss only fires after 2h hold (recently added) |

### Known Risk Gaps (Pre-M3)
1. **maybe_reset_day bug**: Peak balance resets to `initial_balance` instead of `initial_balance + daily_pnl` on day rollover. This means drawdown calculations may be incorrect after the first day.
2. **No weekly loss limit**: A strategy can lose `max_daily_loss_usd` every day indefinitely.
3. **No correlated exposure limit**: Multiple strategies can open positions on the same or correlated markets (e.g., BTC + ETH) without aggregate exposure checks.
4. **No consecutive loss breaker**: A string of consecutive small losses won't trigger any circuit breaker.
5. **No volatility-based sizing**: Position sizes are fixed, not adjusted for market volatility.
6. **No API degradation breaker**: If the Flash Trade API starts failing, the engine will keep retrying without halting.
7. **No paper/live divergence detection**: No framework to detect when live execution diverges from paper predictions.
8. **No slippage model**: Backtest assumes instant fill at candle close — real execution will have worse prices.

### Risk from External Dependencies
| Risk | Description | Mitigation |
|------|-------------|------------|
| Flash Trade API downtime | No price feed → no signals | DNS fallback added; need circuit breaker |
| HL API rate limits | 1200 weight/min/IP | Auto-paginated with max 5000 candles/request |
| Solana RPC congestion | Failed tx submission | Not applicable in paper mode |
| QuickNode endpoint loss | No wallet discovery | Falls back to direct HL Info API |
| Oracle price staleness | Pyth oracle lag on Flash Trade | Confidence interval checked in API response |

---

## 5. Validation Map

### Validation Contract Assertions (M1 — VAL-RECON-*)
| Assertion | Description | Status | Evidence |
|-----------|-------------|--------|----------|
| VAL-RECON-001 | `cargo build --release` exits 0, 0 errors | ✅ PASS | Clean build, 0 errors, 0 warnings |
| VAL-RECON-002 | `cargo test` 0 failures, ≥381 total | ✅ PASS | 381 passed; 0 failed |
| VAL-RECON-003 | `python3 -m pytest` 0 failures | ✅ PASS | 132 passed in 0.31s |
| VAL-RECON-004 | MISSION_REPORT.md has ≥5 required sections | ✅ PASS | This document: 5 main sections + bottleneck list + Imperial Perps |
| VAL-RECON-005 | Edge inventory has all 7 strategies | ✅ PASS | Updated in library/edge-inventory.md |
| VAL-RECON-006 | 10-item ranked bottleneck list | ✅ PASS | See section 6 below |
| VAL-RECON-007 | Flash Trade API reachable | ✅ PASS | `curl -sf https://flashapi.trade/prices/BTC` returns valid JSON |
| VAL-RECON-008 | HL Info API reachable | ✅ PASS | `POST allMids` returns full market mid-price map |
| VAL-RECON-009 | Backtest runs and produces output | ✅ PASS | Momentum-scalper on BTC, 4369 candles, 22 trades, exits with code 0 |
| VAL-RECON-010 | Paper trading starts without panic | ✅ PASS | 30s timeout run (exit 124), initial SOL price $82.65, no panic |
| VAL-RECON-011 | Imperial Perps investigation documented | ✅ PASS | See section 7 below |

### Future Milestone Validation (Not Yet Run)
- **M2 (Cost Model):** VAL-COST-001 through VAL-COST-011 — walk-forward, slippage, fee audit
- **M3 (Risk):** VAL-RISK-001 through VAL-RISK-013 — new risk limits, no bypass paths
- **M4 (Alpha):** VAL-ALPHA-001 through VAL-ALPHA-008 — single improvement, before/after metrics
- **M5 (Tooling):** VAL-TOOL-001 through VAL-TOOL-007 — integration gate for 3 external tools
- **M6 (Gate):** VAL-GATE-001 through VAL-GATE-009 — paper trading promotion gate
- **M7 (Report):** VAL-REPORT-001 through VAL-REPORT-010 — mission memory

---

## 6. Ranked Top-10 Bottleneck List

Ranked by expected impact on profitable survival of a small account ($1000).

| Rank | Bottleneck | Severity | Impact | Description |
|------|-----------|----------|--------|-------------|
| **1** | **No walk-forward validation** | Critical | All strategies are likely overfit to their training window. Backtests run on a single contiguous period with no out-of-sample testing. Any "passing" Sharpe ratio is unreliable. | Need train/test split with separate metrics. M2 target. |
| **2** | **No slippage model** | Critical | Backtests assume instant fill at candle close. Real execution on Flash Trade will have worse fills, especially for thin order books. A 0.05-0.1% slippage per trade can eliminate all edge on a scalper. | Need configurable slippage (basis points). M2 target. |
| **3** | **All strategies unproven** | Critical | All 42 strategy/market combinations previously failed Sharpe ≥ 1.0 over 90 days. No strategy has demonstrated positive expectancy after costs. The system has edge infrastructure but no confirmed edge. | Need M4 alpha improvement after M2/M3 fixes. |
| **4** | **maybe_reset_day bug** | High | Daily peak balance resets to `initial_balance` instead of current balance on day rollover. This means drawdown calculations are wrong after day 1, and the circuit breaker may not trigger when it should. | Fix in M2. One-line change in risk.rs. |
| **5** | **Thin risk engine** | High | Missing: weekly loss limit, correlated exposure limit, consecutive loss breaker, volatility-based sizing, API degradation breaker. A small account can be destroyed by a single bad day or a string of small losses. | M3 target — comprehensive risk engine upgrade. |
| **6** | **Fee sensitivity untested** | High | Strategies use 0.1% taker fee in backtest but no sensitivity analysis has been done. If real fees are higher (Flash Trade dynamic fees), strategies that look marginal will fail. | Need fee sensitivity sweep in M2. |
| **7** | **No regime filter** | Medium | Strategies trade in all market conditions. Momentum scalper performs poorly in ranging markets; mean-reversion fails in trends. No filter prevents trading during unfavorable regimes. | M4 target — regime-aware entry filtering. |
| **8** | **Flash Trade market depth unknown** | Medium | LP consumption strategy depends on pool utilization data, but Flash Trade market depth and liquidity characteristics are not well-characterized. Edge assumptions may be wrong for thin markets. | Need market characterization scan (scan-markets binary exists). |
| **9** | **Single execution venue** | Medium | Zekt only executes on Flash Trade. No fallback if Flash Trade is down, has wider spreads, or changes fee structure. No comparison against other Solana perps (Jupiter Perps, Phoenix). | Consider multi-venue evaluation in future missions. |
| **10** | **No automated promotion gate** | Low-Medium | Paper-to-live promotion requires manual human review, but no formal runbook, metrics checklist, or promotion thresholds exist yet. Risk of premature live promotion. | M6 target — formal promotion gate. |

---

## 7. Imperial Perps Investigation

### Findings
A thorough search for "Imperial Perps" as a Solana perpetuals aggregator yielded no results. There is no known product, protocol, or company by that name in the Solana perps ecosystem.

The Solana perps landscape currently includes:
- **Jupiter Perps** — Dominant Solana perps DEX, $3B TVL, up to 250x leverage, JUP token
- **Flash Trade** — Oracle-based Solana perps (Zekt's current execution target)
- **Ranger Finance** — First Solana perps aggregator, smart order routing across venues
- **Phoenix (Ellipsis Labs)** — Solana DEX with perps support, has a CLI (`vulcan-cli`)
- **Percolator** — In-development perp DEX by Solana co-founder Anatoly Yakovenko
- **Drift Protocol** — Solana perps with v2 order book
- **HFDX** — Gaining traction as Solana traders rotate from Raydium

### Verdict: **DEFER**
"Imperial Perps" does not appear to be an existing product. Possible explanations:
1. The name may be a placeholder or codename for an unreleased product
2. It may refer to a concept (imperial = dominant/leading perps) rather than a specific product
3. It may be a misunderstanding or conflated with one of the existing Solana perps protocols

**Recommendation:** Defer investigation. If the user provides more context (URL, team name, or chain), revisit. For now, the Solana perps aggregator space is well-served by Ranger Finance, and the dominant execution venue is Jupiter Perps.

---

## 8. Commands Run

### Build & Test
```bash
cargo build --release           # Exit 0 — clean build
cargo test                      # Exit 0 — 381 passed, 0 failed
python3 -m pytest analysis/tests/ -v  # Exit 0 — 132 passed
```

### API Connectivity
```bash
curl -sf https://flashapi.trade/prices/BTC   # Exit 0 — valid JSON with Pyth price
curl -sf -X POST https://api.hyperliquid.xyz/info -d '{"type":"allMids"}'  # Exit 0 — full market map
```

### Backtest
```bash
./target/release/zekt --backtest --strategies momentum-scalper --markets BTC \
  --backtest-start 2026-05-15 --backtest-interval 5m --paper-balance 1000
# Exit 0 — 4369 candles loaded, 22+ signals generated, time-stop exits visible
```

### Paper Trading (30s smoke test)
```bash
timeout 30 ./target/release/zekt --paper --strategies momentum-scalper --markets SOL --paper-balance 1000
# Exit 124 (timeout) — initial price $82.65, no panic, no errors
```

---

## 9. Open Risks

| Risk | Severity | Description | Next Step |
|------|----------|-------------|-----------|
| No confirmed edge | Critical | All strategies failed Sharpe ≥ 1.0. System has infrastructure but no proven alpha. | M2/M4: fix backtest integrity, then measure after realistic costs |
| Overfit risk | High | No walk-forward validation, no parameter stability checks. | M2: add train/test split |
| Drawdown calculation bug | High | maybe_reset_day resets peak incorrectly after day rollover. | M2: one-line fix in risk.rs |
| Small account survival | High | $1000 account can be destroyed quickly without weekly limits, consecutive loss breakers. | M3: comprehensive risk engine upgrade |
| Slippage unknowns | Medium | Backtest assumes zero slippage. Real execution may eliminate marginal edge. | M2: add slippage model |
| Single venue risk | Low | Only Flash Trade. No fallback if venue degrades. | Future: evaluate Jupiter Perps as alternative |

---

## 10. Strategy Confidence Table (M2 — VAL-COST-010)

Each strategy ranked by evidence quality. Confidence levels: HIGH (data-driven + validated), MEDIUM (data-driven but unvalidated), LOW (placeholder parameters, no live evidence).

| Strategy | Confidence | Evidence Quality | Fee Sensitivity | Drawdown Profile | Failure Regimes | Key Risk |
|----------|-----------|-----------------|----------------|------------------|----------------|---------|
| momentum-scalper | LOW | Placeholder params, no live PnL data | High — thin margins eroded by fees | Unknown — no backtest validation yet | Choppy/range markets; false breakouts | Overfit to noise in 5m candles |
| lp-consumption | LOW | Placeholder params, theoretical edge only | Medium — depends on LP depth | Shallow (tight TP/SL) | Low-liquidity markets, thin books | Edge may not exist on Flash Trade |
| mean-reversion | LOW | Placeholder params, no data lineage | High — mean reversion margins thin | Moderate — fat tails can hit SL hard | Strong trending markets; regime change | Assumes stationarity that crypto lacks |
| trend-follower | LOW | Placeholder params, theoretical edge | Low — wide TP/SL, fewer trades | Deep (holds through drawdowns) | Choppy/range markets; whipsaws | Low win rate requires strong winners |
| funding-capture | MEDIUM | Live HL funding data, delta-neutral theory | Low — funding yields offset fees | Shallow (capped by SL) | Low funding rate periods; adverse price moves | No spot hedge → directional exposure |
| blueprint-scalper | MEDIUM | Cluster-001 data: 12 wallets, 4711 trades, conf=0.73 | High — 0.14% SL is very tight | Unknown — need walk-forward validation | Regime shift from cluster conditions | Overfit to historical HL data |
| blueprint-mean-revert | MEDIUM | Cluster-004 data: 5 wallets, 518 trades, conf=0.83 | High — 0.29% SL is very tight | Unknown — need walk-forward validation | Regime shift from cluster conditions | Small sample (5 wallets) |

### Assessment Summary
- **No strategy has HIGH confidence** — all are either placeholder-based or data-driven but unvalidated with walk-forward testing
- **Fee sensitivity is the primary concern** — most strategies have tight TP/SL where fees dominate
- **M2 deliverables (walk-forward, slippage model, fee audit) are essential** before any strategy can be promoted
- **Recommended M4 focus**: Walk-forward parameter stability for blueprint strategies (highest ROI)
