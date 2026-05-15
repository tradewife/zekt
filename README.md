# Zekt — Momentum Scalper for Solana Perps

Autonomous momentum scalping bot for Flash Trade, a Solana-based perpetuals DEX. Built from on-chain analysis of Bulk.Trade's devnet competition where the ZEC Momentum Scalper strategy earned $229K across 5 wallets with 67-83% win rates.

## Strategy

Zekt polls Flash Trade oracle prices, detects momentum via velocity + consecutive directional moves, and opens leveraged positions with native TP/SL. All parameters are configurable via TOML.

**Core loop:**
```
Poll price → Detect momentum → Preview trade → Build Solana tx → Sign + submit → Monitor → Exit on signal
```

**Signals:**
- **Entry:** Price velocity exceeds threshold + sufficient consecutive directional moves
- **Exit:** Take profit, stop loss, trailing stop, time stop, momentum loss, reversal detection

**Risk controls:**
- Daily loss limit, max drawdown circuit breaker, cooldown after losses
- Position sizing caps, leverage limits
- Native on-chain TP/SL via Flash Trade trigger orders (optional)

## Quick Start

```bash
# Build
cargo build --release

# Dry run (preview only, no signing)
./target/release/zekt --dry-run

# Live (requires funded Solana wallet)
export SOLANA_KEYPAIR=~/.config/solana/id.json
./scripts/run-zekt.sh --market SOL

# With custom config
./target/release/zekt --config config/perps.toml --keypair ~/.config/solana/id.json --market BTC
```

## Config

Edit `config/perps.toml`:

```toml
[flash]
market = "SOL"          # SOL, BTC, ETH, ZEC, BNB, etc.
leverage = 10.0
slippage_pct = "0.5"

[strategy]
clip_size_usd = 100.0    # USD per trade
direction_bias = "neutral" # long, short, or neutral
momentum_threshold_pct = 0.15
take_profit_pct = 2.5
stop_loss_pct = 1.0
use_native_tp_sl = true  # On-chain TP/SL via Flash trigger orders
```

## Supported Markets

Flash Trade Crypto.1 pool: SOL, BTC, ETH, ZEC, BNB  
Also: XAU, XAG, EUR, JUP, BONK, WIF, PENGU, FARTCOIN, and more.

## Architecture

```
src/
  main.rs        CLI entrypoint (clap)
  config.rs      TOML config parser
  flash_api.rs   Flash Trade REST client (prices, positions, transaction builder)
  executor.rs    Solana keypair loading + tx sign/submit/confirm
  signal.rs      Momentum detector (price velocity, consecutive moves, exit signals)
  risk.rs        Risk manager (SL/TP/trailing, circuit breaker, trade journal)
  engine.rs      Main trading loop
```

## Origin

See `docs/bulktrade-analysis.md` for the full analysis of Bulk.Trade's devnet competition that identified the winning momentum scalper strategy. Key findings:

- 5/10 leaderboard wallets ran the same ZEC momentum scalper bot
- Net PnL: +$229K combined, 67-83% win rate
- Strategy: detect momentum → pile in with fixed clips at 40x → ride 2-5% moves → cut losers fast
- HFT market-making and cross-venue arb strategies were net negative after fees

## Flash Trade API

Base URL: `https://flashapi.trade`

Key endpoints used:
- `GET /prices/{symbol}` — Real-time oracle prices
- `GET /positions/owner/{owner}` — Enriched positions with PnL
- `POST /transaction-builder/open-position` — Build unsigned Solana tx
- `POST /transaction-builder/close-position` — Build close tx
- `POST /transaction-builder/place-trigger-order` — Place TP/SL

No authentication required. Transactions are signed locally with your Solana keypair.

## Risk Warning

This software trades real money on Solana mainnet. Flash Trade supports up to 500x leverage. Use `--dry-run` to preview trades before committing funds. Never risk more than you can afford to lose.

## License

MIT
