# Zekt — Strategy Poaching Recovery Mission

## The Core Problem

We lost the thesis. The mission was: **scrape profitable wallets from perp DEX leaderboards → reverse-engineer their exact bot strategies → replicate them on Flash Trade**. This worked brilliantly in the Bulk.Trade analysis (5 wallets running identical ZEC momentum scalper: 10 ZEC fixed clips, 40x leverage, 98% single-counterparty fills, 83% win rate). But the current Zekt codebase has generic invented strategies because:

1. **Flash Trade has no `userFills` API** — we only got aggregate fstats.io stats (total PnL, win rate), NOT individual fill records. Without fill-level data, we can't identify clip sizes, counterparty concentration, or hold time distribution.
2. **Blueprints show `"markets": ["UNKNOWN"]"`** — we never learned what assets profitable wallets actually trade. The entire strategy selection was blind.
3. **All 4 strategies are generic velocity signals** — invented parameters, not extracted from real profitable wallets. The backtesting pipeline proves they have zero edge on BTC/SOL 5-min candles.
4. **LP Consumption (the real Bulk edge) is dead code** — `PoolSnapshot` is never populated because no engine loop calls `get_pool_data()`. The one strategy with a real thesis doesn't fire. **[FIXED in M1]**
5. **`scrape-leaderboards.rs` uses fake Hyperliquid seed addresses** — the curated `curated_hyperliquid_seed()` function contains made-up EVM addresses with made-up stats. `userFills` returns 0 results for all of them.

## The Pivot: Hyperliquid as Primary Intelligence Source

Hyperliquid has everything we need that Flash Trade lacks:

| Capability | Hyperliquid | Flash Trade |
|---|---|---|
| Individual fills per wallet | `userFills` — 2000 fills, 10K recent | None — would need Solana tx parsing |
| Fill fields returned | coin, side, px, sz, fee, closedPnl, time, dir, hash | N/A |
| Leaderboard data | QuickNode HyperCore API + frontend scraping | fstats.io (aggregate only — no fills) |
| Historical OHLCV | `candleSnapshot` (already integrated in backtest.rs) | No historical data |
| Pool/LP data | Per-market data available | `get_pool_data` (returns data, now used after M1 fix) |
| Trader base size | 1.5M+ wallets | ~50 wallets surfaced by fstats |
| Current positions with PnL | `clearinghouseState` includes leverage + uPnL | `positions/owner/` (no fill history) |

**Strategy: Research on Hyperliquid → Execute on Flash Trade.** Discover winning strategies by deep-analyzing HL fill data (where rich data exists), then replicate the extracted strategies on Flash Trade (where the execution target lives). HL is the intelligence layer; Flash Trade is the execution layer.

---

## ✅ Immediate P0 Fixes (COMPLETED — Milestone 1)

These bugs made the current system produce wrong numbers. **All three are now fixed:**

1. **~~PaperPosition borrow accrual hardcodes 5s~~** ✅ FIXED — `PaperPosition::update_price()` now reads `poll_interval_secs` from config (300s default). Borrow fees were 60x understated; now correctly computed as `(1.0/3600.0) * poll_interval_secs` hours per tick.

2. **~~Cross-cell position limit missing~~** ✅ FIXED — MultiPaperEngine now enforces `max_total_notional_usd` (default $10,000) by summing all `CellPosition.size_usd` values before opening a new position. Prevents uncontrolled exposure across strategy×market cells.

3. **~~Pool/Snapshot never populated~~** ✅ FIXED — Both `engine.rs` and `paper.rs` tick loops now call `flash.get_pool_data()` per tick and construct a `PoolSnapshot` that feeds into strategy snapshots. LP Consumption strategy is now live.

---

## The Pipeline (Rearchitected)

```
┌─────────────────────────────────────────────────────────────────┐
│                  INTELLIGENCE LAYER (Hyperliquid)                │
│                                                                  │
│  HL Leaderboard → Wallet Addresses → userFills → Fill Records   │
│                                           │                      │
│                                    Bulk-Style Analysis           │
│                                           │                      │
│                                    Strategy Blueprints           │
│                                           │                      │
│                                    Backtest on HL candles        │
└──────────────────────────────────────┬──────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────┐
│                  EXECUTION LAYER (Flash Trade)                    │
│                                                                   │
│  Implement extracted strategies → Paper trade with live fees     │
│                             │                                     │
│                     24h net positive PnL?                         │
│                             │                                     │
│                        Live trading                               │
└──────────────────────────────────────────────────────────────────┘
```

---

## Phase A: Real HL Wallet Discovery via QuickNode

**Goal:** 100+ real profitable HL wallet addresses with actual trading activity.

Replace the current `curated_hyperliquid_seed()` (fake addresses with 0 fills) with real data from QuickNode's HyperCore API:

### QuickNode HyperCore API — Primary HL Data Source

**Configuration:** Set `QUICKNODE_HL_URL` env var or pass `--quicknode-url` CLI flag. Create a QuickNode account at quicknode.com, provision a Hyperliquid HyperCore endpoint (free tier available).

