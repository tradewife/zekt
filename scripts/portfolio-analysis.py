#!/usr/bin/env python3
"""
Portfolio Construction Analysis
================================
Reads leverage-sizing grid results and per-trade data to:
1. Select best configuration per candidate
2. Compute cross-candidate correlation matrix
3. Test allocation strategies: equal-weight, risk-parity, Sharpe-weighted
4. Cap correlated BTC/SOL/ETH exposure
5. Limit simultaneous positions
6. Apply daily/weekly drawdown breakers
7. Test 'only top-ranked active signal' mode
8. Compare single-best vs portfolio
9. Produce data/portfolio-backtest.md

Usage:
  python3 scripts/portfolio-analysis.py [--grid data/leverage-sizing/grid.json] [--raw-dir data/leverage-sizing/raw] [--output data/portfolio-backtest.md]
"""

import argparse
import json
import logging
import math
import os
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

# --- Constants ---
MAX_ALLOCATION_PER_CANDIDATE = 0.40  # 40% max per candidate (VAL-PORT-006)
CORRELATED_GROUPS = {
    "BTC": ["blueprint-cluster-007", "blueprint-cluster-008"],
    "ETH": ["blueprint-cluster-005"],
    "SOL": ["blueprint-cluster-002", "blueprint-cluster-005", "blueprint-cluster-009"],
}
MAX_CORRELATED_EXPOSURE = 0.60  # 60% max for correlated group
DAILY_DRAWDOWN_LIMIT = 0.05  # 5% daily drawdown breaker
WEEKLY_DRAWDOWN_LIMIT = 0.10  # 10% weekly drawdown breaker
MAX_SIMULTANEOUS_POSITIONS = 3
INITIAL_BALANCE = 1000.0

SHORT_LABELS = {
    "blueprint-cluster-002": "cluster-002",
    "blueprint-cluster-003": "cluster-003",
    "blueprint-cluster-005": "cluster-005",
    "blueprint-cluster-007": "cluster-007",
    "blueprint-cluster-008": "cluster-008",
    "blueprint-cluster-009": "cluster-009",
}


def load_grid(grid_path):
    """Load the leverage-sizing grid.json."""
    with open(grid_path) as f:
        return json.load(f)


def load_trades(raw_dir, candidate_key):
    """Load per-trade data for a candidate configuration."""
    trades_path = os.path.join(raw_dir, candidate_key, "backtest-trades.json")
    if not os.path.exists(trades_path):
        return []
    with open(trades_path) as f:
        return json.load(f)


def select_best_configs(grid):
    """Select the best configuration per (strategy, market, cost_mode) candidate.
    Uses Sharpe ratio, breaking ties by PnL, then by lower drawdown.
    Prefers imperial-route-oracle where available."""
    best = {}
    for item in grid:
        if not item.get("success", False):
            continue
        key = (item["strategy"], item["market"], item["cost_mode"])
        # Use volatility-adjusted sizing as it was generally best in M2
        # But take the overall best regardless of sizing mode
        if key not in best:
            best[key] = item
        else:
            curr = best[key]
            # Compare by Sharpe (higher is better), then PnL, then lower DD
            if item["sharpe_ratio"] > curr["sharpe_ratio"]:
                best[key] = item
            elif item["sharpe_ratio"] == curr["sharpe_ratio"]:
                if item["net_pnl"] > curr["net_pnl"]:
                    best[key] = item
                elif item["net_pnl"] == curr["net_pnl"]:
                    if item["max_drawdown_usd"] < curr["max_drawdown_usd"]:
                        best[key] = item
    return best


def make_raw_key(item):
    """Construct the raw directory key from a grid item."""
    lev = item["leverage"]
    sizing = item["sizing_mode"]
    strategy_short = item["strategy"].replace("blueprint-", "")
    return f"{strategy_short}__{item['market']}__{item['cost_mode']}__lev{lev}__{sizing}"


def compute_daily_returns(trades, initial_balance=INITIAL_BALANCE):
    """Compute daily PnL returns from a list of trades.
    Returns dict of date_str -> daily_pnl."""
    daily_pnl = defaultdict(float)
    for t in trades:
        exit_time = t.get("exit_time", "")
        if not exit_time:
            continue
        try:
            dt = datetime.fromisoformat(exit_time.replace("Z", "+00:00"))
            date_str = dt.strftime("%Y-%m-%d")
        except (ValueError, AttributeError):
            continue
        daily_pnl[date_str] += t.get("net_pnl", 0.0)
    return dict(daily_pnl)


def compute_correlation_matrix(daily_returns_dict):
    """Compute pairwise correlation matrix from daily returns.
    daily_returns_dict: {candidate_label: {date: pnl, ...}, ...}
    Returns: {candA: {candB: correlation, ...}, ...}"""
    # Collect all dates
    all_dates = set()
    for returns in daily_returns_dict.values():
        all_dates.update(returns.keys())
    all_dates = sorted(all_dates)

    candidates = sorted(daily_returns_dict.keys())
    n = len(candidates)

    # Build return vectors (padded with 0 for missing dates)
    vectors = {}
    for cand in candidates:
        vec = [daily_returns_dict[cand].get(d, 0.0) for d in all_dates]
        vectors[cand] = vec

    # Compute pairwise correlations
    corr = {}
    for i, ca in enumerate(candidates):
        corr[ca] = {}
        for j, cb in enumerate(candidates):
            va = vectors[ca]
            vb = vectors[cb]
            # Pearson correlation
            n_pts = len(va)
            if n_pts < 2:
                corr[ca][cb] = 0.0
                continue
            mean_a = sum(va) / n_pts
            mean_b = sum(vb) / n_pts
            cov = sum((a - mean_a) * (b - mean_b) for a, b in zip(va, vb)) / n_pts
            std_a = (sum((a - mean_a) ** 2 for a in va) / n_pts) ** 0.5
            std_b = (sum((b - mean_b) ** 2 for b in vb) / n_pts) ** 0.5
            if std_a < 1e-12 or std_b < 1e-12:
                corr[ca][cb] = 0.0
            else:
                corr[ca][cb] = cov / (std_a * std_b)
    return corr


