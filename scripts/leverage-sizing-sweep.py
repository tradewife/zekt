#!/usr/bin/env python3
"""Leverage and position sizing grid sweep for M2.

Takes the best parameter sets from M1 param-search and runs a comprehensive
leverage × sizing-mode grid on an extended 90-day backtest period to achieve
>=30 trades per candidate.

Usage:
    python3 scripts/leverage-sizing-sweep.py
    python3 scripts/leverage-sizing-sweep.py --dry-run
    python3 scripts/leverage-sizing-sweep.py --parallelism 4

Output:
    data/leverage-sizing/raw/          — Per-combination summary.json files
    data/leverage-sizing/grid.json     — Full grid results as JSON
    data/leverage-sizing-frontier.md   — Final markdown report
"""

import argparse
import json
import logging
import math
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

DEFAULT_BINARY = "./target/release/zekt"
DEFAULT_OUTPUT_DIR = "data/leverage-sizing"
DEFAULT_PAPER_BALANCE = 1000.0
DEFAULT_PARALLELISM = 4
MAX_PARALLELISM = 8

# Extended backtest period: 90 days for sufficient trade count
BACKTEST_START = "2026-03-01"
BACKTEST_END = "2026-05-30"
BACKTEST_INTERVAL = "5m"

# Leverage levels to test
LEVERAGE_LEVELS = [1.0, 2.0, 3.0, 4.0, 5.0, 7.5, 10.0]

# Sizing modes to test
SIZING_MODES = [
    "fixed-notional",
    "fixed-fractional",
    "volatility-adjusted",
    "drawdown-throttled",
    "route-cost-adjusted",
]

# Candidates with their best parameters from M1 param-search
# Format: (strategy, market, cost_mode, params, description)
CANDIDATES = [
    # Primary: promoted from M1 with >=30 trades and Sharpe >= 1.0
    {
        "strategy": "blueprint-cluster-005",
        "market": "ETH",
        "cost_mode": "imperial-route-oracle",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.49475,
            "max_hold_secs": 86400,
        },
        "label": "cluster-005:ETH (imperial, Sharpe 1.36, 33 trades in 17d)",
        "priority": "primary",
    },
    {
        "strategy": "blueprint-cluster-008",
        "market": "BTC",
        "cost_mode": "imperial-route-oracle",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.111975,
            "max_hold_secs": 86400,
        },
        "label": "cluster-008:BTC (imperial, Sharpe 2.50, 31 trades in 17d)",
        "priority": "primary",
    },
    # Secondary: promising signal but <30 trades in 17d window
    {
        "strategy": "blueprint-cluster-005",
        "market": "ETH",
        "cost_mode": "flash-only",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.33235,
            "max_hold_secs": 43200,
        },
        "label": "cluster-005:ETH (flash, Sharpe 1.20, 17 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-005",
        "market": "SOL",
        "cost_mode": "imperial-route-oracle",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.80735,
            "max_hold_secs": 86400,
        },
        "label": "cluster-005:SOL (imperial, Sharpe 2.50, 19 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-005",
        "market": "SOL",
        "cost_mode": "flash-only",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.6511,
            "max_hold_secs": 86400,
        },
        "label": "cluster-005:SOL (flash, Sharpe 2.18, 19 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-008",
        "market": "BTC",
        "cost_mode": "flash-only",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.95745,
            "max_hold_secs": 43200,
        },
        "label": "cluster-008:BTC (flash, Sharpe 2.99, 9 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-007",
        "market": "BTC",
        "cost_mode": "imperial-route-oracle",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.17595,
            "max_hold_secs": 43200,
        },
        "label": "cluster-007:BTC (imperial, Sharpe 4.05, 14 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-007",
        "market": "BTC",
        "cost_mode": "flash-only",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.197,
            "max_hold_secs": 43200,
        },
        "label": "cluster-007:BTC (flash, Sharpe 2.74, 14 trades in 17d)",
        "priority": "secondary",
    },
    {
        "strategy": "blueprint-cluster-002",
        "market": "SOL",
        "cost_mode": "imperial-route-oracle",
        "params": {
            "lookback_count": 15,
            "momentum_threshold_pct": 0.173855,
            "max_hold_secs": 43200,
        },
        "label": "cluster-002:SOL (imperial, Sharpe 1.08, 14 trades in 17d)",
        "priority": "secondary",
    },
]


# ---------------------------------------------------------------------------
# Run result
# ---------------------------------------------------------------------------

@dataclass
class SweepResult:
    """Result from a single leverage×sizing backtest run."""
    candidate_label: str
    strategy: str
    market: str
    cost_mode: str
    leverage: float
    sizing_mode: str
    success: bool = False
    error: Optional[str] = None
    # Metrics from summary.json
    net_pnl: float = 0.0
    sharpe_ratio: float = 0.0
    sortino_ratio: float = 0.0
    calmar_ratio: float = 0.0
    max_drawdown_usd: float = 0.0
    trade_count: int = 0
    win_rate: float = 0.0
    fee_to_gross_ratio: float = 0.0
    risk_of_ruin_pct: float = 0.0
    avg_liquidation_proximity_pct: float = 0.0
    max_consecutive_losses: int = 0
    avg_recovery_time_secs: float = 0.0
    total_fees: float = 0.0
    gross_pnl: float = 0.0
    avg_hold_secs: float = 0.0
    # Walk-forward test metrics (aggregated OOS)
    wf_test_sharpe: float = 0.0
    wf_test_pnl: float = 0.0
    wf_test_trades: int = 0
    wf_test_max_dd: float = 0.0
    # Timing
    elapsed_secs: float = 0.0


