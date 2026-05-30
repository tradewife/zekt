#!/usr/bin/env python3
"""Batch parameter grid runner for Zekt backtest engine.

Reads a JSON grid specification, generates all parameter combinations,
runs the Rust backtest binary per combination, and produces aggregated
rankings sorted by out-of-sample Sharpe ratio.

Usage:
    python scripts/param-search.py --grid data/param-grid-spec.json
    python scripts/param-search.py --grid data/param-grid-spec.json --dry-run
    python scripts/param-search.py --grid data/param-grid-spec.json --parallelism 4

Grid spec format:
    {
      "candidates": [
        {
          "strategy": "blueprint-cluster-007",
          "market": "BTC",
          "cost_modes": ["flash-only", "imperial-route-oracle"]
        }
      ],
      "parameter_grid": {
        "momentum_threshold_pct": [0.10, 0.15, 0.20],
        "take_profit_pct": [0.5, 1.0, 1.5]
      },
      "leverage": [1.0, 2.0, 3.0],  // optional, default: no leverage sweep
      "walk_forward": {
        "mode": "expanding",  // "single" or "expanding"
        "windows": 5          // only for expanding mode
      },
      "backtest_period": {
        "start": "2026-04-01",
        "end": "2026-05-30",
        "interval": "5m"
      },
      "parallelism": 8,          // max 8 concurrent processes
      "output_dir": "data/param-search-results",
      "paper_balance": 1000.0    // optional, default 1000
    }

Output:
    {output_dir}/rankings.json    — All combinations ranked by OOS Sharpe
    {output_dir}/stability.json   — Per-window stability analysis
    {output_dir}/report.json      — Full report with rankings + failed runs
    {output_dir}/raw/             — Per-combination output directories

Validation assertions fulfilled:
    VAL-M1-031: Reads grid spec JSON, validates, reports total combinations
    VAL-M1-032: Invokes Rust binary per combination with correct flags
    VAL-M1-033: Collects results, produces rankings sorted by test Sharpe
    VAL-M1-034: Handles individual failures gracefully
    VAL-M1-039: Test coverage for grid parsing and result aggregation
"""

import argparse
import itertools
import json
import logging
import os
import re
import subprocess
import sys
import tempfile
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

MAX_PARALLELISM = 8
DEFAULT_PARALLELISM = 4
DEFAULT_OUTPUT_DIR = "data/param-search-results"
DEFAULT_PAPER_BALANCE = 1000.0
DEFAULT_BINARY = "./target/release/zekt"


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass
class RunResult:
    """Result from a single backtest combination run."""
    combo_id: str
    strategy: str
    market: str
    cost_mode: str
    params: Dict[str, Any]
    leverage: Optional[float] = None
    success: bool = False
    error: Optional[str] = None
    summary: Optional[Dict[str, Any]] = None
    elapsed_secs: float = 0.0


# ---------------------------------------------------------------------------
# Grid spec loading and validation
# ---------------------------------------------------------------------------

def load_grid_spec(path: str) -> Dict[str, Any]:
    """Load grid specification from JSON file.

    Args:
        path: Path to the JSON grid spec file.

    Returns:
        Parsed and validated grid specification dict.

    Raises:
        FileNotFoundError: If the file doesn't exist.
        json.JSONDecodeError: If the file is not valid JSON.
        ValueError: If the grid spec is missing required fields.
    """
    with open(path) as f:
        spec = json.load(f)
    return validate_grid_spec(spec)


