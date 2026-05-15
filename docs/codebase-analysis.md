# Zekt Codebase Architecture Analysis

**Generated:** 2026-05-16
**Version:** v0.3 (uncommitted working tree)

---

## Overview

Zekt is a **liquidity-aware momentum scalper** for Flash Trade (Solana perps DEX). It's a single Rust crate with a `[[bin]]` target at `src/main.rs`. The bot detects momentum in illiquid markets, opens leveraged perp positions, and manages exits via TP/SL/trailing stops. It supports three modes: live trading, paper trading (simulated PnL against live prices), and dry-run (single API preview).

---

## Module Dependency Graph

```
main.rs
├── config.rs        (Config, AgentConfig, FlashConfig, StrategyConfig, RiskConfig)
├── engine.rs        (ScalperEngine — live trading loop)
│   ├── flash_api.rs (FlashClient — REST API client)
│   ├── executor.rs  (Executor — Solana keypair + tx sign/submit)
│   ├── signal.rs    (MomentumDetector — entry/exit signal detection)
│   └── risk.rs      (RiskManager, Position, TradeLog — risk + position tracking)
├── paper.rs         (PaperEngine — paper trading loop)
│   ├── flash_api.rs
│   ├── signal.rs
│   └── risk.rs
└── (dry-run uses flash_api.rs directly)
```

`paper.rs` mirrors `engine.rs` but skips all `Executor` interactions (no signing, no RPC).

---

## File-by-File Analysis

### 1. `src/config.rs` — Configuration Loading

**Purpose:** Loads TOML config and provides typed access to all settings.

#### Structs

| Struct | Fields | Description |
|--------|--------|-------------|
| `Config` | `agent`, `flash`, `strategy`, `risk` | Top-level config, maps 1:1 to TOML sections |
| `AgentConfig` | `poll_interval_secs: u64`, `log_level: String` | Runtime behavior settings |
| `FlashConfig` | `api_url`, `rpc_url`, `keypair_path`, `market`, `input_token`, `pool`, `leverage: f64`, `slippage_pct: String` | Flash Trade + Solana connection settings |
| `StrategyConfig` | `direction_bias`, `momentum_threshold_pct: f64`, `lookback_count: usize`, `scale_in_clips: u32`, `clip_size_usd: f64`, `max_hold_secs: u64`, `take_profit_pct: f64`, `stop_loss_pct: f64`, `trailing_stop_pct: f64`, `trailing_activation_pct: f64`, `cooldown_after_loss_secs: u64`, `use_native_tp_sl: bool` | All trading parameters |
| `RiskConfig` | `max_position_notional_usd: f64`, `max_daily_loss_usd: f64`, `max_drawdown_pct: f64` | Risk management limits |

#### Public Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `Config::load` | `(path: &Path) -> Result<Self>` | Reads TOML file, deserializes into `Config` |
| `Config::poll_interval` | `(&self) -> Duration` | Converts `poll_interval_secs` to `Duration` |

#### Patterns
- All config structs derive `Debug, Clone, Deserialize, Serialize`
- Config is loaded once at startup, never mutated at runtime
- `slippage_pct` is stored as `String` (not f64) — passed directly to API
- `direction_bias` is a free-form string ("neutral", "long", "short") — parsed at call sites via `.to_lowercase()`

---

### 2. `src/signal.rs` — Momentum Detection

**Purpose:** Price analysis, momentum detection, and exit signal generation. The "brain" of the strategy.

#### Key Types

| Type | Kind | Description |
|------|------|-------------|
| `Signal` | enum | `MomentumLong { strength, velocity_pct }`, `MomentumShort { strength, velocity_pct }`, `ExitLong { reason }`, `ExitShort { reason }`, `NoSignal` |
| `ExitReason` | enum | `MomentumLost`, `ReversalDetected`, `StopLoss`, `TakeProfit`, `TrailingStop`, `TimeStop` |
| `TradeDirection` | enum | `Long`, `Short`, `Neutral` |
| `PricePoint` | struct | `{ price: f64, timestamp_ms: i64 }` |
| `MomentumSnapshot` | struct | `{ price_count, current_price, price_velocity_pct, direction, strength, volatility_pct }` — computed analysis of current market state |
| `MomentumDetector` | struct | `{ threshold_pct: f64, lookback_count: usize, prices: VecDeque<PricePoint> }` — stateful price buffer |