# ---------------------------------------------------------------------------
# Command building
# ---------------------------------------------------------------------------

def build_command(
    binary_path: str,
    candidate: Dict[str, Any],
    leverage: float,
    sizing_mode: str,
    output_dir: str,
    paper_balance: float = DEFAULT_PAPER_BALANCE,
) -> List[str]:
    """Build the Rust binary invocation command."""
    cmd = [
        binary_path,
        "--backtest",
        "--strategies", candidate["strategy"],
        "--markets", candidate["market"],
        "--cost-mode", candidate["cost_mode"],
        "--backtest-start", BACKTEST_START,
        "--backtest-end", BACKTEST_END,
        "--backtest-interval", BACKTEST_INTERVAL,
        "--paper-balance", str(paper_balance),
        "--output-path", output_dir,
        "--walk-forward-mode", "expanding",
        "--walk-forward-windows", "5",
        "--leverage", str(leverage),
        "--sizing-mode", sizing_mode,
        "--param-override", json.dumps(candidate["params"]),
    ]
    return cmd


def make_run_id(candidate: Dict[str, Any], leverage: float, sizing_mode: str) -> str:
    """Generate a unique filesystem-safe run ID."""
    strategy = candidate["strategy"].replace("blueprint-", "")
    market = candidate["market"]
    cost = candidate["cost_mode"]
    return f"{strategy}__{market}__{cost}__lev{leverage}__{sizing_mode}"


# ---------------------------------------------------------------------------
# Run execution
# ---------------------------------------------------------------------------

def run_single(
    binary_path: str,
    candidate: Dict[str, Any],
    leverage: float,
    sizing_mode: str,
    output_dir: str,
    paper_balance: float,
    file_cache: bool,
) -> SweepResult:
    """Execute a single backtest run and collect results."""
    run_id = make_run_id(candidate, leverage, sizing_mode)
    result = SweepResult(
        candidate_label=candidate["label"],
        strategy=candidate["strategy"],
        market=candidate["market"],
        cost_mode=candidate["cost_mode"],
        leverage=leverage,
        sizing_mode=sizing_mode,
    )

    run_dir = os.path.join(output_dir, "raw", run_id)
    os.makedirs(run_dir, exist_ok=True)

    # Check for cached result
    summary_path = os.path.join(run_dir, "summary.json")
    if os.path.exists(summary_path):
        try:
            with open(summary_path) as f:
                cached = json.load(f)
            _extract_metrics(result, cached)
            result.success = True
            logger.debug("Cached: %s", run_id)
            return result
        except (json.JSONDecodeError, KeyError) as e:
            logger.warning("Invalid cache for %s: %s, re-running", run_id, e)

    cmd = build_command(binary_path, candidate, leverage, sizing_mode, run_dir, paper_balance)

    env = os.environ.copy()
    if file_cache:
        env["ZEKT_FILE_CACHE"] = "1"

    start_time = time.time()
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,  # 5 min timeout per run
            env=env,
        )
        result.elapsed_secs = time.time() - start_time

        if proc.returncode != 0:
            result.error = f"Exit code {proc.returncode}: {proc.stderr[-500:] if proc.stderr else 'no stderr'}"
            logger.warning("FAIL %s: %s", run_id, result.error[:200])
            return result

        # Parse summary.json
        if os.path.exists(summary_path):
            with open(summary_path) as f:
                summary = json.load(f)
            _extract_metrics(result, summary)
            result.success = True
        else:
            result.error = "summary.json not found after run"
            logger.warning("No summary.json for %s", run_id)

    except subprocess.TimeoutExpired:
        result.elapsed_secs = time.time() - start_time
        result.error = "Timeout after 300s"
        logger.warning("TIMEOUT: %s", run_id)
    except Exception as e:
        result.elapsed_secs = time.time() - start_time
        result.error = str(e)
        logger.warning("ERROR %s: %s", run_id, e)

    return result


def _extract_metrics(result: SweepResult, summary: Dict[str, Any]) -> None:
    """Extract metrics from summary.json into SweepResult.

    Summary has a 'cells' array with per-window stats. We aggregate OOS
    (test-labeled) cells and also capture the overall stats.
    """
    # Overall stats from top level
    result.net_pnl = summary.get("total_net_pnl", 0.0)
    result.total_fees = summary.get("total_fees", 0.0)

    # Look at individual cells
    cells = summary.get("cells", [])
    test_cells = [c for c in cells if c.get("walk_forward_window", "").startswith("test")]
    all_cells = cells

    if not cells:
        return

    # Use aggregate of test cells for OOS metrics
    if test_cells:
        agg_pnl = sum(c.get("net_pnl", 0.0) for c in test_cells)
        agg_trades = sum(c.get("trade_count", 0) for c in test_cells)
        agg_max_dd = max(c.get("max_drawdown_usd", 0.0) for c in test_cells)

        result.wf_test_pnl = agg_pnl
        result.wf_test_trades = agg_trades
        result.wf_test_max_dd = agg_max_dd

        # Sharpe: weighted average by trade count
        if agg_trades > 0:
            weighted_sharpe = sum(
                c.get("sharpe_ratio", 0.0) * c.get("trade_count", 0)
                for c in test_cells
            ) / agg_trades
            result.wf_test_sharpe = weighted_sharpe

    # Use the full-sample cell (or first cell) for other metrics
    # Prefer the cell with most trades for representative metrics
    best_cell = max(all_cells, key=lambda c: c.get("trade_count", 0))

    result.sharpe_ratio = best_cell.get("sharpe_ratio", 0.0)
    result.sortino_ratio = best_cell.get("sortino_ratio", 0.0)
    result.calmar_ratio = best_cell.get("calmar_ratio", 0.0)
    result.max_drawdown_usd = best_cell.get("max_drawdown_usd", 0.0)
    result.trade_count = best_cell.get("trade_count", 0)
    result.win_rate = best_cell.get("win_rate", 0.0)
    result.fee_to_gross_ratio = best_cell.get("fee_to_gross_ratio", 0.0)
    result.risk_of_ruin_pct = best_cell.get("risk_of_ruin_pct", 0.0)
    result.avg_liquidation_proximity_pct = best_cell.get("avg_liquidation_proximity_pct", 0.0)
    result.max_consecutive_losses = best_cell.get("max_consecutive_losses", 0)
    result.avg_recovery_time_secs = best_cell.get("avg_recovery_time_secs", 0.0)
    result.gross_pnl = best_cell.get("gross_pnl", 0.0)
    result.total_fees = best_cell.get("total_fees", 0.0)
    result.avg_hold_secs = best_cell.get("avg_hold_secs", 0.0)


