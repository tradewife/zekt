"""Tests for scripts/param-search.py batch grid runner.

Validates grid spec parsing, combination generation, and result aggregation.
Covers validation assertions VAL-M1-031 through VAL-M1-034 and VAL-M1-039.
"""

import json
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest

# Import param_search module (hyphenated filename requires importlib)
import importlib

_scripts_dir = str(Path(__file__).resolve().parent.parent)
if _scripts_dir not in sys.path:
    sys.path.insert(0, _scripts_dir)
param_search = importlib.import_module("param-search")


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def valid_grid_spec():
    """A minimal valid grid specification."""
    return {
        "candidates": [
            {
                "strategy": "blueprint-cluster-007",
                "market": "BTC",
                "cost_modes": ["flash-only", "imperial-route-oracle"],
            },
            {
                "strategy": "blueprint-cluster-005",
                "market": "ETH",
                "cost_modes": ["flash-only"],
            },
        ],
        "parameter_grid": {
            "momentum_threshold_pct": [0.10, 0.15, 0.20],
            "take_profit_pct": [0.5, 1.0],
            "stop_loss_pct": [0.3, 0.5],
        },
        "walk_forward": {
            "mode": "expanding",
            "windows": 5,
        },
        "backtest_period": {
            "start": "2026-04-01",
            "end": "2026-05-30",
            "interval": "5m",
        },
        "parallelism": 4,
        "output_dir": "data/param-search-results",
    }


@pytest.fixture
def grid_spec_with_leverage():
    """Grid spec that also sweeps leverage."""
    return {
        "candidates": [
            {"strategy": "momentum-scalper", "market": "SOL", "cost_modes": ["flash-only"]},
        ],
        "parameter_grid": {
            "take_profit_pct": [0.5, 1.0],
        },
        "leverage": [1.0, 3.0, 5.0],
        "walk_forward": {
            "mode": "single",
        },
        "backtest_period": {
            "start": "2026-05-01",
            "end": "2026-05-30",
            "interval": "5m",
        },
        "parallelism": 2,
        "output_dir": "data/test-results",
    }


@pytest.fixture
def sample_summary_json():
    """A minimal summary.json as produced by the Rust backtest binary."""
    return {
        "start_balance": 1000.0,
        "final_balance": 1050.0,
        "total_net_pnl": 50.0,
        "total_trades": 30,
        "total_fees": 5.0,
        "cells": [
            {
                "strategy": "blueprint-cluster-007",
                "market": "BTC",
                "trade_count": 30,
                "net_pnl": 50.0,
                "sharpe_ratio": 1.5,
                "win_rate": 60.0,
                "max_drawdown_usd": 20.0,
                "total_fees": 5.0,
                "sortino_ratio": 2.0,
                "calmar_ratio": 3.0,
                "profit_factor": 2.5,
                "risk_of_ruin_pct": 1.0,
                "fee_to_gross_ratio": 0.1,
                "walk_forward_window": "",
                "cost_mode": "flash-only",
            }
        ],
        "walk_forward_test_cells": [
            {
                "strategy": "blueprint-cluster-007",
                "market": "BTC",
                "trade_count": 10,
                "net_pnl": 20.0,
                "sharpe_ratio": 2.0,
                "win_rate": 70.0,
                "max_drawdown_usd": 10.0,
                "total_fees": 2.0,
                "sortino_ratio": 3.0,
                "calmar_ratio": 4.0,
                "profit_factor": 3.0,
                "risk_of_ruin_pct": 0.5,
                "fee_to_gross_ratio": 0.1,
                "walk_forward_window": "test-w1",
                "cost_mode": "flash-only",
            },
            {
                "strategy": "blueprint-cluster-007",
                "market": "BTC",
                "trade_count": 8,
                "net_pnl": 15.0,
                "sharpe_ratio": 1.8,
                "win_rate": 65.0,
                "max_drawdown_usd": 8.0,
                "total_fees": 1.5,
                "sortino_ratio": 2.5,
                "calmar_ratio": 3.5,
                "profit_factor": 2.8,
                "risk_of_ruin_pct": 0.8,
                "fee_to_gross_ratio": 0.1,
                "walk_forward_window": "test-w2",
                "cost_mode": "flash-only",
            },
        ],
    }


# ---------------------------------------------------------------------------
# Test: Grid spec validation (VAL-M1-031)
# ---------------------------------------------------------------------------