#### `MomentumDetector` Public Functions

| Function | Description |
|----------|-------------|
| `new(threshold_pct, lookback_count)` | Creates detector with price buffer capacity of `lookback_count * 2` |
| `push_price(price, timestamp_ms)` | Appends price, trims buffer to `lookback_count * 2` |
| `analyze() -> MomentumSnapshot` | Computes velocity, direction, strength, volatility from buffered prices |
| `detect_signal(snapshot) -> Signal` | Returns entry signal if velocity ≥ threshold AND strength ≥ 50.0 |
| `detect_exit(snapshot, is_long, entry_price, current_price, peak_price, hold_secs, max_hold_secs, tp_pct, sl_pct, trail_pct, trail_act_pct) -> Option<Signal>` | Multi-criteria exit detection |

#### Signal Strength Computation
```
strength = velocity_score + consecutive_score - volatility_penalty
where:
  velocity_score = (|velocity| / threshold).min(1.0) * 50.0
  consecutive_score = (|consecutive| / lookback).min(1.0) * 30.0
  volatility_penalty = volatility.min(20.0)
```
Range: 0.0–80.0 (theoretical), threshold for entry is ≥ 50.0.

#### Exit Priority (checked in order)
1. **Stop Loss** — absolute PnL ≤ `-sl_pct`
2. **Take Profit** — absolute PnL ≥ `tp_pct`
3. **Trailing Stop** — activated after `trail_act_pct` profit, triggers on `trail_pct` retracement from peak
4. **Time Stop** — hold duration ≥ `max_hold_secs`
5. **Momentum Lost** — direction becomes Neutral while in profit, held > 2 min
6. **Reversal** — opposite direction detected with strength > 40.0

#### Patterns
- `MomentumDetector` is stateful — maintains a sliding window of prices
- Shorts correctly track `peak_price` as the lowest price seen (not highest)
- Direction threshold is 30% of the configured threshold (hardcoded 0.3 multiplier)
- Minimum 5 prices required for entry signal, 3 for analysis

---

### 3. `src/risk.rs` — Risk Management & Position Tracking

**Purpose:** Position state, risk limits (circuit breaker, drawdown, cooldown), trade journal with atomic writes.

#### Key Structs

| Struct | Key Fields | Description |
|--------|------------|-------------|
| `Position` | `position_key`, `symbol`, `asset`, `is_long`, `entry_price`, `current_price`, `peak_price`, `size_usd`, `leverage`, `open_time` | Tracks a single open position. `peak_price` = highest for longs, lowest for shorts. |
| `RiskManager` | `config: RiskConfig`, `daily_pnl: Mutex<f64>`, `total_fees: Mutex<f64>`, `daily_peak_balance: Mutex<f64>`, `initial_balance: Mutex<f64>`, `trade_date: Mutex<u32>`, `halted: AtomicBool`, `cooldown_until: Mutex<Option<DateTime<Utc>>>` | Thread-safe risk state with interior mutability via Mutex/AtomicBool |
| `TradeRecord` | `symbol`, `direction`, `entry_price`, `exit_price`, `size_usd`, `pnl`, `fees`, `hold_secs`, `exit_reason`, `timestamp` | Serializable trade log entry |
| `TradeLog` | `trades: Vec<TradeRecord>`, `filepath: String` | In-memory trade journal, flushed to JSON on every record |
| `TradeStats` | `total_trades`, `wins`, `win_rate`, `total_pnl`, `total_fees`, `net_pnl`, `avg_hold_secs`, `best_trade`, `worst_trade` | Computed summary statistics |

#### `Position` Methods

| Method | Description |
|--------|-------------|
| `unrealized_pnl_pct()` | `%` PnL based on entry vs current price, directional for long/short |
| `unrealized_pnl_usd()` | Dollar PnL = `size_usd * pnl_pct / 100.0` |
| `hold_duration_secs()` | Seconds since `open_time` |
| `update_price(price)` | Updates `current_price` and `peak_price` (tracks best price for direction) |

#### `RiskManager` Public Functions