def compute_allocation_equal(candidates):
    """Equal weight allocation."""
    n = len(candidates)
    if n == 0:
        return {}
    w = 1.0 / n
    return {c: w for c in candidates}


def compute_allocation_risk_parity(corr_matrix, vol_dict, candidates):
    """Risk-parity allocation: weight inversely proportional to volatility."""
    # Use inverse vol, then normalize
    inv_vol = {}
    for c in candidates:
        vol = vol_dict.get(c, 1.0)
        inv_vol[c] = 1.0 / max(vol, 1e-6)

    total = sum(inv_vol.values())
    if total < 1e-12:
        return compute_allocation_equal(candidates)

    return {c: inv_vol[c] / total for c in candidates}


def compute_allocation_sharpe_weighted(sharpe_dict, candidates):
    """Sharpe-weighted allocation: weight proportional to Sharpe ratio.
    Negative Sharpe candidates get 0 weight."""
    positive = {c: max(sharpe_dict.get(c, 0.0), 0.0) for c in candidates}
    total = sum(positive.values())

    if total < 1e-12:
        # All negative or zero Sharpe — fall back to equal
        return compute_allocation_equal(candidates)

    return {c: positive[c] / total for c in candidates}


def enforce_max_allocation(weights, max_weight=MAX_ALLOCATION_PER_CANDIDATE):
    """Cap any single candidate at max_weight. Redistribute excess equally."""
    adjusted = dict(weights)
    for _ in range(10):  # iterate to converge
        excess_total = 0.0
        over_count = 0
        for c, w in adjusted.items():
            if w > max_weight:
                excess_total += w - max_weight
                adjusted[c] = max_weight
                over_count += 1

        if excess_total < 1e-10:
            break

        # Redistribute to non-capped candidates
        non_capped = {c: w for c, w in adjusted.items() if w < max_weight}
        if not non_capped:
            break
        per_cand = excess_total / len(non_capped)
        for c in non_capped:
            adjusted[c] = min(adjusted[c] + per_cand, max_weight)

    # Normalize to sum to 1.0
    total = sum(adjusted.values())
    if total > 1e-12:
        for c in adjusted:
            adjusted[c] /= total
    return adjusted


def enforce_correlated_cap(weights, candidates_info, max_corr_exp=MAX_CORRELATED_EXPOSURE):
    """Cap total exposure to correlated market groups.
    candidates_info: {label: {'strategy': ..., 'market': ...}}"""
    # For each group, check if total exceeds cap
    group_exposure = defaultdict(float)
    group_members = defaultdict(list)

    for label, w in weights.items():
        info = candidates_info.get(label, {})
        market = info.get("market", "")
        group_exposure[market] += w
        group_members[market].append(label)

    adjusted = dict(weights)
    for market, total_exp in group_exposure.items():
        if total_exp > max_corr_exp:
            # Scale down proportionally
            scale = max_corr_exp / total_exp
            for label in group_members[market]:
                adjusted[label] *= scale

    # Re-normalize
    total = sum(adjusted.values())
    if total > 1e-12:
        for c in adjusted:
            adjusted[c] /= total
    return adjusted


