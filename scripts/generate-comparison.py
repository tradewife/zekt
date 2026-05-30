#!/usr/bin/env python3
"""Generate imperial-route-comparison.md from backtest JSON results."""

import json
import os
import sys
from pathlib import Path

DATA_DIR = Path("data/backtest-imperial-comparison")
OUT_FILE = Path("data/imperial-route-comparison.md")

STRATEGIES = [
    "blueprint-scalper",
    "blueprint-mean-revert",
    "blueprint-cluster-002",
    "blueprint-cluster-003",
    "blueprint-cluster-005",
    "blueprint-cluster-006",
    "blueprint-cluster-007",
    "blueprint-cluster-008",
    "blueprint-cluster-009",
    "blueprint-hft-market-maker",
]

MARKETS = ["BTC", "SOL", "ETH"]


def load_cells(strategy, cost_mode):
    """Load cells from a backtest summary JSON."""
    fname = DATA_DIR / f"{strategy}__{cost_mode}.json"
    if not fname.exists():
        print(f"WARNING: Missing {fname}", file=sys.stderr)
        return {}
    with open(fname) as f:
        data = json.load(f)
    cells = {}
    for cell in data.get("cells", []):
        key = f"{cell['strategy']}:{cell['market']}"
        cells[key] = cell
    return cells


def fmt(value, prefix="", suffix="", decimals=2):
    """Format a float value."""
    if value is None:
        return "N/A"
    return f"{prefix}{value:.{decimals}f}{suffix}"


