# Imperial Integration Gate — Read-Only API Client

**Status:** Complete (Milestones M1–M4)
**Date:** 2026-05-31
**Module:** `src/imperial.rs`, `src/route_cost.rs`

## Overview

The Imperial Integration Gate defines the boundary between Zekt's trading infrastructure and Imperial's Solana perps aggregator API. All integration is **read-only** — no trading, no JWT, no POST/PUT/DELETE requests.

## Architecture

```
ImperialClient (src/imperial.rs)
    ↓ HTTP GET only
    ↓ https://api.imperial.space
    ↓ No auth headers
RouteCostOracle (src/route_cost.rs)
    ↓ Uses ImperialClient.get_route()
    ↓ Compares venue costs
    ↓ Falls back to Flash-only on failure
LiquidationZoneCapture (src/liquidation.rs)
    ↓ Uses ImperialClient for OI stats + depth
    ↓ Fuses with Hyperliquid data
ReplayPipeline (src/replay.rs)
    ↓ Replays captured data through strategy
    ↓ Promotion gate validates results
```

## Endpoints Used (All GET, No Auth)

| Endpoint | Purpose | Response Type |
|----------|---------|---------------|
| `GET /api/v1/route` | Venue routing + cost breakdown | `ImperialRouteResponse` |
| `GET /api/v1/funding-rates` | Cross-venue funding/borrow rates | `Vec<ImperialFundingRateRow>` |
| `GET /api/v1/mark-prices` | Cross-venue mark prices | `Vec<ImperialMarkPriceRow>` |
| `GET /api/v1/phoenix/depth` | Phoenix order book depth | `ImperialPhoenixDepth` |
| `GET /api/v1/phoenix/markets` | Phoenix market configs | `Vec<ImperialPhoenixMarket>` |
| `GET /api/v1/flash/markets` | Flash Trade market configs | `Vec<ImperialFlashMarket>` |
| `GET /api/v1/gmtrade/markets` | GMTrade market configs | `Vec<ImperialGmtradeMarket>` |
| `GET /api/v1/gmtrade/liquidity` | GMTrade available liquidity | `Vec<ImperialGmtradeLiquidity>` |
| `GET /api/v1/priority-fee` | Solana priority fee | `ImperialPriorityFee` |
| `GET /api/v1/stats/markets` | Volume, OI, trader count | `ImperialStatsMarkets` |

## Forbidden Endpoints (NEVER Called)

| Endpoint | Reason |
|----------|--------|
| `/mobile/connect` | JWT-gated, trading |
| `/mobile/exchange` | JWT-gated, trading |
| `/mobile/orders` | JWT-gated, trading |
| `/deposit/build-tx` | Deposit flow, not needed |
| Any POST/PUT/DELETE | Read-only constraint |

## Route Cost Oracle Integration

### Cost Model

The Route Cost Oracle (`src/route_cost.rs`) queries Imperial's route endpoint for each candidate trade:

1. **Query:** `GET /api/v1/route?asset=SOL&side=long&notional=1000&leverage=5`
2. **Response:** Recommended venue + full cost breakdown (open fee, close fee, slippage, borrow, liq risk)
3. **Decision:** Compare Imperial cost vs Flash-only model
4. **Threshold:** If Imperial cost < Flash by ≥ `improvement_threshold_bps` → `route_improved`
5. **Veto:** If total cost > edge budget → `vetoed`
6. **Fallback:** If Imperial unavailable → Flash-only costs with degradation log

### Integration Points

| Component | Integration |
|-----------|-------------|
| `BacktestEngine` | `cost_mode = "imperial-route-oracle"` uses oracle for fee estimation |
| `LiquidationCascadeHunter` | Route cost veto in `detect_entry()` — rejects trades with excessive costs |
| `ReplayPipeline` | Replays with captured route costs, validates against baseline |

## Safety Constraints

1. **No live trading** — `--keypair` never used
2. **No Imperial JWT** — no auth headers on any request
3. **Read-only** — all Imperial API calls are GET requests
4. **Graceful degradation** — if Imperial is unavailable, fall back to Flash-only costs
5. **Config-gated** — `[route-oracle] enabled = false` by default
6. **Paper-only strategy** — `liquidation-cascade-hunter` blocked from live engine

## Verification

- **Compile-time:** `grep -c ".post(" src/imperial.rs` → 0
- **Compile-time:** `grep -c "/mobile/" src/imperial.rs` → 0
- **Runtime:** No `Authorization` header in any request
- **Test:** `cargo test imperial` → all tests pass
- **Test:** `cargo test route_cost` → all tests pass
- **Test:** `cargo test replay` → all tests pass
