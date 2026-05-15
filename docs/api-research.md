# Leaderboard & Trader Data API Research

**Date**: 2026-05-16
**Purpose**: Assess feasibility of scraping trader leaderboard data from perp DEX platforms for the Zekt bot's copy-trading / signal-sourcing pipeline.

---

## 1. Flash Trade (fstats.io) — ⭐ HIGH FEASIBILITY

### Source URLs Tested
- `https://fstats.io/leaderboards` — Public Next.js analytics dashboard with Leaderboards tab (Volume, PnL, Referrers)
- `https://fstats.io/api/v1/leaderboards/volume?days=30` — ✅ **WORKS**
- `https://fstats.io/api/v1/leaderboards/pnl?days=30` — ✅ **WORKS**
- `https://fstats.io/api/v1/leaderboards/referrers?days=30` — ✅ **WORKS**
- `https://flashapi.trade/api/leaderboard` — 404
- `https://flashapi.trade/api/traders` — 404

### Data Available
**Volume Leaderboard** (`/api/v1/leaderboards/volume?days=N`):
```json
{
  "leaderboard": [
    {
      "owner": "EZpxuxaTrZZxxbizKd5woydoYZMSfKpagJrRBUiNuw8Y",  // Solana wallet address
      "num_trades": 118,
      "total_volume_usd": 26187699.54,
      "avg_trade_size": 221929.66,
      "largest_trade": 999967.11,
      "total_pnl": -10003.53,
      "wins": 18,
      "losses": 27,
      "rank": 1,
      "win_rate": 40.0
    }
  ],
  "metadata": {
    "period_days": 30,
    "limit": 50,
    "min_volume": 0.0
  }
}
```

**PnL Leaderboard** (`/api/v1/leaderboards/pnl?days=N`):
```json
{
  "leaderboard": [
    {
      "owner": "BxkieMNZXqJf1GnAHjn7VjFHGLR3fKxuJG1MWEUuVrnB",
      "num_trades": 4,
      "total_volume_usd": 317893.74,
      "gross_pnl": 21762.39,
      "entry_fees": 558.51,
      "net_pnl": 21203.88,
      "wins": 2,
      "losses": 2,
      "rank": 1,
      "win_rate": 50.0
    }
  ]
}
```

**Referrers Leaderboard** (`/api/v1/leaderboards/referrers?days=N`):
```json
{
  "leaderboard": [
    {
      "referrer": "5YXUSniRCHAMGUFzKgdDH4BpsV1frMqnaMBPzCyaAyNK",
      "referred_trades": 109,
      "referred_volume_usd": 12752571.37,
      "total_rebates_usd": 487.93,
      "unique_traders": 3,
      "rank": 1
    }
  ]
}
```

### Rate Limits
- No auth required
- Tested 5 rapid sequential requests (~370ms each) — all returned HTTP 200
- No rate limiting observed at low volume
- Response time: ~370-400ms per request

### Valid `days` Parameters
- Supports: `1`, `7`, `30`, `90` (maps to the UI period selector)
- Returns top 50 traders per period

### Feasibility: **HIGH**
- Public API, no auth, rich data with wallet addresses, PnL, win rate, trade count
- Data is aggregated (no per-trade breakdown)
- Best source for Flash Trade trader discovery

---

## 2. Jupiter Perps — ⭐ HIGH FEASIBILITY

### Source URLs Tested
- `https://jup.ag/perps-leaderboard` — Public page (200), client-rendered Next.js
- `https://perps-api.jup.ag/v1/top-traders?marketMint=...&year=...&week=current` — ✅ **WORKS**
- `https://perps-api.jup.ag/v1/trader-stats?walletAddress=...&marketMint=...&year=...&week=current` — ✅ **WORKS** (returns individual trader stats)
- `https://developers.jup.ag/docs/perps` — Documentation exists

