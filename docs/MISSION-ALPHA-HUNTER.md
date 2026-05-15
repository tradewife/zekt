# Mission: Alpha Hunter — Strategy Poaching Engine

## Mission Statement

Turn Zekt from a single-strategy momentum scalper into a multi-strategy trading system by reverse-engineering winning wallets scraped from perp DEX leaderboards, extracting their strategies as quantified blueprints, implementing them in Rust for Flash Trade execution, and validating via paper trading with full fee accounting. The success criterion: at least one poached strategy that shows positive net expected PnL over 24+ hours of paper trading.

**Duration:** 3-4 days of autonomous agent-driven work.

**Why This Works:** We proved this approach with Bulk.Trade's devnet competition (see `docs/bulktrade-analysis.md`). We reverse-engineered 5 wallets running the same momentum scalper on ZEC, identified the strategy parameters, and built Zekt v1 from that blueprint. Now we generalize the pipeline: scrape leaderboards -> classify wallets -> extract strategies -> implement -> validate.

---

## Phase 1: Leaderboard Scraping & Wallet Discovery

**Goal:** Build a database of 50-100+ profitable wallet addresses from multiple perp DEX leaderboards.

### Data Sources (Prioritized)

1. **Flash Trade (Primary Target)**
   - fstats.io — Flash Trade analytics dashboard. Likely has leaderboard/trader ranking pages.
   - flash.trade UI — Check for any public leaderboard endpoints.
   - Flash Trade API (`GET /positions/owner/{owner}`) — Once we have addresses, fetch their positions.

2. **Jupiter Perps (Solana)**
   - jup.ag/perps-leaderboard — Public leaderboard page at https://jup.ag/perps-leaderboard.
   - Jupiter Perps API — Check https://developers.jup.ag/docs/perps for trade history endpoints.

3. **Hyperliquid (Cross-chain reference)**
   - app.hyperliquid.xyz/leaderboard — Public leaderboard.
   - Hyperliquid Info API — `POST https://api.hyperliquid.xyz/info` with `{"type": "leaderboard"}` or similar.
   - HyperTracker API (docs.coinmarketman.com/endpoints/leaderboards) — Curated leaderboard data.

4. **GMX (Arbitrum/Avalanche reference)**
   - gmx.house/arbitrum/leaderboard — Third-party GMX leaderboard scraper.
   - GMX subgraph — On-chain trade data via The Graph.

5. **General Solana Wallet Intelligence**
   - Birdeye (birdeye.so/solana/trader-board) — Solana wallet rankings by volume/PnL.
   - Solana RPC — For any Solana-based DEX, get transaction history via `getSignaturesForAddress`.

### Implementation

Create `src/bin/scrape-leaderboards.rs` as a CLI binary:

```
cargo run --bin scrape-leaderboards -- --source flash --output data/wallets.json
cargo run --bin scrape-leaderboards -- --source jupiter --output data/wallets.json
cargo run --bin scrape-leaderboards -- --source hyperliquid --output data/wallets.json
cargo run --bin scrape-leaderboards -- --source all --output data/wallets.json
```

Output format (`data/wallets.json`):
```json
[
  {
    "address": "B8YxkfYZemxat86P7xFEiwxD4G3JEPyugQe5geMQMvz9",
    "source": "flash-trade",
    "rank": 1,
    "pnl_usd": 159672.0,
    "win_rate_pct": 83.0,
    "total_trades": 12,
    "markets_traded": ["ZEC"],
    "scraped_at": "2026-05-16T00:00:00Z"
  }
]
```

### Constraints
- Only use public APIs and publicly accessible pages. No auth, no login-gated data.
- Respect rate limits (configurable delay between requests).
- Use `reqwest` (already a dependency) + optional `scraper` crate for HTML parsing if needed.
- All wallet data stored locally in `data/` directory (gitignored).

### Acceptance Criteria
- [ ] CLI tool compiles and runs without errors
- [ ] Successfully scrapes at least 20 wallets from at least 2 different sources
- [ ] Output is valid JSON with the schema above
- [ ] Rate limiting is configurable via CLI flag

---

## Phase 2: Wallet Analysis & Strategy Classification

**Goal:** For each discovered wallet, fetch their full trade history and classify their strategy using the same analytical framework from `docs/bulktrade-analysis.md`.

### Analysis Pipeline

For each wallet in `data/wallets.json`:

1. **Fetch trade history** from the relevant DEX API
   - Flash Trade: `GET /positions/owner/{owner}` for current positions; for historical trades may need Solana RPC `getSignaturesForAddress` + transaction parsing
   - Hyperliquid: `POST https://api.hyperliquid.xyz/info` with `{"type": "userFills", "user": "<address>"}`
   - Jupiter: Perps API trade history endpoints

2. **Compute metrics per wallet:**
   - **Clip size consistency** — What % of fills are the same size? (Bot signature: >70% same clip = automated)
   - **Hold time distribution** — Median, P25, P75, max. (Scalper: <30min median. Swing: >2hr median)
   - **Direction bias** — Net long/short ratio. (Momentum: directional. Market-maker: neutral)
   - **Win rate** — % of profitable trades
   - **PnL distribution** — Mean, median, max winner, max loser, skewness
   - **Fee-adjusted PnL** — Net after all fees (critical — Bulk analysis showed 3 wallets were net negative after fees)
   - **Counterparty concentration** — % of fills against a single LP (LP consumption edge)
   - **Market concentration** — % of trades in a single market (specialist vs generalist)
   - **Time patterns** — Fill intervals (sub-second = HFT bot), time-of-day patterns
   - **Scale-in behavior** — Do they accumulate position over multiple fills?
   - **Leverage usage** — Average and max leverage

3. **Classify into strategy types:**

| Strategy | Signatures |
|----------|-----------|
| Momentum Scalper | Fixed clips, short holds, directional streaks, high win rate, single market |
| Mean Reversion | Fade moves, oscillating long/short, tight range, high frequency |
| Trend Follower | Long holds, trailing behavior, wider stops, ride winners |
| LP Consumption Detector | Single counterparty dominance, scale-in on LP depletion |
| Swing Trader | Fewer trades, larger sizes, multi-hour holds, human-like timing |
| HFT Market Maker | Sub-second fills, neutral bias, many markets, low per-trade PnL |
| Grid/Martingale | Systematic sizing patterns, size increases after losses |
| Unknown | Doesn't fit any pattern — skip |

### Implementation

Create `src/bin/analyze-wallet.rs`:
```
cargo run --bin analyze-wallet -- --address B8Yx...Mvz9 --source flash
cargo run --bin analyze-wallet -- --wallets data/wallets.json --output data/reports/
```

Output per wallet: `data/reports/{address}.json`
Output blueprints: `data/strategy-blueprints/{strategy-name}.json`

### Strategy Blueprint Format

```json
{
  "strategy_name": "lp-consumption-scalper",
  "source_wallets": ["B8Yx...", "5Np87..."],
  "confidence": 0.92,
  "parameters": {
    "markets": ["ZEC"],
    "clip_size_usd": 500.0,
    "leverage": 40.0,
    "direction_bias": "long",
    "entry": {
      "signal": "lp_consumption_velocity",
      "threshold_pct": 0.15,
      "confirmation_count": 3
    },
    "exit": {
      "take_profit_pct": 2.5,
      "stop_loss_pct": 1.0,
      "trailing_stop_pct": 0.8,
      "trailing_activation_pct": 1.5,
      "max_hold_secs": 1800
    },
    "risk": {
      "max_position_notional_usd": 1000.0,
      "daily_loss_limit_usd": 200.0,
      "cooldown_after_loss_secs": 300
    }
  },
  "backtest_metrics": {
    "total_trades_analyzed": 85,
    "win_rate": 0.78,
    "avg_winner_usd": 3200,
    "avg_loser_usd": -800,
    "net_pnl_after_fees_usd": 159672,
    "sharpe_estimate": 2.1
  }
}
```

### Acceptance Criteria
- [ ] CLI tool fetches and analyzes trade history for any given wallet address
- [ ] At least 3 distinct strategy types identified across analyzed wallets
- [ ] Fee-adjusted PnL calculated for every wallet (no false positives from fee-ignorant analysis)
- [ ] Strategy blueprints output with quantified parameters suitable for direct implementation
- [ ] Wallets with net-negative fee-adjusted PnL are flagged and excluded from strategy extraction

---

## Phase 3: Strategy Implementation in Rust

**Goal:** Implement the top 2-3 identified strategies as pluggable modules in Zekt's existing architecture.

### Architecture Changes

Refactor `signal.rs` to use a `Strategy` trait:

```rust
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    fn detect_entry(&self, snapshot: &PriceSnapshot) -> Option<EntrySignal>;
    fn detect_exit(&self, position: &Position, snapshot: &PriceSnapshot) -> Option<ExitSignal>;
    fn parameters(&self) -> &StrategyParams;
}
```

The existing momentum scalper (`signal.rs` logic) becomes `MomentumScalperStrategy`. New strategies become additional implementors:

### Strategies to Implement (Based on Phase 2 Results)

**Priority 1: LP Consumption Detector** (the real edge from Bulk analysis)
- Detect when a single LP's depth is being consumed in one direction
- Entry: consumption rate exceeds threshold (position changes from dominant LP)
- Exit: momentum stalls (LP refreshes or consumption rate drops)
- This is what made the Bulk.Trade ZEC bot profitable — the LP was the signal

**Priority 2: Mean Reversion Scalper**
- Fade momentum spikes (enter opposite direction after sharp moves)
- Entry: price velocity exceeds threshold THEN reverses
- Exit: price returns to mean (VWAP or moving average)
- Works best in ranging markets

**Priority 3: Trend Follower** (if data supports it)
- Enter on confirmed momentum breakouts
- Wider stops, trailing exits, longer holds
- Lower win rate but larger winners

### Config Changes

Add strategy selection to `config/perps.toml`:
```toml
[strategy]
active = "momentum-scalper"  # or "lp-consumption", "mean-reversion", "trend-follower"

[strategy.momentum-scalper]
clip_size_usd = 100.0
momentum_threshold_pct = 0.15
# ... existing params ...

[strategy.lp-consumption]
clip_size_usd = 500.0
consumption_rate_threshold = 0.3
lp_concentration_min_pct = 80.0
# ...

[strategy.mean-reversion]
clip_size_usd = 200.0
reversion_threshold_pct = 1.0
mean_lookback_count = 120
# ...
```

Add CLI flag: `--strategy <name>`

### Acceptance Criteria
- [ ] `Strategy` trait defined and implemented
- [ ] Existing momentum scalper refactored to implement the trait
- [ ] At least 2 new strategies implemented
- [ ] Strategy selectable via CLI flag and config
- [ ] All strategies work with both live engine and paper engine
- [ ] Code compiles without warnings (`cargo build --release`)

---

## Phase 4: Paper Trading Validation

**Goal:** Run each implemented strategy through 24+ hours of paper trading against live Flash Trade prices with full fee accounting.

### Fee Accounting (Critical)

Every simulated trade must account for:
1. **Entry fee** — Fetched from `POST /transaction-builder/open-position` preview
2. **Exit fee** — Fetched from `POST /transaction-builder/close-position` preview
3. **Borrow fee** — Accrued per hour on notional position size (check pool utilization data)
4. **Price impact** — Estimated from pool depth / utilization

The paper engine already exists in `src/paper.rs`. Enhance it to:
- Query live fee estimates from Flash Trade API for each simulated trade
- Track cumulative fees separately from PnL
- Report net PnL (after fees) as the primary metric

### Multi-Market Testing

Run each strategy across multiple Flash Trade markets simultaneously:
- SOL-USD, BTC-USD, ETH-USD (liquid majors)
- ZEC-USD, BNB-USD (mid-liquidity)
- JUP-USD, BONK-USD, WIF-USD (illiquid/meme — where LP consumption edge may exist)

This identifies which strategy works best on which market structure.

### Head-to-Head Comparison

For each strategy x market combination, track over 24+ hours:

| Metric | Description |
|--------|-------------|
| Gross PnL | Before fees |
| Net PnL | After all fees (the only metric that matters) |
| Fee ratio | Fees as % of gross PnL |
| Win rate | % of profitable trades |
| Trade count | Number of round-trip trades |
| Avg hold time | Mean time in position |
| Max drawdown | Largest peak-to-trough decline |
| Sharpe-like | Mean return / std dev of returns |
| Best market | Which market had highest net PnL |

### Output

Results logged to `data/paper-results/{strategy}-{market}-{timestamp}.json`

Summary table generated at mission end showing all strategy x market combinations ranked by net PnL.

### Acceptance Criteria
- [ ] Paper engine queries live fee estimates for each simulated trade
- [ ] All fee components tracked and reported
- [ ] At least 2 strategies paper traded for 24+ hours each
- [ ] At least 3 markets tested per strategy
- [ ] Head-to-head comparison table generated
- [ ] At least one strategy shows positive net expected PnL (after fees)

---

## Phase 5: Market Scanner (Bonus / Integration)

**Goal:** Automatically rank Flash Trade markets by the characteristics that make them exploitable.

This addresses the TODO item from SESSION-CONTEXT.md: "Market scanner — Rank Flash Trade markets by liquidity concentration."

### Implementation

Create `src/bin/scan-markets.rs` that queries Flash Trade API:

1. `GET /raw/markets` — All available markets
2. `GET /pool-data` — Pool utilization, AUM
3. For each market, compute:
   - **LP concentration** — How concentrated is the counterparty? (Higher = better for LP consumption)
   - **Liquidity thinness** — How easy to move the price? (Lower liquidity = more exploitable)
   - **Volatility** — Recent price range (moderate volatility = best for scalping)
   - **Fee efficiency** — Can we cover fees with typical moves?
   - **Market score** — Weighted composite: 0.4 * LP_concentration + 0.3 * thinness + 0.3 * volatility

Output: Ranked market list with scores, updated periodically.

### Acceptance Criteria
- [ ] CLI tool ranks all Flash Trade markets
- [ ] Output includes market score and component breakdown
- [ ] Results inform which markets to run strategies on

---

## Technical Constraints

### Build System
- Single Rust crate, `[[bin]]` entries in `Cargo.toml` for each CLI tool
- All new dependencies must be added to `Cargo.toml` (prefer already-installed crates)
- New crates may be needed: `scraper` (HTML parsing), `csv` (if needed), `tokio` features

### Coding Standards
- Follow existing conventions in CLAUDE.md exactly
- `tracing` for logging, `anyhow::Result` for errors, `parse_f64_safe` for parsing
- All new CLI tools use `clap` for argument parsing
- Atomic file writes for any data output (write .tmp, rename)
- No `println!` in library/binary code

### Data Directory
- `data/` is gitignored (contains wallet data, reports, paper results)
- All intermediate data persists between phases
- Strategy blueprints are the handoff artifact between Phase 2 and Phase 3

### Safety
- Paper trading only for validation — no live execution in this mission
- All wallet data is public on-chain data — no PII or private keys involved
- Rate limit all external API calls (configurable, default 1 req/sec)

---

## Success Criteria (Mission Complete When)

1. **Phase 1:** 50+ wallets scraped from 2+ DEX sources
2. **Phase 2:** 3+ strategy types classified with quantified blueprints
3. **Phase 3:** 2+ new strategies implemented as Rust modules
4. **Phase 4:** At least 1 strategy shows **positive net PnL after fees** over 24h paper trading
5. **Phase 5 (Bonus):** Market scanner producing actionable rankings

The mission is oriented around one hard truth from the Bulk.Trade analysis: **3 out of 10 leaderboard wallets were net negative after fees**. Fee-awareness is not optional. Every strategy must prove itself net profitable after all costs.

---

## File Map (Expected New Files)

```
src/bin/
  scrape-leaderboards.rs    Phase 1 — Leaderboard scraper
  analyze-wallet.rs          Phase 2 — Wallet trade analyzer
  scan-markets.rs            Phase 5 — Market scanner

src/
  strategy.rs                Phase 3 — Strategy trait + implementations
  signal.rs                  Refactored to implement Strategy trait

data/
  wallets.json               Phase 1 output — ranked wallet database
  reports/                   Phase 2 output — per-wallet analysis
    {address}.json
  strategy-blueprints/       Phase 2 output — extracted strategy params
    {strategy-name}.json
  paper-results/             Phase 4 output — paper trading results
    {strategy}-{market}-{ts}.json
```

---

## Execution Notes for the Agent

1. **Start with Phase 1** — You can't analyze wallets you haven't discovered. Focus on Flash Trade and Jupiter first (same chain, same tooling). Hyperliquid/GMX are bonus.

2. **Phase 2 is the intellectual core** — The quality of strategy extraction determines everything. Spend time here. The Bulk.Trade analysis (`docs/bulktrade-analysis.md`) is your template. Follow that methodology exactly: clip size consistency, counterparty concentration, hold time distribution, fee-adjusted PnL.

3. **Phase 3 must stay faithful to Phase 2's blueprints** — Don't implement a generic strategy. Implement the specific parameters extracted from real profitable wallets. The parameters ARE the strategy.

4. **Phase 4 is the truth teller** — No strategy ships without positive net PnL in paper trading. If all strategies are net negative, the mission still succeeds by identifying that truth — but try harder on fee optimization before giving up.

5. **Fee accounting is the #1 priority in Phase 4** — The Bulk analysis showed that HFT market-making looks profitable before fees but loses $43K after. This is the trap. Always net after fees.

6. **If you get stuck on an API** — Fall back to Solana RPC. Every Flash Trade transaction is on-chain. You can always parse raw transaction logs to reconstruct trade history. It's slower but always available.

7. **Commit frequently** — Each phase completion is a commit. Each working CLI tool is a commit. Don't accumulate uncommitted work.