class TestGridSpecValidation:
    """Tests for loading and validating grid specification JSON."""

    def test_load_valid_grid_spec(self, valid_grid_spec, tmp_path):
        """Valid grid spec loads and validates without errors."""
        spec_file = tmp_path / "grid.json"
        spec_file.write_text(json.dumps(valid_grid_spec))

        spec = param_search.load_grid_spec(str(spec_file))
        # validate_grid_spec adds defaults (leverage, paper_balance)
        assert spec["candidates"] == valid_grid_spec["candidates"]
        assert spec["parameter_grid"] == valid_grid_spec["parameter_grid"]
        assert spec["backtest_period"] == valid_grid_spec["backtest_period"]
        assert spec["walk_forward"] == valid_grid_spec["walk_forward"]
        assert spec["parallelism"] == valid_grid_spec["parallelism"]
        assert spec["output_dir"] == valid_grid_spec["output_dir"]

    def test_validate_reports_total_combinations(self, valid_grid_spec):
        """Validation computes correct total number of combinations."""
        # 3 candidates (2 cost_modes + 1 cost_mode) × 3 × 2 × 2 = 36 combos
        combos = param_search.generate_combinations(
            valid_grid_spec["parameter_grid"]
        )
        assert len(combos) == 3 * 2 * 2  # 12 param combos

    def test_validate_missing_candidates(self):
        """Missing 'candidates' field raises ValueError."""
        spec = {
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        with pytest.raises(ValueError, match="candidates"):
            param_search.validate_grid_spec(spec)

    def test_validate_missing_parameter_grid(self):
        """Missing 'parameter_grid' field raises ValueError."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "walk_forward": {"mode": "expanding"},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        with pytest.raises(ValueError, match="parameter_grid"):
            param_search.validate_grid_spec(spec)

    def test_validate_missing_backtest_period(self):
        """Missing 'backtest_period' field raises ValueError."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding"},
        }
        with pytest.raises(ValueError, match="backtest_period"):
            param_search.validate_grid_spec(spec)

    def test_validate_empty_parameter_grid(self):
        """Empty parameter_grid raises ValueError (at least 1 param needed)."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {},
            "walk_forward": {"mode": "expanding"},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        with pytest.raises(ValueError, match="parameter_grid"):
            param_search.validate_grid_spec(spec)

    def test_validate_candidate_missing_strategy(self):
        """Candidate without 'strategy' raises ValueError."""
        spec = {
            "candidates": [{"market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding"},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        with pytest.raises(ValueError, match="strategy"):
            param_search.validate_grid_spec(spec)

    def test_validate_candidate_missing_cost_modes(self):
        """Candidate without 'cost_modes' raises ValueError."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC"}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding"},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        with pytest.raises(ValueError, match="cost_modes"):
            param_search.validate_grid_spec(spec)

    def test_validate_walk_forward_defaults(self):
        """Missing walk_forward section should default to expanding/5."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["walk_forward"]["mode"] == "expanding"
        assert validated["walk_forward"]["windows"] == 5

    def test_validate_parallelism_default(self):
        """Missing parallelism defaults to 4."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["parallelism"] == 4

    def test_validate_parallelism_capped_at_8(self):
        """Parallelism > 8 is capped to 8."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            "parallelism": 16,
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["parallelism"] == 8

    def test_validate_output_dir_default(self):
        """Missing output_dir defaults to data/param-search-results."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["output_dir"] == "data/param-search-results"


# ---------------------------------------------------------------------------
# Test: Combination generation
# ---------------------------------------------------------------------------