**Key QuickNode methods for wallet discovery:**
- `hl_batchClearinghouseStates` — Batch scan wallet positions (efficient for 100+ wallets in single request)
- `hl_batchPortfolioStates` — Portfolio snapshots for multiple wallets
- `userFills` / `userFillsByTime` — Validate wallet activity and compute initial PnL (returns coin, side, px, sz, fee, closedPnl, time, dir, hash, startPosition)

### Implementation Plan

1. **Extend `scrape-leaderboards.rs`** to use QuickNode HyperCore API endpoint (configurable via `--quicknode-url` flag or `QUICKNODE_HL_URL` env var)
2. **Use batch methods** (`hl_batchClearinghouseStates`) to scan wallets efficiently (not one-at-a-time)
3. **Validate with `userFills`/`userFillsByTime`** — verify wallet activity, compute initial PnL
4. **Filter criteria:** >50 fills, positive net PnL after fees, active in last 30 days
5. **Output:** `data/wallets-hl.json` with real addresses and summary stats
6. **Remove or disable** old `curated_hyperliquid_seed()` function — no more fake data

**Fallback:** Direct Hyperliquid Info API (`api.hyperliquid.xyz/info`) for `userFills`, `userFillsByTime`, `clearinghouseState` when QuickNode is not configured.

**Acceptance:** 100+ addresses where `userFills` returns actual fill data (not empty arrays).

---

## Phase B: Fill-Level Deep Analysis (The Bulk Method)

**Goal:** For each wallet, fetch `userFills` and replicate the exact Bulk.Trade analysis methodology.

The HL `userFills` response mirrors what Bulk.Trade gave us:

```json
{
  "coin": "BTC", "side": "B", "px": "104500.0", "sz": "0.01",
  "fee": "0.00105", "closedPnl": "45.20", "time": 1716163200000,
  "dir": "Open Long", "hash": "0x...", "startPosition": "0.015"
}
```

This enables ALL the Bulk.Trade metrics:

| Metric | How We Compute It | Bulk Equivalent |
|---|---|---|
| **Clip size consistency** | % of fills where `sz` is within ±10% of the median size | Same (10 ZEC clip was 80-88% consistent) |
| **Hold time distribution** | Cluster open/close fills into positions; compute entry→exit time per cluster | Same (5-75 min, median 20 min) |
| **Direction bias** | Ratio of Open Long / Open Short fills | Same |
| **Win rate** | % of position clusters where cumulative `closedPnl` > 0 | Same (67-83%) |
| **PnL distribution** | Per-cluster PnL: mean, median, max winner, max loser, skewness | Same |
| **Fee-adjusted PnL** | Sum of all `closedPnl` — sum of all `fee` | Same (3/10 wallets were net negative after fees) |
| **Market concentration** | Count of unique `coin` values per wallet. Specialist = 1-2 markets | Same (5/5 ZEC wallets traded ZEC-USD only) |
| **Fill interval** | Time gap between consecutive fills. Sub-30s = bot | Same |
| **Scale-in behavior** | Multiple `Open Long` fills on same `coin` within N minutes = scaled entry | Same (ZEC scalper used 5-10 fills per entry) |
| **Time-of-day patterns** | Active hours distribution. 24/7 = bot | Same |

**Implementation:** Python analysis pipeline in `analysis/` directory:
- `position_clustering.py` — Group fills into open→close position cycles
- `wallet_metrics.py` — Per-wallet metrics (clip consistency, hold time, win rate, PnL, fee-adjusted, fill intervals)
- `strategy_classifier.py` — Classify wallets into strategy types with confidence and evidence
- `entry_reconstruction.py` — Reconstruct entry triggers from candle data
- `cluster_analysis.py` — Find groups of wallets running identical strategies
- `blueprint_generator.py` — Generate data-derived strategy blueprints

Tests: Each module has a corresponding `tests/test_<module>.py`. Run with `python -m pytest analysis/tests/ -v`.

**Acceptance:** At least 3 distinct strategy clusters identified across wallets, with fill-level evidence backing each classification.

---

## Phase C: Entry Trigger Reconstruction

**Goal:** Don't just classify strategies — reconstruct the actual entry signals.

For each entry cluster (group of fills opening a position):
1. Query HL `candleSnapshot` for the 30 minutes BEFORE the first open fill
2. Compute: price velocity, volume spike, ATR expansion, consecutive directional moves
3. Find the common pattern across all entries by the same wallet
4. If wallet #1, #2, and #3 all enter on a 0.3% velocity + 3 consecutive tick pattern → that's the entry signal

**This is what made the Bulk analysis successful:** they found that 5 different wallets all used identical 10-ZEC clips, 40x leverage, and entered on the same LP consumption signal. We need to find the Hyperliquid equivalent of that pattern.

---

## Phase D: Flash Trade Market Intelligence

**Goal:** Identify which Flash Trade markets are exploitable and which HL strategies transfer.