| Function | Description |
|----------|-------------|
| `new(config, initial_balance)` | Initializes with today's date, peak = initial_balance |
| `check_can_trade(balance) -> Result<(), String>` | Checks halted, cooldown, daily loss limit, max drawdown. Sets halted if limits exceeded. |
| `check_position_size(notional_usd) -> Result<(), String>` | Validates against `max_position_notional_usd` |
| `record_trade_result(pnl, fees, balance)` | Updates daily PnL, total fees, peak balance. Auto-resets day on date change. |
| `set_cooldown(secs)` | Sets cooldown from now + secs |
| `is_halted() -> bool` | Returns circuit breaker state |
| `total_fees() -> f64` | Returns cumulative fees |

#### `TradeLog` Functions

| Function | Description |
|----------|-------------|
| `new(filepath)` | Creates empty journal |
| `record(trade)` | Appends trade, calls `flush()` |
| `stats() -> TradeStats` | Computes aggregate stats over all trades |
| `flush()` | **Atomic write**: writes to `.tmp`, renames to target. Falls back to direct write on rename failure. |

#### Daily Reset Logic
- `maybe_reset_day()` is called before every `check_can_trade` and `record_trade_result`
- Compares current UTC day against `trade_date`
- On rolver: resets `daily_pnl` to 0.0, recalculates `daily_peak_balance` from `initial_balance + daily_pnl`

#### Patterns
- `RiskManager` uses `Arc<RiskManager>` in both engines — shared ownership
- Interior mutability via `Mutex` for all mutable state (not async-aware, but acceptable since locks are short-lived)
- `AtomicBool` for `halted` flag — checked without locking
- Position update logic is symmetric for longs/shorts via `peak_price` convention
- Trade journal uses atomic write pattern (write .tmp → rename)

---

### 4. `src/flash_api.rs` — Flash Trade REST Client

**Purpose:** HTTP client for all Flash Trade API endpoints. Handles price fetching, position queries, transaction building (open/close/trigger orders), pool data, and fee previews.

#### Key Structs

| Struct | Purpose |
|--------|---------|
| `FlashClient` | HTTP client wrapper (`reqwest::Client` + `base_url`) |
| `PriceData` | Price response: `price: u64`, `exponent: i32`, `price_ui: f64`, `timestamp_us: u64` |
| `FlashPosition` | Full position data: `position_key`, `owner`, `pool`, `custody`, `collateral`, `side`, `asset`, `size`, `size_usd`, `collateral_usd`, `leverage`, `entry_price`, `mark_price`, `liquidation_price`, `unrealized_pnl_usd`, `unrealized_pnl_pct`, `borrow_fee`, `open_time` |
| `FlashMarket` | Market metadata: `pool`, `name`, `symbol`, `asset`, `side`, `market_account`, `custody_account`, `token_mint`, `token_vault`, `oracle`, `max_leverage`, `fee_pct` |
| `OpenPositionRequest` | Open position request body (input/output tokens, amount, leverage, trade_type, owner, TP/SL, slippage) |
| `OpenPositionResponse` | Open position response (leverage, entry/liquidation price, fees, notional, transaction_base64, TP/SL quotes, error) |
| `ClosePositionRequest` | Close position request (position_key, USD amount, withdraw token, slippage) |
| `ClosePositionResponse` | Close position response (receive amount, prices, settled_pnl, fees, transaction_base64, error) |
| `PlaceTriggerRequest` | Trigger order request (owner, position_key, order_type, price, slippage) |
| `PlaceTriggerResponse` | Trigger order response (transaction_base64, error) |
| `TriggerQuote` | TP/SL quote data (exit_price, profit/loss_usd, pnl_percentage) |
| `PoolData` | Pool metrics (pool_pubkey, aum_usd, utilization) |