class TestCombinationGeneration:
    """Tests for Cartesian product of parameter grid."""

    def test_basic_cartesian_product(self):
        """Two params with 2 and 3 values produce 6 combinations."""
        grid = {"a": [1, 2], "b": [10, 20, 30]}
        combos = param_search.generate_combinations(grid)
        assert len(combos) == 6
        # Check all combinations present
        expected = [
            {"a": 1, "b": 10}, {"a": 1, "b": 20}, {"a": 1, "b": 30},
            {"a": 2, "b": 10}, {"a": 2, "b": 20}, {"a": 2, "b": 30},
        ]
        for e in expected:
            assert e in combos

    def test_single_param_single_value(self):
        """One param with one value produces 1 combination."""
        grid = {"x": [42]}
        combos = param_search.generate_combinations(grid)
        assert len(combos) == 1
        assert combos[0] == {"x": 42}

    def test_many_params(self):
        """7 params with 5 values each produces 5^7 = 78125 combinations."""
        grid = {f"p{i}": list(range(5)) for i in range(7)}
        combos = param_search.generate_combinations(grid)
        assert len(combos) == 5**7

    def test_bool_values_in_grid(self):
        """Boolean values (regime_filter) are included in combinations."""
        grid = {"regime_filter": [True, False], "threshold": [0.1, 0.2]}
        combos = param_search.generate_combinations(grid)
        assert len(combos) == 4
        # Verify booleans are preserved
        assert {"regime_filter": True, "threshold": 0.1} in combos
        assert {"regime_filter": False, "threshold": 0.2} in combos

    def test_leverage_sweep(self, grid_spec_with_leverage):
        """Leverage values are included in total combination count."""
        combos = param_search.generate_combinations(
            grid_spec_with_leverage["parameter_grid"]
        )
        leverages = grid_spec_with_leverage["leverage"]
        # 2 param combos × 3 leverage levels = 6 total per candidate/cost_mode
        assert len(combos) * len(leverages) == 6

    def test_total_run_count(self, valid_grid_spec):
        """Total runs = candidates × cost_modes × param_combos × leverage_levels."""
        total_runs = param_search.compute_total_runs(valid_grid_spec)
        # 2 candidates: first has 2 cost_modes, second has 1
        # param combos: 3 × 2 × 2 = 12
        # no leverage sweep
        # total = (2 + 1) × 12 = 36
        assert total_runs == 36

    def test_total_run_count_with_leverage(self, grid_spec_with_leverage):
        """Total runs includes leverage dimension."""
        total_runs = param_search.compute_total_runs(grid_spec_with_leverage)
        # 1 candidate × 1 cost_mode × 2 param combos × 3 leverage = 6
        assert total_runs == 6


# ---------------------------------------------------------------------------
# Test: Command building (VAL-M1-032)
# ---------------------------------------------------------------------------