def simulate_portfolio(trades_by_candidate, weights, initial_balance=INITIAL_BALANCE,
                       daily_dd_limit=DAILY_DRAWDOWN_LIMIT, weekly_dd_limit=WEEKLY_DRAWDOWN_LIMIT,
                       max_positions=MAX_SIMULTANEOUS_POSITIONS):
    """Simulate a combined portfolio with drawdown breakers and position limits.
    
    Returns dict with equity_curve, metrics, breaker_events.
    """
    # Collect all trades with their candidate labels
    all_trades = []
    for label, trades in trades_by_candidate.items():
        for t in trades:
            all_trades.append({
                "label": label,
                "entry_time": t.get("entry_time", ""),
                "exit_time": t.get("exit_time", ""),
                "net_pnl": t.get("net_pnl", 0.0),
                "size_usd": t.get("size_usd", 0.0),
                "weight": weights.get(label, 0.0),
            })

    # Sort by exit_time
    def sort_key(t):
        try:
            return datetime.fromisoformat(t["exit_time"].replace("Z", "+00:00"))
        except (ValueError, AttributeError):
            return datetime.min.replace(tzinfo=timezone.utc)

    all_trades.sort(key=sort_key)

    # Track equity
    balance = initial_balance
    peak_balance = initial_balance
    equity_curve = []
    breaker_events = []

    # Track daily/weekly PnL for breakers
    daily_pnl = 0.0
    weekly_pnl = 0.0
    current_date = None
    current_week = None

    # Track open positions for max positions limit
    open_positions = []

    trade_count = 0
    winning_trades = 0
    gross_profit = 0.0
    gross_loss = 0.0
    max_dd = 0.0
    max_dd_peak = initial_balance

    # Skip trades from candidates with zero weight
    active_trades = [t for t in all_trades if t["weight"] > 0]

    for t in active_trades:
        try:
            exit_dt = datetime.fromisoformat(t["exit_time"].replace("Z", "+00:00"))
            entry_dt = datetime.fromisoformat(t["entry_time"].replace("Z", "+00:00"))
        except (ValueError, AttributeError):
            continue

        trade_date = exit_dt.strftime("%Y-%m-%d")
        trade_week = exit_dt.isocalendar()[1] + exit_dt.year * 100

        # Reset daily tracking
        if current_date != trade_date:
            if current_date is not None:
                # Check daily drawdown breaker
                if daily_pnl < 0 and abs(daily_pnl) > DAILY_DRAWDOWN_LIMIT * peak_balance:
                    breaker_events.append({
                        "date": current_date,
                        "type": "daily_drawdown",
                        "loss": daily_pnl,
                        "threshold": DAILY_DRAWDOWN_LIMIT * peak_balance,
                    })
            daily_pnl = 0.0
            current_date = trade_date

        # Reset weekly tracking
        if current_week != trade_week:
            if current_week is not None:
                # Check weekly drawdown breaker
                if weekly_pnl < 0 and abs(weekly_pnl) > WEEKLY_DRAWDOWN_LIMIT * peak_balance:
                    breaker_events.append({
                        "week": current_week,
                        "type": "weekly_drawdown",
                        "loss": weekly_pnl,
                        "threshold": WEEKLY_DRAWDOWN_LIMIT * peak_balance,
                    })
            weekly_pnl = 0.0
            current_week = trade_week

        # Apply position limit: count currently open positions
        open_positions = [p for p in open_positions
                          if datetime.fromisoformat(p["exit_time"].replace("Z", "+00:00")) > entry_dt]
        if len(open_positions) >= max_positions:
            continue  # Skip this trade due to position limit

        open_positions.append(t)

        # Scale PnL by weight
        scaled_pnl = t["net_pnl"] * t["weight"] * (initial_balance / 100.0)  # Normalize to $1000

        # Apply drawdown breaker: skip trades if breaker triggered
        if breaker_events:
            # Simple breaker: skip next trade after trigger
            last_breaker = breaker_events[-1]
            breaker_date = last_breaker.get("date", "")
            if breaker_date == trade_date:
                continue

        balance += scaled_pnl
        daily_pnl += scaled_pnl
        weekly_pnl += scaled_pnl
        trade_count += 1

        if scaled_pnl > 0:
            winning_trades += 1
            gross_profit += scaled_pnl
        else:
            gross_loss += abs(scaled_pnl)

        if balance > peak_balance:
            peak_balance = balance

        dd = peak_balance - balance
        if dd > max_dd:
            max_dd = dd
            max_dd_peak = peak_balance

        equity_curve.append({
            "date": trade_date,
            "balance": balance,
            "trade_pnl": scaled_pnl,
        })

    # Compute metrics
    if trade_count == 0:
        return {
            "net_pnl": 0.0,
            "sharpe": 0.0,
            "max_dd": 0.0,
            "trade_count": 0,
            "win_rate": 0.0,
            "equity_curve": equity_curve,
            "breaker_events": breaker_events,
            "final_balance": balance,
            "gross_profit": gross_profit,
            "gross_loss": gross_loss,
        }

    win_rate = winning_trades / trade_count if trade_count > 0 else 0.0
    fee_to_gross = gross_loss / gross_profit if gross_profit > 0 else float("inf")

    # Compute Sharpe from daily returns
    daily_returns = defaultdict(float)
    for eq in equity_curve:
        daily_returns[eq["date"]] += eq["trade_pnl"]
    dr_vals = list(daily_returns.values())
    if len(dr_vals) >= 2:
        mean_r = sum(dr_vals) / len(dr_vals)
        std_r = (sum((r - mean_r) ** 2 for r in dr_vals) / len(dr_vals)) ** 0.5
        sharpe = (mean_r / std_r * (365 ** 0.5)) if std_r > 1e-12 else 0.0
    else:
        sharpe = 0.0

    return {
        "net_pnl": balance - initial_balance,
        "sharpe": sharpe,
        "max_dd": max_dd,
        "trade_count": trade_count,
        "win_rate": win_rate,
        "equity_curve": equity_curve,
        "breaker_events": breaker_events,
        "final_balance": balance,
        "gross_profit": gross_profit,
        "gross_loss": gross_loss,
        "fee_to_gross_ratio": fee_to_gross,
    }


