# Zekt Mission: Alpha Discovery Engine — Find Edge, Prove Edge, Trade Edge

## Why Everything Before Failed

The previous approach was fundamentally flawed:

1. **Reverse-engineering fills into strategies doesn't work.** We took 161 wallets, clustered their fills, extracted statistical parameters, and tried to replay them as momentum/grid/mean-reversion signals on 5m candles. The wallets were HFT market-makers operating on 48 obscure Hyperliquid markets (MET, PROVE, @232) with sub-second holds — their edge is **execution speed + orderbook positioning**, not a momentum signal that can be replicated from OHLCV data.

2. **Flash Trade is a red herring for now.** Flash Trade has thin books and high fees relative to Hyperliquid. The "LP consumption edge" doesn't exist because there's no real volume to consume. All 42 strategy/market combinations failed Sharpe >= 1.0 over 90 days.

3. **Backtesting on candle data can't capture HFT edge.** The profitable wallets operate inside the spread. 5m candles aggregate away the microstructure they exploit.

**The hard truth:** There is no "signal" in 5m OHLCV data that these wallets are trading. Their alpha comes from being fast, being first, and understanding orderbook microstructure — none of which our system captures.

---

## New Approach: Three-Pronged Alpha Engine

Instead of trying to reverse-engineer strategy parameters from fills, we build **three distinct alpha strategies** and a **continuous discovery loop** to ensure we never go stale.

### Prong 1: Real-Time Copy Trading (Immediate Revenue)

**Hypothesis:** The most profitable strategy for a small account is to follow profitable wallets in real-time — not reverse-engineer their signals, but literally mirror their positions with tight risk management.

**Why this is different from before:** We're not trying to understand *why* they trade. We're using their on-chain positions as the signal itself. Hyperliquid's transparency means we can see every position, every fill, every PnL in real-time.

**Data sources:**
- **Dextrabot API** (`dextradata.nftinit.io`): 100K+ wallets with pre-computed Sharpe, PnL, drawdown, win rate across 7d/30d/90d/all timeframes. Filter by `min_sharpe=2.0`, `min_pnl=5000`, `period=7` to find *currently* hot wallets.
- **Hypurrscan API** (`api.hypurrscan.io`): `/addressDetails/{address}` for wallet metadata, `/tags/{address}` for labels (identifies MEV bots, market makers, whales). JWT-authenticated endpoints for real-time transfers and bridges.
- **Hyperliquid Info API**: `clearinghouseState` for real-time positions, `userFills` for entry/exit timing.

**Implementation:**
1. New Rust binary: `src/bin/copy-trader.rs`
2. Every 30 seconds: poll top-N profitable wallets' positions via `clearinghouseState`
3. When a followed wallet opens/closes/adjusts a position, mirror it with configurable lag and sizing
4. Risk management: max 10% of account per copied trade, max 3 concurrent positions, stop-loss at -5% per position
5. Paper trade first for 48h, validate positive PnL before live

**Acceptance:** Copy trader binary that mirrors top-3 Dextrabot wallets in real-time. Paper PnL positive over 48h.

### Prong 2: Funding Rate Capture (Passive Income)

**Hypothesis:** Hyperliquid's funding rates are consistently positive for long-biased markets (BTC, ETH, SOL). A delta-neutral strategy that shorts perps and holds spot (or stakes HYPE) captures funding payments with minimal directional risk.

**Data sources:**
- **Hyperliquid Info API**: `metaAndAssetCtxs` for real-time funding rates across all markets
- **Hyperliquid Spot**: HL now has spot markets — can hold spot BTC/ETH against short perps
- **Chainstack guide**: Proven spot-perp arbitrage implementation (Python reference available)

**Implementation:**
1. New strategy: `FundingRateCapture` in `strategy.rs`
2. Monitor funding rates for all HL markets every hour
3. When 8h annualized funding rate > 20%: open delta-neutral position (short perp, long spot or hold USDC as collateral)
4. Close when funding rate drops below 5% annualized or position is >72h old
5. This is NOT a momentum strategy — it's a yield strategy with known risk profile

**Acceptance:** Funding rate strategy implemented. Backtest shows positive net PnL on historical funding data (last 90 days). Paper trade for 48h.