#### API Endpoints Used

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/prices/{symbol}` | Get current oracle price for a symbol |
| GET | `/prices` | Get all prices |
| GET | `/positions/owner/{owner}?includePnlInLeverageDisplay=true` | Get all positions for a wallet |
| GET | `/raw/markets` | Get all market configurations |
| POST | `/transaction-builder/open-position` | Build unsigned open-position tx |
| POST | `/transaction-builder/close-position` | Build unsigned close-position tx |
| POST | `/transaction-builder/place-trigger-order` | Build unsigned trigger order tx |
| GET | `/pool-data` | Get pool utilization data |
| POST | `/preview/exit-fee` | Preview exit fee for a position (used by paper engine) |

#### `FlashClient` Public Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `new(base_url)` | `FlashClient` | Creates client with 30s timeout |
| `get_price(symbol)` | `Result<f64>` | Fetches single price, returns `price_ui` |
| `get_prices()` | `Result<Vec<PriceData>>` | Fetches all prices |
| `get_positions(owner)` | `Result<Vec<FlashPosition>>` | Fetches positions for wallet (404 → empty vec) |
| `get_position_for_market(owner, asset, side)` | `Result<Option<FlashPosition>>` | Filters positions by asset+side |
| `get_markets()` | `Result<Vec<FlashMarket>>` | Lists all markets |
| `preview_open_position(...)` | `Result<OpenPositionResponse>` | Preview without building tx (no owner) |
| `build_open_position(...)` | `Result<OpenPositionResponse>` | Build tx with owner, optional TP/SL |
| `build_close_position(...)` | `Result<ClosePositionResponse>` | Build close tx |
| `build_trigger_order(...)` | `Result<PlaceTriggerResponse>` | Build trigger order tx |
| `get_pool_data()` | `Result<Vec<serde_json::Value>>` | Raw pool data |
| `preview_exit_fee(position_key, close_usd)` | `Result<f64>` | Estimate exit fee in USD |

#### Error Handling Pattern
- All API responses include an optional `err: Option<String>` field
- Errors are checked after deserialization: `if let Some(ref err) = resp.err { warn!(...) }`
- `classify_api_error()` in `engine.rs` categorizes errors (INSUFFICIENT_BALANCE, RATE_LIMITED, etc.)
- `preview_exit_fee` has complex fallback parsing for different response shapes

#### Patterns
- All request/response structs use `#[serde(rename_all = "camelCase")]` for API compatibility
- Amounts are formatted as strings with `format!("{:.2}", amount)`
- `open_position_inner` is the private core that both `preview_open_position` and `build_open_position` delegate to
- `skip_serializing_if = "Option::is_none"` on optional request fields
- 30-second HTTP client timeout

---

### 5. `src/executor.rs` — Solana Transaction Signing

**Purpose:** Loads Solana keypair, fetches USDC balance, signs transactions with fresh blockhashes, submits to Solana RPC.

#### Structs

| Struct | Fields | Description |
|--------|--------|-------------|
| `Executor` | `rpc: Arc<RpcClient>`, `keypair: Keypair` | Solana interaction handler |

#### Constants
- `USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"`
- `CONFIRM_POLL_MS: u64 = 500`
- `CONFIRM_MAX_POLLS: u32 = 60` (30 seconds total)

#### Public Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `new(rpc_url, keypair_path)` | `Result<Executor>` | Loads keypair (JSON array or bs58), creates RPC client with "confirmed" commitment |
| `wallet_pubkey()` | `String` | Returns keypair's pubkey as string |
| `get_balance()` | `Result<u64>` | SOL balance in lamports |
| `get_usdc_balance()` | `Result<f64>` | USDC balance via SPL token ATA. Returns 0.0 if no token account. |
| `sign_and_send(tx_base64)` | `Result<Signature>` | Decodes tx, fetches fresh blockhash, signs, sends, polls for confirmation |
| `sign_and_send_with_retry(tx_base64, max_retries)` | `Result<Signature>` | Retries sign_and_send up to max_retries times with 2s delay |

#### Transaction Flow
1. Base64 decode → bincode deserialize → `Transaction`
2. Fetch fresh blockhash via `spawn_blocking` (RPC calls are synchronous)
3. `tx.sign(&[&self.keypair], recent_blockhash)` — signs with single signer
4. Serialize + `spawn_blocking` → `rpc.send_transaction_with_config` (skip_preflight: true, max_retries: 3)
5. Poll for confirmation: `spawn_blocking` → loop `confirm_transaction` every 500ms for up to 30s

#### Keypair Loading
- Supports two formats: JSON array `[1,2,3,...]` (standard Solana) and bs58-encoded string
- Expands `~/` in path using `$HOME` env var