def validate_grid_spec(spec: Dict[str, Any]) -> Dict[str, Any]:
    """Validate grid specification structure and apply defaults.

    Required fields: candidates, parameter_grid, backtest_period.
    Optional fields: walk_forward, parallelism, output_dir, leverage, paper_balance.

    Args:
        spec: Raw grid specification dict.

    Returns:
        Validated grid specification with defaults applied.

    Raises:
        ValueError: If required fields are missing or invalid.
    """
    # Required: candidates
    if "candidates" not in spec or not spec["candidates"]:
        raise ValueError("Grid spec must have non-empty 'candidates' array")
    for i, c in enumerate(spec["candidates"]):
        if "strategy" not in c:
            raise ValueError(f"Candidate {i} missing 'strategy' field")
        if "cost_modes" not in c or not c["cost_modes"]:
            raise ValueError(f"Candidate {i} missing or empty 'cost_modes' field")

    # Required: parameter_grid
    if "parameter_grid" not in spec or not spec["parameter_grid"]:
        raise ValueError("Grid spec must have non-empty 'parameter_grid' dict")

    # Required: backtest_period
    if "backtest_period" not in spec:
        raise ValueError("Grid spec must have 'backtest_period' section")
    bp = spec["backtest_period"]
    if "start" not in bp or "end" not in bp:
        raise ValueError("'backtest_period' must have 'start' and 'end' fields")

    # Apply defaults
    spec.setdefault("walk_forward", {"mode": "expanding", "windows": 5})
    wf = spec["walk_forward"]
    wf.setdefault("mode", "expanding")
    wf.setdefault("windows", 5)

    spec.setdefault("parallelism", DEFAULT_PARALLELISM)
    spec["parallelism"] = max(1, min(spec["parallelism"], MAX_PARALLELISM))

    spec.setdefault("output_dir", DEFAULT_OUTPUT_DIR)
    spec.setdefault("paper_balance", DEFAULT_PAPER_BALANCE)
    spec.setdefault("leverage", None)  # No leverage sweep by default

    return spec


# ---------------------------------------------------------------------------
# Combination generation
# ---------------------------------------------------------------------------

def generate_combinations(parameter_grid: Dict[str, List[Any]]) -> List[Dict[str, Any]]:
    """Generate all parameter combinations (Cartesian product).

    Args:
        parameter_grid: Dict mapping parameter names to lists of values.

    Returns:
        List of dicts, each representing one parameter combination.
    """
    keys = sorted(parameter_grid.keys())
    value_lists = [parameter_grid[k] for k in keys]
    combos = []
    for values in itertools.product(*value_lists):
        combo = dict(zip(keys, values))
        combos.append(combo)
    return combos


def compute_total_runs(spec: Dict[str, Any]) -> int:
    """Compute total number of backtest runs from grid spec.

    Total = sum(cost_modes per candidate) × param_combos × leverage_levels.
    """
    num_param_combos = len(generate_combinations(spec["parameter_grid"]))
    num_leverage = len(spec["leverage"]) if spec.get("leverage") else 1
    total_cost_modes = sum(len(c["cost_modes"]) for c in spec["candidates"])
    return total_cost_modes * num_param_combos * num_leverage


# ---------------------------------------------------------------------------
# Run ID generation
# ---------------------------------------------------------------------------

def _sanitize_fs(name: str) -> str:
    """Sanitize a string to be filesystem-safe (no spaces, slashes, colons, etc.)."""
    return re.sub(r"[^a-zA-Z0-9_\-.]", "_", name)


def make_run_id(
    strategy: str,
    market: str,
    cost_mode: str,
    params: Dict[str, Any],
    leverage: Optional[float],
    index: int,
) -> str:
    """Generate a unique, filesystem-safe run ID.

    Format: {strategy}__{market}__{cost_mode}__{sanitized_params}[_lev{X}].{index}
    """
    # Sanitize params into a short hash-like string
    param_str = "_".join(f"{k}-{v}" for k, v in sorted(params.items()))
    param_str = _sanitize_fs(param_str)
    # Truncate if too long
    if len(param_str) > 80:
        param_str = param_str[:80]

    parts = [
        _sanitize_fs(strategy),
        _sanitize_fs(market),
        _sanitize_fs(cost_mode),
        param_str,
    ]
    if leverage is not None:
        parts.append(f"lev{leverage}")

    run_id = "__".join(parts) + f".{index}"
    return run_id


# ---------------------------------------------------------------------------
# Command building
# ---------------------------------------------------------------------------