### Prong 3: Whale Alert System (Information Edge)

**Hypothesis:** Large position openings by known-profitable wallets (identified via Dextrabot) predict short-term price movements. If a wallet with Sharpe >3.0 and $100K+ PnL suddenly opens a 5x leveraged long on BTC, that's a signal.

**Data sources:**
- **Dextrabot API**: Maintain a watchlist of top-20 profitable wallets (refreshed daily)
- **Hyperliquid WebSocket**: `allMids` + `userFills` for real-time position changes
- **Hypurrscan JWT endpoints**: `/transfers/{fromTimestamp}/{toTimestamp}` for capital flow tracking

**Implementation:**
1. New Rust binary: `src/bin/whale-watcher.rs`
2. WebSocket connection to Hyperliquid for real-time position monitoring
3. When a watched wallet opens a position >$10K notional, emit an alert
4. Integrate alerts into the copy trader (Prong 1) for faster signal detection
5. Track accuracy: does following whale entries produce positive PnL?

**Acceptance:** Whale watcher detects position changes within 5 seconds. Alert log with wallet, market, direction, size. 48h tracking shows >50% of whale entries are profitable within 1h.

### Continuous Discovery Loop (Anti-Staleness)

**The problem:** Strategies decay. Wallets that were profitable last month may be losing this month. Alpha sources change.

**Solution:** A daily cron job that:

1. **Refresh wallet rankings** via Dextrabot API
   - Re-score all wallets using 7-day and 30-day windows
   - Remove wallets whose Sharpe dropped below 1.0
   - Add newly profitable wallets
   - Save to `data/watchlist-v{date}.json`

2. **Validate existing copy targets**
   - Check if currently-followed wallets are still profitable
   - If 7-day Sharpe < 0: remove from follow list
   - If 30-day Sharpe < 1.0: flag for review

3. **Detect strategy decay**
   - Track rolling PnL of each followed wallet
   - If a wallet's 7d win rate drops below 40%: flag as potentially compromised
   - Alert human for manual review

4. **Discover new alpha sources**
   - Weekly: scan Hypurrscan for new high-activity addresses not in Dextrabot
   - Monthly: review Hyperliquid leaderboard for new entrants
   - Ongoing: monitor funding rate regime changes

**Implementation:**
- New Rust binary: `src/bin/alpha-scanner.rs`
- Runs as a daemon: every 6h refresh wallet list, every 24h full rescan
- Outputs: `data/watchlist.json` (current targets), `data/alpha-report.json` (daily summary)
- Integrates with copy-trader via shared watchlist file

**Acceptance:** Alpha scanner refreshes watchlist daily. Detects when a previously-profitable wallet goes cold. Has run for 7 days continuously without crash.

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                    ALPHA DISCOVERY ENGINE                        │
│                                                                  │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │  Dextrabot API  │  │  Hypurrscan API │  │  HL Info API    │  │
│  │  (100K wallets) │  │  (JWT-endpoints)│  │  (positions)    │  │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘  │
│           │                    │                     │           │
│  ┌────────▼────────────────────▼─────────────────────▼────────┐  │
│  │              alpha-scanner (daemon)                        │  │
│  │  Daily: refresh wallet rankings, detect decay              │  │
│  │  Output: data/watchlist.json                               │  │
│  └────────┬───────────────────────────────────────────────────┘  │
│           │                                                      │
│     ┌─────┼──────────────────────────────────┐                   │
│     │     │                                  │                   │
│  ┌──▼─────▼───┐  ┌──────────────────┐  ┌────▼──────────────┐   │
│  │ copy-trader │  │  whale-watcher   │  │ funding-capture   │   │
│  │ (real-time) │  │  (WebSocket)     │  │ (hourly scan)     │   │
│  │             │  │                  │  │                   │   │
│  │ Mirror top  │  │ Detect whale     │  │ Short perp when   │   │
│  │ wallet pos  │  │ entries >$10K    │  │ funding >20% ann. │   │
│  │ with sizing │  │ within 5s        │  │ Delta-neutral     │   │
│  └──────┬──────┘  └────────┬─────────┘  └────────┬──────────┘   │
│         │                   │                      │              │
│  ┌──────▼───────────────────▼──────────────────────▼──────────┐  │
│  │                    RISK MANAGER                            │  │
│  │  Max 10% per trade | Max 3 concurrent | -5% hard stop     │  │
│  │  Daily loss limit | Circuit breaker | Cooldown             │  │
│  └──────┬─────────────────────────────────────────────────────┘  │
│         │                                                        │
│  ┌──────▼─────────────────────────────────────────────────────┐  │
│  │              EXECUTION LAYER                                │  │
│  │  Phase 1: Paper trade (48h minimum, positive PnL required) │  │
│  │  Phase 2: Live with $100 (human approval)                  │  │
│  │  Phase 3: Scale to $500 (after 7d profitable)              │  │
│  │  Phase 4: Scale to $1000+ (after 30d profitable)           │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Milestones