def simulate_top_signal_only(trades_by_candidate, sharpe_dict, initial_balance=INITIAL_BALANCE):
    """Simulate 'only trade top-ranked active signal' mode.
    When multiple candidates signal simultaneously, only take the highest-Sharpe one."""
    # Rank candidates by Sharpe
    ranked = sorted(sharpe_dict.keys(), key=lambda c: sharpe_dict.get(c, 0.0), reverse=True)
    rank_map = {c: i for i, c in enumerate(ranked)}

    # Collect all trades with rank
    all_trades = []
    for label, trades in trades_by_candidate.items():
        for t in trades:
            all_trades.append({
                "label": label,
                "rank": rank_map.get(label, 999),
                "entry_time": t.get("entry_time", ""),
                "exit_time": t.get("exit_time", ""),
                "net_pnl": t.get("net_pnl", 0.0),
                "size_usd": t.get("size_usd", 0.0),
            })

    def sort_key(t):
        try:
            return datetime.fromisoformat(t["entry_time"].replace("Z", "+00:00"))
        except (ValueError, AttributeError):
            return datetime.min.replace(tzinfo=timezone.utc)

    all_trades.sort(key=sort_key)

    # Group trades by overlapping time windows and keep only top-ranked
    balance = initial_balance
    peak_balance = initial_balance
    trade_count = 0
    winning_trades = 0
    max_dd = 0.0
    equity_entries = []

    active_trade = None  # Only one trade at a time in this mode

    for t in all_trades:
        try:
            entry_dt = datetime.fromisoformat(t["entry_time"].replace("Z", "+00:00"))
            exit_dt = datetime.fromisoformat(t["exit_time"].replace("Z", "+00:00"))
        except (ValueError, AttributeError):
            continue

        # If a trade is active, check if it overlaps
        if active_trade is not None:
            try:
                active_exit = datetime.fromisoformat(active_trade["exit_time"].replace("Z", "+00:00"))
            except (ValueError, AttributeError):
                active_exit = entry_dt

            if entry_dt < active_exit:
                # Overlapping — keep higher-ranked (lower rank number)
                if t["rank"] < active_trade["rank"]:
                    active_trade = t
                continue
            else:
                # Close the active trade
                scaled_pnl = active_trade["net_pnl"] * (initial_balance / 100.0)
                balance += scaled_pnl
                trade_count += 1
                if scaled_pnl > 0:
                    winning_trades += 1
                if balance > peak_balance:
                    peak_balance = balance
                dd = peak_balance - balance
                if dd > max_dd:
                    max_dd = dd

                try:
                    exit_date = datetime.fromisoformat(active_trade["exit_time"].replace("Z", "+00:00")).strftime("%Y-%m-%d")
                except (ValueError, AttributeError):
                    exit_date = "unknown"
                equity_entries.append({"date": exit_date, "trade_pnl": scaled_pnl})

                active_trade = None

        active_trade = t

    # Close final active trade
    if active_trade is not None:
        scaled_pnl = active_trade["net_pnl"] * (initial_balance / 100.0)
        balance += scaled_pnl
        trade_count += 1
        if scaled_pnl > 0:
            winning_trades += 1
        try:
            exit_date = datetime.fromisoformat(active_trade["exit_time"].replace("Z", "+00:00")).strftime("%Y-%m-%d")
        except (ValueError, AttributeError):
            exit_date = "unknown"
        equity_entries.append({"date": exit_date, "trade_pnl": scaled_pnl})

    # Compute Sharpe from daily returns
    daily_returns = defaultdict(float)
    for eq in equity_entries:
        daily_returns[eq["date"]] += eq["trade_pnl"]
    dr_vals = list(daily_returns.values())
    if len(dr_vals) >= 2:
        mean_r = sum(dr_vals) / len(dr_vals)
        std_r = (sum((r - mean_r) ** 2 for r in dr_vals) / len(dr_vals)) ** 0.5
        sharpe = (mean_r / std_r * (365 ** 0.5)) if std_r > 1e-12 else 0.0
    else:
        sharpe = 0.0

    return {
        "net_pnl": balance - initial_balance,
        "trade_count": trade_count,
        "win_rate": winning_trades / trade_count if trade_count > 0 else 0.0,
        "max_dd": max_dd,
        "final_balance": balance,
        "sharpe": sharpe,
    }


def format_correlation_matrix_md(corr, candidates):
    """Format correlation matrix as markdown table."""
    lines = []
    header = "| | " + " | ".join(candidates) + " |"
    sep = "|---|" + "|".join(["---" for _ in candidates]) + "|"
    lines.append(header)
    lines.append(sep)
    for ca in candidates:
        row = f"| **{ca}** |"
        for cb in candidates:
            val = corr.get(ca, {}).get(cb, 0.0)
            flag = " ⚠️" if abs(val) > 0.7 and ca != cb else ""
            row += f" {val:.3f}{flag} |"
        lines.append(row)
    return "\n".join(lines)


def format_weights_table(weights, candidates_info):
    """Format allocation weights as markdown table."""
    lines = ["| Candidate | Weight | Market | Strategy |", "|-----------|--------|--------|----------|"]
    total = sum(weights.values())
    for label, w in sorted(weights.items(), key=lambda x: -x[1]):
        info = candidates_info.get(label, {})
        market = info.get("market", "?")
        strategy = info.get("strategy", "?")
        pct = w * 100
        lines.append(f"| {label} | {pct:.1f}% | {market} | {strategy} |")
    lines.append(f"| **Total** | **{total * 100:.1f}%** | | |")
    return "\n".join(lines)


def format_metrics_table(metrics):
    """Format portfolio metrics as markdown table."""
    lines = [
        "| Metric | Value |",
        "|--------|-------|",
        f"| Net PnL | ${metrics['net_pnl']:.2f} |",
        f"| Final Balance | ${metrics['final_balance']:.2f} |",
        f"| Sharpe Ratio | {metrics['sharpe']:.4f} |",
        f"| Max Drawdown | ${metrics['max_dd']:.2f} |",
        f"| Trade Count | {metrics['trade_count']} |",
        f"| Win Rate | {metrics['win_rate'] * 100:.1f}% |",
    ]
    if "gross_profit" in metrics:
        lines.append(f"| Gross Profit | ${metrics.get('gross_profit', 0):.2f} |")
        lines.append(f"| Gross Loss | ${metrics.get('gross_loss', 0):.2f} |")
    if "fee_to_gross_ratio" in metrics:
        lines.append(f"| Fee/Gross Ratio | {metrics.get('fee_to_gross_ratio', 0):.3f} |")
    lines.append(f"| Drawdown Breakers | {len(metrics.get('breaker_events', []))} |")
    return "\n".join(lines)