def build_command(
    binary_path: str,
    strategy: str,
    market: str,
    cost_mode: str,
    params: Dict[str, Any],
    leverage: Optional[float],
    walk_forward: Dict[str, Any],
    backtest_period: Dict[str, Any],
    output_dir: str,
    paper_balance: float = DEFAULT_PAPER_BALANCE,
) -> List[str]:
    """Build the Rust binary invocation command.

    Args:
        binary_path: Path to the zekt binary.
        strategy: Strategy name.
        market: Market symbol.
        cost_mode: "flash-only" or "imperial-route-oracle".
        params: Parameter override dict.
        leverage: Optional leverage value.
        walk_forward: Walk-forward config dict.
        backtest_period: Backtest period config dict.
        output_dir: Output directory for this run.
        paper_balance: Starting balance.

    Returns:
        Command as list of strings for subprocess.run().
    """
    cmd = [
        binary_path,
        "--backtest",
        "--strategies", strategy,
        "--markets", market,
        "--cost-mode", cost_mode,
        "--backtest-start", backtest_period["start"],
        "--backtest-end", backtest_period["end"],
        "--backtest-interval", backtest_period.get("interval", "5m"),
        "--paper-balance", str(paper_balance),
        "--output-path", output_dir,
        "--walk-forward-mode", walk_forward.get("mode", "expanding"),
    ]

    # Walk-forward windows (only for expanding mode)
    if walk_forward.get("mode", "single") == "expanding":
        cmd.extend(["--walk-forward-windows", str(walk_forward.get("windows", 5))])

    # Leverage override
    if leverage is not None:
        cmd.extend(["--leverage", str(leverage)])

    # Parameter overrides
    if params:
        cmd.extend(["--param-override", json.dumps(params)])

    return cmd


# ---------------------------------------------------------------------------
# Run execution
# ---------------------------------------------------------------------------

def run_single_combination(
    binary_path: str,
    strategy: str,
    market: str,
    cost_mode: str,
    params: Dict[str, Any],
    leverage: Optional[float],
    walk_forward: Dict[str, Any],
    backtest_period: Dict[str, Any],
    output_base_dir: str,
    combo_id: str,
    paper_balance: float = DEFAULT_PAPER_BALANCE,
    timeout_secs: int = 600,
) -> RunResult:
    """Run a single backtest combination.

    Args:
        binary_path: Path to the zekt binary.
        strategy: Strategy name.
        market: Market symbol.
        cost_mode: Cost mode string.
        params: Parameter overrides.
        leverage: Optional leverage.
        walk_forward: Walk-forward config.
        backtest_period: Backtest period config.
        output_base_dir: Base output directory.
        combo_id: Unique run identifier.
        paper_balance: Starting balance.
        timeout_secs: Timeout per run in seconds.

    Returns:
        RunResult with success status and collected data.
    """
    run_dir = os.path.join(output_base_dir, "raw", combo_id)
    os.makedirs(run_dir, exist_ok=True)

    cmd = build_command(
        binary_path=binary_path,
        strategy=strategy,
        market=market,
        cost_mode=cost_mode,
        params=params,
        leverage=leverage,
        walk_forward=walk_forward,
        backtest_period=backtest_period,
        output_dir=run_dir,
        paper_balance=paper_balance,
    )

    logger.debug("Running: %s", " ".join(cmd))
    start_time = time.time()

    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
        )
        elapsed = time.time() - start_time

        if proc.returncode != 0:
            stderr_tail = (proc.stderr or "")[-500:]
            error_msg = f"Exit code {proc.returncode}: {stderr_tail}"
            logger.warning("FAILED %s: %s", combo_id, error_msg[:200])
            return RunResult(
                combo_id=combo_id,
                strategy=strategy,
                market=market,
                cost_mode=cost_mode,
                params=params,
                leverage=leverage,
                success=False,
                error=error_msg,
                elapsed_secs=elapsed,
            )

        # Collect results
        result = collect_run_result(
            run_dir=run_dir,
            combo_id=combo_id,
            strategy=strategy,
            market=market,
            cost_mode=cost_mode,
            params=params,
            leverage=leverage,
        )
        result.elapsed_secs = elapsed

        if result.success:
            trade_count = 0
            if result.summary:
                trade_count = result.summary.get("total_trades", 0)
            logger.info(
                "OK %s: %d trades, %.1fs", combo_id, trade_count, elapsed
            )
        else:
            logger.warning("COLLECT FAILED %s: %s", combo_id, result.error)

        return result

    except subprocess.TimeoutExpired:
        elapsed = time.time() - start_time
        logger.error("TIMEOUT %s after %ds", combo_id, timeout_secs)
        return RunResult(
            combo_id=combo_id,
            strategy=strategy,
            market=market,
            cost_mode=cost_mode,
            params=params,
            leverage=leverage,
            success=False,
            error=f"Timeout after {timeout_secs}s",
            elapsed_secs=elapsed,
        )
    except Exception as e:
        elapsed = time.time() - start_time
        logger.error("ERROR %s: %s", combo_id, str(e))
        return RunResult(
            combo_id=combo_id,
            strategy=strategy,
            market=market,
            cost_mode=cost_mode,
            params=params,
            leverage=leverage,
            success=False,
            error=str(e),
            elapsed_secs=elapsed,
        )