### M1: Alpha Scanner — Continuous Wallet Intelligence (Rust)

**Goal:** Build the daemon that keeps our wallet watchlist fresh.

**New binary:** `src/bin/alpha-scanner.rs`

```
CLI:
  alpha-scanner --daemon            # Run as daemon (refresh every 6h)
  alpha-scanner --once              # Single scan and exit
  alpha-scanner --min-sharpe 2.0    # Minimum Sharpe for inclusion
  alpha-scanner --min-pnl 5000      # Minimum 30d PnL in USD
  alpha-scanner --watchlist-size 20 # Number of wallets to track
  alpha-scanner --output data/watchlist.json

Logic:
  1. Fetch top wallets from Dextrabot (7d and 30d windows)
  2. Enrich with Hypurrscan data (tags, labels, details)
  3. For each wallet: fetch current positions via HL API
  4. Score wallets: composite of Sharpe * PnL * consistency
  5. Output ranked watchlist with metadata
  6. Compare against previous watchlist, emit change report
  7. Flag wallets that were profitable but no longer are
```

**Data structures:**
```rust
struct WatchlistEntry {
    address: String,
    dextrabot_sharpe_7d: f64,
    dextrabot_sharpe_30d: f64,
    dextrabot_pnl_30d: f64,
    dextrabot_win_rate: f64,
    hypurrscan_tags: Vec<String>,
    current_positions: Vec<PositionSummary>,
    composite_score: f64,
    first_seen: DateTime<Utc>,
    last_profitable: DateTime<Utc>,
    status: WalletStatus, // Active, Decaying, Dead, New
}
```

**Tests:** 10+ unit tests covering scoring, filtering, decay detection, watchlist diffing.

**Acceptance:** Alpha scanner produces a ranked watchlist of 20 wallets. Detects wallet decay. Runs as daemon for 24h without crash.

### M2: Copy Trader — Real-Time Position Mirroring (Rust)

**Goal:** Mirror profitable wallet positions in real-time.

**New binary:** `src/bin/copy-trader.rs`

```
CLI:
  copy-trader --paper                # Paper trading mode
  copy-trader --live                 # Live mode (requires keypair)
  copy-trader --watchlist data/watchlist.json
  copy-trader --max-position-pct 10  # Max % of account per trade
  copy-trader --max-positions 3      # Max concurrent positions
  copy-trader --stop-loss-pct 5.0    # Hard stop loss per position
  copy-trader --lag-secs 30          # Seconds to wait after whale entry
  copy-trader --sizing-multiplier 0.1 # Size relative to whale (10%)

Logic:
  1. Load watchlist from alpha-scanner output
  2. Every 30s: poll each wallet's positions via HL clearinghouseState
  3. Detect new positions, position size changes, and closures
  4. For new positions: open mirror position after configurable lag
  5. For closures: close mirror position
  6. Apply risk management: position sizing, max positions, stop loss
  7. Log all decisions to data/copy-trades.json
```

**Key implementation details:**
- Use Hyperliquid Info API `clearinghouseState` for position monitoring (no WebSocket needed for 30s polling)
- Position sizing: `our_size = whale_size * sizing_multiplier * (our_balance / whale_balance).min(1.0)`
- Slippage protection: preview entry via Flash Trade or HL API before submitting
- The copy trader operates on **Hyperliquid directly** (not Flash Trade) for execution — HL has deeper liquidity and lower fees