#### Patterns
- `Arc<RpcClient>` — shared RPC client, cloned into `spawn_blocking` closures
- All synchronous RPC calls go through `tokio::task::spawn_blocking` to avoid blocking the async runtime
- Fresh blockhash fetched before every signing (avoids ~60s expiry window)
- Confirmation polling is blocking (inside `spawn_blocking`)
- USDC balance uses Associated Token Account derivation

---

### 6. `src/engine.rs` — Live Trading Engine

**Purpose:** Main loop for live trading: poll price → detect signal → build/sign/submit tx → monitor position → close on exit.

#### Structs

| Struct | Fields | Description |
|--------|--------|-------------|
| `ScalperEngine` | `config: Config`, `flash: FlashClient`, `executor: Executor`, `detector: MomentumDetector`, `risk: Arc<RiskManager>`, `trade_log: TradeLog`, `position: Option<Position>`, `running: Arc<AtomicBool>` | Complete live trading state machine |

#### Public Functions

| Function | Description |
|----------|-------------|
| `new(config, executor)` | Creates engine with all sub-components |
| `shutdown_handle()` | Returns `Arc<AtomicBool>` for graceful shutdown |
| `run()` | Main loop: init → sync existing position → tick loop → final stats |

#### Private Functions (the main flow)

| Function | Description |
|----------|-------------|
| `sync_existing_position()` | On startup, fetches on-chain positions and reconciles local state |
| `verify_position_on_chain()` | Re-checks local position against chain. Clears if gone (liquidated/closed externally). |
| `tick()` | Fetches price → pushes to detector → analyzes → routes to `handle_no_position` or `manage_position` |
| `handle_no_position(snapshot, price)` | Checks risk/balance → detects signal → calls `open_position` |
| `open_position(is_long, clip, leverage, price, strength, velocity)` | Previews → builds tx with optional TP/SL → signs+sends → waits 3s → finds new position on-chain |
| `manage_position(snapshot, price)` | Updates price → detects exit → calls `close_position` |
| `close_position(exit_price, reason)` | Takes position → verifies on-chain → builds close tx → signs+sends → records result → sets cooldown if loss |
| `find_position(wallet, is_long)` | Searches on-chain positions for matching market+side |

#### Free Functions

| Function | Description |
|----------|-------------|
| `now_ms() -> i64` | Current timestamp in milliseconds |
| `parse_f64_safe(s, field_name) -> Result<f64>` | Parse string to f64 with error context (replaces old `parse_f64` that silently returned 0.0) |
| `classify_api_error(err) -> String` | Categorizes API error strings for better logging |

#### Error Handling
- Tick errors: logged, engine sleeps 10s, continues
- Preview errors: logged via `classify_api_error`, position open aborted
- Build errors: logged, position open aborted
- Sign/submit errors: logged, position state preserved for retry
- Close failure: position put back (`self.position = Some(pos)`) for retry
- Position not found on-chain during close: estimated PnL recorded, trade logged

#### Graceful Shutdown
- `running: Arc<AtomicBool>` checked every tick
- Set to `false` by ctrlc handler in `main.rs`
- On exit: verifies if on-chain position still exists, logs warning if open
- Final stats printed (trades, win rate, net PnL)

#### Patterns Worth Preserving
- Two-phase open: preview first (for logging), then build (for signing)
- Position key from on-chain: after opening, engine waits 3s then queries chain to get the actual position key
- TP/SL prices computed from entry price, only included if `use_native_tp_sl` is true
- `find_position` is a shared utility for syncing state between local and on-chain

---

### 7. `src/paper.rs` — Paper Trading Engine

**Purpose:** Simulates the full trading loop against live prices with realistic fee estimation, but never signs or submits real transactions.

#### Constants
- `FALLBACK_FEE_RATE: f64 = 0.001` (0.1% — used when API preview fails)
- `BORROW_FEE_HOURLY: f64 = 0.0001` (0.01% per hour on position notional)

#### Key Structs

| Struct | Fields | Description |
|--------|--------|-------------|
| `PaperEngine` | `config`, `flash`, `detector`, `risk`, `trade_log`, `position: Option<PaperPosition>`, `running`, `sim_balance: f64`, `pending_entry_fee: f64` | Paper trading state machine (mirrors `ScalperEngine`) |
| `PaperPosition` (private) | `inner: Position`, `entry_fee: f64`, `accrued_borrow_fee: f64` | Extended position with fee tracking |