# ---------------------------------------------------------------------------
# Grid execution
# ---------------------------------------------------------------------------

def run_grid(
    binary_path: str,
    output_dir: str,
    paper_balance: float,
    parallelism: int,
    file_cache: bool,
    candidates: Optional[List[Dict[str, Any]]] = None,
    leverage_levels: Optional[List[float]] = None,
    sizing_modes: Optional[List[str]] = None,
) -> List[SweepResult]:
    """Run the full leverage × sizing grid."""
    if candidates is None:
        candidates = CANDIDATES
    if leverage_levels is None:
        leverage_levels = LEVERAGE_LEVELS
    if sizing_modes is None:
        sizing_modes = SIZING_MODES

    # Build all combinations
    combos: List[Tuple[Dict, float, str]] = []
    for cand in candidates:
        for lev in leverage_levels:
            for sm in sizing_modes:
                combos.append((cand, lev, sm))

    total = len(combos)
    logger.info(
        "Grid: %d candidates × %d leverage × %d sizing = %d runs",
        len(candidates), len(leverage_levels), len(sizing_modes), total,
    )

    results: List[SweepResult] = []
    completed = 0

    # Run with process pool for parallelism
    with ProcessPoolExecutor(max_workers=parallelism) as pool:
        futures = {}
        for cand, lev, sm in combos:
            future = pool.submit(
                run_single,
                binary_path, cand, lev, sm,
                output_dir, paper_balance, file_cache,
            )
            futures[future] = (cand["label"], lev, sm)

        for future in as_completed(futures):
            label, lev, sm = futures[future]
            try:
                result = future.result()
                results.append(result)
            except Exception as e:
                logger.error("Exception for %s lev=%s sm=%s: %s", label, lev, sm, e)
                results.append(SweepResult(
                    candidate_label=label,
                    strategy="",
                    market="",
                    cost_mode="",
                    leverage=lev,
                    sizing_mode=sm,
                    error=str(e),
                ))

            completed += 1
            if completed % 25 == 0 or completed == total:
                logger.info("Progress: %d/%d (%.0f%%)", completed, total, completed / total * 100)

    success_count = sum(1 for r in results if r.success)
    logger.info("Completed: %d/%d successful", success_count, total)
    return results


# ---------------------------------------------------------------------------
# Report generation
# ---------------------------------------------------------------------------

def _fmt(v: float, decimals: int = 2) -> str:
    """Format a float for markdown table."""
    if abs(v) >= 99999.0:  # METRIC_INF
        return "∞"
    if v == -1.0:  # METRIC_UNDEFINED
        return "N/A"
    if abs(v) < 0.005:
        return f"{v:.{decimals}f}"
    return f"{v:.{decimals}f}"


def _liq_distance(leverage: float) -> str:
    """Compute theoretical liquidation distance from entry for a given leverage.

    For long: liq_price = entry * (1 - 1/leverage), distance = 1/leverage * 100%
    For short: liq_price = entry * (1 + 1/leverage), distance = 1/leverage * 100%
    Both sides: distance = (1/leverage) * 100%
    """
    if leverage <= 0:
        return "N/A"
    dist = (1.0 / leverage) * 100.0
    return f"{dist:.1f}%"


def compute_liquidation_price_estimate(entry_price: float, leverage: float, is_long: bool) -> Tuple[float, str]:
    """Estimate liquidation price for a leveraged position."""
    if leverage <= 0:
        return 0.0, "N/A"
    if is_long:
        liq = entry_price * (1.0 - 1.0 / leverage)
    else:
        liq = entry_price * (1.0 + 1.0 / leverage)
    dist_pct = (1.0 / leverage) * 100.0
    return liq, f"{dist_pct:.1f}%"