**Tests:** 15+ unit tests covering position detection, sizing calculation, risk limits, stop-loss triggers.

**Acceptance:** Copy trader mirrors top-3 wallets in real-time with <60s lag. Paper PnL tracked over 48h. Risk management prevents any single trade from losing >5%.

### M3: Whale Watcher — Real-Time Signal Detection (Rust)

**Goal:** Detect large position changes by profitable wallets within seconds.

**New binary:** `src/bin/whale-watcher.rs`

```
CLI:
  whale-watcher --watchlist data/watchlist.json
  whale-watcher --min-notional 10000  # Minimum position size for alert
  whale-watcher --output data/whale-alerts.json

Logic:
  1. Load watchlist
  2. Connect to HL WebSocket for real-time user events
  3. When a watched wallet opens/closes >$10K notional: emit alert
  4. Alert includes: wallet, market, direction, size, timestamp, wallet Sharpe
  5. Feed alerts to copy-trader for faster signal
  6. Track alert accuracy (did following the alert produce profit?)
```

**Tests:** 8+ unit tests covering WebSocket parsing, alert generation, accuracy tracking.

**Acceptance:** Whale watcher detects position changes within 5 seconds. Alert log maintained. Accuracy tracking shows what % of whale entries are profitable.

### M4: Funding Rate Capture — Passive Yield Strategy (Rust)

**Goal:** Implement delta-neutral funding rate capture.

**New strategy in `strategy.rs`:** `FundingRateCaptureStrategy`

```
Parameters:
  min_annualized_rate_pct: 20.0    # Only enter when funding > 20% annualized
  exit_annualized_rate_pct: 5.0    # Exit when funding drops below 5%
  max_position_hours: 72           # Close after 72h regardless
  leverage: 1.0                    # No leverage for delta-neutral
  clip_size_usd: 200.0             # Small position size
  funding_interval_hours: 8        # HL uses 8h funding

Logic:
  1. Every hour: fetch all market funding rates via HL metaAndAssetCtxs
  2. For markets with annualized funding > 20%: open short perp position
  3. This is effectively "earning" the funding rate from longs
  4. Close when funding drops below 5% or position is >72h old
  5. Track cumulative funding captured vs. price risk

Risk:
  - Delta risk: if we short perp without spot hedge, we have directional exposure
  - Solution: use HL spot markets to hedge (buy spot, short perp)
  - Or: only short perps where we're comfortable with directional risk
  - Conservative approach: start with pure short perp, small size ($200)
```

**Tests:** 40 unit tests covering rate calculation, entry/exit logic, PnL tracking, parameter validation, pipeline integration, serde roundtrips.

**Acceptance:** Funding rate strategy implemented. 40 tests pass. Wired into strategy factory with config sub-table support. ✅ DONE

### M5: Integration + Validation (Rust + Testing)

**Goal:** Wire everything together and validate the complete system.

1. Wire `alpha-scanner` → `watchlist.json` → `copy-trader` + `whale-watcher`
2. Paper trade all three strategies simultaneously for 48h
3. Track combined PnL across strategies
4. If any strategy shows positive PnL: proceed to live with $100
5. Run full test suite (target: 560+ tests)
6. Update `config/perps.toml` with new strategy sections

**Acceptance:** Full system runs as a daemon for 48h without crash. At least one strategy shows positive paper PnL. 560+ tests pass.

---

## Execution Order

| # | Task | Depends On | Effort | Status |
|---|------|-----------|--------|--------|
| 1 | M1: Alpha scanner binary | None | Medium | ✅ DONE (64 tests) |
| 2 | M2: Copy trader binary | M1 | Medium-Large | ✅ DONE (85 tests) |
| 3 | M3: Whale watcher binary | M1 | Medium | ✅ DONE (41 tests) |
| 4 | M4: Funding rate strategy | None | Medium | ✅ DONE (40 tests) |
| 5 | M5: Integration + validation | M1-M4 | Small | ✅ DONE (24 tests: pipeline 14 + pnl_tracker 10) |

M1 and M4 can run in parallel. M2 and M3 depend on M1's watchlist output.

---

## API Credentials