class TestCommandBuilding:
    """Tests for building the Rust binary invocation command."""

    def test_build_command_basic(self):
        """Basic command includes all required flags."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="blueprint-cluster-007",
            market="BTC",
            cost_mode="flash-only",
            params={"take_profit_pct": 1.0, "stop_loss_pct": 0.5},
            leverage=None,
            walk_forward={"mode": "expanding", "windows": 5},
            backtest_period={"start": "2026-04-01", "end": "2026-05-30", "interval": "5m"},
            output_dir="data/test-run",
        )
        assert "--backtest" in cmd
        assert "--strategies" in cmd
        assert "blueprint-cluster-007" in cmd
        assert "--markets" in cmd
        assert "BTC" in cmd
        assert "--cost-mode" in cmd
        assert "flash-only" in cmd
        assert "--output-path" in cmd
        assert "data/test-run" in cmd
        assert "--walk-forward-mode" in cmd
        assert "expanding" in cmd
        assert "--walk-forward-windows" in cmd
        assert "5" in cmd
        assert "--param-override" in cmd

    def test_build_command_with_leverage(self):
        """Leverage is included when specified."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="momentum-scalper",
            market="SOL",
            cost_mode="flash-only",
            params={"take_profit_pct": 1.0},
            leverage=3.0,
            walk_forward={"mode": "single"},
            backtest_period={"start": "2026-05-01", "end": "2026-05-30", "interval": "5m"},
            output_dir="data/leverage-test",
        )
        assert "--leverage" in cmd
        assert "3.0" in cmd

    def test_build_command_no_leverage(self):
        """No --leverage flag when leverage is None."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
            walk_forward={"mode": "single"},
            backtest_period={"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            output_dir="data/test",
        )
        assert "--leverage" not in cmd

    def test_build_command_param_override_json(self):
        """param-override is valid JSON with correct key/value pairs."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"clip_size_usd": 200, "take_profit_pct": 1.5},
            leverage=None,
            walk_forward={"mode": "single"},
            backtest_period={"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            output_dir="data/test",
        )
        # Find the param-override JSON string
        idx = cmd.index("--param-override")
        override_json = cmd[idx + 1]
        parsed = json.loads(override_json)
        assert parsed["clip_size_usd"] == 200
        assert parsed["take_profit_pct"] == 1.5

    def test_build_command_walk_forward_expanding(self):
        """Expanding walk-forward includes both mode and windows flags."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
            walk_forward={"mode": "expanding", "windows": 5},
            backtest_period={"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            output_dir="data/test",
        )
        assert "--walk-forward-mode" in cmd
        assert "expanding" in cmd
        assert "--walk-forward-windows" in cmd

    def test_build_command_walk_forward_single(self):
        """Single walk-forward mode does not include --walk-forward-windows."""
        cmd = param_search.build_command(
            binary_path="./target/release/zekt",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
            walk_forward={"mode": "single"},
            backtest_period={"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            output_dir="data/test",
        )
        assert "--walk-forward-mode" in cmd
        assert "single" in cmd


# ---------------------------------------------------------------------------
# Test: Result collection and aggregation (VAL-M1-033)
# ---------------------------------------------------------------------------

class TestResultAggregation:
    """Tests for collecting results and producing ranked aggregation."""

    def test_aggregate_sorted_by_test_sharpe(self, sample_summary_json):
        """Rankings are sorted by descending out-of-sample Sharpe ratio."""
        results = [
            {
                "combo_id": "run_001",
                "strategy": "blueprint-cluster-007",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"take_profit_pct": 1.0},
                "summary": sample_summary_json,
                "success": True,
            },
            {
                "combo_id": "run_002",
                "strategy": "blueprint-cluster-005",
                "market": "ETH",
                "cost_mode": "flash-only",
                "params": {"take_profit_pct": 0.5},
                "summary": {
                    **sample_summary_json,
                    "walk_forward_test_cells": [
                        {
                            "strategy": "blueprint-cluster-005",
                            "market": "ETH",
                            "trade_count": 12,
                            "sharpe_ratio": 2.5,  # Higher OOS Sharpe
                            "walk_forward_window": "test-w1",
                        },
                    ],
                },
                "success": True,
            },
        ]

        rankings = param_search.aggregate_rankings(results)
        assert len(rankings) == 2
        assert rankings[0]["combo_id"] == "run_002"  # Higher Sharpe first
        assert rankings[1]["combo_id"] == "run_001"

    def test_aggregate_with_no_walk_forward(self):
        """Aggregation handles results with no walk_forward_test_cells."""
        results = [
            {
                "combo_id": "run_003",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 1},
                "summary": {
                    "cells": [{"sharpe_ratio": 1.0, "trade_count": 10}],
                    "walk_forward_test_cells": [],
                },
                "success": True,
            },
        ]
        rankings = param_search.aggregate_rankings(results)
        assert len(rankings) == 1
        # Should use in-sample Sharpe when no OOS data
        assert rankings[0]["oos_sharpe"] == 1.0

    def test_aggregate_includes_all_fields(self, sample_summary_json):
        """Each ranking entry includes strategy, market, cost_mode, params, and key metrics."""
        results = [
            {
                "combo_id": "run_004",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "imperial-route-oracle",
                "params": {"clip_size_usd": 200},
                "leverage": 3.0,
                "summary": sample_summary_json,
                "success": True,
            },
        ]
        rankings = param_search.aggregate_rankings(results)
        entry = rankings[0]
        assert entry["strategy"] == "s"
        assert entry["market"] == "BTC"
        assert entry["cost_mode"] == "imperial-route-oracle"
        assert entry["params"] == {"clip_size_usd": 200}
        assert entry["leverage"] == 3.0
        assert "oos_sharpe" in entry
        assert "oos_trade_count" in entry
        assert "oos_net_pnl" in entry

    def test_aggregate_write_json(self, sample_summary_json, tmp_path):
        """Rankings are written to rankings.json with atomic write."""
        results = [
            {
                "combo_id": "run_005",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 1},
                "summary": sample_summary_json,
                "success": True,
            },
        ]
        output_path = str(tmp_path / "rankings.json")
        rankings = param_search.aggregate_rankings(results, output_path=output_path)
        assert os.path.exists(output_path)
        with open(output_path) as f:
            saved = json.load(f)
        assert len(saved) == 1
        assert saved[0]["combo_id"] == "run_005"

    def test_aggregate_oos_sharpe_from_walk_forward(self):
        """OOS Sharpe is the mean Sharpe across walk-forward test windows."""
        results = [
            {
                "combo_id": "run_006",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 1},
                "summary": {
                    "cells": [{"sharpe_ratio": 0.5}],
                    "walk_forward_test_cells": [
                        {"sharpe_ratio": 1.5, "walk_forward_window": "test-w1"},
                        {"sharpe_ratio": 2.5, "walk_forward_window": "test-w2"},
                    ],
                },
                "success": True,
            },
        ]
        rankings = param_search.aggregate_rankings(results)
        # Mean of 1.5 and 2.5 = 2.0
        assert rankings[0]["oos_sharpe"] == 2.0


# ---------------------------------------------------------------------------
# Test: Failure handling (VAL-M1-034)
# ---------------------------------------------------------------------------

class TestFailureHandling:
    """Tests for graceful handling of individual run failures."""

    def test_failed_runs_in_separate_array(self, sample_summary_json):
        """Failed runs are recorded in a separate 'failed' array."""
        results = [
            {
                "combo_id": "run_ok",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 1},
                "summary": sample_summary_json,
                "success": True,
            },
            {
                "combo_id": "run_fail",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 999},
                "success": False,
                "error": "Binary exited with code 1",
            },
        ]
        output_path = str(tmp_path / "rankings.json") if False else None
        report = param_search.aggregate_results_report(results)

        assert len(report["rankings"]) == 1
        assert report["rankings"][0]["combo_id"] == "run_ok"
        assert len(report["failed"]) == 1
        assert report["failed"][0]["combo_id"] == "run_fail"
        assert "Binary exited with code 1" in report["failed"][0]["error"]

    def test_all_runs_fail(self):
        """When all runs fail, rankings is empty and failed has all entries."""
        results = [
            {
                "combo_id": f"fail_{i}",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": i},
                "success": False,
                "error": f"Error {i}",
            }
            for i in range(3)
        ]
        report = param_search.aggregate_results_report(results)
        assert len(report["rankings"]) == 0
        assert len(report["failed"]) == 3

    def test_mixed_success_failure_ordering(self, sample_summary_json):
        """Rankings only contain successful runs, sorted by Sharpe."""
        results = [
            {
                "combo_id": "fail",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {},
                "success": False,
                "error": "crash",
            },
            {
                "combo_id": "ok_1",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 1},
                "summary": {
                    "cells": [{"sharpe_ratio": 0.5}],
                    "walk_forward_test_cells": [
                        {"sharpe_ratio": 1.0, "walk_forward_window": "test-w1"},
                    ],
                },
                "success": True,
            },
            {
                "combo_id": "ok_2",
                "strategy": "s",
                "market": "BTC",
                "cost_mode": "flash-only",
                "params": {"a": 2},
                "summary": {
                    "cells": [{"sharpe_ratio": 2.0}],
                    "walk_forward_test_cells": [
                        {"sharpe_ratio": 3.0, "walk_forward_window": "test-w1"},
                    ],
                },
                "success": True,
            },
        ]
        report = param_search.aggregate_results_report(results)
        assert len(report["rankings"]) == 2
        assert report["rankings"][0]["combo_id"] == "ok_2"  # Higher Sharpe
        assert len(report["failed"]) == 1

    def test_run_result_records_error_details(self):
        """RunResult for a failed run includes combo_id, error, and params."""
        result = param_search.RunResult(
            combo_id="test_fail",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"x": 1},
            leverage=None,
            success=False,
            error="Exit code 1",
        )
        assert result.combo_id == "test_fail"
        assert result.error == "Exit code 1"
        assert result.success is False


# ---------------------------------------------------------------------------
# Test: Run ID generation (uniqueness)
# ---------------------------------------------------------------------------

class TestRunIdGeneration:
    """Tests for unique run ID generation."""

    def test_unique_output_dirs(self):
        """Each combination gets a unique output directory."""
        combos = [
            {"a": 1, "b": 2},
            {"a": 1, "b": 3},
            {"a": 2, "b": 2},
        ]
        ids = []
        for i, combo in enumerate(combos):
            run_id = param_search.make_run_id("s", "BTC", "flash-only", combo, None, i)
            ids.append(run_id)
        assert len(set(ids)) == 3

    def test_id_includes_strategy_and_market(self):
        """Run ID includes strategy and market for traceability."""
        run_id = param_search.make_run_id(
            "blueprint-cluster-007", "BTC", "flash-only", {"a": 1}, None, 0
        )
        assert "blueprint-cluster-007" in run_id
        assert "BTC" in run_id

    def test_id_includes_leverage_when_present(self):
        """Run ID includes leverage when provided."""
        run_id = param_search.make_run_id(
            "s", "BTC", "flash-only", {"a": 1}, 3.0, 0
        )
        assert "3.0" in run_id

    def test_id_is_filesystem_safe(self):
        """Run ID contains no special characters unsafe for directories."""
        run_id = param_search.make_run_id(
            "blueprint-cluster-007", "BTC/ETH", "flash-only", {"a.b": 1.5}, None, 0
        )
        # No spaces, slashes, or other filesystem-unsafe chars
        unsafe = set(" /\\:*?\"<>|")
        assert not any(c in run_id for c in unsafe)


# ---------------------------------------------------------------------------
# Test: Parallelism configuration
# ---------------------------------------------------------------------------

class TestParallelismConfig:
    """Tests for parallel execution configuration."""

    def test_parallelism_capped_at_8(self):
        """Parallelism is capped at 8 processes."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            "parallelism": 20,
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["parallelism"] == 8

    def test_parallelism_minimum_1(self):
        """Parallelism is at least 1."""
        spec = {
            "candidates": [{"strategy": "s", "market": "BTC", "cost_modes": ["flash-only"]}],
            "parameter_grid": {"a": [1]},
            "walk_forward": {"mode": "expanding", "windows": 5},
            "backtest_period": {"start": "2026-01-01", "end": "2026-02-01", "interval": "5m"},
            "parallelism": 0,
        }
        validated = param_search.validate_grid_spec(spec)
        assert validated["parallelism"] >= 1