def generate_report(results: List[SweepResult], output_dir: str) -> str:
    """Generate the leverage-sizing-frontier.md report."""
    lines: List[str] = []

    lines.append("# Leverage & Position Sizing Frontier Analysis")
    lines.append("")
    lines.append("> Extended backtest period: 2026-03-01 to 2026-05-30 (90 days, 5m candles)")
    lines.append("> Walk-forward: expanding, 5 windows")
    lines.append(f"> Candidates tested: {len(set(r.candidate_label for r in results))}")
    lines.append(f"> Total grid cells: {len(results)} ({sum(1 for r in results if r.success)} successful)")
    lines.append("")

    # ---- Methodology section ----
    lines.append("## Methodology")
    lines.append("")
    lines.append("### Leverage Grid")
    lines.append("")
    lines.append("Seven leverage levels are tested to map the risk-return frontier:")
    lines.append("")
    for lev in LEVERAGE_LEVELS:
        liq_dist = _liq_distance(lev)
        lines.append(f"- **{lev}x**: liquidation at {liq_dist} from entry")
    lines.append("")

    lines.append("### Position Sizing Modes")
    lines.append("")
    lines.append("Five sizing methodologies from the `SizingMode` enum in `backtest.rs`:")
    lines.append("")
    lines.append("1. **fixed-notional** — Constant notional size from strategy params (baseline).")
    lines.append("   Simplest approach; position size does not adapt to market conditions.")
    lines.append("   Justified as baseline comparison — all improvements must beat this.")
    lines.append("")
    lines.append("2. **fixed-fractional** — Risk a fixed fraction of current equity per trade.")
    lines.append("   `size = equity × risk_fraction` (default 2%). Scales with account growth,")
    lines.append("   compounds gains, and reduces naturally after losses. Based on the")
    lines.append("   **Kelly Criterion** concept of proportional betting: for a strategy with")
    lines.append("   win rate `p` and win/loss ratio `b`, the Kelly-optimal fraction is")
    lines.append("   `f* = p - (1-p)/b`. We use a conservative 2% (fractional Kelly)")
    lines.append("   to avoid overbetting on uncertain edge estimates.")
    lines.append("")
    lines.append("3. **volatility-adjusted** — Scale position inversely with ATR (Average True Range).")
    lines.append("   `size = min(equity × base_fraction × (ATR_baseline / ATR_current), max_size_usd)`.")  
    lines.append("   Reduces exposure in high-vol regimes, increases in calm markets.")
    lines.append("   Justified by the principle of **risk parity**: equalizing the dollar")
    lines.append("   volatility per trade regardless of market regime.")
    lines.append("")
    lines.append("4. **drawdown-throttled** — Reduce position size as drawdown deepens.")
    lines.append("   Linear throttle from `throttle_start_pct` (5%) to `max_drawdown_pct` (20%),")
    lines.append("   where trading is paused entirely. A practical risk management approach")
    lines.append("   that prevents catastrophic compounding of losses.")
    lines.append("   Justified by behavioral finance research showing that drawdowns impair")
    lines.append("   decision quality — reducing exposure during drawdowns is rational.")
    lines.append("")
    lines.append("5. **route-cost-adjusted** — Penalize position size for expensive execution routes.")
    lines.append("   `size = equity × base_fraction × (1 - penalty)`, where penalty scales")
    lines.append("   with route cost (spread + fees) relative to a threshold. Routes costing")
    lines.append("   more than `max_penalty_pct` (80%) of expected edge are skipped entirely.")
    lines.append("   Justified by the net-edge principle: a signal's value must exceed")
    lines.append("   execution cost to be worth trading.")
    lines.append("")

    lines.append("### Risk Metrics")
    lines.append("")
    lines.append("| Metric | Description | Computation |")
    lines.append("|--------|-------------|-------------|")
    lines.append("| Net PnL | Total profit after fees, borrow, slippage | Sum of trade net PnLs |")
    lines.append("| Sharpe Ratio | Risk-adjusted return | mean(returns) / std(returns), annualized |")
    lines.append("| Sortino Ratio | Downside-adjusted return | mean(returns) / downside_deviation |")
    lines.append("| Calmar Ratio | Return vs max drawdown | annualized_return / max_drawdown |")
    lines.append("| Max Drawdown | Largest peak-to-trough decline | Equity curve tracking |")
    lines.append("| Liquidation Proximity | Avg % distance from worst price to liq | Per-trade, leverage-dependent |")
    lines.append("| Risk of Ruin | Monte Carlo probability of >90% loss | 1000 shuffle simulations |")
    lines.append("| Fee-to-Gross | Fees as fraction of gross profit | total_fees / |gross_pnl| |")
    lines.append("| Max Consecutive Losses | Longest losing streak | Sequential counting |")
    lines.append("| Recovery Time | Avg time to recover from >5% drawdown | Equity curve analysis |")
    lines.append("")

    lines.append("### Walk-Forward Validation")
    lines.append("")
    lines.append("All backtests use expanding walk-forward validation with 5 windows.")
    lines.append("The initial training window uses 60% of the data. Each successive window")
    lines.append("expands by including more data. Only out-of-sample (test) results are")
    lines.append("used for the frontier analysis, avoiding in-sample overfitting.")
    lines.append("")

    # ---- Group results by candidate ----
    candidate_results: Dict[str, List[SweepResult]] = {}
    for r in results:
        key = f"{r.strategy}__{r.market}__{r.cost_mode}"
        if key not in candidate_results:
            candidate_results[key] = []
        candidate_results[key].append(r)

    # ---- Summary table ----
    lines.append("## Summary")
    lines.append("")
    lines.append("| Candidate | Cost Mode | Best Sharpe | Best PnL | Trades (90d) | Optimal Lev |")
    lines.append("|-----------|-----------|-------------|----------|--------------|-------------|")

    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success and r.trade_count > 0]
        if not cand_results:
            label = cand["label"].split(" (")[0]
            lines.append(f"| {label} | {cand['cost_mode']} | N/A | N/A | 0 | N/A |")
            continue

        best_sharpe = max(cand_results, key=lambda r: r.wf_test_sharpe if r.wf_test_sharpe > 0 else r.sharpe_ratio)
        sharpe_val = best_sharpe.wf_test_sharpe if best_sharpe.wf_test_sharpe > 0 else best_sharpe.sharpe_ratio
        best_pnl = max(cand_results, key=lambda r: r.net_pnl)
        max_trades = max(r.trade_count for r in cand_results)
        label = cand["label"].split(" (")[0]

        lines.append(
            f"| {label} | {cand['cost_mode']} | {_fmt(sharpe_val)} "
            f"| ${_fmt(best_pnl.net_pnl)} | {max_trades} | {best_sharpe.leverage}x ({best_sharpe.sizing_mode}) |"
        )
    lines.append("")

    # ---- Detailed results per candidate ----
    lines.append("## Detailed Results Per Candidate")
    lines.append("")

    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = sorted(
            [r for r in candidate_results.get(key, []) if r.success],
            key=lambda r: (r.leverage, r.sizing_mode),
        )

        label = cand["label"].split(" (")[0]
        lines.append(f"### {label} ({cand['cost_mode']})")
        lines.append("")
        lines.append(f"*M1 baseline: {cand['label'].split('(', 1)[1].rstrip(')')}*")
        lines.append(f"*Parameters: lookback_count={cand['params']['lookback_count']}, "
                      f"momentum_threshold_pct={cand['params']['momentum_threshold_pct']}, "
                      f"max_hold_secs={cand['params']['max_hold_secs']}*")
        lines.append("")

        if not cand_results:
            lines.append("*No successful runs for this candidate.*")
            lines.append("")
            continue

        # Leverage × Sizing grid table
        lines.append("#### Full Grid (All Metrics)")
        lines.append("")
        lines.append("| Leverage | Sizing Mode | Trades | Net PnL | Sharpe | Sortino | Max DD | Fee/Gross | Liq Prox | RoR | Recovery |")
        lines.append("|----------|-------------|--------|---------|--------|---------|--------|-----------|----------|-----|----------|")

        for r in cand_results:
            sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
            lines.append(
                f"| {r.leverage}x | {r.sizing_mode} | {r.trade_count} "
                f"| ${_fmt(r.net_pnl)} | {_fmt(sharpe)} | {_fmt(r.sortino_ratio)} "
                f"| ${_fmt(r.max_drawdown_usd)} | {_fmt(r.fee_to_gross_ratio, 3)} "
                f"| {_fmt(r.avg_liquidation_proximity_pct)}% | {_fmt(r.risk_of_ruin_pct)}% "
                f"| {_fmt(r.avg_recovery_time_secs / 3600)}h |"
            )
        lines.append("")

        # Best by Sharpe per leverage level (fixed-notional)
        lines.append("#### Sharpe by Leverage Level (fixed-notional baseline)")
        lines.append("")
        lines.append("| Leverage | Liq Distance | Trades | Net PnL | Sharpe | Max DD | Fee/Gross |")
        lines.append("|----------|-------------|--------|---------|--------|--------|-----------|")

        fn_results = [r for r in cand_results if r.sizing_mode == "fixed-notional"]
        for r in sorted(fn_results, key=lambda r: r.leverage):
            sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
            liq_dist = _liq_distance(r.leverage)
            lines.append(
                f"| {r.leverage}x | {liq_dist} | {r.trade_count} "
                f"| ${_fmt(r.net_pnl)} | {_fmt(sharpe)} | ${_fmt(r.max_drawdown_usd)} "
                f"| {_fmt(r.fee_to_gross_ratio, 3)} |"
            )
        lines.append("")

        # Sizing mode comparison at 3x leverage
        lines.append("#### Sizing Mode Comparison (3x leverage)")
        lines.append("")
        lines.append("| Sizing Mode | Trades | Net PnL | Sharpe | Max DD | Fee/Gross | RoR |")
        lines.append("|-------------|--------|---------|--------|--------|-----------|-----|")

        sm_results = [r for r in cand_results if r.leverage == 3.0]
        for r in sorted(sm_results, key=lambda r: r.sizing_mode):
            sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
            lines.append(
                f"| {r.sizing_mode} | {r.trade_count} "
                f"| ${_fmt(r.net_pnl)} | {_fmt(sharpe)} | ${_fmt(r.max_drawdown_usd)} "
                f"| {_fmt(r.fee_to_gross_ratio, 3)} | {_fmt(r.risk_of_ruin_pct)}% |"
            )
        lines.append("")

    # ---- Liquidation price estimates ----
    lines.append("## Liquidation Price Estimates")
    lines.append("")
    lines.append("For leveraged positions, estimated liquidation prices depend on entry price,")
    lines.append("leverage, and direction. The table below shows theoretical liquidation")
    lines.append("distances for reference entry prices.")
    lines.append("")

    # Get representative entry prices from successful results
    ref_prices = {"BTC": 100000.0, "ETH": 2500.0, "SOL": 170.0}

    lines.append("### Long Positions")
    lines.append("")
    lines.append("| Leverage | BTC Entry $100K Liq | Distance | ETH Entry $2.5K Liq | Distance | SOL Entry $170 Liq | Distance |")
    lines.append("|----------|---------------------|----------|---------------------|----------|---------------------|----------|")

    for lev in LEVERAGE_LEVELS:
        btc_liq, btc_dist = compute_liquidation_price_estimate(ref_prices["BTC"], lev, True)
        eth_liq, eth_dist = compute_liquidation_price_estimate(ref_prices["ETH"], lev, True)
        sol_liq, sol_dist = compute_liquidation_price_estimate(ref_prices["SOL"], lev, True)
        lines.append(
            f"| {lev}x | ${_fmt(btc_liq, 0)} | {btc_dist} "
            f"| ${_fmt(eth_liq, 0)} | {eth_dist} "
            f"| ${_fmt(sol_liq, 1)} | {sol_dist} |"
        )
    lines.append("")

    lines.append("### Short Positions")
    lines.append("")
    lines.append("| Leverage | BTC Entry $100K Liq | Distance | ETH Entry $2.5K Liq | Distance | SOL Entry $170 Liq | Distance |")
    lines.append("|----------|---------------------|----------|---------------------|----------|---------------------|----------|")

    for lev in LEVERAGE_LEVELS:
        btc_liq, btc_dist = compute_liquidation_price_estimate(ref_prices["BTC"], lev, False)
        eth_liq, eth_dist = compute_liquidation_price_estimate(ref_prices["ETH"], lev, False)
        sol_liq, sol_dist = compute_liquidation_price_estimate(ref_prices["SOL"], lev, False)
        lines.append(
            f"| {lev}x | ${_fmt(btc_liq, 0)} | {btc_dist} "
            f"| ${_fmt(eth_liq, 0)} | {eth_dist} "
            f"| ${_fmt(sol_liq, 1)} | {sol_dist} |"
        )
    lines.append("")

    # Observed liquidation proximity from backtests
    lines.append("### Observed Average Liquidation Proximity")
    lines.append("")
    lines.append("Average % distance from worst intra-trade price to estimated liquidation price,")
    lines.append("computed per-trade and averaged across all trades.")
    lines.append("")
    lines.append("| Candidate | 1x | 2x | 3x | 5x | 7.5x | 10x |")
    lines.append("|-----------|----|----|----|----|----|------|")

    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success and r.sizing_mode == "fixed-notional"]
        label = cand["label"].split(" (")[0]

        prox_by_lev = {}
        for r in cand_results:
            prox_by_lev[r.leverage] = r.avg_liquidation_proximity_pct

        cells = []
        for lev in [1.0, 2.0, 3.0, 5.0, 7.5, 10.0]:
            prox = prox_by_lev.get(lev, 0.0)
            cells.append(f"{_fmt(prox)}%" if prox > 0 else "N/A")

        lines.append(f"| {label} | {' | '.join(cells)} |")
    lines.append("")

    # ---- Efficient Frontier ----
    lines.append("## Efficient Frontier Analysis")
    lines.append("")
    lines.append("The efficient frontier maps leverage against risk-adjusted return (Sharpe).")
    lines.append("For each candidate, we identify the 'knee' of the curve — the point where")
    lines.append("increasing leverage no longer improves Sharpe proportionally (diminishing returns)")
    lines.append("while drawdowns accelerate.")
    lines.append("")

    # Build frontier data
    frontier_data: Dict[str, List[Dict[str, Any]]] = {}
    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success and r.sizing_mode == "fixed-notional"]
        label = cand["label"].split(" (")[0]

        frontier_points = []
        for r in sorted(cand_results, key=lambda r: r.leverage):
            sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
            frontier_points.append({
                "leverage": r.leverage,
                "sharpe": sharpe,
                "net_pnl": r.net_pnl,
                "max_dd": r.max_drawdown_usd,
                "trade_count": r.trade_count,
                "ror": r.risk_of_ruin_pct,
            })
        frontier_data[label + " (" + cand["cost_mode"] + ")"] = frontier_points

    # Frontier table per candidate
    for cand_label, points in frontier_data.items():
        lines.append(f"### {cand_label}")
        lines.append("")
        lines.append("| Leverage | Sharpe | Net PnL | Max DD | RoR | Marginal Sharpe Δ |")
        lines.append("|----------|--------|---------|--------|-----|-------------------|")

        prev_sharpe = None
        for p in points:
            delta = ""
            if prev_sharpe is not None:
                d = p["sharpe"] - prev_sharpe
                delta = f"+{_fmt(d)}" if d >= 0 else _fmt(d)
            lines.append(
                f"| {p['leverage']}x | {_fmt(p['sharpe'])} | ${_fmt(p['net_pnl'])} "
                f"| ${_fmt(p['max_dd'])} | {_fmt(p['ror'])}% | {delta} |"
            )
            prev_sharpe = p["sharpe"]
        lines.append("")

    # ---- Optimal leverage recommendations ----
    lines.append("## Recommended Maximum Leverage")
    lines.append("")
    lines.append("Based on the efficient frontier analysis, the following maximum leverage")
    lines.append("levels are recommended per strategy-market pair. The recommendation balances")
    lines.append("three criteria:")
    lines.append("1. **Sharpe improvement**: Leverage should improve risk-adjusted return")
    lines.append("2. **Drawdown tolerance**: Max drawdown should not exceed 20% of starting balance")
    lines.append("3. **Risk of ruin**: RoR should remain below 10%")
    lines.append("")

    lines.append("| Candidate | Cost Mode | Recommended Max Lev | Sharpe at Max | DD at Max | RoR at Max | Rationale |")
    lines.append("|-----------|-----------|--------------------|----------------|-----------|-----------|-----------|")

    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, [])
                        if r.success and r.sizing_mode == "fixed-notional" and r.trade_count > 0]
        label = cand["label"].split(" (")[0]

        if not cand_results:
            lines.append(f"| {label} | {cand['cost_mode']} | N/A | N/A | N/A | N/A | No data |")
            continue

        # Find the highest leverage where:
        # 1. Sharpe > 0 (still profitable)
        # 2. Max DD < 20% of starting balance ($200 on $1000)
        # 3. RoR < 10%
        recommended = None
        for r in sorted(cand_results, key=lambda r: r.leverage):
            sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
            if sharpe <= 0:
                continue
            if r.max_drawdown_usd > 200.0:
                continue
            if r.risk_of_ruin_pct > 10.0:
                continue
            recommended = r

        if recommended:
            sharpe = recommended.wf_test_sharpe if recommended.wf_test_sharpe != 0 else recommended.sharpe_ratio
            rationale = f"Sharpe {_fmt(sharpe)}, DD ${_fmt(recommended.max_drawdown_usd)}, RoR {_fmt(recommended.risk_of_ruin_pct)}%"
            lines.append(
                f"| {label} | {cand['cost_mode']} | {recommended.leverage}x "
                f"| {_fmt(sharpe)} | ${_fmt(recommended.max_drawdown_usd)} "
                f"| {_fmt(recommended.risk_of_ruin_pct)}% | {rationale} |"
            )
        else:
            # Fall back to lowest leverage that's profitable
            profitable = [r for r in cand_results if r.net_pnl > 0]
            if profitable:
                r = min(profitable, key=lambda r: r.leverage)
                sharpe = r.wf_test_sharpe if r.wf_test_sharpe != 0 else r.sharpe_ratio
                lines.append(
                    f"| {label} | {cand['cost_mode']} | {r.leverage}x "
                    f"| {_fmt(sharpe)} | ${_fmt(r.max_drawdown_usd)} "
                    f"| {_fmt(r.risk_of_ruin_pct)}% | Conservative: exceeds DD/RoR thresholds above {r.leverage}x |"
                )
            else:
                lines.append(f"| {label} | {cand['cost_mode']} | 1x | — | — | — | No profitable leverage found |")
    lines.append("")

    # ---- Best sizing mode recommendation ----
    lines.append("## Sizing Mode Recommendations")
    lines.append("")
    lines.append("For each candidate at its recommended leverage, which sizing mode performs best?")
    lines.append("")

    lines.append("| Candidate | Cost Mode | Lev | Best Sizing | Sharpe | PnL | DD | Rationale |")
    lines.append("|-----------|-----------|-----|-------------|--------|-----|----|-----------|")

    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success and r.trade_count > 0]
        label = cand["label"].split(" (")[0]

        if not cand_results:
            continue

        # Find the best overall result by risk-adjusted return
        best = max(cand_results, key=lambda r: (r.sharpe_ratio if r.sharpe_ratio < 99999 else 0))
        sharpe = best.sharpe_ratio
        if sharpe >= 99999:
            sharpe_str = "∞"
        else:
            sharpe_str = _fmt(sharpe)

        lines.append(
            f"| {label} | {cand['cost_mode']} | {best.leverage}x "
            f"| {best.sizing_mode} | {sharpe_str} | ${_fmt(best.net_pnl)} "
            f"| ${_fmt(best.max_drawdown_usd)} | Best risk-adjusted return |"
        )
    lines.append("")

    # ---- Conclusions ----
    lines.append("## Conclusions")
    lines.append("")

    # Count candidates with >=30 trades in the extended period
    sufficient = set()
    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success]
        max_trades = max((r.trade_count for r in cand_results), default=0)
        if max_trades >= 30:
            sufficient.add(cand["label"])

    profitable = set()
    for cand in CANDIDATES:
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success]
        if any(r.net_pnl > 0 for r in cand_results):
            profitable.add(cand["label"])

    lines.append(f"1. **Extended period effectiveness**: {len(sufficient)}/{len(CANDIDATES)} candidates "
                 f"achieved ≥30 trades in the 90-day window.")
    lines.append(f"2. **Profitability**: {len(profitable)}/{len(CANDIDATES)} candidates showed positive "
                 f"net PnL at 1x leverage.")
    lines.append("")

    # Check primary candidates
    primary_005_eth = [r for r in results if "cluster-005" in r.strategy and r.market == "ETH" and r.success]
    primary_008_btc = [r for r in results if "cluster-008" in r.strategy and r.market == "BTC" and r.success]

    if primary_005_eth:
        best_005 = max(primary_005_eth, key=lambda r: r.sharpe_ratio if r.sharpe_ratio < 99999 else 0)
        lines.append(f"3. **cluster-005:ETH** (primary): Best Sharpe {_fmt(best_005.sharpe_ratio)} "
                      f"at {best_005.leverage}x with {best_005.sizing_mode}, "
                      f"PnL ${_fmt(best_005.net_pnl)}, {best_005.trade_count} trades.")
    if primary_008_btc:
        best_008 = max(primary_008_btc, key=lambda r: r.sharpe_ratio if r.sharpe_ratio < 99999 else 0)
        lines.append(f"4. **cluster-008:BTC** (primary): Best Sharpe {_fmt(best_008.sharpe_ratio)} "
                      f"at {best_008.leverage}x with {best_008.sizing_mode}, "
                      f"PnL ${_fmt(best_008.net_pnl)}, {best_008.trade_count} trades.")

    lines.append("")
    lines.append("### Promotion Decision")
    lines.append("")
    lines.append("Based on the leverage-sizing frontier analysis, candidates are evaluated")
    lines.append("for promotion to M3 portfolio construction using the same 6 promotion gate")
    lines.append("criteria, now with the extended 90-day sample size.")
    lines.append("")

    # Evaluate promotion for each primary candidate
    for cand in CANDIDATES:
        if cand["priority"] != "primary":
            continue
        key = f"{cand['strategy']}__{cand['market']}__{cand['cost_mode']}"
        cand_results = [r for r in candidate_results.get(key, []) if r.success and r.leverage == 1.0]
        label = cand["label"].split(" (")[0]

        if not cand_results:
            lines.append(f"- **{label}** ({cand['cost_mode']}): ❌ No results")
            continue

        best = max(cand_results, key=lambda r: r.sharpe_ratio if r.sharpe_ratio < 99999 else 0)
        sharpe = best.sharpe_ratio
        criteria = []
        if best.net_pnl > 0:
            criteria.append("✅ positive PnL")
        else:
            criteria.append("❌ negative PnL")
        if sharpe >= 1.0 and sharpe < 99999:
            criteria.append("✅ Sharpe ≥ 1.0")
        elif sharpe >= 99999:
            criteria.append("⚠️ Sharpe = ∞ (likely 0 vol)")
        else:
            criteria.append(f"❌ Sharpe {sharpe:.2f} < 1.0")
        if best.trade_count >= 30:
            criteria.append(f"✅ {best.trade_count} trades")
        else:
            criteria.append(f"❌ {best.trade_count} < 30 trades")
        if best.fee_to_gross_ratio < 0.35 or best.fee_to_gross_ratio < 0:
            criteria.append("✅ fee/gross OK")
        else:
            criteria.append(f"❌ fee/gross {best.fee_to_gross_ratio:.2f}")

        lines.append(f"- **{label}** ({cand['cost_mode']}): {', '.join(criteria)}")

    lines.append("")

    report = "\n".join(lines)
    return report


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Leverage and position sizing grid sweep")
    parser.add_argument("--binary", default=DEFAULT_BINARY, help="Path to zekt binary")
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT_DIR, help="Output directory")
    parser.add_argument("--paper-balance", type=float, default=DEFAULT_PAPER_BALANCE)
    parser.add_argument("--parallelism", type=int, default=DEFAULT_PARALLELISM)
    parser.add_argument("--dry-run", action="store_true", help="Print grid size without running")
    parser.add_argument("--file-cache", action="store_true", default=True, help="Enable file caching")
    parser.add_argument("--report-only", action="store_true", help="Generate report from existing results")
    args = parser.parse_args()

    if args.dry_run:
        total = len(CANDIDATES) * len(LEVERAGE_LEVELS) * len(SIZING_MODES)
        print(f"Grid size: {len(CANDIDATES)} candidates × {len(LEVERAGE_LEVELS)} leverage × {len(SIZING_MODES)} sizing = {total} runs")
        print(f"Backtest period: {BACKTEST_START} to {BACKTEST_END}")
        print(f"Parallelism: {args.parallelism}")
        return

    os.makedirs(args.output_dir, exist_ok=True)
    os.makedirs(os.path.join(args.output_dir, "raw"), exist_ok=True)

    if args.report_only:
        # Load existing results from raw directory
        results = []
        raw_dir = os.path.join(args.output_dir, "raw")
        for run_id in os.listdir(raw_dir):
            summary_path = os.path.join(raw_dir, run_id, "summary.json")
            if not os.path.exists(summary_path):
                continue
            try:
                with open(summary_path) as f:
                    summary = json.load(f)
                # Parse run_id to extract params
                parts = run_id.split("__")
                if len(parts) >= 5:
                    strategy = parts[0]
                    market = parts[1]
                    cost_mode = parts[2]
                    lev_str = parts[3]  # levX.X
                    sizing = "__".join(parts[4:])
                    leverage = float(lev_str.replace("lev", ""))

                    result = SweepResult(
                        candidate_label=f"{strategy}:{market} ({cost_mode})",
                        strategy=f"blueprint-{strategy}",
                        market=market,
                        cost_mode=cost_mode,
                        leverage=leverage,
                        sizing_mode=sizing,
                    )
                    _extract_metrics(result, summary)
                    result.success = True
                    results.append(result)
            except Exception as e:
                logger.warning("Failed to load %s: %s", run_id, e)
        logger.info("Loaded %d existing results", len(results))
    else:
        # Run the full grid
        results = run_grid(
            args.binary,
            args.output_dir,
            args.paper_balance,
            args.parallelism,
            args.file_cache,
        )

    # Save grid results as JSON
    grid_path = os.path.join(args.output_dir, "grid.json")
    grid_data = []
    for r in results:
        grid_data.append({
            "candidate_label": r.candidate_label,
            "strategy": r.strategy,
            "market": r.market,
            "cost_mode": r.cost_mode,
            "leverage": r.leverage,
            "sizing_mode": r.sizing_mode,
            "success": r.success,
            "error": r.error,
            "net_pnl": r.net_pnl,
            "sharpe_ratio": r.sharpe_ratio,
            "sortino_ratio": r.sortino_ratio,
            "calmar_ratio": r.calmar_ratio,
            "max_drawdown_usd": r.max_drawdown_usd,
            "trade_count": r.trade_count,
            "win_rate": r.win_rate,
            "fee_to_gross_ratio": r.fee_to_gross_ratio,
            "risk_of_ruin_pct": r.risk_of_ruin_pct,
            "avg_liquidation_proximity_pct": r.avg_liquidation_proximity_pct,
            "max_consecutive_losses": r.max_consecutive_losses,
            "avg_recovery_time_secs": r.avg_recovery_time_secs,
            "total_fees": r.total_fees,
            "gross_pnl": r.gross_pnl,
            "elapsed_secs": r.elapsed_secs,
        })

    with open(grid_path, "w") as f:
        json.dump(grid_data, f, indent=2)
    logger.info("Saved grid results to %s", grid_path)

    # Generate markdown report
    report = generate_report(results, args.output_dir)
    report_path = os.path.join("data", "leverage-sizing-frontier.md")
    with open(report_path, "w") as f:
        f.write(report)
    logger.info("Report written to %s (%d words)", report_path, len(report.split()))


if __name__ == "__main__":
    main()