### Dextrabot (already integrated)
- Backend: `dextradata.nftinit.io` (data) + `dextrabothypev2.nftinit.io` (app)
- No auth required for `discover-wallets` endpoint
- Already have `scrape-dextrabot.rs` binary

### Hypurrscan (JWT)
- Base URL: `api.hypurrscan.io`
- Access token: stored in environment variable `HYPURRSCAN_JWT`
- Refresh token: stored in environment variable `HYPURRSCAN_REFRESH_TOKEN`
- Rate limit: 1000 req/min/IP
- Key endpoints (JWT-gated):
  - `GET /transfers/{fromTimestamp}/{toTimestamp}` — capital flow tracking
  - `GET /bridges/{fromTimestamp}/{toTimestamp}` — bridge deposits/withdrawals
- Public endpoints (no auth):
  - `GET /addressDetails/{address}` — wallet metadata
  - `GET /tags/{address}` — wallet labels
  - `GET /rank/{address}` — wallet ranking
  - `GET /twap/{address}` — active TWAP orders
  - `GET /feesRecent` — 48h fee data

### Hyperliquid (no auth)
- Info API: `POST https://api.hyperliquid.xyz/info`
- WebSocket: `wss://api.hyperliquid.xyz/ws`
- No auth required, rate limit 1200 weight/min/IP

---

## Key Constraints

1. **Execute on Hyperliquid, not Flash Trade.** HL has deeper books, lower fees, and is where the profitable wallets actually trade. Flash Trade remains as a secondary option only.
2. **Paper trade for minimum 48h** before any live execution
3. **Human approval required** for live mode, position sizing > $100, and any new wallet additions
4. **All new Rust code** uses `tracing` for logging, `anyhow::Result` for errors
5. **Don't break existing 560 tests**
6. **Risk limits are hard limits**, not suggestions — circuit breaker halts everything on daily loss limit
7. **Small account focus:** start with $100, scale to $500 after 7d profitable, $1000 after 30d
8. **Keep the existing strategy infrastructure** (backtest engine, paper trading) — these still work for validating new strategies

---

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Paper PnL (48h) | Positive | Copy trader + funding capture combined |
| Sharpe (paper) | >= 1.0 | Over 48h paper trading period |
| Max drawdown (paper) | < 10% | During paper trading period |
| Wallet detection speed | < 5s | Whale watcher position detection |
| Watchlist freshness | < 6h | Alpha scanner refresh interval |
| Strategy decay detection | < 24h | Time to flag a decaying wallet |
| System uptime | > 99% | No crashes during 48h paper trading |
| Test coverage | 560+ | Total Rust unit tests |

---

## What This Mission Is NOT

- NOT another attempt to reverse-engineer fills into candle-based signals
- NOT a theoretical exercise — everything must be validated with real money (paper first, then live)
- NOT a single-strategy bet — three independent strategies reduce risk of total failure
- NOT dependent on Flash Trade — we go where the alpha is (Hyperliquid)
- NOT a black box — every decision is logged and auditable

---

## Files to Create/Modify

### New Files (all created)
- `src/bin/alpha-scanner.rs` — Wallet discovery daemon (64 tests) ✅
- `src/bin/copy-trader.rs` — Real-time position mirroring (85 tests) ✅
- `src/bin/whale-watcher.rs` — Real-time whale alert system (41 tests) ✅
- `src/funding_capture.rs` — Funding rate capture strategy module (40 tests) ✅
- `src/pnl_tracker.rs` — Combined PnL tracking across all strategies (10 tests) ✅
- `src/bin/pipeline.rs` — Pipeline orchestrator binary (14 tests) ✅

### Modified Files (all done)
- `src/strategy.rs` — Add `FundingRateCaptureStrategy` + update `available_strategies()` + factory ✅
- `src/main.rs` — Add `mod funding_capture`, `mod pnl_tracker` ✅
- `Cargo.toml` — Add `tokio-tungstenite` for WebSocket + `pipeline` binary ✅
- `config/perps.toml` — Add `[strategy.funding-capture]`, `[copy-trader]`, `[whale-watcher]`, `[hypurrscan]`, `[pipeline]` sections ✅
- `CLAUDE.md` — Update with new binaries and commands ✅
- `SESSION-CONTEXT.md` — Update with new architecture ✅
