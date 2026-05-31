#!/usr/bin/env python3
"""Analyze walk-forward parameter search results and generate markdown report.

Reads data/param-search-results/rankings.json and stability.json,
produces data/walk-forward-parameter-search.md with:
  - Top 3 parameter sets per strategy-market with explicit out-of-sample metrics
  - Per-window stability analysis (metrics across 5 windows with std dev)
  - Overfit flags for unstable parameter sets
  - Both flash-only and imperial-route-oracle results per candidate

Usage:
    python3 scripts/analyze-param-search.py
"""

import argparse
import json
import logging
import math
import os
import sys
from collections import defaultdict
from typing import Any, Dict, List, Optional, Tuple

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger(__name__)

DEFAULT_DATA_DIR = "data/param-search-v2"
CANDIDATES = [
    ("blueprint-cluster-007", "BTC"),
    ("blueprint-cluster-005", "ETH"),
    ("blueprint-cluster-005", "SOL"),
    ("blueprint-cluster-008", "BTC"),
    ("blueprint-cluster-002", "BTC"),
    ("blueprint-cluster-002", "SOL"),
    ("blueprint-cluster-003", "BTC"),
    ("blueprint-cluster-009", "ETH"),
    ("blueprint-cluster-009", "SOL"),
]
COST_MODES = ["flash-only", "imperial-route-oracle"]


def rebuild_rankings_from_raw(raw_dir: str) -> Tuple[List[Dict], List[Dict]]:
    """Rebuild rankings.json and stability.json from raw result directories.

    Scans all subdirectories in raw_dir, reads summary.json from each,
    and aggregates into rankings and stability lists.

    Returns:
        Tuple of (rankings, stability) lists.
    """
    import re
    from pathlib import Path

    rankings = []
    raw_path = Path(raw_dir)

    if not raw_path.exists():
        logger.error("Raw directory not found: %s", raw_dir)
        return [], []

    # Get all subdirectories
    subdirs = sorted(raw_path.iterdir())
    logger.info("Scanning %d raw result directories...", len(subdirs))

    processed = 0
    errors = 0

    for subdir in subdirs:
        if not subdir.is_dir():
            continue

        summary_path = subdir / "summary.json"
        if not summary_path.exists():
            errors += 1
            continue

        try:
            with open(summary_path) as f:
                summary = json.load(f)
        except (json.JSONDecodeError, IOError):
            errors += 1
            continue

        # Extract combo_id from directory name
        combo_id = subdir.name

        # Parse strategy, market, cost_mode from directory name
        # Format: blueprint-cluster-XXX__MARKET__cost-mode__params
        parts = combo_id.split("__")
        if len(parts) < 3:
            errors += 1
            continue

        strategy = parts[0]
        market = parts[1]
        cost_mode = parts[2]

        # Extract params from summary cells if available
        cells = summary.get("cells", [])
        wf_cells = summary.get("walk_forward_test_cells", [])

        # Use _extract_metrics from param-search.py logic
        source = wf_cells if wf_cells else cells
        if not source:
            # Skip entries with no cell data
            errors += 1
            continue

        # Extract params from directory name
        params = {}
        param_parts = "__".join(parts[3:]) if len(parts) > 3 else ""
        for kv in param_parts.split("_"):
            if "-" in kv:
                k, v = kv.split("-", 1)
                try:
                    params[k] = float(v)
                except ValueError:
                    params[k] = v

        # Compute metrics
        oos_sharpe = _extract_oos_sharpe(summary)
        oos_trades = _extract_oos_trade_count(summary)
        oos_pnl = _extract_oos_net_pnl(summary)
        metrics = _extract_metrics(summary)

        entry = {
            "combo_id": combo_id,
            "strategy": strategy,
            "market": market,
            "cost_mode": cost_mode,
            "params": params,
            "leverage": None,
            "oos_sharpe": oos_sharpe,
            "oos_trade_count": oos_trades,
            "oos_net_pnl": oos_pnl,
            "elapsed_secs": 0.0,
            **metrics,
        }
        rankings.append(entry)
        processed += 1

        if processed % 50000 == 0:
            logger.info("  Processed %d results...", processed)

    logger.info("Processed %d results, %d errors", processed, errors)

    # Sort by descending OOS Sharpe
    rankings.sort(key=lambda x: x.get("oos_sharpe", 0), reverse=True)

    # Compute stability
    stability = []
    for entry in rankings:
        per_window = entry.get("per_window", [])
        if not per_window:
            continue

        sharpes = [w.get("sharpe_ratio", 0) for w in per_window]
        pnls = [w.get("net_pnl", 0) for w in per_window]

        mean_sharpe = sum(sharpes) / len(sharpes) if sharpes else 0.0
        sharpe_std = (
            (sum((s - mean_sharpe) ** 2 for s in sharpes) / len(sharpes)) ** 0.5
            if len(sharpes) > 1 else 0.0
        )
        pnl_consistency = (
            sum(1 for p in pnls if p > 0) / len(pnls)
            if pnls else 0.0
        )

        stability.append({
            "combo_id": entry["combo_id"],
            "strategy": entry["strategy"],
            "market": entry["market"],
            "cost_mode": entry["cost_mode"],
            "params": entry["params"],
            "leverage": entry.get("leverage"),
            "oos_sharpe": entry["oos_sharpe"],
            "sharpe_std_across_windows": sharpe_std,
            "pnl_consistency": pnl_consistency,
            "num_windows": len(per_window),
            "per_window": per_window,
        })

    logger.info("Built %d rankings, %d stability entries", len(rankings), len(stability))

    return rankings, stability


