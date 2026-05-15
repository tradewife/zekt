# Zekt — Session Context

## What This Is
Zekt is a **liquidity-aware momentum scalper** for Flash Trade (Solana perps DEX, live on mainnet). It hunts for illiquid markets where a single dominant LP provides most of the depth, then detects when that LP is being consumed in one direction as a momentum signal.

This is market-structure arbitrage, not token-picking. On Bulk.Trade the vehicle was ZEC (illiquid, single LP). On Flash Trade the right market could be anything — what matters is thin books and concentrated counterparty flow.

## Origin
The strategy was reverse-engineered from the Bulk.Trade devnet competition where 5/10 leaderboard wallets earned $229K combined on ZEC-USD. But ZEC wasn't the point — it was chosen because it had a single dominant LP (2Gg7..v2di) providing 98%+ of fills. The bots detected when that LP was being consumed in one direction (momentum) and rode the move. See `docs/bulktrade-analysis.md` for the full analysis.

## Architecture
```
Flash Trade API (prices, positions, tx builder)
  → Momentum Detector (price velocity + consecutive moves)
  → Risk Manager (SL/TP/trailing, circuit breaker, cooldown)
  → Executor (Solana keypair sign + RPC submit)
  → Trade Journal (JSON log of all closed trades)
```

## Commands
```bash
# Build
cargo build --release

# Dry run (preview only)
./target/release/zekt --dry-run
./target/release/zekt --dry-run --market BTC

# Live trading
export SOLANA_KEYPAIR=~/.config/solana/id.json
./scripts/run-zekt.sh
./scripts/run-zekt.sh --market SOL
./scripts/run-zekt.sh --dry-run  # preview mode

# Config
./target/release/zekt --config config/perps.toml --keypair ~/.config/solana/id.json
```

## Config (`config/perps.toml`)

### Flash Trade Connection
- `flash.api_url` = `https://flashapi.trade`
- `flash.rpc_url` = `https://api.mainnet-beta.solana.com`
- `flash.keypair_path` = Solana keypair file (JSON array or bs58)
- `flash.market` = Trading pair (SOL, BTC, ETH, ZEC, BNB, etc.)
- `flash.input_token` = Collateral token (USDC)
- `flash.leverage` = Leverage multiplier (default 10.0)
- `flash.slippage_pct` = Slippage tolerance (default "0.5")

### Strategy Parameters
- `strategy.clip_size_usd` = $100 per trade
- `strategy.momentum_threshold_pct` = 0.15% price velocity to trigger
- `strategy.lookback_count` = 60 price points in analysis window
- `strategy.direction_bias` = "neutral" (can be "long" or "short")
- `strategy.take_profit_pct` = 2.5%
- `strategy.stop_loss_pct` = 1.0%
- `strategy.trailing_stop_pct` = 0.8% (after 1.5% activation)
- `strategy.max_hold_secs` = 1800 (30 min time stop)
- `strategy.cooldown_after_loss_secs` = 300 (5 min cooldown)
- `strategy.use_native_tp_sl` = true (Flash on-chain trigger orders)

### Risk Limits
- `risk.max_position_notional_usd` = $1000
- `risk.max_daily_loss_usd` = $200
- `risk.max_drawdown_pct` = 15%

## Supported Markets (Flash Trade Crypto.1)
SOL, BTC, ETH, ZEC, BNB (+ forex, equities, meme tokens on other pools)

## Flash Trade API Key Endpoints
- `GET /prices/{symbol}` → Oracle price (Pyth)
- `GET /positions/owner/{owner}` → Enriched positions with PnL
- `POST /transaction-builder/open-position` → Unsigned Solana tx (with optional TP/SL)
- `POST /transaction-builder/close-position` → Unsigned close tx
- `POST /transaction-builder/place-trigger-order` → TP/SL trigger orders
- No auth required. API is public. Transactions signed locally.

## Critical Flash Trade Rules
- **One position per market per side per wallet** — cannot hold independent positions at different entries
- **Blockhash expiry ~60s** — sign and submit promptly
- **Max 5 trigger orders** (TP/SL) per market position
- **Min collateral >$10** after fees for trigger orders
- **SOL positions use JitoSOL** as underlying collateral on-chain
- **All amounts are UI format** (human-readable) in API requests

## Emergency Halt
```bash
pkill -f zekt  # Kill the running bot
```

## Key Files
- `src/engine.rs` — Main trading loop (poll → detect → execute → manage)
- `src/flash_api.rs` — Flash Trade REST client
- `src/executor.rs` — Solana tx signing and submission
- `src/signal.rs` — Momentum detector
- `src/risk.rs` — Risk manager + trade journal
- `config/perps.toml` — All tunable parameters
- `docs/bulktrade-analysis.md` — Original Bulk.Trade wallet analysis

## TODO / Next Steps
- [ ] **Market scanner** — Rank Flash Trade markets by liquidity concentration (find thin books with dominant LPs, like ZEC was on Bulk)
- [ ] **LP detection** — Identify dominant counterparties via position changes and fill patterns on Flash
- [ ] **LP consumption rate signal** — Detect when a large LP is being eaten in one direction (the real edge from Bulk analysis)
- [ ] WebSocket streaming for real-time price updates (instead of polling)
- [ ] Backtesting engine against historical prices
- [ ] Adaptive momentum threshold (adjust based on volatility regime)
- [ ] Scale-in logic (add to winning positions when momentum accelerates)
- [ ] Fee-awareness (track Flash's hourly borrow rate, avoid high-fee periods)
