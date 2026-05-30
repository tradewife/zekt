# Paper-Trading Promotion Gate

**Version:** 1.0  
**Date:** 2026-05-30  
**Status:** Active — ALL items required before any live promotion

---

## 1. Runbook: Paper Trading Commands

### Single-Strategy Paper Trading (minimum 24h)
```bash
# Momentum scalper on BTC — 24h paper run
timeout 86400 ./target/release/zekt --paper \
  --strategies momentum-scalper \
  --markets BTC \
  --paper-balance 1000

# Mean reversion on SOL — 24h paper run  
timeout 86400 ./target/release/zekt --paper \
  --strategies mean-reversion \
  --markets SOL \
  --paper-balance 1000

# All strategies on BTC+SOL — 48h paper run (recommended)
timeout 172800 ./target/release/zekt --paper \
  --strategies momentum-scalper,mean-reversion,trend-follower,funding-capture \
  --markets BTC,SOL \
  --paper-balance 1000
```

### Multi-Strategy Paper Trading via Pipeline (48h recommended)
```bash
cargo run --bin pipeline -- \
  --paper-balance 1000 \
  --duration-hours 48 \
  --strategies momentum-scalper,mean-reversion,trend-follower \
  --markets BTC,SOL
```

### Backtest for Pre-Paper Validation
```bash
# Run backtest first to validate strategy parameters
./target/release/zekt --backtest \
  --strategies momentum-scalper \
  --markets BTC \
  --backtest-start 2026-05-15 \
  --backtest-interval 5m \
  --paper-balance 1000
```

### 60-Second Smoke Test
```bash
# Verify paper trading works before committing to 24h run
timeout 60 ./target/release/zekt --paper \
  --strategies momentum-scalper \
  --markets SOL \
  --paper-balance 1000
# Expected: no panic, ≥1 price tick logged, no errors
```

---

## 2. Duration Requirements

**Minimum:** 24 hours (86400 seconds)  
**Recommended:** 48-72 hours  
**Rationale:** 
- 24h captures at least 1 full funding rate cycle (8h on most venues)
- 48h captures 2 full daily PnL resets and multiple regime transitions
- Flash Trade market conditions can vary significantly across sessions
- Shorter durations have insufficient statistical power (< 20 trades typical)

---

## 3. Metrics to Collect

| # | Metric | Source | Extraction |
|---|--------|--------|------------|
| 1 | **Net PnL (after fees)** | Paper engine log | `grep "net PnL" paper-run.log` or `perps-trades.json` → sum(pnl - fees) |
| 2 | **Sharpe Ratio** | Paper engine summary | Final log line: "Sharpe ratio" |
| 3 | **Max Drawdown %** | Risk manager | `grep "drawdown" paper-run.log` or computed from balance curve |
| 4 | **Win Rate %** | Paper engine summary | Final log line: "win rate" |
| 5 | **Trade Count** | Paper engine summary | Final log line: "total trades" |
| 6 | **Fee Ratio** (fees/notional) | Paper trades JSON | `sum(fees) / sum(size_usd)` from `perps-trades.json` |
| 7 | **Slippage Estimate** | Price comparison | Compare expected vs actual entry prices in trade log |
| 8 | **Average Hold Time** | Paper trades JSON | `avg(hold_secs)` from trade records |
| 9 | **Fee Breakdown** (entry/exit/borrow/slippage) | Paper trades JSON | Per-trade `entry_fee`, `exit_fee`, `borrow_fee` fields |
| 10 | **Regime Distribution** | Paper engine log | `grep "regime" paper-run.log` — count LowVol/Trending/HighVol/Choppy |

---

## 4. Promotion Thresholds

| Threshold | Value | Rationale |
|-----------|-------|-----------|
| **Positive expectancy** | Net PnL > $0 after all costs | Must be profitable after entry + exit + borrow + slippage fees |
| **Minimum trade count** | ≥ 20 trades in 24h | Statistical significance floor (< 20 is noise) |
| **Max drawdown** | ≤ 10% | $100 account → max $10 drawdown from peak |
| **Sharpe ratio** | ≥ 0.5 (relaxed from 1.0 for paper) | Paper Sharpe ≥ 0.5 → likely ≥ 0.3 live (slippage drag) |
| **Fee ratio** | ≤ 50% of gross PnL | If fees consume > 50% of gross profit, edge is too thin |
| **Win rate** | ≥ 30% | Below 30% implies poor entry timing or adverse selection |