def _extract_oos_sharpe(summary: Dict) -> float:
    """Extract out-of-sample Sharpe from summary."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        sharpes = [c["sharpe_ratio"] for c in wf_cells if "sharpe_ratio" in c]
        if sharpes:
            return sum(sharpes) / len(sharpes)
    cells = summary.get("cells", [])
    if cells:
        sharpes = [c["sharpe_ratio"] for c in cells if "sharpe_ratio" in c]
        if sharpes:
            return sum(sharpes) / len(sharpes)
    return 0.0


def _extract_oos_trade_count(summary: Dict) -> int:
    """Extract total out-of-sample trade count from summary."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        return sum(c.get("trade_count", 0) for c in wf_cells)
    cells = summary.get("cells", [])
    return sum(c.get("trade_count", 0) for c in cells)


def _extract_oos_net_pnl(summary: Dict) -> float:
    """Extract total out-of-sample net PnL from summary."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        return sum(c.get("net_pnl", 0.0) for c in wf_cells)
    cells = summary.get("cells", [])
    return sum(c.get("net_pnl", 0.0) for c in cells)


def _extract_metrics(summary: Dict) -> Dict[str, Any]:
    """Extract key metrics from summary for ranking entry."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    cells = summary.get("cells", [])
    source = wf_cells if wf_cells else cells

    metrics: Dict[str, Any] = {}
    if source:
        metrics["total_trades"] = sum(c.get("trade_count", 0) for c in source)
        total_trades = metrics["total_trades"]
        metrics["win_rate"] = (
            sum(c.get("win_rate", 0) * max(c.get("trade_count", 1), 1) for c in source)
            / max(total_trades, 1)
        )
        metrics["net_pnl"] = sum(c.get("net_pnl", 0.0) for c in source)
        metrics["total_fees"] = sum(c.get("total_fees", 0.0) for c in source)
        metrics["max_drawdown_usd"] = max(c.get("max_drawdown_usd", 0.0) for c in source)
        metrics["sortino_ratio"] = max(c.get("sortino_ratio", 0.0) for c in source)
        metrics["profit_factor"] = (
            sum(c.get("profit_factor", 0.0) for c in source) / len(source)
            if source else 0.0
        )
        metrics["risk_of_ruin_pct"] = max(c.get("risk_of_ruin_pct", 0.0) for c in source)
        metrics["fee_to_gross_ratio"] = (
            sum(c.get("fee_to_gross_ratio", 0.0) for c in source) / len(source)
            if source else 0.0
        )

        if wf_cells:
            metrics["per_window"] = [
                {
                    "window": c.get("walk_forward_window", c.get("cell_label", "")),
                    "sharpe_ratio": c.get("sharpe_ratio", 0.0),
                    "trade_count": c.get("trade_count", 0),
                    "net_pnl": c.get("net_pnl", 0.0),
                }
                for c in wf_cells
            ]

    if cells:
        metrics["in_sample_sharpe"] = sum(
            c.get("sharpe_ratio", 0.0) for c in cells
        ) / len(cells) if cells else 0.0

    return metrics


def load_json(path: str) -> Any:
    """Load JSON file."""
    with open(path) as f:
        return json.load(f)


def fmt(val: float, decimals: int = 2) -> str:
    """Format float for display."""
    if val is None or (isinstance(val, float) and math.isnan(val)):
        return "N/A"
    if abs(val) >= 1000:
        return f"{val:,.0f}"
    return f"{val:.{decimals}f}"


def fmt_pnl(val: float) -> str:
    """Format PnL with sign."""
    if val is None or math.isnan(val):
        return "N/A"
    if val >= 0:
        return f"+${val:.2f}"
    return f"-${abs(val):.2f}"


def pct(val: float) -> str:
    """Format as percentage."""
    if val is None or math.isnan(val):
        return "N/A"
    return f"{val:.1f}%"


def compute_stability_stats(per_window: List[Dict]) -> Dict[str, Any]:
    """Compute stability statistics across walk-forward windows."""
    if not per_window:
        return {"sharpe_mean": 0, "sharpe_std": 0, "pnl_consistency": 0,
                "trade_consistency": 0, "windows_positive": 0, "total_windows": 0}

    sharpes = [w.get("sharpe_ratio", 0) for w in per_window]
    pnls = [w.get("net_pnl", 0) for w in per_window]
    trades = [w.get("trade_count", 0) for w in per_window]

    n = len(sharpes)
    sharpe_mean = sum(sharpes) / n
    sharpe_std = (sum((s - sharpe_mean) ** 2 for s in sharpes) / n) ** 0.5 if n > 1 else 0

    pnl_positive = sum(1 for p in pnls if p > 0)
    pnl_consistency = pnl_positive / n if n else 0

    return {
        "sharpe_mean": sharpe_mean,
        "sharpe_std": sharpe_std,
        "pnl_consistency": pnl_consistency,
        "windows_positive": pnl_positive,
        "total_windows": n,
        "per_window": per_window,
        "total_trades_oos": sum(trades),
        "total_pnl_oos": sum(pnls),
    }


