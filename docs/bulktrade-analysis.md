# Bulk.Trade Devnet Competition — Top 10 Wallet Analysis

## Executive Summary

Analyzed 10 leaderboard wallets from the Bulk.Trade devnet perps competition. **3 distinct strategies identified**, with the **ZEC Momentum Scalper** being the dominant and most profitable approach. At least 5 of the top 10 wallets are running automated bots. Two strategies (HFT market-making, aggressive taker arb) are **net negative** after fees.

**Key finding:** The leaderboard ranks by realized PnL *before* fees. Two wallets (#3, #4, #10) are actually losing money when fees are included.

---

## PnL Leaderboard (Net After Fees)

| Rank | Address | Net PnL | Win Rate | Strategy | Bot? |
|------|---------|---------|----------|----------|------|
| #1 | B8YxkfYZ...Mvz9 | **+$159,672** | 83% (10/12) | ZEC Momentum Scalper | YES |
| #2 | jYLSWmUX...6sMK | **+$20,931** | 33% (1/3) | BTC Swing Trader | Partial |
| #6 | HJGKXUGc...PX44 | **+$19,176** | 78% (7/9) | ZEC Short Scalper | YES |
| #8 | 28tRzhaw...wJBe | **+$18,923** | 50% (1/2) | ZEC Sniper | Unclear |
| #7 | GPb4n63M...oGSK | **+$16,505** | 67% (12/18) | ZEC+XRP Mixed Scalper | YES |
| #5 | 5Np87jRD...hgEn | **+$15,382** | 75% (6/8) | ZEC Long Scalper | YES |
| #9 | CeRz6GvV...j1Rs | **+$15,675** | 80% (4/5) | ETH Swing Trader | Partial |
| **#3** | **AWKS5eEs...zdQN** | **-$42,833** | 1% (1/88) | HFT Market Maker | YES |
| **#4** | **92dDwGdN...WB5A** | **-$26,899** | 0% (0/19) | BTC/ETH Arb | YES |
| **#10** | **EYbxQZMq...rpu6** | **-$20,324** | 0% (0/13) | BTC Arb | YES |

---

## Strategy A: ZEC Momentum Scalper (WINNER)

**Used by:** #1, #5, #6, #7, #8  
**Net PnL:** +$229,658 combined  

### Bot Signatures Identified
- **Fixed 10 ZEC clip size** — 80-88% of all fills are exactly 10.00 units (hardcoded template)
- **100% taker execution** — Never posts passive orders, always hits bids/asks
- **Single counterparty** — 98%+ fills against `2Gg7MCvwmEQ2xSGJomhAVs6Dauf2nLTy8rG9Xm8Lv2di` (Bulk's ZEC LP)
- **Sub-30-second fill intervals** — Automated order placement
- **Zero slot overlap** between wallets — Run at different times (same operator?)
- **Single market focus** — ZEC-USD exclusively for 4/5 wallets

### Quantified Parameters

| Parameter | Value |
|-----------|-------|
| Instrument | ZEC-USD |
| Clip size | 10 ZEC fixed (aggressive taker) |
| Leverage | 40-50x |
| Position size | 750-1,000 ZEC per trade (~$400K-$550K notional) |
| Hold time | 5-45 min (median ~20 min) |
| Win rate | 67-83% |
| Avg winning trade | $2,000-$5,000 |
| Max winning trade | $151,633 (#1's massive ZEC short) |
| Cut losers | Fast — losing trades held <5 min |
| Ride winners | Hold 30-75 min on big moves |

### Strategy Logic (Reconstructed)
1. **Identify momentum direction** on ZEC-USD (trend detection)
2. **Open position with fixed 10-unit clips** via aggressive market orders
3. **Scale in** — accumulate position over multiple fills (avg ~80 fills per position)
4. **Monitor trend** — if momentum continues, hold and add
5. **Exit on momentum failure** — close quickly when direction reverses
6. **Take profit** on 2-5% moves (at 40x leverage = 80-200% return)

### Variant Differences
- **#1 (Long-biased):** Net +13,283 ZEC accumulated. Aggressive buyer. Best performer.
- **#5 (Mild long):** Only longs on ZEC. More conservative sizing.
- **#6 (Short-only):** Net -1,836 ZEC. Exclusively shorts. 78% win rate.
- **#7 (Market neutral):** Net zero on ZEC. Pure spread/scalp capture. Most consistent.
- **#8 (Sniper):** Only 73 fills total, but caught one massive $19K move. Selective.

---

## Strategy B: Multi-Asset Swing Trader (PROFITABLE)

**Used by:** #2, #9  
**Net PnL:** +$36,606 combined  

### Parameters
| Parameter | #2 | #9 |
|-----------|----|----|
| Instruments | SOL, BTC, ETH | ETH only |
| Leverage | 50x on BTC | ~20x |
| Position size | $6.8M BTC, $20M SOL | $300 ETH |
| Hold time | 2-573 min | 11-775 min |
| Win rate | 33% (big wins) | 80% |
| Approach | Directional + funding capture | Trend-following on ETH |

### Notes
- #2's profit comes from a single $58K BTC short held for 9.5 hours
- #9 shorts ETH with high conviction, holds for hours
- Both are more human-like — fewer trades, larger sizes, longer holds
- #2 takes advantage of funding rates ($5,310 positive funding)

---

## Strategy C: HFT Market Maker (FAILED)

**Used by:** #3  
**Net PnL:** -$42,833  

### What happened
- Ran 5000 fills across 9 markets in 48 minutes (100 fills/min)
- Earned only $233 in realized PnL
- **Lost $39,966 to fees and $3,100 to funding**
- 0% win rate on SOL (82 consecutive losing trades)

### Why it failed
1. **Fee structure** — Bulk's taker fees destroy HFT margins
2. **No spread edge** — Can't capture enough to cover costs
3. **Too many markets** — Spread thin across 9 instruments
4. **Sub-second fills** — Literally paying to trade

---

## Strategy D: Aggressive BTC/ETH Arb (FAILED)

**Used by:** #4, #10  
**Net PnL:** -$47,223 combined  

Both wallets have **0% win rate** — every single trade is a loser. Killed by $43K+ in fees. Likely testing stat-arb or cross-venue strategies that don't have enough spread on devnet.

---

## Counterparty Analysis: 2Gg7..v2di

- **Identity:** Bulk.Trade's ZEC-USD market maker / liquidity provider
- **Fills:** 5000+ (capped), ZEC-USD only
- **Role:** Provides the bid/ask that all ZEC cluster bots hit
- **6.1% of fills** are against leaderboard wallets
- This is likely Bulk's own market-making bot or a designated LP

---

## Strategy to Emulate: ZEC Momentum Scalper

### Why this is the best strategy to replicate:
1. **Highest net PnL** — $229K combined across 5 wallets
2. **Clear, automatable rules** — Fixed clip size, single market, simple signals
3. **High win rate** — 67-83% across all practitioners
4. **Defined risk** — Quick exit on losers (<5 min)
5. **Scalable** — Works with 10-unit clips, can adjust size
6. **Low complexity** — Single instrument, no cross-market logic needed

### Implementation Blueprint

```
STRATEGY: Momentum Scalper (Perps)
===================================

1. MARKET SELECTION
   - Focus on 1-2 illiquid perps markets (like ZEC on Bulk)
   - Markets with single dominant LP = predictable counterparty
   - Avoid markets where HFT bots are active (SOL, BTC on Bulk)

2. SIGNAL GENERATION
   - Detect momentum via price velocity: 3+ consecutive fills in same direction from LP
   - Volume spike: 2x normal fill rate from LP in last 60 seconds
   - Price momentum: price moved >0.5% in last 2 minutes

3. ENTRY
   - Fixed clip size (10 units or $5,000 notional)
   - 100% aggressive taker (market orders)
   - Scale in over 5-10 fills
   - Target: 750-1000 unit position ($400K-$550K at 40x)

4. EXIT
   - Stop loss: -1% from entry price (at 40x = -40% of margin)
   - Take profit: +2% to +5% from entry (at 40x = +80% to +200%)
   - Time stop: Close if no momentum after 15 minutes
   - Trailing stop: Lock in profits at 50% retracement from peak

5. RISK MANAGEMENT
   - Max 1 position at a time
   - Max 40x leverage
   - 5-minute cooldown after a loss
   - Daily loss limit: 10% of account
   - Position sizing: 25% of account as margin

6. EXECUTION
   - Monitor LP orderbook for fill patterns
   - Enter when LP is being consumed in one direction (momentum)
   - Exit when LP refreshes and momentum stalls
```

### For the new perps project, this means:
- Build a **single-instrument momentum scalper** first
- Start with **illiquid markets** where a single LP dominates
- Use **fixed clip sizes** for simplicity and consistency
- Focus on **fast signal detection** (LP consumption rate)
- Keep **fee-aware** — this strategy works because Bulk's fee structure rewards momentum over HFT

---

## Data Collection Details

- **API:** `POST https://exchange-api.bulk.trade/api/v1/account`
- **Endpoints:** `{type: "fills"}`, `{type: "positions"}`, `{type: "fullAccount"}`
- **Timestamps:** Nanoseconds from Unix epoch
- **Fill fields:** maker, taker, orderIdMaker, orderIdTaker, isBuy, symbol, amount, price, makerFee, takerFee, fee, reasonCode, iso, counterpartyHint, slot, timestamp
- **Position fields:** owner, symbol, quantity, maxQuantity, totalVolume, avgOpenPrice, avgClosePrice, realizedPnl, fees, funding, openTime, closeTime, closeReason