### Data Available
**Top Traders** (`/v1/top-traders?marketMint=So11111111111111111111111111111111111111112&year=2026&week=current`):
```json
{
  "endTimestamp": 1779062399,
  "marketMint": "So11111111111111111111111111111111111111112",
  "startTimestamp": 1778457600,
  "topTradersByPnl": [
    {
      "owner": "dBm5g3BF5aRfFo1SwAqExkf3Tv1hmtNp8rtf6ojW62d",
      "totalPnlUsd": "58287977126",
      "totalVolumeUsd": "24315036149242"
    }
  ],
  "topTradersByVolume": [
    {
      "owner": "97u89o2Kafqh6StNvrHBUJNvuATAQJZP7FeQCKfuUEy3",
      "totalPnlUsd": "7753264210",
      "totalVolumeUsd": "35656518293405"
    }
  ],
  "totalVolumeUsd": "701313466320882"
}
```

### Key Parameters
- `marketMint`: Token mint address (e.g., `So11111111111111111111111111111111111111112` for SOL)
- `year`: Calendar year (e.g., `2026`)
- `week`: Week number or `"current"`
- Returns both PnL-ranked and Volume-ranked traders (50 each)
- Response includes PnL values in raw integer (divide by appropriate decimal)

### Rate Limits
- No auth required
- No rate limiting observed
- Response time: fast

### Feasibility: **HIGH**
- Public API with wallet addresses and PnL/volume data
- Per-market, per-week granularity
- Individual trader stats also available via `/v1/trader-stats`
- Note: PnL values appear to be in raw integer format (not human-readable USD) — needs decimal adjustment

---

## 3. Hyperliquid — MEDIUM FEASIBILITY

### Source URLs Tested
- `POST https://api.hyperliquid.xyz/info` with `{"type": "leaderboard"}` — ❌ 422 "Failed to deserialize the JSON body into the target type"
- `POST https://api.hyperliquid.xyz/info` with `{"type": "allMids"}` — ✅ Returns all market mid prices
- `POST https://api.hyperliquid.xyz/info` with `{"type": "userFills", "user": "0x..."}` — ✅ Returns trade fills for any wallet
- `POST https://api.hyperliquid.xyz/info` with `{"type": "clearinghouseState", "user": "0x..."}` — ✅ Returns positions + margin state
- `POST https://api.hyperliquid.xyz/info` with `{"type": "vaultSummaries"}` — ✅ Returns vault data
- `POST https://api.hyperliquid.xyz/info` with `{"type": "userFillsByTime", "user": "0x...", "startTime": ...}` — ✅ Time-filtered fills
- `https://app.hyperliquid.xyz/leaderboard` — Client-side SPA page exists
- Third-party: Nansen API has a Hyperliquid leaderboard endpoint (paid)

### Data Available
**userFills** (for any wallet address):
```json
[
  {
    "coin": "BTC",
    "px": "104402.0",
    "sz": "0.00101",
    "side": "B",
    "time": 1778884174034,
    "startPosition": "0.0",
    "dir": "Open Long",
    "closedPnl": "0.0",
    "hash": "0x...",
    "oid": 123456,
    "crossed": false,
    "fee": "0.02644",
    "tid": 987654,
    "feeToken": "USDC",
    "twapId": null
  }
]
```

**clearinghouseState** (for any wallet):
```json
{
  "marginSummary": {
    "accountValue": "557.13644",
    "totalNtlPos": "0.0",
    "totalRawUsd": "557.13644",
    "totalMarginUsed": "0.0"
  },
  "assetPositions": [],
  "time": 1778884312507
}
```

### No Direct Leaderboard API
- The Hyperliquid `POST /info` API does **not** expose a `leaderboard` type
- The app.hyperliquid.xyz leaderboard page is a client-side SPA — data is loaded via the API but the exact leaderboard endpoint was not reverse-engineered
- Third-party scrapers (Apify, Nansen) exist for Hyperliquid leaderboard data but are paid services

### Feasibility: **MEDIUM**
- No public leaderboard endpoint — cannot get top trader rankings directly
- **CAN** query any wallet's trade history (`userFills`) and positions (`clearinghouseState`) if you already have their address
- Third-party options (Nansen, Apify scraper) could provide leaderboard data at cost
- EVM addresses (0x...) format, not Solana

---

## 4. GMX (gmx.house) — LOW FEASIBILITY

