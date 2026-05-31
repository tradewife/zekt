# Mission: Walk-Forward Edge Hardening + Leverage-Aware Position Sizing

## Objective

Convert Zekt from "cost-improved but still net-negative" into a strategy selection and sizing system with at least one genuinely promotable strategy-market pair.

The mission must use real Hyperliquid candle data, Imperial route-oracle cost mode, walk-forward validation, and risk-of-ruin checks. The goal is not to find the highest backtest PnL. The goal is to find stable, repeatable edge that can survive paper trading.

## Starting Evidence

Previous mission results:

- Period: 2026-04-01 to 2026-05-30
- Markets: BTC, SOL, ETH
- Strategies: 10 blueprint strategies
- Cost modes: Flash-only vs Imperial-route-oracle
- Imperial reduced total fees from $651.67 to $154.24.
- Imperial improved total net PnL from -$342.24 to -$120.35.
- Profitable pairs improved from 5/30 to 10/30.
- No pair reached robust Sharpe quality.
- Best meaningful pair: `blueprint-cluster-007:BTC`, +$4.24 net, Sharpe 0.39, 13 trades.

Conclusion: routing helped, but edge is still too weak. The next bottleneck is parameter quality, leverage/sizing, and sample robustness.

## Non-Negotiables

- No live trading.
- No Imperial JWT or order placement.
- No strategy promotion from fewer than 30 trades unless explicitly labeled "insufficient sample."
- No leverage increase unless risk-of-ruin, max drawdown, and liquidation distance are measured.
- No optimization on a single full-period result.
- No strategy is promotable unless out-of-sample metrics pass.
- All results must use `imperial-route-oracle` and also compare against `flash-only`.

## Workstream 1: Backtest Harness Upgrade

Add or verify CLI/config support for:

- `--cost-mode flash-only|imperial-route-oracle`
- walk-forward enabled from config or CLI
- slippage bps from config or CLI
- leverage override per run
- strategy parameter override grid
- output path per run to avoid overwriting previous summaries

Acceptance:

- A single command can run one strategy/market/parameter/leverage combination.
- A batch runner can execute a grid and write structured JSON/CSV/Markdown summaries.
- Existing tests still pass.

## Workstream 2: Candidate Selection

Focus only on pairs that showed promise under Imperial routing:

Primary candidates:

- `blueprint-cluster-007:BTC`
- `blueprint-cluster-005:ETH`
- `blueprint-cluster-005:SOL`
- `blueprint-cluster-008:BTC`
- `blueprint-cluster-002:BTC`
- `blueprint-cluster-002:SOL`
- `blueprint-cluster-003:BTC`
- `blueprint-cluster-009:ETH`
- `blueprint-cluster-009:SOL`

Exclude or deprioritize:

- `blueprint-hft-market-maker`
- consistently negative mean-revert variants
- zero-trade pairs
- pairs with fewer than 10 trades unless being tested for signal scarcity

Acceptance:

- Produce `data/candidate-strategy-set.md`.
- Explain why each candidate is included or excluded.

## Workstream 3: Parameter Search

For each candidate, sweep:

- entry threshold
- lookback window
- take profit
- stop loss
- max hold time
- trailing stop enable/disable
- regime filter on/off
- minimum route improvement bps
- edge budget threshold

Use conservative grids first. Do not explode the search space.

Validation:

- Use walk-forward train/test split.
- Rank by out-of-sample expectancy, Sharpe, Sortino, Calmar, max drawdown, and trade count.
- Penalize parameter sets that only work in one market or one short window.
- Record parameter stability, not just best PnL.

Acceptance:

- Produce `data/walk-forward-parameter-search.md`.
- Identify top 3 parameter sets per strategy-market.
- Mark all overfit candidates clearly.

## Workstream 4: Leverage and Position Sizing Search

For only the top parameter candidates, test leverage and sizing:

Leverage grid:

- 1x
- 2x
- 3x
- 4x
- 5x
- 7.5x
- 10x

Sizing grid:

- fixed notional
- fixed fractional equity risk
- volatility-adjusted sizing
- drawdown-throttled sizing
- route-cost-adjusted sizing

Metrics:

- net PnL
- Sharpe
- Sortino
- Calmar
- max drawdown
- liquidation proximity
- longest losing streak
- risk of ruin estimate
- recovery time after drawdown
- fee-to-gross-profit ratio

Acceptance:

- Produce `data/leverage-sizing-frontier.md`.
- Identify efficient frontier: best return per unit drawdown.
- Recommend max leverage per strategy-market.
- If higher leverage increases PnL but creates unacceptable drawdown, reject it.

## Workstream 5: Portfolio Construction

Do not evaluate strategies only in isolation. Build a portfolio view:

- combine top candidates,
- cap correlated BTC/SOL/ETH exposure,
- limit simultaneous positions,
- apply daily/weekly drawdown breakers,
- test capital allocation weights,
- test "only trade top-ranked active signal" mode.

Acceptance:

- Produce `data/portfolio-backtest.md`.
- Compare single-best strategy vs portfolio.
- Recommend one paper-trading basket, or reject all.

## Workstream 6: Liquidation Capture Background Run

Run liquidation-zone capture in parallel for 24-72 hours if feasible.

This is data gathering only.

Acceptance:

- Produce `data/liquidation-zone-capture-summary.md`.
- Report signal count, confidence distribution, source freshness, and whether enough data exists for the next liquidation-specific mission.

## Promotion Gate

A strategy-market or portfolio is promotable to paper trading only if it satisfies:

- out-of-sample net PnL positive,
- Sharpe >= 1.0, or clearly improving toward it with strong Sortino/Calmar,
- at least 30 trades in validation window,
- max drawdown acceptable for a $1,000 account,
- fee-to-gross-profit ratio below 35%,
- stable parameters across adjacent grid values,
- no single trade accounts for most of the profit,
- Imperial route mode improves or preserves results,
- risk-of-ruin acceptable under proposed leverage.

If no candidate passes, the correct outcome is "no paper promotion."

## Final Deliverables

- `data/candidate-strategy-set.md`
- `data/walk-forward-parameter-search.md`
- `data/leverage-sizing-frontier.md`
- `data/portfolio-backtest.md`
- `data/liquidation-zone-capture-summary.md`
- updated `MISSION_REPORT.md`
- recommended paper-trading config, or explicit rejection of all candidates

## Executive Decision Required

Choose one:

- Promote one optimized strategy-market pair to paper trading.
- Promote a small portfolio basket to paper trading.
- Continue parameter search with narrower candidates.
- Continue liquidation capture for a dedicated liquidation mission.
- Reject current blueprint suite as insufficient and return to wallet discovery.