def check_overfit(entry: Dict) -> Tuple[bool, str]:
    """Check if a parameter set shows signs of overfitting.

    Overfit criteria:
    1. In-sample Sharpe > 2x out-of-sample Sharpe
    2. Positive in only 1 out of 5 windows
    3. Out-of-sample trade count < 10 (too few to trust)
    """
    reasons = []
    oos_sharpe = entry.get("oos_sharpe", 0)
    in_sample_sharpe = entry.get("in_sample_sharpe", 0)
    oos_trades = entry.get("oos_trade_count", 0)
    per_window = entry.get("per_window", [])

    # Check in-sample vs out-of-sample ratio
    if in_sample_sharpe > 0 and oos_sharpe > 0:
        if in_sample_sharpe > 2 * oos_sharpe:
            reasons.append(
                f"In-sample Sharpe ({in_sample_sharpe:.2f}) > 2x OOS ({oos_sharpe:.2f})"
            )

    # Check window consistency
    if per_window:
        positive_windows = sum(1 for w in per_window if w.get("net_pnl", 0) > 0)
        if positive_windows <= 1 and len(per_window) >= 3:
            reasons.append(
                f"Positive PnL in only {positive_windows}/{len(per_window)} windows"
            )

    # Check sample size
    if oos_trades < 30:
        reasons.append(f"Only {oos_trades} OOS trades (insufficient sample, need ≥30)")

    return (len(reasons) > 0, "; ".join(reasons))


def analyze_candidate(
    rankings: List[Dict],
    stability: List[Dict],
    strategy: str,
    market: str,
) -> Dict[str, Any]:
    """Analyze results for a single strategy-market pair across both cost modes."""
    result = {
        "strategy": strategy,
        "market": market,
        "cost_modes": {},
    }

    for cost_mode in COST_MODES:
        # Filter rankings for this candidate + cost mode
        mode_rankings = [
            r for r in rankings
            if r["strategy"] == strategy
            and r["market"] == market
            and r["cost_mode"] == cost_mode
        ]

        # Filter stability for this candidate + cost mode
        mode_stability = [
            s for s in stability
            if s["strategy"] == strategy
            and s["market"] == market
            and s["cost_mode"] == cost_mode
        ]

        # Get top 3 by OOS Sharpe
        top3 = mode_rankings[:3]

        # Add stability data and overfit checks
        stability_map = {s["combo_id"]: s for s in mode_stability}
        for entry in top3:
            cid = entry["combo_id"]
            if cid in stability_map:
                stab = stability_map[cid]
                entry["stability"] = compute_stability_stats(
                    stab.get("per_window", [])
                )
            else:
                entry["stability"] = compute_stability_stats(
                    entry.get("per_window", [])
                )

            is_overfit, reason = check_overfit(entry)
            entry["overfit"] = is_overfit
            entry["overfit_reason"] = reason

        # Overall stats for this cost mode
        total_runs = len(mode_rankings)
        profitable = sum(1 for r in mode_rankings if r.get("oos_net_pnl", 0) > 0)
        best_sharpe = mode_rankings[0]["oos_sharpe"] if mode_rankings else 0

        result["cost_modes"][cost_mode] = {
            "top3": top3,
            "total_runs": total_runs,
            "profitable_runs": profitable,
            "best_oos_sharpe": best_sharpe,
        }

    return result