#### `PaperPosition` Methods
- `update_price(price)` — delegates to `inner.update_price()`, then accrues borrow fee pro-rated for the tick interval
- `config_poll_interval_secs()` — hardcoded to 5 (TODO: use actual config)
- `total_fees()` — returns `entry_fee + accrued_borrow_fee`

#### Public Functions

| Function | Description |
|----------|-------------|
| `new(config, starting_balance)` | Creates paper engine with simulated balance |
| `shutdown_handle()` | Returns shutdown `Arc<AtomicBool>` |
| `run()` | Main loop (same structure as `ScalperEngine`) |

#### Private Functions

| Function | Description |
|----------|-------------|
| `tick()` | Same as live engine: fetch price → push → analyze → route |
| `handle_no_position(snapshot, price)` | Checks risk against `sim_balance` → detects signal → calls `paper_open` |
| `paper_open(is_long, clip, leverage, price, strength, velocity)` | Calls live API preview for real entry fee → creates `PaperPosition` → deducts entry fee from `sim_balance` |
| `manage_position(snapshot, price)` | Updates price (with borrow fee accrual) → detects exit → calls `paper_close` |
| `paper_close(exit_price, reason)` | Computes gross PnL → estimates exit fee from live API (fallback to 0.1%) → computes net PnL → updates `sim_balance` → records trade |

#### Fee Accounting (Key Differentiator)
1. **Entry fee** — fetched from live API preview at open time (real fee estimate)
2. **Exit fee** — fetched from live API `preview_exit_fee` at close time (real fee estimate)
3. **Borrow fee** — accrued incrementally each tick: `size_usd * BORROW_FEE_HOURLY * hours_per_tick`
4. **Total fees** = entry_fee + exit_fee + accrued_borrow_fee
5. **Net PnL** = gross_pnl - exit_fee - accrued_borrow_fee (entry fee already deducted from balance at open)

#### Patterns Worth Preserving
- Mirrors `ScalperEngine` structure exactly (same signal/risk/flash_api flow)
- Live API fee estimation ensures realistic PnL (not theoretical)
- `sim_balance` tracks the virtual account, updated on every open/close
- Fallback fee rate prevents API failures from blocking paper trading
- Shutdown shows open position with full fee breakdown

---

### 8. `src/main.rs` — CLI & Entry Point

**Purpose:** CLI argument parsing, mode selection (live/paper/dry-run), engine lifecycle management.