**ALL thresholds must be met simultaneously.** Missing any single threshold = DO NOT PROMOTE.

---

## 5. Monitoring Checklist

Check these items during and after the paper trading run:

| # | Check | Method | Frequency | Trigger for Human Review |
|---|-------|--------|-----------|--------------------------|
| 1 | **API health** | `curl -sf https://flashapi.trade/prices/BTC` | Every 5 min | >3 consecutive failures or latency >5s |
| 2 | **Position status** | Paper engine log: "OPENED", "CLOSED" | Continuous | Position open >4h without exit signal |
| 3 | **Cumulative PnL** | Running net PnL in log | Every trade | Net PnL < -5% of starting balance |
| 4 | **Fee ratio** | Running fee / running gross PnL | Every 10 trades | Fee ratio > 60% of gross PnL |
| 5 | **Regime transitions** | Log entries for regime changes | Continuous | >10 regime transitions per hour (unstable) |
| 6 | **Circuit breaker** | "HALTED" in log | Immediate | Any circuit breaker trigger |

---

## 6. Human Review Triggers

These conditions require immediate human review before continuing:

| # | Trigger | Threshold | Action |
|---|---------|-----------|--------|
| 1 | **Daily loss exceeds 50% of limit** | Daily PnL < -$250 (limit: $500) | Pause, review trade quality |
| 2 | **Consecutive losses** | ≥ 3 consecutive losses (pre-M3 circuit breaker) | Review strategy compatibility with current regime |
| 3 | **API degradation** | ≥ 3 consecutive API failures | Halt, check Flash Trade status page |
| 4 | **No trades in 6h** | Zero entries in 6 hours during active market | Check regime filter settings, may be over-filtering |
| 5 | **Single trade > 50% of balance** | Position notional > $500 on $1000 account | Review sizing logic, reduce clip_size_usd |

---

## 7. Human Approval Checklist

Before promoting from paper to live, the following items require explicit human sign-off:

- [ ] **Live enable**: I have reviewed the paper trading results and confirm positive expectancy after costs.
  - Net PnL: $____ (attach paper run log)
  - Sharpe ratio: ____ (attach summary)
  - Verified by: ________________ Date: __________

- [ ] **Size increase**: I confirm the proposed live position size is appropriate.
  - Paper clip size: $____
  - Proposed live clip size: $____
  - Max single position: $____ (must not exceed 5% of account)
  - Verified by: ________________ Date: __________

- [ ] **Leverage confirmation**: I confirm the proposed leverage level is acceptable.
  - Paper leverage: ____x
  - Proposed live leverage: ____x
  - Max drawdown at this leverage: ____% (from paper data)
  - Verified by: ________________ Date: __________

- [ ] **Risk limits confirmed**: I have reviewed all risk limits and confirm they are appropriate for live trading.
  - max_daily_loss_usd: $____
  - max_weekly_loss_usd: $____
  - max_drawdown_pct: ____%
  - consecutive_loss_circuit_breaker: ____
  - Verified by: ________________ Date: __________

- [ ] **Keypair security**: I confirm the Solana keypair is stored securely and not in git history.
  - Keypair path: ________________
  - Key not in git: ☐ Confirmed
  - Verified by: ________________ Date: __________

---

## 8. No-Bypass Statement

**ALL items in this promotion gate are mandatory.** There is no auto-promotion mechanism. No script, automation, or agent may enable live trading without explicit human sign-off on every checklist item above.

If any threshold is not met, the strategy MUST remain in paper/backtest mode until:
1. The strategy parameters are modified
2. A new 24h+ paper run is completed
3. All thresholds are met on the new run
4. All checklist items are re-signed

---

## 9. Smoke Test Verification

Before any extended paper run, verify the command works:

```bash
# 60-second smoke test
timeout 60 ./target/release/zekt --paper \
  --strategies momentum-scalper \
  --markets SOL \
  --paper-balance 1000

# Expected output:
# - ≥ 1 "Price tick" log line
# - No "panic" in output
# - Exit code 124 (timeout) or 0 (clean shutdown)
# - No "HALTED" or "circuit breaker" messages
```