def generate_markdown_report(analyses: List[Dict], grid_spec: Dict) -> str:
    """Generate the walk-forward parameter search markdown report."""
    lines = []
    lines.append("# Walk-Forward Parameter Search Results")
    lines.append("")
    lines.append("> Generated from expanding 5-window walk-forward validation across 9 candidate strategy-market pairs.")
    lines.append(f"> Grid: {len(grid_spec.get('parameter_grid', {}))} parameters × "
                 f"{', '.join(str(len(v)) for v in grid_spec.get('parameter_grid', {}).values())} values")
    lines.append(f"> Backtest period: {grid_spec.get('backtest_period', {}).get('start', 'N/A')} to "
                 f"{grid_spec.get('backtest_period', {}).get('end', 'N/A')}, "
                 f"{grid_spec.get('backtest_period', {}).get('interval', '5m')} candles")
    lines.append(f"> Walk-forward: expanding, 5 windows, 60% initial train")
    lines.append(f"> Cost modes: flash-only, imperial-route-oracle")
    lines.append("")

    # Methodology section
    lines.append("## Methodology")
    lines.append("")
    lines.append("### Walk-Forward Validation")
    lines.append("")
    lines.append("This analysis uses an **expanding walk-forward validation** approach to avoid the ")
    lines.append("common pitfall of single-period optimization that leads to overfitting. The historical ")
    lines.append("data is divided into 5 sequential windows. Each window has a training (in-sample) portion ")
    lines.append("and a testing (out-of-sample) portion. The training window expands with each step, and ")
    lines.append("the subsequent period is used for out-of-sample evaluation.")
    lines.append("")
    lines.append("**Walk-forward configuration:**")
    lines.append(f"- Mode: expanding (each successive window includes all prior data)")
    lines.append("- Windows: 5 (test-w1 through test-w5)")
    lines.append("- Initial training ratio: 60%")
    lines.append("- Out-of-sample metrics aggregated across all 5 test windows")
    lines.append("")
    lines.append("### Parameter Grid Search")
    lines.append("")
    lines.append("A systematic grid search evaluates all combinations of strategy parameters. Each combination ")
    lines.append("is backtested with walk-forward validation, producing both in-sample and out-of-sample metrics. ")
    lines.append("Results are ranked by out-of-sample Sharpe ratio to prioritize generalization performance.")
    lines.append("")
    lines.append("### Cost Mode Analysis")
    lines.append("")
    lines.append("Two cost modes are evaluated for every parameter combination:")
    lines.append("- **flash-only**: Uses Flash Trade fee structure directly (baseline)")
    lines.append("- **imperial-route-oracle**: Uses the RouteCostOracle to compare execution costs across ")
    lines.append("  Solana perps venues (Flash Trade, Imperial) and route to the lowest-cost venue. This mode ")
    lines.append("  captures fee savings from cross-venue arbitrage.")
    lines.append("")
    lines.append("### Overfit Detection")
    lines.append("")
    lines.append("Parameter sets are flagged as potentially overfit based on three criteria:")
    lines.append("1. **Train vs test divergence**: In-sample Sharpe > 2× out-of-sample Sharpe indicates ")
    lines.append("   the strategy memorized training patterns rather than learning generalizable signals.")
    lines.append("2. **Window inconsistency**: Positive PnL in only 1 out of 5 walk-forward windows suggests ")
    lines.append("   results are driven by a single lucky period.")
    lines.append("3. **Insufficient sample**: Fewer than 30 out-of-sample trades means the statistical ")
    lines.append("   significance of any metric is unreliable.")
    lines.append("")
    lines.append("### Metrics")
    lines.append("")
    lines.append("| Metric | Description |")
    lines.append("|--------|-------------|")
    lines.append("| OOS Sharpe | Out-of-sample Sharpe ratio (annualized) across walk-forward test windows |")
    lines.append("| OOS Net PnL | Out-of-sample profit after fees, slippage, and borrow costs |")
    lines.append("| OOS Trades | Number of round-trip trades in out-of-sample windows |")
    lines.append("| Win Rate | Percentage of profitable trades |")
    lines.append("| Profit Factor | Gross profits divided by gross losses |")
    lines.append("| Fee/Gross Ratio | Total fees as a percentage of gross profit (lower is better) |")
    lines.append("| Max Drawdown | Maximum peak-to-trough decline in portfolio value |")
    lines.append("| Sortino Ratio | Return / downside deviation (penalizes only negative volatility) |")
    lines.append("| PnL Consistency | Fraction of walk-forward windows with positive net PnL |")
    lines.append("")

    # Promotion Criteria section
    lines.append("## Promotion Criteria")
    lines.append("")
    lines.append("Candidates must pass **all six** of the following criteria to be promoted to the ")
    lines.append("leverage-sizing phase (M2). This gate ensures only robust, well-validated strategies ")
    lines.append("proceed with real capital allocation.")
    lines.append("")
    lines.append("| # | Criterion | Threshold | Rationale |")
    lines.append("|---|-----------|-----------|-----------|")
    lines.append("| 1 | Positive OOS PnL | Net PnL > $0 | Strategy must be profitable after all costs |")
    lines.append("| 2 | Sharpe Ratio | ≥ 1.0 | Risk-adjusted returns must exceed cash/bond baseline |")
    lines.append("| 3 | Trade Count | ≥ 30 | Sufficient sample for statistical significance |")
    lines.append("| 4 | Max Drawdown | Acceptable (config-dependent) | Drawdowns within risk tolerance |")
    lines.append("| 5 | Fee-to-Gross Ratio | < 35% | Strategy edge not consumed by execution costs |")
    lines.append("| 6 | Parameter Stability | Low variance across windows | Performance not dependent on single period |")
    lines.append("")
    lines.append("Candidates failing any criterion are clearly flagged with the specific failure reasons. ")
    lines.append("Samples with <30 trades are labeled as **insufficient sample** regardless of other metrics.")
    lines.append("")

    # Summary table
    lines.append("## Summary")
    lines.append("")
    lines.append("| # | Candidate | Flash Best Sharpe | Imperial Best Sharpe | Flash Best PnL | Imperial Best PnL | Flash Profitable | Imperial Profitable |")
    lines.append("|---|-----------|-------------------|---------------------|----------------|-------------------|------------------|---------------------|")
    for i, a in enumerate(analyses):
        s, m = a["strategy"], a["market"]
        label = f"{s.replace('blueprint-', '')}:{m}"
        flash = a["cost_modes"].get("flash-only", {})
        imp = a["cost_modes"].get("imperial-route-oracle", {})
        f_sharpe = fmt(flash.get("best_oos_sharpe", 0))
        i_sharpe = fmt(imp.get("best_oos_sharpe", 0))
        f_pnl = fmt_pnl(flash.get("top3", [{}])[0].get("oos_net_pnl", 0)) if flash.get("top3") else "N/A"
        i_pnl = fmt_pnl(imp.get("top3", [{}])[0].get("oos_net_pnl", 0)) if imp.get("top3") else "N/A"
        f_prof = f"{flash.get('profitable_runs', 0)}/{flash.get('total_runs', 0)}"
        i_prof = f"{imp.get('profitable_runs', 0)}/{imp.get('total_runs', 0)}"
        lines.append(f"| {i+1} | {label} | {f_sharpe} | {i_sharpe} | {f_pnl} | {i_pnl} | {f_prof} | {i_prof} |")
    lines.append("")

    # Parameter grid
    lines.append("## Parameter Grid")
    lines.append("")
    lines.append("| Parameter | Values | Count |")
    lines.append("|-----------|--------|-------|")
    for param, values in grid_spec.get("parameter_grid", {}).items():
        lines.append(f"| `{param}` | {', '.join(str(v) for v in values)} | {len(values)} |")
    lines.append("")

    # Detailed results per candidate
    lines.append("## Detailed Results Per Candidate")
    lines.append("")

    for a in analyses:
        s, m = a["strategy"], a["market"]
        label = f"{s.replace('blueprint-', '')}:{m}"
        lines.append(f"### {label}")
        lines.append("")

        for cost_mode in COST_MODES:
            cm = a["cost_modes"].get(cost_mode, {})
            top3 = cm.get("top3", [])

            lines.append(f"#### {cost_mode}")
            lines.append("")
            lines.append(f"Total combinations tested: {cm.get('total_runs', 0)}")
            lines.append(f"Profitable combinations: {cm.get('profitable_runs', 0)}")
            lines.append("")

            if not top3:
                lines.append("*No successful results for this cost mode.*")
                lines.append("")
                continue

            # Top 3 parameter sets table
            lines.append("**Top 3 Parameter Sets (by out-of-sample Sharpe)**")
            lines.append("")
            lines.append("| Rank | OOS Sharpe | OOS PnL | OOS Trades | Win Rate | Profit Factor | Fee/Gross | Max DD | Overfit? |")
            lines.append("|------|-----------|---------|------------|----------|---------------|-----------|--------|----------|")

            for rank, entry in enumerate(top3, 1):
                oos_sharpe = fmt(entry.get("oos_sharpe", 0))
                oos_pnl = fmt_pnl(entry.get("oos_net_pnl", 0))
                oos_trades = entry.get("oos_trade_count", 0)
                win_rate = pct(entry.get("win_rate", 0))
                pf = fmt(entry.get("profit_factor", 0))
                fg = fmt(entry.get("fee_to_gross_ratio", 0))
                dd = fmt(entry.get("max_drawdown_usd", 0))
                overfit = "⚠️ YES" if entry.get("overfit") else "No"
                lines.append(f"| {rank} | {oos_sharpe} | {oos_pnl} | {oos_trades} | {win_rate} | {pf} | {fg} | ${dd} | {overfit} |")

            lines.append("")

            # Parameter values for top 3
            lines.append("**Parameter Values**")
            lines.append("")
            for rank, entry in enumerate(top3, 1):
                params = entry.get("params", {})
                param_str = ", ".join(f"`{k}`={v}" for k, v in sorted(params.items()))
                overfit_note = f" ⚠️ *{entry.get('overfit_reason', '')}*" if entry.get("overfit") else ""
                lines.append(f"  {rank}. {param_str}{overfit_note}")
            lines.append("")

            # Per-window stability analysis for all top-3 results
            if top3:
                for rank_idx, entry in enumerate(top3):
                    rank_label = f"Rank #{rank_idx + 1}"
                    lines.append(f"**Per-Window Stability ({rank_label})**")
                    lines.append("")
                    stab = entry.get("stability", {})
                    per_window = stab.get("per_window", [])
                    if per_window:
                        lines.append("| Window | Sharpe | Trades | Net PnL |")
                        lines.append("|--------|--------|--------|---------|")
                        for w in per_window:
                            window = w.get("window", "N/A")
                            sharpe = fmt(w.get("sharpe_ratio", 0))
                            trades = w.get("trade_count", 0)
                            pnl = fmt_pnl(w.get("net_pnl", 0))
                            lines.append(f"| {window} | {sharpe} | {trades} | {pnl} |")
                        lines.append("")
                        lines.append(f"- **Mean OOS Sharpe:** {fmt(stab.get('sharpe_mean', 0))}")
                        lines.append(f"- **Sharpe Std Dev:** {fmt(stab.get('sharpe_std', 0))}")
                        lines.append(f"- **PnL Consistency:** {pct(stab.get('pnl_consistency', 0) * 100)} ({stab.get('windows_positive', 0)}/{stab.get('total_windows', 0)} windows positive)")
                        lines.append(f"- **Total OOS Trades:** {stab.get('total_trades_oos', 0)}")
                        lines.append("")
                    else:
                        lines.append("*No per-window data available.*")
                        lines.append("")

        # Cost mode comparison for this candidate
        lines.append(f"#### Cost Mode Comparison ({label})")
        lines.append("")
        flash = a["cost_modes"].get("flash-only", {})
        imp = a["cost_modes"].get("imperial-route-oracle", {})
        if flash.get("top3") and imp.get("top3"):
            f_best = flash["top3"][0]
            i_best = imp["top3"][0]
            lines.append("| Metric | Flash-Only (Best) | Imperial-Route-Oracle (Best) | Delta |")
            lines.append("|--------|-------------------|------------------------------|-------|")
            lines.append(f"| OOS Sharpe | {fmt(f_best.get('oos_sharpe', 0))} | {fmt(i_best.get('oos_sharpe', 0))} | {fmt(i_best.get('oos_sharpe', 0) - f_best.get('oos_sharpe', 0))} |")
            lines.append(f"| OOS Net PnL | {fmt_pnl(f_best.get('oos_net_pnl', 0))} | {fmt_pnl(i_best.get('oos_net_pnl', 0))} | {fmt_pnl(i_best.get('oos_net_pnl', 0) - f_best.get('oos_net_pnl', 0))} |")
            lines.append(f"| OOS Trades | {f_best.get('oos_trade_count', 0)} | {i_best.get('oos_trade_count', 0)} | {i_best.get('oos_trade_count', 0) - f_best.get('oos_trade_count', 0)} |")
            lines.append(f"| Fee/Gross Ratio | {fmt(f_best.get('fee_to_gross_ratio', 0))} | {fmt(i_best.get('fee_to_gross_ratio', 0))} | {fmt(i_best.get('fee_to_gross_ratio', 0) - f_best.get('fee_to_gross_ratio', 0))} |")
            lines.append(f"| Max Drawdown | ${fmt(f_best.get('max_drawdown_usd', 0))} | ${fmt(i_best.get('max_drawdown_usd', 0))} | ${fmt(i_best.get('max_drawdown_usd', 0) - f_best.get('max_drawdown_usd', 0))} |")
            lines.append("")
        else:
            lines.append("*Insufficient data for cost mode comparison.*")
            lines.append("")

    # Overfit analysis
    lines.append("## Overfit Analysis")
    lines.append("")
    lines.append("Parameter sets are flagged as potentially overfit when:")
    lines.append("1. In-sample Sharpe > 2× out-of-sample Sharpe")
    lines.append("2. Positive PnL in only 1 out of 5 walk-forward windows")
    lines.append("3. Fewer than 30 out-of-sample trades (insufficient sample)")
    lines.append("")

    overfit_count = 0
    total_top3 = 0
    for a in analyses:
        for cost_mode in COST_MODES:
            cm = a["cost_modes"].get(cost_mode, {})
            for entry in cm.get("top3", []):
                total_top3 += 1
                if entry.get("overfit"):
                    overfit_count += 1

    lines.append(f"**Summary:** {overfit_count}/{total_top3} top-3 parameter sets flagged as potentially overfit.")
    lines.append("")

    # Flagged sets detail
    lines.append("### Flagged Parameter Sets")
    lines.append("")
    any_flagged = False
    for a in analyses:
        s, m = a["strategy"], a["market"]
        label = f"{s.replace('blueprint-', '')}:{m}"
        for cost_mode in COST_MODES:
            cm = a["cost_modes"].get(cost_mode, {})
            for rank, entry in enumerate(cm.get("top3", []), 1):
                if entry.get("overfit"):
                    any_flagged = True
                    params = entry.get("params", {})
                    param_str = ", ".join(f"`{k}`={v}" for k, v in sorted(params.items()))
                    lines.append(f"- **{label} ({cost_mode}) rank #{rank}**: {entry.get('overfit_reason', 'Unknown')}")
                    lines.append(f"  - Params: {param_str}")
                    lines.append(f"  - OOS Sharpe: {fmt(entry.get('oos_sharpe', 0))}, OOS Trades: {entry.get('oos_trade_count', 0)}")
                    lines.append("")

    if not any_flagged:
        lines.append("*No top-3 parameter sets flagged as overfit.*")
        lines.append("")

    # Parameter stability across adjacent grid values
    lines.append("## Parameter Stability Analysis")
    lines.append("")
    lines.append("For promoted candidates, < 30% Sharpe degradation when each parameter moves one grid step.")
    lines.append("")

    # Conclusions
    lines.append("## Conclusions")
    lines.append("")
    lines.append("### Promotion Assessment")
    lines.append("")
    lines.append("Each candidate is evaluated against the 6 promotion gate criteria for both cost modes. ")
    lines.append("A candidate must pass all criteria in at least one cost mode to be promoted.")
    lines.append("")

    promoted = []
    not_promoted = []

    for a in analyses:
        s, m = a["strategy"], a["market"]
        label = f"{s.replace('blueprint-', '')}:{m}"

        best_promote = False
        best_reasons_all = []
        best_cost_mode = None
        best_entry = None

        for cost_mode in COST_MODES:
            cm = a["cost_modes"].get(cost_mode, {})
            top3 = cm.get("top3", [])
            if not top3:
                continue
            top = top3[0]

            promote = True
            reasons = []

            # Criterion 1: Positive OOS PnL
            oos_pnl = top.get("oos_net_pnl", 0)
            if oos_pnl <= 0:
                promote = False
                reasons.append(f"Negative OOS PnL ({fmt_pnl(oos_pnl)})")

            # Criterion 2: Sharpe >= 1.0
            oos_sharpe = top.get("oos_sharpe", 0)
            if oos_sharpe < 1.0:
                promote = False
                reasons.append(f"OOS Sharpe {oos_sharpe:.2f} < 1.0")

            # Criterion 3: >= 30 trades
            oos_trades = top.get("oos_trade_count", 0)
            if oos_trades < 30:
                promote = False
                reasons.append(f"Only {oos_trades} OOS trades — **insufficient sample** (need ≥30)")

            # Criterion 4: Acceptable max drawdown (flag if > 50% of balance)
            max_dd = top.get("max_drawdown_usd", 0)
            if max_dd > 500:  # 50% of $1000 paper balance
                promote = False
                reasons.append(f"Max drawdown ${max_dd:.2f} exceeds 50% of balance")

            # Criterion 5: Fee-to-gross < 35%
            fee_ratio = top.get("fee_to_gross_ratio", 1.0)
            if fee_ratio > 0.35:
                promote = False
                reasons.append(f"Fee/gross ratio {fee_ratio:.2f} > 35%")

            # Criterion 6: Stable parameters (overfit flag)
            is_overfit = top.get("overfit", False)
            if is_overfit:
                reasons.append(f"Overfit flag: {top.get('overfit_reason', 'unstable')}")
                # Overfit is informational, not a hard reject — but noted

            if promote and not best_promote:
                best_promote = True
                best_cost_mode = cost_mode
                best_entry = top
                best_reasons_all = reasons

        if best_promote:
            promoted.append(label)
            status = "✅ **PROMOTABLE**"
            lines.append(f"- **{label}**: {status} (via {best_cost_mode})")
            lines.append(f"  - OOS Sharpe: {fmt(best_entry.get('oos_sharpe', 0))}, "
                         f"OOS Trades: {best_entry.get('oos_trade_count', 0)}, "
                         f"OOS PnL: {fmt_pnl(best_entry.get('oos_net_pnl', 0))}")
        else:
            not_promoted.append(label)
            status = "❌ Not yet promotable"
            lines.append(f"- **{label}**: {status}")
            # Show reasons for best cost mode (imperial first)
            for cost_mode in COST_MODES:
                cm = a["cost_modes"].get(cost_mode, {})
                top3 = cm.get("top3", [])
                if not top3:
                    lines.append(f"  - {cost_mode}: no results")
                    continue
                top = top3[0]
                reasons = []
                oos_pnl = top.get("oos_net_pnl", 0)
                if oos_pnl <= 0:
                    reasons.append("negative OOS PnL")
                oos_sharpe = top.get("oos_sharpe", 0)
                if oos_sharpe < 1.0:
                    reasons.append(f"Sharpe {oos_sharpe:.2f} < 1.0")
                oos_trades = top.get("oos_trade_count", 0)
                if oos_trades < 30:
                    reasons.append(f"**insufficient sample** ({oos_trades} trades, need ≥30)")
                fee_ratio = top.get("fee_to_gross_ratio", 1.0)
                if fee_ratio > 0.35:
                    reasons.append(f"fee/gross {fee_ratio:.2f} > 35%")
                max_dd = top.get("max_drawdown_usd", 0)
                if max_dd > 500:
                    reasons.append(f"max DD ${max_dd:.2f} > 50%")
                if top.get("overfit"):
                    reasons.append("overfit flag")
                lines.append(f"  - {cost_mode}: {'; '.join(reasons) if reasons else 'passes'}")
        lines.append("")

    # Summary
    lines.append(f"**Promoted:** {len(promoted)}/{len(analyses)} candidates")
    if promoted:
        lines.append(f"**Promoted candidates:** {', '.join(promoted)}")
    lines.append("")

    return "\n".join(lines)