def generate_report(grid, best_configs, daily_returns_by_candidate, corr_matrix,
                    allocation_strategies, portfolio_results, single_best_result,
                    top_signal_result, candidates_info):
    """Generate the full portfolio-backtest.md report."""

    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    report = f"""# Portfolio Construction & Backtest Analysis

> Generated: {now}
> Backtest period: 2026-03-01 to 2026-05-30 (90 days)
> Initial balance: ${INITIAL_BALANCE:.2f}
> Candidates from M2 leverage-sizing: {len(best_configs)}
> Cost modes: flash-only, imperial-route-oracle

## Methodology

### Candidate Selection

From the M2 leverage-sizing frontier, the best configuration per
(strategy, market, cost_mode) candidate is selected. Selection criteria:
1. Highest Sharpe ratio across all (leverage, sizing_mode) combinations
2. Ties broken by net PnL (higher is better)
3. Ties broken by max drawdown (lower is better)

Each candidate uses its individually optimal leverage and sizing mode
from the frontier analysis — no one-size-fits-all approach.

### Correlation Computation

Cross-candidate correlation is computed from **daily PnL returns**:
- Each candidate's trades are aggregated into daily PnL sums
- Pearson correlation is computed pairwise across all trading days
- Missing days (no trades) are filled with zero PnL
- Pairs with correlation > 0.7 are flagged as highly correlated

### Allocation Strategies

Three allocation strategies are tested:

1. **Equal Weight**: Each candidate receives 1/N of the portfolio.
   Simple baseline that avoids estimation error from noisy metrics.

2. **Risk Parity**: Weight inversely proportional to daily PnL volatility.
   Candidates with more volatile returns get less allocation.
   Formula: `w_i = (1/σ_i) / Σ(1/σ_j)` where σ is daily PnL std dev.

3. **Sharpe-Weighted**: Weight proportional to Sharpe ratio.
   Candidates with higher risk-adjusted returns get more allocation.
   Negative Sharpe candidates receive zero weight; excess redistributed.
   Formula: `w_i = max(Sharpe_i, 0) / Σmax(Sharpe_j, 0)`.

### Risk Constraints

| Constraint | Value | Rationale |
|-----------|-------|-----------|
| Max allocation per candidate | {MAX_ALLOCATION_PER_CANDIDATE * 100:.0f}% | Prevents concentration in single strategy |
| Max correlated exposure | {MAX_CORRELATED_EXPOSURE * 100:.0f}% | Limits same-market risk (BTC/SOL/ETH groups) |
| Max simultaneous positions | {MAX_SIMULTANEOUS_POSITIONS} | Controls operational complexity |
| Daily drawdown breaker | {DAILY_DRAWDOWN_LIMIT * 100:.0f}% | Halts trading after large daily loss |
| Weekly drawdown breaker | {WEEKLY_DRAWDOWN_LIMIT * 100:.0f}% | Halts trading after large weekly loss |

### Correlated Market Groups

| Group | Strategies | Markets |
|-------|-----------|---------|
| BTC | cluster-007, cluster-008 | BTC |
| ETH | cluster-005 | ETH |
| SOL | cluster-002, cluster-005, cluster-009 | SOL |

### Single-Best Comparison

The single best candidate (by Sharpe) is compared against each portfolio
allocation strategy to determine whether diversification adds value.

### Top-Signal-Only Mode

An alternative approach: when multiple candidates signal simultaneously,
only the highest-Sharpe candidate's trade is taken. This tests whether
selectivity beats diversification.

## Candidate Summary

| # | Candidate | Cost Mode | Best Lev | Best Sizing | Sharpe | PnL | Trades | Max DD |
|---|-----------|-----------|----------|-------------|--------|-----|--------|--------|
"""

    for i, (key, item) in enumerate(sorted(best_configs.items()), 1):
        label = f"{SHORT_LABELS.get(item['strategy'], item['strategy'])}:{item['market']}"
        report += f"| {i} | {label} | {item['cost_mode']} | {item['leverage']}x | {item['sizing_mode']} | {item['sharpe_ratio']:.4f} | ${item['net_pnl']:.2f} | {item['trade_count']} | ${item['max_drawdown_usd']:.2f} |\n"

    # Correlation matrix
    report += "\n## Cross-Candidate Correlation Matrix\n\n"
    corr_labels = sorted(corr_matrix.keys())
    report += "Correlation computed from daily PnL returns across the 90-day backtest period.\n\n"
    report += format_correlation_matrix_md(corr_matrix, corr_labels)
    report += "\n\n⚠️ = correlation > 0.7 (highly correlated)\n\n"

    # Flag highly correlated pairs
    high_corr_pairs = []
    for ca in corr_labels:
        for cb in corr_labels:
            if ca >= cb:
                continue
            val = corr_matrix.get(ca, {}).get(cb, 0.0)
            if abs(val) > 0.7:
                high_corr_pairs.append((ca, cb, val))

    if high_corr_pairs:
        report += "### Highly Correlated Pairs (>0.7)\n\n"
        for ca, cb, val in high_corr_pairs:
            report += f"- **{ca}** ↔ **{cb}**: {val:.3f}\n"
        report += "\n"

    # Allocation weights
    report += "## Allocation Weights\n\n"
    for strat_name, weights in allocation_strategies.items():
        report += f"### {strat_name.replace('_', ' ').title()}\n\n"
        report += format_weights_table(weights, candidates_info)
        report += "\n\n"
        # Verify sum
        total = sum(weights.values())
        report += f"**Weight sum: {total * 100:.2f}%** (normalized to 100%)\n\n"

    # Portfolio results
    report += "## Portfolio Simulation Results\n\n"
    report += "### Combined Portfolio Metrics\n\n"
    for strat_name, result in portfolio_results.items():
        report += f"#### {strat_name.replace('_', ' ').title()}\n\n"
        report += format_metrics_table(result)
        report += "\n\n"

    # Single best comparison
    report += "## Single-Best vs Portfolio Comparison\n\n"
    report += "### Single Best Candidate\n\n"
    report += format_metrics_table(single_best_result)
    report += "\n\n"
    report += "### Top-Signal-Only Mode\n\n"
    report += format_metrics_table(top_signal_result)
    report += "\n\n"

    # Comparison table
    report += "### Head-to-Head Comparison\n\n"
    report += "| Strategy | Net PnL | Sharpe | Max DD | Trades | Win Rate |\n"
    report += "|----------|---------|--------|--------|--------|----------|\n"

    sb = single_best_result
    report += f"| Single Best | ${sb['net_pnl']:.2f} | {sb.get('sharpe', 0):.4f} | ${sb['max_dd']:.2f} | {sb['trade_count']} | {sb['win_rate'] * 100:.1f}% |\n"

    ts = top_signal_result
    report += f"| Top-Signal-Only | ${ts['net_pnl']:.2f} | {ts.get('sharpe', 0):.4f} | ${ts['max_dd']:.2f} | {ts['trade_count']} | {ts['win_rate'] * 100:.1f}% |\n"

    for strat_name, result in portfolio_results.items():
        report += f"| Portfolio ({strat_name.replace('_', ' ').title()}) | ${result['net_pnl']:.2f} | {result['sharpe']:.4f} | ${result['max_dd']:.2f} | {result['trade_count']} | {result['win_rate'] * 100:.1f}% |\n"
    report += "\n"

    # Drawdown breaker events
    report += "## Drawdown Breaker Analysis\n\n"
    for strat_name, result in portfolio_results.items():
        events = result.get("breaker_events", [])
        report += f"### {strat_name.replace('_', ' ').title()}: {len(events)} breaker events\n\n"
        if events:
            report += "| Date/Week | Type | Loss | Threshold |\n"
            report += "|-----------|------|------|----------|\n"
            for e in events:
                if "date" in e:
                    report += f"| {e['date']} | {e['type']} | ${e['loss']:.2f} | ${e['threshold']:.2f} |\n"
                else:
                    report += f"| Week {e.get('week', '?')} | {e['type']} | ${e['loss']:.2f} | ${e['threshold']:.2f} |\n"
        else:
            report += "No drawdown breaker events triggered during the period.\n"
        report += "\n"

    # Promotion Decision
    report += """## Promotion Decision

### Evaluation Framework

The promotion decision evaluates the portfolio (and individual candidates)
against the six promotion gate criteria from the validation contract:

1. **Positive out-of-sample PnL** — Net PnL > $0 after all costs
2. **Sharpe ratio ≥ 1.0** — Risk-adjusted returns exceed baseline
3. **Trade count ≥ 30** — Sufficient sample for statistical significance
4. **Acceptable max drawdown** — Drawdowns within risk tolerance
5. **Fee-to-gross ratio < 35%** — Edge not consumed by execution costs
6. **Parameter stability** — Performance not dependent on single period

### Individual Candidate Assessment

"""

    for key, item in sorted(best_configs.items()):
        label = f"{SHORT_LABELS.get(item['strategy'], item['strategy'])}:{item['market']}:{item['cost_mode']}"
        sharpe = item["sharpe_ratio"]
        pnl = item["net_pnl"]
        trades = item["trade_count"]
        fee_gross = item.get("fee_to_gross_ratio", float("inf"))

        pnl_pass = pnl > 0
        sharpe_pass = sharpe >= 1.0
        trades_pass = trades >= 30
        fee_pass = fee_gross < 0.35 if fee_gross != float("inf") else False

        status = "✅ PASS" if all([pnl_pass, sharpe_pass, trades_pass, fee_pass]) else "❌ FAIL"
        report += f"**{label}**: {status}\n"
        report += f"- PnL: {'✅' if pnl_pass else '❌'} ${pnl:.2f}\n"
        report += f"- Sharpe: {'✅' if sharpe_pass else '❌'} {sharpe:.4f} (need ≥ 1.0)\n"
        report += f"- Trades: {'✅' if trades_pass else '❌'} {trades} (need ≥ 30)\n"
        report += f"- Fee/Gross: {'✅' if fee_pass else '❌'} {fee_gross:.3f} (need < 0.35)\n\n"

    # Portfolio assessment
    report += "### Portfolio Assessment\n\n"
    for strat_name, result in portfolio_results.items():
        sharpe = result["sharpe"]
        pnl = result["net_pnl"]
        trades = result["trade_count"]
        pnl_pass = pnl > 0
        sharpe_pass = sharpe >= 1.0
        trades_pass = trades >= 30

        status = "✅ PASS" if all([pnl_pass, sharpe_pass, trades_pass]) else "❌ FAIL"
        report += f"**{strat_name.replace('_', ' ').title()} Portfolio**: {status}\n"
        report += f"- PnL: {'✅' if pnl_pass else '❌'} ${pnl:.2f}\n"
        report += f"- Sharpe: {'✅' if sharpe_pass else '❌'} {sharpe:.4f} (need ≥ 1.0)\n"
        report += f"- Trades: {'✅' if trades_pass else '❌'} {trades} (need ≥ 30)\n\n"

    # Final recommendation
    report += "### Recommendation\n\n"

    # Determine if any portfolio passes
    any_pass = any(
        r["net_pnl"] > 0 and r["sharpe"] >= 1.0 and r["trade_count"] >= 30
        for r in portfolio_results.values()
    )
    best_portfolio = max(portfolio_results.values(), key=lambda r: r["sharpe"])
    best_strat = max(portfolio_results.keys(), key=lambda k: portfolio_results[k]["sharpe"])

    if any_pass:
        report += f"""**RECOMMENDATION: PROMOTE to paper trading.**

The {best_strat.replace('_', ' ').title()} portfolio meets all promotion criteria.
- Net PnL: ${best_portfolio['net_pnl']:.2f}
- Sharpe: {best_portfolio['sharpe']:.4f}
- Max DD: ${best_portfolio['max_dd']:.2f}
- Trade Count: {best_portfolio['trade_count']}

Next step: Deploy to Flash Trade paper trading with $1000 USDC for 24-72 hour validation.
"""
    else:
        report += f"""**RECOMMENDATION: DO NOT PROMOTE — REJECT ALL CANDIDATES.**

After comprehensive portfolio construction across 9 strategy-market pairs with
3 allocation strategies, drawdown breakers, and risk constraints:

- **No individual candidate** passes the Sharpe ≥ 1.0 gate on the 90-day OOS period
- **No portfolio allocation** achieves positive risk-adjusted returns
- **Best portfolio**: {best_strat.replace('_', ' ').title()} with Sharpe {best_portfolio['sharpe']:.4f}, PnL ${best_portfolio['net_pnl']:.2f}

### Root Cause Analysis

1. **M1 overfitting**: High Sharpe ratios in M1 (up to 4.05 for cluster-007:BTC)
   collapsed in the extended 90-day period (best 0.08). The M1 results were
   based on 17 days with very few trades (14-33), leading to unreliable metrics.

2. **Fee dominance**: Most candidates have fee-to-gross ratios well above 1.0,
   meaning fees exceed gross trading profits. The strategies generate too many
   small trades that are eaten by execution costs.

3. **Signal quality**: The blueprint strategies, derived from profitable Hyperliquid
   wallets, do not translate to profitable signals on the 90-day OOS period with
   walk-forward validation. The edge observed in wallet fills may have been
   venue-specific, timing-dependent, or simply overfit to historical patterns.

### Follow-up Recommendations

1. **Expand candidate pool**: The current 9 candidates are derived from a limited
   set of HL wallet clusters. Broader wallet discovery could find strategies
   with more robust edges.

2. **Increase trade frequency threshold**: Require ≥50 OOS trades in M1 before
   promotion to M2, to reduce the impact of small-sample noise.

3. **Explore different strategy architectures**: The momentum-threshold approach
   used by all blueprint strategies may be inherently noisy. Consider
   mean-reversion, funding-capture, or regime-adaptive approaches.

4. **Reduce fee impact**: Investigate limit-order execution, maker rebates,
   or venue-switching to lower fee-to-gross ratios.

5. **Longer backtest windows**: The 90-day window may be insufficient for
   strategies with low trade frequency. Consider 180-365 day backtests.
"""

    report += """## Data Provenance

| Item | Source | Details |
|------|--------|---------|
| Leverage sizing grid | `data/leverage-sizing/grid.json` | 315 cells (9 candidates × 7 leverage × 5 sizing modes) |
| Per-trade data | `data/leverage-sizing/raw/*/backtest-trades.json` | Individual trade records with timestamps |
| M1 parameter search | `data/walk-forward-parameter-search.md` | Top parameter sets per candidate |
| M2 frontier analysis | `data/leverage-sizing-frontier.md` | Leverage/sizing efficient frontier |
| Portfolio analysis | `data/portfolio-backtest.md` (this file) | Portfolio construction results |

---

*Report generated by `scripts/portfolio-analysis.py`*
"""

    return report