# ---------------------------------------------------------------------------
# Result collection
# ---------------------------------------------------------------------------

def collect_run_result(
    run_dir: str,
    combo_id: str,
    strategy: str,
    market: str,
    cost_mode: str,
    params: Dict[str, Any],
    leverage: Optional[float] = None,
) -> RunResult:
    """Collect results from a completed run's summary.json.

    Args:
        run_dir: Directory where summary.json should exist.
        combo_id: Run identifier.
        strategy: Strategy name.
        market: Market symbol.
        cost_mode: Cost mode string.
        params: Parameter overrides used.
        leverage: Leverage used.

    Returns:
        RunResult with summary data or error.
    """
    summary_path = os.path.join(run_dir, "summary.json")

    if not os.path.exists(summary_path):
        return RunResult(
            combo_id=combo_id,
            strategy=strategy,
            market=market,
            cost_mode=cost_mode,
            params=params,
            leverage=leverage,
            success=False,
            error=f"summary.json not found in {run_dir}",
        )

    try:
        with open(summary_path) as f:
            summary = json.load(f)
    except (json.JSONDecodeError, IOError) as e:
        return RunResult(
            combo_id=combo_id,
            strategy=strategy,
            market=market,
            cost_mode=cost_mode,
            params=params,
            leverage=leverage,
            success=False,
            error=f"Failed to parse summary.json: {e}",
        )

    return RunResult(
        combo_id=combo_id,
        strategy=strategy,
        market=market,
        cost_mode=cost_mode,
        params=params,
        leverage=leverage,
        success=True,
        summary=summary,
    )


# ---------------------------------------------------------------------------
# Aggregation and ranking
# ---------------------------------------------------------------------------

def _extract_oos_sharpe(summary: Dict[str, Any]) -> float:
    """Extract out-of-sample Sharpe from summary.

    If walk_forward_test_cells exist, returns mean Sharpe across test windows.
    Otherwise falls back to in-sample Sharpe from cells.
    """
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        sharpes = [
            c["sharpe_ratio"]
            for c in wf_cells
            if "sharpe_ratio" in c
        ]
        if sharpes:
            return sum(sharpes) / len(sharpes)

    # Fallback to in-sample cells
    cells = summary.get("cells", [])
    if cells:
        sharpes = [c["sharpe_ratio"] for c in cells if "sharpe_ratio" in c]
        if sharpes:
            return sum(sharpes) / len(sharpes)

    return 0.0


def _extract_oos_trade_count(summary: Dict[str, Any]) -> int:
    """Extract total out-of-sample trade count from summary."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        return sum(c.get("trade_count", 0) for c in wf_cells)

    cells = summary.get("cells", [])
    return sum(c.get("trade_count", 0) for c in cells)


def _extract_oos_net_pnl(summary: Dict[str, Any]) -> float:
    """Extract total out-of-sample net PnL from summary."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    if wf_cells:
        return sum(c.get("net_pnl", 0.0) for c in wf_cells)

    cells = summary.get("cells", [])
    return sum(c.get("net_pnl", 0.0) for c in cells)