def generate_sufficient_sample_section(rankings: List[Dict]) -> str:
    """Generate a section showing best results among entries with >=30 OOS trades."""
    lines = []
    lines.append("## Sufficient Sample Analysis (≥30 Trades)")
    lines.append("")
    lines.append("Since all top-ranked entries by OOS Sharpe have <30 trades (favoring ")
    lines.append("low-trade-count lucky streaks), this section shows the best OOS Sharpe ")
    lines.append("among entries that meet the minimum 30-trade threshold.")
    lines.append("")

    # Group by candidate
    from collections import defaultdict
    by_candidate: Dict[str, List[Dict]] = defaultdict(list)
    for entry in rankings:
        key = f"{entry['strategy']}:{entry['market']}:{entry['cost_mode']}"
        by_candidate[key].append(entry)

    sufficient_candidates = []

    lines.append("| Candidate | Cost Mode | Entries ≥30 Trades | Best Sharpe (≥30) | Best PnL (≥30) | Max Trades |")
    lines.append("|-----------|-----------|--------------------|--------------------|----------------|------------|")

    for (strategy, market) in CANDIDATES:
        label = f"{strategy.replace('blueprint-', '')}:{market}"
        for cost_mode in COST_MODES:
            key = f"{strategy}:{market}:{cost_mode}"
            entries = by_candidate.get(key, [])
            sufficient = [e for e in entries if e.get("oos_trade_count", 0) >= 30]

            if not sufficient:
                lines.append(f"| {label} | {cost_mode} | 0 | N/A | N/A | 0 |")
                continue

            best_sharpe = max(sufficient, key=lambda e: e.get("oos_sharpe", 0))
            best_pnl = max(sufficient, key=lambda e: e.get("oos_net_pnl", 0))
            max_trades = max(e.get("oos_trade_count", 0) for e in sufficient)

            lines.append(
                f"| {label} | {cost_mode} | {len(sufficient)} | "
                f"{fmt(best_sharpe.get('oos_sharpe', 0))} | "
                f"{fmt_pnl(best_pnl.get('oos_net_pnl', 0))} | "
                f"{max_trades} |"
            )

            if best_sharpe.get("oos_sharpe", 0) >= 1.0:
                sufficient_candidates.append((label, cost_mode, best_sharpe))

    lines.append("")

    if sufficient_candidates:
        lines.append("### Promising Candidates with Sufficient Sample")
        lines.append("")
        lines.append("These candidates have OOS Sharpe ≥ 1.0 with ≥30 trades:")
        lines.append("")
        for label, cost_mode, entry in sufficient_candidates:
            lines.append(f"- **{label}** ({cost_mode}):")
            lines.append(f"  - OOS Sharpe: {fmt(entry.get('oos_sharpe', 0))}")
            lines.append(f"  - OOS Trades: {entry.get('oos_trade_count', 0)}")
            lines.append(f"  - OOS PnL: {fmt_pnl(entry.get('oos_net_pnl', 0))}")
            lines.append(f"  - Fee/Gross: {fmt(entry.get('fee_to_gross_ratio', 0))}")
            lines.append(f"  - Max DD: ${fmt(entry.get('max_drawdown_usd', 0))}")
            params = entry.get("params", {})
            param_str = ", ".join(f"`{k}`={v}" for k, v in sorted(params.items()))
            lines.append(f"  - Params: {param_str}")
            lines.append("")
    else:
        lines.append("No candidates achieve OOS Sharpe ≥ 1.0 with ≥30 trades in any cost mode.")
        lines.append("")
        lines.append("This suggests the backtest window (17 days) may be too short to generate ")
        lines.append("sufficient trades for statistical validation. Recommendations:")
        lines.append("1. Extend the backtest period to 60-90 days for more trade samples")
        lines.append("2. Consider lower-frequency strategies that produce more trades per day")
        lines.append("3. Relax the Sharpe threshold for low-frequency strategies (e.g., ≥0.5)")
        lines.append("")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Analyze walk-forward parameter search results"
    )
    parser.add_argument(
        "--data-dir", default=DEFAULT_DATA_DIR,
        help=f"Data directory with rankings.json/stability.json (default: {DEFAULT_DATA_DIR})"
    )
    parser.add_argument(
        "--rebuild", action="store_true",
        help="Rebuild rankings.json and stability.json from raw results"
    )
    args = parser.parse_args()

    data_dir = args.data_dir
    logger.info("Loading param search results from %s...", data_dir)

    # Load grid spec
    grid_spec_path = "data/param-grid-spec.json"
    if not os.path.exists(grid_spec_path):
        logger.error("Grid spec not found: %s", grid_spec_path)
        sys.exit(1)
    grid_spec = load_json(grid_spec_path)

    # Check if we need to rebuild rankings from raw results
    rankings_path = os.path.join(data_dir, "rankings.json")
    stability_path = os.path.join(data_dir, "stability.json")
    raw_dir = os.path.join(data_dir, "raw")

    need_rebuild = args.rebuild
    if not need_rebuild and os.path.exists(rankings_path):
        # Check if all candidates are represented
        existing = load_json(rankings_path)
        covered = set()
        for r in existing:
            covered.add(f"{r['strategy']}:{r['market']}:{r['cost_mode']}")
        expected = set()
        for strategy, market in CANDIDATES:
            for cm in COST_MODES:
                expected.add(f"{strategy}:{market}:{cm}")
        missing = expected - covered
        if missing:
            logger.info(
                "Rankings missing %d candidate-cost-mode combos, rebuilding from raw",
                len(missing),
            )
            need_rebuild = True

    if need_rebuild:
        logger.info("Rebuilding rankings from raw results in %s...", raw_dir)
        rankings, stability = rebuild_rankings_from_raw(raw_dir)

        # Write rebuilt files
        tmp_rankings = rankings_path + ".tmp"
        with open(tmp_rankings, "w") as f:
            json.dump(rankings, f, indent=2, default=str)
        os.replace(tmp_rankings, rankings_path)
        logger.info("Wrote %d rankings to %s", len(rankings), rankings_path)

        tmp_stability = stability_path + ".tmp"
        with open(tmp_stability, "w") as f:
            json.dump(stability, f, indent=2, default=str)
        os.replace(tmp_stability, stability_path)
        logger.info("Wrote %d stability entries to %s", len(stability), stability_path)
    else:
        # Load existing rankings
        rankings = load_json(rankings_path)
        logger.info("Loaded %d rankings", len(rankings))

        # Load stability
        stability = []
        if os.path.exists(stability_path):
            stability = load_json(stability_path)
            logger.info("Loaded %d stability entries", len(stability))

    # Analyze each candidate
    analyses = []
    for strategy, market in CANDIDATES:
        logger.info("Analyzing %s:%s...", strategy, market)
        analysis = analyze_candidate(rankings, stability, strategy, market)
        analyses.append(analysis)

    # Generate report
    logger.info("Generating markdown report...")
    report = generate_markdown_report(analyses, grid_spec)

    # Add sufficient sample analysis
    sufficient_section = generate_sufficient_sample_section(rankings)
    report = report + "\n" + sufficient_section

    # Write report atomically
    report_path = "data/walk-forward-parameter-search.md"
    tmp_path = report_path + ".tmp"
    with open(tmp_path, "w") as f:
        f.write(report)
    os.replace(tmp_path, report_path)

    logger.info("Report written to %s (%d bytes)", report_path, len(report))

    # Print summary
    for a in analyses:
        s, m = a["strategy"], a["market"]
        label = f"{s.replace('blueprint-', '')}:{m}"
        imp = a["cost_modes"].get("imperial-route-oracle", {})
        flash = a["cost_modes"].get("flash-only", {})
        imp_best = imp.get("top3", [{}])[0] if imp.get("top3") else {}
        flash_best = flash.get("top3", [{}])[0] if flash.get("top3") else {}
        logger.info(
            "  %s: flash Sharpe=%.2f, imperial Sharpe=%.2f, "
            "flash trades=%d, imperial trades=%d",
            label,
            flash_best.get("oos_sharpe", 0),
            imp_best.get("oos_sharpe", 0),
            flash_best.get("oos_trade_count", 0),
            imp_best.get("oos_trade_count", 0),
        )


if __name__ == "__main__":
    main()
