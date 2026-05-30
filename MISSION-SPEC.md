# Mission: Imperial Route Oracle + Liquidation-Zone Alpha Validation

## Objective

Move Zekt toward profitability by validating two high-impact upgrades:

1. Replace single-venue Flash assumptions with an Imperial read-only route/cost oracle across Solana perps venues.
2. Build a liquidation-zone intelligence layer that detects liquidation clusters, cascade risk, and post-liquidation trade setups.

The mission succeeds only if it produces evidence that one or both upgrades improve net expectancy after realistic costs. No live trading. No Imperial JWT trading integration without explicit human approval.

## Why This Mission

The previous Zekt mission proved:

- Existing strategies are still net-negative.
- Regime filtering reduced losses mainly by avoiding trades.
- Current costs and venue assumptions may be suppressing edge.
- Blueprint strategies and new alpha sources need validation.

Imperial is relevant because its API exposes route recommendations, venue cost breakdowns, funding/borrow rates, mark prices, priority fees, Phoenix depth, market stats, and positions/order surfaces. Its docs also claim cheaper swap fees and pro order types. These claims must be measured, not trusted.

The liquidation-hunter example is useful only as a pattern: liquidation event storage, volume thresholds, VWAP protection, pending-order de-duplication, paper mode, and protection-order checks. Do not use Aster or Aster execution.

## Non-Negotiables

- No live trading.
- No Imperial JWT generation, storage, or trading calls unless separately approved.
- No Aster integration.
- No strategy promotion without out-of-sample or paper evidence.
- No liquidation "knife catching" without confirmation filters.
- Route savings must be measured after fees, borrow/funding, priority fee, slippage, and liquidation-risk cost.
- Liquidation signals must be logged first, replayed second, paper-traded third.

## Workstream 1: Imperial Read-Only Client

Implement a read-only `ImperialClient` for public/non-signing endpoints:

- `/api/v1/route`
- `/api/v1/funding-rates`
- `/api/v1/mark-prices`
- `/api/v1/phoenix/depth`
- `/api/v1/phoenix/markets`
- `/api/v1/flash/markets`
- `/api/v1/gmtrade/markets`
- `/api/v1/gmtrade/liquidity`
- `/api/v1/priority-fee`
- `/api/v1/stats/markets`

Do not implement:

- `/mobile/connect`
- `/mobile/exchange`
- `/mobile/orders`
- `/deposit/build-tx`
- any JWT-gated trading endpoint

Acceptance:

- Typed Rust client compiles.
- Mock tests cover response parsing.
- Live smoke test is optional and read-only.
- All route responses record best venue and full cost breakdown.

## Workstream 2: Imperial Route Cost Model

Extend Zekt's backtest/paper cost model with an `imperial-route-oracle` mode.

For each candidate trade, estimate:

- best venue,
- taker/open fee,
- close fee,
- borrow/funding over expected hold,
- priority fee,
- liquidation-risk expected cost if exposed by route response,
- excluded/sticky venue behavior,
- max leverage constraints,
- market support.

Decision rule:

- If Imperial expected cost is lower than current Flash model by a configured bps threshold, mark trade as `route_improved`.
- If route cost exceeds expected edge budget, veto trade.
- If route source is stale/missing, fall back to existing Flash assumptions and log degradation.

Acceptance:

- Before/after backtest table: Flash-only cost model vs Imperial route model.
- Metrics: net PnL, total fees, fee bps, route-selected venue counts, veto count, Sharpe, drawdown.
- A route improvement must be proven on blueprint strategies, not only placeholder strategies.

## Workstream 3: Blueprint Strategy Revalidation

Run the existing blueprint strategies through the new cost model:

- `blueprint-scalper`
- `blueprint-mean-revert`
- `blueprint-cluster-002` through `blueprint-cluster-009`
- `blueprint-hft-market-maker` only if cost assumptions are realistic enough

Validation windows:

- minimum 90 days where data exists,
- walk-forward enabled,
- regime filter enabled and disabled comparison,
- multiple markets: BTC, SOL, ETH, and any high-scoring Flash/Imperial-supported markets.

Acceptance:

- Produce ranked strategy table.
- Promote none unless positive net expectancy after realistic costs.
- Identify whether Imperial routing turns any near-break-even strategy positive.

## Workstream 4: Liquidation-Zone Intelligence Capture

Build a native Zekt liquidation intelligence layer.

Data sources to evaluate:

- Hyperliquid `clearinghouseState` for known wallets to aggregate liquidation prices, side, notional, and distance to mark.
- Hyperliquid fills for liquidation-like bursts and forced-flow inference.
- Imperial market stats for open interest imbalance and market crowding.
- Imperial mark prices and Phoenix depth for fragility/orderbook confirmation.
- Public liquidation feeds only if available without privileged access.

Output structure:

```json
{
  "symbol": "SOL",
  "timestamp_ms": 1770000000000,
  "mark_price": 150.25,
  "zones": [
    {
      "price": 147.50,
      "side_at_risk": "long",
      "estimated_notional_usd": 1250000,
      "wallet_count": 42,
      "distance_bps": 183,
      "confidence": 0.71,
      "source_mix": ["hyperliquid_positions", "oi_imbalance"]
    }
  ]
}
```

Acceptance:

- Persist liquidation-zone snapshots.
- Track source freshness and confidence.
- No trading decisions yet unless replay/paper validation exists.

## Workstream 5: Liquidation Strategy Prototype

Create a paper-only strategy:

`liquidation-cascade-hunter`

Two setup types:

1. Cascade continuation:
   - price approaches high-confidence liquidation zone,
   - liquidation/forced-flow proxy spikes,
   - mark velocity confirms direction,
   - depth thins or imbalance confirms continuation,
   - route cost is below edge budget.

2. Exhaustion reversal:
   - liquidation burst occurs,
   - price reclaims VWAP or prior zone,
   - depth refills,
   - velocity decays,
   - spread normalizes.

Required gates:

- liquidation-zone confidence minimum,
- volume/notional z-score,
- price distance to zone,
- VWAP filter,
- spread/depth filter,
- regime compatibility,
- route-cost veto,
- max one pending trade per symbol/side,
- mandatory stop and take profit,
- time stop.

Acceptance:

- Strategy compiles behind feature/config flag.
- Paper-only by default.
- Unit tests for zone scoring, entry gates, duplicate-order blocking, stale data blocking, and exit logic.

## Workstream 6: Replay and Paper Validation

Because historical liquidation data may be incomplete, validation has two stages:

1. Capture phase:
   - run read-only capture for at least 24-72 hours,
   - collect liquidation zones, route costs, mark prices, depth, OI stats.

2. Replay phase:
   - replay captured data through `liquidation-cascade-hunter`,
   - compare against no-trade baseline and existing strategies.

Promotion threshold for paper trading:

- positive simulated net expectancy after route costs,
- max drawdown within policy,
- no stale-data trades,
- no duplicate pending trades,
- at least 30 signal events or explicitly mark insufficient sample.

## Final Deliverables

- `docs/imperial-integration-gate.md`
- `docs/liquidation-zone-methodology.md`
- `data/imperial-route-comparison.md`
- `data/liquidation-zone-capture-summary.md`
- updated `MISSION_REPORT.md`
- typed read-only Imperial client
- liquidation-zone data model
- paper-only liquidation strategy prototype if data quality supports it

## Executive Decision Required At End

Choose one:

- Keep Flash-only, reject Imperial for now.
- Use Imperial as read-only route/cost oracle only.
- Approve next mission for Imperial paper-order integration.
- Continue liquidation data capture.
- Promote liquidation strategy to extended paper trading.
- Reject liquidation strategy due to no measurable edge.