1. **Pool data ranking** — Fetch `get_pool_data()` for all Flash Trade markets. Score each by:
   - LP concentration (higher = better for LP consumption strategies)
   - Available leverage (higher = more capital-efficient)
   - 24h volume (enough to fill our clips without slippage)
   - Utilization ratio (moderate = can enter; too high = don't enter)

2. **Asset mapping** — Match HL coins to Flash Trade symbols (BTC→BTC, SOL→SOL, etc.). Identify assets ONLY on Flash Trade (BONK, HYPE, KMNO, JUP, WIF) where thin-book LP consumption edge might exist.

3. **Strategy-market fit** — For each extracted HL strategy, score compatibility with each Flash Trade market based on liquidity characteristics.

**Implementation:** New CLI tool `src/bin/scan-markets.rs` or integrate into the existing market analysis.

---

## Phase E: Implement Extracted Strategies

**Goal:** Implement strategies with parameters FROM the blueprints, not invented defaults.

The current 4 strategies (momentum-scalper, mean-reversion, trend-follower, lp-consumption) are placeholders. The real strategies will be reconstructed from fill patterns. When we find:

**Example:** "HL wallet cluster #7: 12 wallets trading BTC-PERP with 0.5 BTC fixed clips, 3x leverage, entering on volume spike >2SD above 20-period mean + 2 consecutive directional ticks, exiting on time stop (45 min) or trailing stop (1.5% retracement from peak), 72% win rate, $4.3K avg winner, $800 avg loser, 2.8 Sharpe"

...we implement that EXACTLY. The parameters come from the blueprint JSON. The strategy code implements the reconstructed logic.

**Acceptance:** Every strategy parameter in `config/perps.toml` is traceable to a specific blueprint from a specific wallet cluster with fill-level evidence.

---

## Phase F: Validate (The Truth Teller)

**Goal:** 24+ hours of paper trading on Flash Trade with positive net PnL after all fees.

The paper engine already works (with the P0 fixes applied). The validation pipeline:

1. Backtest on HL candles first → if Sharpe < 1.0, iterate on parameters
2. Paper trade on Flash Trade → 24h run with live fee estimates
3. All fee components tracked: entry fee (live API preview), exit fee (live API preview), borrow fee (accrued per hour, correctly calculated after M1 fix)

**Acceptance:** At least one strategy shows positive net PnL after fees over 24+ hours. If no strategy passes, the mission still succeeds by identifying the truth — but parameters are iterated first.

---

## Infrastructure for Autonomy (Post-Mission)

Once the poaching pipeline proves itself, build the self-evolution layer:

1. **SQLite price + fill cache** — Store all data locally. Eliminates the 5-hour dead-start window after restart.
2. **Strategy state persistence** — Serialize price buffers and strategy state on every tick. Resume instantly.
3. **Prometheus metrics endpoint** — `/metrics` with balance, positions, PnL, trade counts, API latency, error rates.
4. **Slack/Discord webhook alerts** — On circuit breaker, large loss, engine crash, or no ticks for 10 minutes.
5. **Continuous monitoring loop** — Weekly: re-scrape HL leaderboard → find new profitable wallets → extract new strategies → compare against existing → backtest → paper → promote to live if beating current strategies.
6. **Self-tuning** — Analyze trade history weekly. Optimize TP/SL/trailing per strategy. Auto-apply if improvement > 10% Sharpe.

---

## Execution Order

| # | Task | Why This First | Status |
|---|------|----------------|--------|
| **1** | Fix P0 bugs (borrow accrual, position limits, pool data) | Current numbers are objectively wrong | ✅ DONE |
| **2** | Update root .md files to reflect new architecture | Documentation must match new direction | ✅ DONE |
| **3** | Alpha Discovery Engine (M1-M4) | Build alpha infrastructure before analysis | ✅ DONE |
| **4** | Integration + Validation (M5) | Wire and validate the full pipeline | ✅ DONE |
| **5** | Replace fake HL seed addresses with QuickNode wallet discovery | Can't analyze wallets we don't have | Pending |
| **6** | Rewrite HL wallet analysis for fill-level Bulk methodology | This is the mission's intellectual core | Pending |
| **7** | Run deep analysis on 100+ real HL wallets | Find actual strategy clusters | Pending |
| **8** | Reconstruct entry/exit triggers from fill timing + candle data | Extract logic, not just parameters | Pending |
| **9** | Flash Trade market intelligence (pool ranking) | Identify exploitable execution targets | Pending |
| **10** | Implement extracted strategies with real parameters | Faithful replicas, not generics | Pending |
| **11** | Backtest → paper → live validation pipeline | Prove edge exists on target venue | Pending |

---

## Success Criterion

**At least one poached strategy (extracted from Hyperliquid wallet fill analysis) that shows positive net PnL after all fees over 24+ hours of paper trading on Flash Trade.**

This is the same criterion from the original MISSION-ALPHA-HUNTER.md. The difference is that we now know WHERE to find the wallets (Hyperliquid, via QuickNode HyperCore API) and WHAT data we need (individual fills via `userFills`). The Bulk.Trade analysis proved this methodology works. We're just applying it to the right data source.