### Source URLs Tested
- `https://gmx.house/arbitrum/leaderboard` — Returns client-rendered SPA (200), no SSR data
- Analyzed JS bundle (`index-9b7891cd.js`) for API endpoints

### Data Available
- The frontend reads directly from Arbitrum RPC + The Graph subgraph
- No dedicated public API for leaderboard data
- JS references `https://api.thegraph.com/subgraphs/name/nissoh/blueberry-club-arbitrum` (NFT-related, not GMX leaderboard)
- No separate backend API discovered — all data comes from on-chain RPC calls

### Feasibility: **LOW**
- No public leaderboard API
- Data requires querying Arbitrum RPC directly (expensive, slow)
- Would need to read GMX contract events from chain — not practical for real-time leaderboard
- GMX is not on Solana anyway (EVM-only)

---

## 5. Birdeye — BLOCKED

### Source URLs Tested
- `https://birdeye.so/trader-board` — HTTP 403 (Cloudflare challenge)
- `https://birdeye.so/leaderboard` — HTTP 403 (Cloudflare challenge)

### Feasibility: **BLOCKED**
- Cloudflare bot protection prevents programmatic access
- Birdeye has a documented API (api.birdeye.so) but it requires an API key (paid tiers)
- Not feasible without browser automation or paid API access

---

## 6. Solana RPC (getSignaturesForAddress) — MEDIUM FEASIBILITY

### Approach
Use Solana RPC `getSignaturesForAddress` to fetch all transaction signatures for a given wallet, then parse each transaction to reconstruct trade history.

### Testing
- `getSignaturesForAddress` requires a valid base58 Solana pubkey (program or account)
- Flash Trade program IDs are valid Solana addresses
- The approach works but is **extremely** expensive:
  - Each signature requires a separate `getTransaction` call to decode
  - Flash Trade uses custom instruction data — would need to reverse-engineer instruction layouts
  - Rate limited on public RPC (~100 req/10s on `api.mainnet-beta.solana.com`)

### Feasibility: **MEDIUM** (for targeted wallets only)
- Works for reconstructing any individual wallet's trade history on any Solana perps DEX
- Impractical for bulk leaderboard scanning (too many RPC calls)
- Better to use DEX-specific APIs (fstats.io, Jupiter) when available
- Useful as a fallback for trade verification or when no API exists

---

## Summary Ranking

| Source | Feasibility | Data Quality | Auth Required | Wallet Addresses | Key Limitation |
|--------|-------------|-------------|---------------|------------------|----------------|
| **Flash Trade (fstats.io)** | ⭐ HIGH | Excellent | No | ✅ Solana | Aggregated only, no per-trade |
| **Jupiter Perps** | ⭐ HIGH | Good | No | ✅ Solana | Per-week, per-market; raw integer PnL |
| **Hyperliquid** | MEDIUM | Excellent per-user | No | ✅ EVM (0x...) | No leaderboard endpoint; per-user only |
| **GMX** | LOW | Unknown | N/A | ✅ EVM | No API; on-chain only; not Solana |
| **Birdeye** | BLOCKED | Unknown | API Key | ✅ Solana | Cloudflare + paid API |
| **Solana RPC** | MEDIUM | Full detail | No | ✅ Any | Extremely expensive; no aggregation |

## Recommended Strategy

1. **Primary**: Use **fstats.io API** for Flash Trade trader discovery — wallet addresses, PnL, win rate, trade count, all without auth
2. **Secondary**: Use **Jupiter Perps API** for cross-referencing top traders on Jupiter's perps markets
3. **Deep dive**: Use **Hyperliquid userFills** to get per-trade data for any wallet you already know about
4. **Fallback**: Solana RPC `getSignaturesForAddress` for wallets where no API data exists
5. **Skip**: GMX (not Solana), Birdeye (blocked/paid)

### Integration Notes for Zekt
- fstats.io returns 50 traders per request — poll daily to build a ranked trader database
- Filter for traders with `win_rate > 60%` and `net_pnl > 0` across multiple time periods
- Cross-reference with Jupiter leaderboard to find traders active on multiple venues
- Use Hyperliquid `userFills` to validate trader skill with actual trade-level data