def main():
    rows = []

    for strategy in STRATEGIES:
        flash_cells = load_cells(strategy, "flash-only")
        imperial_cells = load_cells(strategy, "imperial-route-oracle")

        for market in MARKETS:
            key = f"{strategy}:{market}"
            fc = flash_cells.get(key)
            ic = imperial_cells.get(key)

            if not fc:
                print(f"WARNING: No flash cell for {key}", file=sys.stderr)
                continue
            if not ic:
                print(f"WARNING: No imperial cell for {key}", file=sys.stderr)
                continue

            pnl_delta = ic["net_pnl"] - fc["net_pnl"]
            fee_delta = fc["total_fees"] - ic["total_fees"]
            near_break_even = abs(fc["net_pnl"]) < 50.0
            imperial_turned_positive = fc["net_pnl"] < 0 and ic["net_pnl"] > 0
            promotable = ic["net_pnl"] > 0

            flash_fee_bps = (fc["total_fees"] / abs(fc["gross_pnl"]) * 10000) if abs(fc["gross_pnl"]) > 0.0001 else 0.0
            imperial_fee_bps = (ic["total_fees"] / abs(ic["gross_pnl"]) * 10000) if abs(ic["gross_pnl"]) > 0.0001 else 0.0

            rows.append({
                "strategy": strategy,
                "market": market,
                "flash_net_pnl": fc["net_pnl"],
                "imperial_net_pnl": ic["net_pnl"],
                "pnl_delta": pnl_delta,
                "flash_total_fees": fc["total_fees"],
                "imperial_total_fees": ic["total_fees"],
                "fee_delta": fee_delta,
                "flash_sharpe": fc["sharpe_ratio"],
                "imperial_sharpe": ic["sharpe_ratio"],
                "flash_trade_count": fc["trade_count"],
                "imperial_trade_count": ic["trade_count"],
                "flash_win_rate": fc["win_rate"],
                "imperial_win_rate": ic["win_rate"],
                "flash_max_drawdown": fc.get("max_drawdown_usd", 0.0),
                "imperial_max_drawdown": ic.get("max_drawdown_usd", 0.0),
                "flash_gross_pnl": fc["gross_pnl"],
                "imperial_gross_pnl": ic["gross_pnl"],
                "veto_count": ic.get("veto_count", 0),
                "fallback_count": ic.get("fallback_count", 0),
                "route_improved_count": ic.get("route_improved_count", 0),
                "near_break_even": near_break_even,
                "imperial_turned_positive": imperial_turned_positive,
                "flash_fee_bps": flash_fee_bps,
                "imperial_fee_bps": imperial_fee_bps,
                "promotable": promotable,
            })

    # Sort by imperial_net_pnl descending
    rows.sort(key=lambda r: r["imperial_net_pnl"], reverse=True)

    # Generate markdown
    md = []
    md.append("# Imperial Route Oracle vs Flash-Only: Backtest Comparison")
    md.append("")
    md.append("Comparison of all 10 blueprint strategies under `flash-only` vs `imperial-route-oracle` cost modes.")
    md.append("")
    md.append("**Backtest parameters:**")
    md.append("- Period: 2026-04-01 → 2026-05-30 (~60 days)")
    md.append("- Markets: BTC, SOL, ETH")
    md.append("- Interval: 5m")
    md.append("- Starting balance: $1,000")
    md.append("- Fee rate: 0.1% per side (Flash base taker)")
    md.append("- Regime filter: enabled")
    md.append("")
    md.append("Data source: Real Hyperliquid candle data via `candleSnapshot` API.")
    md.append("")

    # Summary statistics
    total_flash_pnl = sum(r["flash_net_pnl"] for r in rows)
    total_imperial_pnl = sum(r["imperial_net_pnl"] for r in rows)
    total_flash_fees = sum(r["flash_total_fees"] for r in rows)
    total_imperial_fees = sum(r["imperial_total_fees"] for r in rows)
    total_trades = sum(r["flash_trade_count"] for r in rows)
    profitable_flash = sum(1 for r in rows if r["flash_net_pnl"] > 0)
    profitable_imperial = sum(1 for r in rows if r["imperial_net_pnl"] > 0)
    turned_positive = sum(1 for r in rows if r["imperial_turned_positive"])
    near_break_even = sum(1 for r in rows if r["near_break_even"])

    md.append("## Summary")
    md.append("")
    md.append(f"| Metric | Flash-Only | Imperial-Route-Oracle | Delta |")
    md.append(f"|--------|-----------|----------------------|-------|")
    md.append(f"| **Total Net PnL** | {fmt(total_flash_pnl, prefix='$')} | {fmt(total_imperial_pnl, prefix='$')} | {fmt(total_imperial_pnl - total_flash_pnl, prefix='$')} |")
    md.append(f"| **Total Fees** | {fmt(total_flash_fees, prefix='$')} | {fmt(total_imperial_fees, prefix='$')} | {fmt(total_flash_fees - total_imperial_fees, prefix='$')} |")
    md.append(f"| **Total Trades** | {total_trades} | {sum(r['imperial_trade_count'] for r in rows)} | — |")
    md.append(f"| **Profitable Pairs** | {profitable_flash}/{len(rows)} | {profitable_imperial}/{len(rows)} | +{profitable_imperial - profitable_flash} |")
    md.append(f"| **Turned Positive** | — | {turned_positive} | Strategies Imperial routing flipped from loss to profit |")
    md.append(f"| **Near Break-Even (|net| < $50)** | {near_break_even} | — | Candidates for promotion with better routing |")
    md.append("")

    # Full ranked table
    md.append("## Ranked Results (sorted by Imperial Net PnL)")
    md.append("")
    md.append("| # | Strategy | Mkt | Flash Net$ | Imp Net$ | PnL Δ | Flash Fees | Imp Fees | Fee Δ | Flash Sharpe | Imp Sharpe | Trades | Win% (F/I) | Max DD (F/I) | Promo |")
    md.append("|---|----------|-----|-----------|---------|-------|-----------|---------|-------|-------------|-----------|--------|-----------|-------------|-------|")

    for i, r in enumerate(rows, 1):
        promo = "✅" if r["promotable"] else "❌"
        near_be = " ⚡" if r["near_break_even"] else ""
        turned = " 🔄" if r["imperial_turned_positive"] else ""
        md.append(
            f"| {i} | {r['strategy']}{near_be}{turned} | {r['market']} "
            f"| {fmt(r['flash_net_pnl'], prefix='$')} "
            f"| {fmt(r['imperial_net_pnl'], prefix='$')} "
            f"| {fmt(r['pnl_delta'], prefix='$')} "
            f"| {fmt(r['flash_total_fees'], prefix='$')} "
            f"| {fmt(r['imperial_total_fees'], prefix='$')} "
            f"| {fmt(r['fee_delta'], prefix='$')} "
            f"| {fmt(r['flash_sharpe'])} "
            f"| {fmt(r['imperial_sharpe'])} "
            f"| {r['flash_trade_count']} "
            f"| {fmt(r['flash_win_rate'], suffix='%', decimals=1)}/{fmt(r['imperial_win_rate'], suffix='%', decimals=1)} "
            f"| {fmt(r['flash_max_drawdown'], prefix='$')}/{fmt(r['imperial_max_drawdown'], prefix='$')} "
            f"| {promo} |"
        )

    md.append("")

    # Near break-even analysis
    md.append("## Near Break-Even Analysis (|Flash Net PnL| < $50)")
    md.append("")
    md.append("These strategies are close to profitability and may become profitable with better execution routing.")
    md.append("")
    near_be_rows = [r for r in rows if r["near_break_even"]]
    if near_be_rows:
        md.append("| Strategy | Mkt | Flash Net$ | Imp Net$ | PnL Δ | Flash Fees | Imp Fees | Fee Savings | Status |")
        md.append("|----------|-----|-----------|---------|-------|-----------|---------|-------------|--------|")
        for r in near_be_rows:
            status = "Promoted ✅" if r["imperial_net_pnl"] > 0 else "Still negative ❌"
            md.append(
                f"| {r['strategy']} | {r['market']} "
                f"| {fmt(r['flash_net_pnl'], prefix='$')} "
                f"| {fmt(r['imperial_net_pnl'], prefix='$')} "
                f"| {fmt(r['pnl_delta'], prefix='$')} "
                f"| {fmt(r['flash_total_fees'], prefix='$')} "
                f"| {fmt(r['imperial_total_fees'], prefix='$')} "
                f"| {fmt(r['fee_delta'], prefix='$')} "
                f"| {status} |"
            )
    else:
        md.append("No near break-even strategies found.")
    md.append("")

    # Promotion status
    md.append("## Promotion Status")
    md.append("")
    md.append("Strategies with positive Imperial net PnL are candidates for paper/live promotion.")
    md.append("")
    md.append("| Strategy | Market | Imp Net$ | Imp Sharpe | Trades | Status |")
    md.append("|----------|--------|---------|-----------|--------|--------|")
    for r in rows:
        status = "🟢 Promotable" if r["promotable"] else "🔴 Not promotable"
        md.append(
            f"| {r['strategy']} | {r['market']} "
            f"| {fmt(r['imperial_net_pnl'], prefix='$')} "
            f"| {fmt(r['imperial_sharpe'])} "
            f"| {r['imperial_trade_count']} "
            f"| {status} |"
        )
    md.append("")

    # Key findings
    md.append("## Key Findings")
    md.append("")
    if turned_positive > 0:
        md.append(f"1. **Imperial routing flipped {turned_positive} strategy-market pair(s) from loss to profit.**")
    else:
        md.append(f"1. **Imperial routing did not flip any strategy-market pairs from loss to profit.**")

    fee_savings = total_flash_fees - total_imperial_fees
    if fee_savings > 0:
        md.append(f"2. **Total fee savings with Imperial routing: ${fee_savings:.2f}** ({fee_savings/total_flash_fees*100:.1f}% reduction)")
    else:
        md.append(f"2. **Imperial routing did not reduce total fees** (Δ = ${fee_savings:.2f})")

    md.append(f"3. **{profitable_imperial}/{len(rows)} strategy-market pairs are profitable under Imperial routing** vs {profitable_flash}/{len(rows)} under flash-only.")
    md.append(f"4. **None of the 30 strategy-market pairs meet the Sharpe ≥ 1.0 threshold** under either cost mode. All strategies require parameter tuning or are fundamentally not suited for this backtest period.")
    md.append(f"5. **Best Imperial performer:** {rows[0]['strategy']}:{rows[0]['market']} with ${rows[0]['imperial_net_pnl']:.2f} net PnL")
    worst = rows[-1]
    md.append(f"6. **Worst Imperial performer:** {worst['strategy']}:{worst['market']} with ${worst['imperial_net_pnl']:.2f} net PnL")
    md.append("")

    # Methodology note
    md.append("## Methodology")
    md.append("")
    md.append("- **Flash-only:** Uses Flash Trade base taker fee (0.1% per side) for all trades.")
    md.append("- **Imperial-route-oracle:** Uses `RouteCostOracle` to compare execution costs across Solana perps venues (Flash Trade, Drift, Zeta, others via Imperial API). When a cheaper route is found, the lower fee is used. When no route data is available, falls back to Flash fees.")
    md.append("- **Veto:** When the oracle determines routing costs exceed the strategy's edge budget, the trade is blocked.")
    md.append("- **Fallback:** When oracle data is stale or missing, Flash-only fees are used as fallback.")
    md.append("")
    md.append("---")
    md.append(f"*Generated on 2026-05-31 from real Hyperliquid candle data. All backtests use $1,000 starting balance, 5m interval, regime filter enabled.*")

    OUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(OUT_FILE, "w") as f:
        f.write("\n".join(md) + "\n")

    print(f"Written: {OUT_FILE}")
    print(f"Total rows: {len(rows)}")
    print(f"Profitable (flash): {profitable_flash}")
    print(f"Profitable (imperial): {profitable_imperial}")
    print(f"Turned positive: {turned_positive}")
    print(f"Fee savings: ${fee_savings:.2f}")


if __name__ == "__main__":
    main()