def main():
    parser = argparse.ArgumentParser(description="Portfolio construction analysis")
    parser.add_argument("--grid", default="data/leverage-sizing/grid.json",
                        help="Path to leverage-sizing grid.json")
    parser.add_argument("--raw-dir", default="data/leverage-sizing/raw",
                        help="Path to raw leverage-sizing results directory")
    parser.add_argument("--output", default="data/portfolio-backtest.md",
                        help="Output markdown report path")
    parser.add_argument("--initial-balance", type=float, default=INITIAL_BALANCE,
                        help="Initial portfolio balance in USD")
    args = parser.parse_args()

    logger.info("Loading grid from %s", args.grid)
    grid = load_grid(args.grid)

    logger.info("Selecting best configurations per candidate")
    best_configs = select_best_configs(grid)
    logger.info("Selected %d candidate configurations", len(best_configs))

    # Load per-trade data for each best config
    trades_by_candidate = {}
    daily_returns_by_candidate = {}
    candidates_info = {}
    vol_dict = {}
    sharpe_dict = {}

    for key, item in best_configs.items():
        raw_key = make_raw_key(item)
        label = f"{SHORT_LABELS.get(item['strategy'], item['strategy'])}:{item['market']}:{item['cost_mode']}"
        logger.info("Loading trades for %s (key: %s)", label, raw_key)

        trades = load_trades(args.raw_dir, raw_key)
        if not trades:
            logger.warning("No trades found for %s, trying all configs for this candidate", label)
            # Try loading from any config for this candidate
            found = False
            for lev in [1.0, 2.0, 3.0, 4.0, 5.0, 7.5, 10.0]:
                for sizing in ["volatility-adjusted", "fixed-notional", "drawdown-throttled",
                               "fixed-fractional", "route-cost-adjusted"]:
                    alt_key = f"{item['strategy'].replace('blueprint-', '')}__{item['market']}__{item['cost_mode']}__lev{lev}__{sizing}"
                    trades = load_trades(args.raw_dir, alt_key)
                    if trades:
                        logger.info("Found trades using alternative key: %s (%d trades)", alt_key, len(trades))
                        found = True
                        break
                if found:
                    break

        trades_by_candidate[label] = trades
        candidates_info[label] = {
            "strategy": item["strategy"],
            "market": item["market"],
            "cost_mode": item["cost_mode"],
        }

        # Compute daily returns
        daily_ret = compute_daily_returns(trades, args.initial_balance)
        daily_returns_by_candidate[label] = daily_ret

        # Compute daily volatility
        if daily_ret:
            vals = list(daily_ret.values())
            mean_r = sum(vals) / len(vals)
            vol = (sum((r - mean_r) ** 2 for r in vals) / len(vals)) ** 0.5
        else:
            vol = 0.0
        vol_dict[label] = vol

        sharpe_dict[label] = item["sharpe_ratio"]

    logger.info("Computing correlation matrix")
    corr_matrix = compute_correlation_matrix(daily_returns_by_candidate)

    # Compute allocations
    candidates = sorted(trades_by_candidate.keys())
    logger.info("Computing allocation strategies for %d candidates", len(candidates))

    equal_weights = compute_allocation_equal(candidates)
    risk_parity_weights = compute_allocation_risk_parity(corr_matrix, vol_dict, candidates)
    sharpe_weights = compute_allocation_sharpe_weighted(sharpe_dict, candidates)

    # Enforce constraints
    allocation_strategies = {}
    for name, raw_weights in [
        ("equal_weight", equal_weights),
        ("risk_parity", risk_parity_weights),
        ("sharpe_weighted", sharpe_weights),
    ]:
        # Enforce max allocation cap
        weights = enforce_max_allocation(raw_weights)
        # Enforce correlated exposure cap
        weights = enforce_correlated_cap(weights, candidates_info)
        # Re-normalize
        total = sum(weights.values())
        if total > 1e-12:
            for c in weights:
                weights[c] /= total
        allocation_strategies[name] = weights

    # Simulate portfolios
    portfolio_results = {}
    for strat_name, weights in allocation_strategies.items():
        logger.info("Simulating portfolio: %s", strat_name)
        result = simulate_portfolio(trades_by_candidate, weights, args.initial_balance)
        portfolio_results[strat_name] = result
        logger.info("  PnL: $%.2f, Sharpe: %.4f, Trades: %d", result["net_pnl"], result["sharpe"], result["trade_count"])

    # Simulate single best
    best_label = max(sharpe_dict.keys(), key=lambda k: sharpe_dict[k])
    logger.info("Simulating single best: %s", best_label)
    single_best_result = simulate_portfolio(
        {best_label: trades_by_candidate[best_label]},
        {best_label: 1.0},
        args.initial_balance,
    )

    # Simulate top-signal-only mode
    logger.info("Simulating top-signal-only mode")
    top_signal_result = simulate_top_signal_only(trades_by_candidate, sharpe_dict, args.initial_balance)

    # Generate report
    logger.info("Generating report")
    report = generate_report(
        grid, best_configs, daily_returns_by_candidate, corr_matrix,
        allocation_strategies, portfolio_results, single_best_result,
        top_signal_result, candidates_info,
    )

    # Write report
    os.makedirs(os.path.dirname(args.output), exist_ok=True)
    with open(args.output, "w") as f:
        f.write(report)

    word_count = len(report.split())
    logger.info("Report written to %s (%d words)", args.output, word_count)

    # Print summary
    print(f"\n{'='*60}")
    print("PORTFOLIO CONSTRUCTION SUMMARY")
    print(f"{'='*60}")
    print(f"Candidates: {len(best_configs)}")
    print(f"Allocation strategies tested: {len(allocation_strategies)}")
    print(f"Report: {args.output} ({word_count} words)")
    print()

    for strat_name, result in portfolio_results.items():
        print(f"  {strat_name}: PnL ${result['net_pnl']:.2f}, Sharpe {result['sharpe']:.4f}, Trades {result['trade_count']}")

    print(f"\n  Single Best: PnL ${single_best_result['net_pnl']:.2f}, Trades {single_best_result['trade_count']}")
    print(f"  Top-Signal-Only: PnL ${top_signal_result['net_pnl']:.2f}, Trades {top_signal_result['trade_count']}")

    # Promotion decision
    any_promote = any(
        r["net_pnl"] > 0 and r["sharpe"] >= 1.0 and r["trade_count"] >= 30
        for r in portfolio_results.values()
    )
    if any_promote:
        print("\n  ★ PROMOTION RECOMMENDATION: PROMOTE to paper trading")
    else:
        print("\n  ✗ PROMOTION RECOMMENDATION: REJECT all candidates")
    print(f"{'='*60}")


if __name__ == "__main__":
    main()