def _extract_metrics(summary: Dict[str, Any]) -> Dict[str, Any]:
    """Extract key metrics from summary for ranking entry."""
    wf_cells = summary.get("walk_forward_test_cells", [])
    cells = summary.get("cells", [])

    # Aggregate metrics from walk-forward test cells or regular cells
    source = wf_cells if wf_cells else cells

    metrics = {}
    if source:
        # Aggregate across all cells
        metrics["total_trades"] = sum(c.get("trade_count", 0) for c in source)
        metrics["win_rate"] = (
            sum(c.get("win_rate", 0) * c.get("trade_count", 1) for c in source)
            / max(metrics["total_trades"], 1)
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

        # Per-window metrics for stability analysis
        if wf_cells:
            metrics["per_window"] = [
                {
                    "window": c.get("walk_forward_window", ""),
                    "sharpe_ratio": c.get("sharpe_ratio", 0.0),
                    "trade_count": c.get("trade_count", 0),
                    "net_pnl": c.get("net_pnl", 0.0),
                }
                for c in wf_cells
            ]

    # In-sample metrics (from full cells)
    if cells:
        metrics["in_sample_sharpe"] = sum(
            c.get("sharpe_ratio", 0.0) for c in cells
        ) / len(cells) if cells else 0.0

    return metrics


def _get_field(obj, name, default=None):
    """Get a field from either a dataclass or a dict."""
    if isinstance(obj, dict):
        return obj.get(name, default)
    return getattr(obj, name, default)


def aggregate_rankings(
    results: List[Any],
    output_path: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """Aggregate successful results into rankings sorted by OOS Sharpe.

    Args:
        results: List of RunResult objects or dicts with success/summary fields.
        output_path: Optional path to write rankings JSON.

    Returns:
        List of ranking entries sorted by descending OOS Sharpe.
    """
    rankings = []

    for r in results:
        if not _get_field(r, "success") or _get_field(r, "summary") is None:
            continue

        summary = _get_field(r, "summary")
        oos_sharpe = _extract_oos_sharpe(summary)
        oos_trades = _extract_oos_trade_count(summary)
        oos_pnl = _extract_oos_net_pnl(summary)
        metrics = _extract_metrics(summary)

        entry = {
            "combo_id": _get_field(r, "combo_id"),
            "strategy": _get_field(r, "strategy"),
            "market": _get_field(r, "market"),
            "cost_mode": _get_field(r, "cost_mode"),
            "params": _get_field(r, "params"),
            "leverage": _get_field(r, "leverage"),
            "oos_sharpe": oos_sharpe,
            "oos_trade_count": oos_trades,
            "oos_net_pnl": oos_pnl,
            "elapsed_secs": _get_field(r, "elapsed_secs", 0.0),
            **metrics,
        }
        rankings.append(entry)

    # Sort by descending OOS Sharpe
    rankings.sort(key=lambda x: x["oos_sharpe"], reverse=True)

    if output_path:
        atomic_write_json(output_path, rankings)

    return rankings


def aggregate_results_report(
    results: List[Any],
) -> Dict[str, Any]:
    """Build full results report with rankings and failed runs.

    Args:
        results: List of RunResult objects or dicts with success/error fields.

    Returns:
        Dict with 'rankings' and 'failed' arrays.
    """
    rankings = aggregate_rankings([r for r in results if _get_field(r, "success")])

    failed = []
    for r in results:
        if not _get_field(r, "success"):
            failed.append({
                "combo_id": _get_field(r, "combo_id"),
                "strategy": _get_field(r, "strategy"),
                "market": _get_field(r, "market"),
                "cost_mode": _get_field(r, "cost_mode"),
                "params": _get_field(r, "params"),
                "leverage": _get_field(r, "leverage"),
                "error": _get_field(r, "error"),
            })

    return {"rankings": rankings, "failed": failed}


def compute_stability(rankings: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Compute per-window stability analysis for rankings.

    For each ranked entry with per-window metrics, compute:
    - Sharpe std dev across windows
    - PnL consistency (fraction of windows with positive PnL)

    Args:
        rankings: Sorted rankings list from aggregate_rankings().

    Returns:
        List of stability entries.
    """
    stability = []
    for entry in rankings:
        per_window = entry.get("per_window", [])
        if not per_window:
            continue

        sharpes = [w["sharpe_ratio"] for w in per_window]
        pnls = [w["net_pnl"] for w in per_window]

        mean_sharpe = sum(sharpes) / len(sharpes) if sharpes else 0.0
        sharpe_std = (
            (sum((s - mean_sharpe) ** 2 for s in sharpes) / len(sharpes)) ** 0.5
            if len(sharpes) > 1
            else 0.0
        )
        pnl_consistency = (
            sum(1 for p in pnls if p > 0) / len(pnls)
            if pnls
            else 0.0
        )

        stability.append({
            "combo_id": entry["combo_id"],
            "strategy": entry["strategy"],
            "market": entry["market"],
            "cost_mode": entry["cost_mode"],
            "params": entry["params"],
            "leverage": entry["leverage"],
            "oos_sharpe": entry["oos_sharpe"],
            "sharpe_std_across_windows": sharpe_std,
            "pnl_consistency": pnl_consistency,
            "num_windows": len(per_window),
            "per_window": per_window,
        })

    return stability


# ---------------------------------------------------------------------------
# Atomic file writes
# ---------------------------------------------------------------------------

def atomic_write_json(path: str, data: Any) -> None:
    """Write JSON data to file atomically (write .tmp then rename).

    Args:
        path: Target file path.
        data: Data to serialize as JSON.
    """
    tmp_path = path + ".tmp"
    with open(tmp_path, "w") as f:
        json.dump(data, f, indent=2, default=str)
    os.replace(tmp_path, path)


# ---------------------------------------------------------------------------
# Batch execution
# ---------------------------------------------------------------------------

def build_all_tasks(
    spec: Dict[str, Any],
    output_dir: str,
) -> List[Dict[str, Any]]:
    """Build the full list of tasks from grid spec.

    Each task is a dict of kwargs for run_single_combination().
    """
    param_combos = generate_combinations(spec["parameter_grid"])
    leverages = spec.get("leverage") or [None]
    walk_forward = spec["walk_forward"]
    backtest_period = spec["backtest_period"]
    paper_balance = spec.get("paper_balance", DEFAULT_PAPER_BALANCE)

    tasks = []
    idx = 0

    for candidate in spec["candidates"]:
        strategy = candidate["strategy"]
        market = candidate["market"]

        for cost_mode in candidate["cost_modes"]:
            for params in param_combos:
                for lev in leverages:
                    combo_id = make_run_id(strategy, market, cost_mode, params, lev, idx)
                    tasks.append({
                        "strategy": strategy,
                        "market": market,
                        "cost_mode": cost_mode,
                        "params": params,
                        "leverage": lev,
                        "walk_forward": walk_forward,
                        "backtest_period": backtest_period,
                        "output_base_dir": output_dir,
                        "combo_id": combo_id,
                        "paper_balance": paper_balance,
                    })
                    idx += 1

    return tasks


def run_batch(
    spec: Dict[str, Any],
    binary_path: str = DEFAULT_BINARY,
    output_dir: Optional[str] = None,
    dry_run: bool = False,
    timeout_secs: int = 600,
) -> Dict[str, Any]:
    """Execute the full parameter grid search.

    Args:
        spec: Validated grid specification.
        binary_path: Path to the zekt binary.
        output_dir: Override output directory from spec.
        dry_run: If True, only print tasks without executing.
        timeout_secs: Timeout per run in seconds.

    Returns:
        Full results report dict with 'rankings' and 'failed' arrays.
    """
    if output_dir is None:
        output_dir = spec["output_dir"]

    os.makedirs(os.path.join(output_dir, "raw"), exist_ok=True)

    tasks = build_all_tasks(spec, output_dir)
    total = len(tasks)
    parallelism = spec["parallelism"]

    logger.info("Grid: %d combinations", total)
    logger.info("Parallelism: %d processes", parallelism)
    logger.info("Output: %s", output_dir)

    if dry_run:
        logger.info("DRY RUN — printing tasks without execution")
        for i, task in enumerate(tasks):
            cmd = build_command(
                binary_path=binary_path,
                strategy=task["strategy"],
                market=task["market"],
                cost_mode=task["cost_mode"],
                params=task["params"],
                leverage=task["leverage"],
                walk_forward=task["walk_forward"],
                backtest_period=task["backtest_period"],
                output_dir=os.path.join(output_dir, "raw", task["combo_id"]),
                paper_balance=task["paper_balance"],
            )
            logger.info("[%d/%d] %s", i + 1, total, " ".join(cmd))
        return {"rankings": [], "failed": [], "dry_run": True, "total_tasks": total}

    results: List[RunResult] = []
    completed = 0

    # Use ProcessPoolExecutor for parallel execution
    with ProcessPoolExecutor(max_workers=parallelism) as executor:
        future_to_task = {}
        for task in tasks:
            future = executor.submit(
                run_single_combination,
                binary_path=binary_path,
                timeout_secs=timeout_secs,
                **task,
            )
            future_to_task[future] = task

        for future in as_completed(future_to_task):
            completed += 1
            task = future_to_task[future]
            try:
                result = future.result()
                results.append(result)
            except Exception as e:
                logger.error(
                    "[%d/%d] EXCEPTION for %s: %s",
                    completed, total, task["combo_id"], str(e),
                )
                results.append(RunResult(
                    combo_id=task["combo_id"],
                    strategy=task["strategy"],
                    market=task["market"],
                    cost_mode=task["cost_mode"],
                    params=task["params"],
                    leverage=task["leverage"],
                    success=False,
                    error=f"Executor exception: {e}",
                ))

            # Progress log
            ok = sum(1 for r in results if r.success)
            fail = sum(1 for r in results if not r.success)
            logger.info(
                "Progress: %d/%d done (%d ok, %d failed)",
                completed, total, ok, fail,
            )

    # Build report
    report = aggregate_results_report(results)

    # Write outputs
    rankings = report["rankings"]
    failed = report["failed"]

    # Write rankings
    atomic_write_json(
        os.path.join(output_dir, "rankings.json"),
        rankings,
    )
    logger.info("Rankings: %d entries written to rankings.json", len(rankings))

    # Write stability analysis
    stability = compute_stability(rankings)
    atomic_write_json(
        os.path.join(output_dir, "stability.json"),
        stability,
    )
    logger.info("Stability: %d entries written to stability.json", len(stability))

    # Write full report
    full_report = {
        "total_tasks": total,
        "successful": len(rankings),
        "failed_count": len(failed),
        "rankings": rankings,
        "failed": failed,
    }
    atomic_write_json(
        os.path.join(output_dir, "report.json"),
        full_report,
    )

    logger.info(
        "COMPLETE: %d/%d successful, %d failed",
        len(rankings), total, len(failed),
    )

    return full_report


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Batch parameter grid runner for Zekt backtest engine",
    )
    parser.add_argument(
        "--grid", required=True,
        help="Path to JSON grid specification file",
    )
    parser.add_argument(
        "--binary", default=DEFAULT_BINARY,
        help=f"Path to zekt binary (default: {DEFAULT_BINARY})",
    )
    parser.add_argument(
        "--output-dir",
        help="Override output directory from grid spec",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print tasks without executing",
    )
    parser.add_argument(
        "--parallelism", type=int,
        help="Override parallelism from grid spec (max 8)",
    )
    parser.add_argument(
        "--timeout", type=int, default=600,
        help="Timeout per run in seconds (default: 600)",
    )
    args = parser.parse_args()

    # Load and validate grid spec
    logger.info("Loading grid spec from %s", args.grid)
    spec = load_grid_spec(args.grid)

    # Apply CLI overrides
    if args.parallelism:
        spec["parallelism"] = max(1, min(args.parallelism, MAX_PARALLELISM))

    # Report grid info
    total = compute_total_runs(spec)
    param_combos = len(generate_combinations(spec["parameter_grid"]))
    num_leverage = len(spec["leverage"]) if spec.get("leverage") else 1
    num_candidates = len(spec["candidates"])
    num_cost_modes = sum(len(c["cost_modes"]) for c in spec["candidates"])

    logger.info("Grid spec loaded:")
    logger.info("  Candidates: %d", num_candidates)
    logger.info("  Cost mode entries: %d", num_cost_modes)
    logger.info("  Parameter combinations: %d", param_combos)
    logger.info("  Leverage levels: %d", num_leverage)
    logger.info("  Total runs: %d", total)
    logger.info("  Parallelism: %d", spec["parallelism"])

    if total == 0:
        logger.error("No combinations to run — check grid spec")
        sys.exit(1)

    # Execute
    report = run_batch(
        spec=spec,
        binary_path=args.binary,
        output_dir=args.output_dir,
        dry_run=args.dry_run,
        timeout_secs=args.timeout,
    )

    # Print top results
    if report.get("rankings"):
        logger.info("Top 10 by out-of-sample Sharpe:")
        for i, entry in enumerate(report["rankings"][:10]):
            logger.info(
                "  #%d: %s (Sharpe=%.2f, trades=%d, PnL=$%.2f)",
                i + 1,
                entry["combo_id"],
                entry["oos_sharpe"],
                entry.get("oos_trade_count", 0),
                entry.get("oos_net_pnl", 0.0),
            )

    # Exit with error if all runs failed
    if not args.dry_run and report.get("failed_count", 0) == total:
        logger.error("All %d runs failed", total)
        sys.exit(1)


if __name__ == "__main__":
    main()