#### CLI Arguments (clap)

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--config` | PathBuf | `config/perps.toml` | Config file path |
| `--keypair` | Option<String> | None | Override keypair path |
| `--market` | Option<String> | None | Override market |
| `--dry-run` | bool | false | Single preview, no signing |
| `--paper` | bool | false | Full paper trading loop |
| `--paper-balance` | f64 | 1000.0 | Starting balance for paper mode |

#### Mode Logic
```
(dry_run, paper) → action
(true, false)    → run_dry()   — single API preview, exit
(false, true)    → run_paper() — PaperEngine loop
(true, true)     → bail!       — incompatible flags
(false, false)   → run_live()  — ScalperEngine loop
```

#### Runtime Setup
1. Parse CLI args
2. Load config from TOML (with CLI overrides for market and keypair)
3. Initialize `tracing_subscriber` with env filter fallback to config's `log_level`
4. Route to appropriate run function
5. Live/paper modes: set up `ctrlc` handler with `running` AtomicBool

---

## Uncommitted Changes (git diff summary)

There are **11 modified files** with **+1101 / -181 lines** of changes (plus 2 untracked files):

### Modified Files

| File | Changes Summary |
|------|----------------|
| `CLAUDE.md` | Updated coding guidelines (v0.2 → v0.3 conventions) |
| `Cargo.lock` | Lockfile updates for dependency changes |
| `Cargo.toml` | Added `[[bin]]` target, `[workspace]` table |
| `README.md` | Documentation updates |
| `SESSION-CONTEXT.md` | Session state updates |
| `src/engine.rs` | Major: graceful shutdown (`AtomicBool`), USDC balance checks (was SOL estimate), `parse_f64_safe` (was silent 0.0), `classify_api_error`, `verify_position_on_chain`, position size validation, final sync at shutdown, TradeRecord on missing positions |
| `src/executor.rs` | Major: `Arc<RpcClient>` (was bare), `spawn_blocking` for all RPC calls (was blocking async runtime), fresh blockhash (was stale), `get_usdc_balance()` via SPL ATA, confirmation polling via `spawn_blocking`, retry logic |
| `src/flash_api.rs` | Added `preview_exit_fee()` method for paper trading fee estimation |
| `src/main.rs` | Added `--paper` mode, `--paper-balance` flag, `run_paper()`, version bump to v0.3 |
| `src/risk.rs` | Major: `record_trade_result` now takes `balance` param (was just pnl+fees), daily peak balance tracking, `check_position_size`, daily PnL reset, initial balance tracking |
| `src/signal.rs` | Enhanced exit detection (trailing stop, time stop, momentum lost, reversal), improved strength computation |

### Untracked Files

| File | Description |
|------|-------------|
| `docs/MISSION-ALPHA-HUNTER.md` | Multi-strategy roadmap (leaderboard scraping, wallet analysis, strategy trait refactor, paper validation) |
| `src/paper.rs` | New file — complete paper trading engine with fee accounting |

### Key Themes in Uncommitted Changes
1. **Safety**: Silent 0.0 parsing → `parse_f64_safe` with errors; stale blockhash → fresh blockhash; SOL estimate → real USDC balance
2. **Async correctness**: Blocking RPC calls moved to `spawn_blocking`; `Arc<RpcClient>` for shared ownership
3. **Fee-awareness**: Paper trading with live fee estimation; fee tracking in trade journal
4. **Graceful shutdown**: `AtomicBool` + ctrlc handler + final position sync
5. **Risk improvements**: Position size limits, drawdown tracking, daily reset, cooldowns

---

## Cross-Cutting Patterns Worth Preserving

### 1. Two-Engine Architecture
`ScalperEngine` (live) and `PaperEngine` (paper) share the same signal/risk/flash_api modules but differ in execution. Both follow the same `tick → handle_no_position | manage_position → open | close` flow. Any refactoring should maintain this symmetry.

### 2. State Machine Pattern
Both engines use `Option<Position>` as a simple state machine:
- `None` → look for entry signals
- `Some(_)` → manage position, look for exit signals
- Close → `self.position.take()` → back to `None`

### 3. API Error Tolerance
The engines never panic on API errors. Every API call is wrapped in match/if-let with logging and graceful fallback. This is critical for a trading bot — uptime matters more than perfection.

### 4. Atomic Writes
`TradeLog::flush()` writes to `.tmp` then renames — prevents corrupt JSON from partial writes. This pattern should be used for any new data persistence.

### 5. Separation of Concerns
- `signal.rs` — Pure analysis, no side effects, no I/O
- `risk.rs` — State tracking, no I/O
- `flash_api.rs` — Pure I/O, no state
- `executor.rs` — Solana I/O, no trading logic
- `engine.rs` / `paper.rs` — Orchestration only

### 6. Config as Data
All parameters are in TOML, loaded once, never mutated. This makes it easy to add new parameters without touching runtime logic.

---

## Potential Refactoring Targets (for future work)

1. **Shared engine trait**: `ScalperEngine` and `PaperEngine` share ~60% code. A `TradingEngine` trait could reduce duplication.
2. **Fee estimation abstraction**: Paper engine's fee logic is tightly coupled to `FlashClient`. A `FeeEstimator` trait could support multiple fee models.
3. **Position type**: `Position` in `risk.rs` is used by both engines. `PaperPosition` wraps it. Consider a trait or generic.
4. **Signal module**: `MomentumDetector` is the only strategy. The MISSION-ALPHA-HUNTER doc proposes a `Strategy` trait — this is the right direction.
5. **Config polling interval**: `PaperPosition::config_poll_interval_secs()` hardcodes 5s instead of reading from config.
6. **Error types**: Mixed `anyhow::Result` and `Result<_, String>` in risk module. Consider unified error types.