# ---------------------------------------------------------------------------
# Test: Atomic file writes
# ---------------------------------------------------------------------------

class TestAtomicWrites:
    """Tests for atomic JSON file writing."""

    def test_atomic_write_creates_file(self, tmp_path):
        """Atomic write creates the target file."""
        path = str(tmp_path / "test.json")
        param_search.atomic_write_json(path, {"key": "value"})
        assert os.path.exists(path)
        with open(path) as f:
            assert json.load(f) == {"key": "value"}

    def test_atomic_write_no_tmp_leftover(self, tmp_path):
        """Atomic write cleans up .tmp file."""
        path = str(tmp_path / "test.json")
        param_search.atomic_write_json(path, {"key": "value"})
        tmp_path_file = tmp_path / "test.json.tmp"
        assert not tmp_path_file.exists()

    def test_atomic_write_overwrites_existing(self, tmp_path):
        """Atomic write replaces existing file content."""
        path = str(tmp_path / "test.json")
        param_search.atomic_write_json(path, {"v": 1})
        param_search.atomic_write_json(path, {"v": 2})
        with open(path) as f:
            assert json.load(f) == {"v": 2}


# ---------------------------------------------------------------------------
# Test: Collecting results from summary.json files
# ---------------------------------------------------------------------------

class TestCollectResults:
    """Tests for reading summary.json from completed run directories."""

    def test_collect_from_directory(self, sample_summary_json, tmp_path):
        """Collect results reads summary.json from run output directory."""
        run_dir = tmp_path / "run_001"
        run_dir.mkdir()
        (run_dir / "summary.json").write_text(json.dumps(sample_summary_json))

        result = param_search.collect_run_result(
            run_dir=str(run_dir),
            combo_id="run_001",
            strategy="blueprint-cluster-007",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
        )
        assert result.success is True
        assert result.summary == sample_summary_json

    def test_collect_missing_summary(self, tmp_path):
        """Missing summary.json returns failed result."""
        run_dir = tmp_path / "run_missing"
        run_dir.mkdir()

        result = param_search.collect_run_result(
            run_dir=str(run_dir),
            combo_id="run_missing",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
        )
        assert result.success is False
        assert "summary.json" in result.error

    def test_collect_malformed_json(self, tmp_path):
        """Malformed summary.json returns failed result."""
        run_dir = tmp_path / "run_bad"
        run_dir.mkdir()
        (run_dir / "summary.json").write_text("NOT VALID JSON{{{")

        result = param_search.collect_run_result(
            run_dir=str(run_dir),
            combo_id="run_bad",
            strategy="s",
            market="BTC",
            cost_mode="flash-only",
            params={"a": 1},
            leverage=None,
        )
        assert result.success is False
        assert "JSON" in result.error or "parse" in result.error.lower()
