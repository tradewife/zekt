"""Tests for pyramiding analysis script."""

import json
import math
import os
import tempfile
from pathlib import Path

import pytest

# Add scripts dir to path
import sys
scripts_dir = str(Path(__file__).resolve().parent.parent)
if scripts_dir not in sys.path:
    sys.path.insert(0, scripts_dir)

# Import using importlib since the module name has a hyphen-safe path
import importlib.util
spec = importlib.util.spec_from_file_location("pyramiding_analysis", str(Path(scripts_dir) / "pyramiding-analysis.py"))
_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(_mod)

# Re-export needed symbols
compute_sharpe = _mod.compute_sharpe
compute_sortino = _mod.compute_sortino
compute_calmar = _mod.compute_calmar
compute_net_expectancy = _mod.compute_net_expectancy
try_add_tranche = _mod.try_add_tranche
generate_synthetic_price_series = _mod.generate_synthetic_price_series
generate_zone_scenarios = _mod.generate_zone_scenarios
load_captured_snapshots = _mod.load_captured_snapshots
get_base_prices_from_snapshots = _mod.get_base_prices_from_snapshots
run_variant = _mod.run_variant
generate_report = _mod.generate_report
atomic_write = _mod.atomic_write
STARTING_BALANCE = _mod.STARTING_BALANCE
TARGET_SIZE_USD = _mod.TARGET_SIZE_USD
MAX_TRANCHES = _mod.MAX_TRANCHES
TRANCHES_FRACTIONS = _mod.TRANCHES_FRACTIONS
FEE_RATE = _mod.FEE_RATE
ROUTE_COST_BPS = _mod.ROUTE_COST_BPS
PyramidPosition = _mod.PyramidPosition
PyramidTranche = _mod.PyramidTranche
AddTrancheContext = _mod.AddTrancheContext
Trade = _mod.Trade
VariantResult = _mod.VariantResult

# Make module importable by name
sys.modules['pyramiding_analysis'] = _mod


# ─── Helpers ──────────────────────────────────────────────────────────


def make_position(is_long=True, variant="reclaim") -> PyramidPosition:
    """Create a test pyramid position."""
    return PyramidPosition(
        symbol="BTC",
        is_long=is_long,
        target_size_usd=TARGET_SIZE_USD,
        tranche_fractions=list(TRANCHES_FRACTIONS),
        max_tranches=MAX_TRANCHES,
    )


def make_ctx(
    price=100.0,
    ts=1_780_000_000_000,
    data_ts=None,
    reclaim=False,
    higher_low=False,
    retest=False,
    atr=2.0,
    corr_exp=0.0,
) -> AddTrancheContext:
    """Create a test tranche context."""
    return AddTrancheContext(
        current_price=price,
        timestamp_ms=ts,
        data_timestamp_ms=data_ts or ts,
        reclaim_detected=reclaim,
        higher_low_detected=higher_low,
        retest_successful=retest,
        current_atr=atr,
        correlated_exposure_usd=corr_exp,
    )


# ─── Metric Computation Tests ─────────────────────────────────────────


class TestSharpeRatio:
    def test_empty_returns(self):
        assert compute_sharpe([]) == 0.0

    def test_constant_returns(self):
        # All same return → std = 0 → sharpe = 0
        assert compute_sharpe([1.0] * 10) == 0.0

    def test_positive_sharpe_for_ascending(self):
        returns = [i * 0.1 for i in range(1, 11)]
        sharpe = compute_sharpe(returns)
        assert sharpe > 0.0

    def test_negative_sharpe_for_descending(self):
        returns = [-i * 0.1 for i in range(1, 11)]
        sharpe = compute_sharpe(returns)
        assert sharpe < 0.0

    def test_single_return(self):
        # Single return → variance = 0 → sharpe = 0
        assert compute_sharpe([5.0]) == 0.0


class TestSortinoRatio:
    def test_empty_returns(self):
        assert compute_sortino([]) == 0.0

    def test_all_positive_returns(self):
        # No downside → downside_dev = 0 → sortino = 0
        returns = [1.0, 2.0, 3.0, 4.0, 5.0]
        assert compute_sortino(returns) == 0.0

    def test_mixed_returns(self):
        returns = [1.0, -0.5, 2.0, -1.0, 0.5]
        sortino = compute_sortino(returns)
        # Mean = 0.4, downside = [-0.5, -1.0] → downside_dev > 0
        assert sortino != 0.0

    def test_all_negative_returns(self):
        returns = [-1.0, -2.0, -3.0]
        sortino = compute_sortino(returns)
        assert sortino < 0.0  # Negative mean with downside deviation


class TestCalmarRatio:
    def test_zero_starting_balance(self):
        assert compute_calmar(100.0, 0.0, 50.0, 100) == 0.0

    def test_zero_drawdown(self):
        assert compute_calmar(100.0, 1000.0, 0.0, 100) == 0.0

    def test_zero_data_points(self):
        assert compute_calmar(100.0, 1000.0, 50.0, 0) == 0.0

    def test_positive_calmar(self):
        calmar = compute_calmar(100.0, 1000.0, 50.0, 1260)
        # Net PnL 100 / 1000 = 10% return, max DD 50/1000 = 5%
        # Annualized return = 10% * (1260/1260) = 10%, Calmar = 10/5 = 2.0
        assert calmar > 0.0
        assert abs(calmar - 2.0) < 0.01


class TestNetExpectancy:
    def test_empty_trades(self):
        assert compute_net_expectancy([]) == 0.0

    def test_all_winners(self):
        trades = [
            Trade("BTC", "long", 100, 101, 100, 1.0, 0.1, 0.1, 0.03, 0.77, 100, "TP", 1000, 2000, 1, 100, False),
            Trade("BTC", "long", 100, 102, 100, 2.0, 0.1, 0.1, 0.03, 1.77, 200, "TP", 3000, 4000, 1, 100, False),
        ]
        expectancy = compute_net_expectancy(trades)
        assert expectancy > 0.0

    def test_all_losers(self):
        trades = [
            Trade("BTC", "long", 100, 99, 100, -1.0, 0.1, 0.1, 0.03, -1.23, 100, "SL", 1000, 2000, 1, 100, False),
        ]
        expectancy = compute_net_expectancy(trades)
        assert expectancy < 0.0

    def test_mixed_trades(self):
        trades = [
            Trade("BTC", "long", 100, 101, 100, 1.0, 0.1, 0.1, 0.03, 0.77, 100, "TP", 1000, 2000, 1, 100, False),
            Trade("BTC", "long", 100, 99, 100, -1.0, 0.1, 0.1, 0.03, -1.23, 100, "SL", 3000, 4000, 1, 100, False),
        ]
        expectancy = compute_net_expectancy(trades)
        # Win rate 50%, avg win 0.77, avg loss 1.23, avg route 0.03
        # (0.5 * 0.77) - (0.5 * 1.23) - 0.03 = 0.385 - 0.615 - 0.03 = -0.26
        assert expectancy < 0.0


# ─── Pyramid Position Tests ───────────────────────────────────────────


class TestPyramidPosition:
    def test_empty_position(self):
        pos = make_position()
        assert pos.total_size_usd() == 0.0
        assert pos.avg_entry_price() == 0.0
        assert pos.tranche_count() == 0

    def test_is_stop_hit_empty(self):
        pos = make_position()
        assert not pos.is_stop_hit(100.0)

    def test_unrealized_pnl_empty(self):
        pos = make_position()
        assert pos.unrealized_pnl(100.0) == 0.0


# ─── Tranche Addition Tests ───────────────────────────────────────────


class TestNoneVariant:
    def test_none_single_tranche(self):
        pos = make_position(variant="none")
        ctx = make_ctx(price=100.0)
        tranche, reason = try_add_tranche("none", pos, ctx)
        assert tranche is not None
        assert reason == "probe"
        assert pos.tranche_count() == 1

    def test_none_rejects_second_tranche(self):
        pos = make_position(variant="none")
        ctx = make_ctx(price=100.0)
        try_add_tranche("none", pos, ctx)

        # Price moves up (position profitable)
        ctx2 = make_ctx(price=110.0)
        tranche, reason = try_add_tranche("none", pos, ctx2)
        assert tranche is None
        assert "no pyramiding" in reason


class TestReclaimVariant:
    def test_reclaim_probe(self):
        pos = make_position()
        ctx = make_ctx(price=100.0)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is not None
        assert reason == "probe"

    def test_reclaim_rejects_without_reclaim(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))

        ctx = make_ctx(price=105.0, reclaim=False, higher_low=False)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is None
        assert "reclaim" in reason.lower()

    def test_reclaim_adds_with_reclaim_and_higher_low(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))

        ctx = make_ctx(price=105.0, reclaim=True, higher_low=True)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is not None
        assert reason == "confirm"

    def test_reclaim_rejects_without_higher_low(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))

        ctx = make_ctx(price=105.0, reclaim=True, higher_low=False)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is None
        assert "higher low" in reason.lower()


class TestRetestVariant:
    def test_retest_probe(self):
        pos = make_position()
        ctx = make_ctx(price=100.0)
        tranche, reason = try_add_tranche("retest", pos, ctx)
        assert tranche is not None
        assert reason == "probe"

    def test_retest_rejects_without_retest(self):
        pos = make_position()
        try_add_tranche("retest", pos, make_ctx(price=100.0))

        ctx = make_ctx(price=105.0, retest=False)
        tranche, reason = try_add_tranche("retest", pos, ctx)
        assert tranche is None
        assert "retest" in reason.lower()

    def test_retest_adds_on_successful_retest(self):
        pos = make_position()
        try_add_tranche("retest", pos, make_ctx(price=100.0))

        ctx = make_ctx(price=105.0, retest=True)
        tranche, reason = try_add_tranche("retest", pos, ctx)
        assert tranche is not None
        assert reason == "confirm"


class TestProfitFundedVariant:
    def test_profit_funded_probe(self):
        pos = make_position()
        ctx = make_ctx(price=100.0)
        tranche, reason = try_add_tranche("profit_funded", pos, ctx)
        assert tranche is not None
        assert reason == "probe"

    def test_profit_funded_caps_tranche_to_profit(self):
        pos = make_position()
        try_add_tranche("profit_funded", pos, make_ctx(price=100.0))

        # Price moves up — unrealized PnL at 120 = (120-100)/100 * 250 = 50
        unrealized_before = pos.unrealized_pnl(120.0)
        assert unrealized_before > 0

        ctx = make_ctx(price=120.0)
        tranche, reason = try_add_tranche("profit_funded", pos, ctx)
        assert tranche is not None
        # Tranche size should be capped to the unrealized profit BEFORE adding
        assert tranche.size_usd <= unrealized_before + 0.01

    def test_profit_funded_rejects_no_profit(self):
        pos = make_position(is_long=True)
        try_add_tranche("profit_funded", pos, make_ctx(price=100.0))

        # Price drops — no profit
        ctx = make_ctx(price=95.0)
        tranche, reason = try_add_tranche("profit_funded", pos, ctx)
        assert tranche is None


class TestAtrTrailVariant:
    def test_atr_trail_probe(self):
        pos = make_position()
        ctx = make_ctx(price=100.0, atr=2.0)
        tranche, reason = try_add_tranche("atr_trail", pos, ctx)
        assert tranche is not None
        assert reason == "probe_atr"

    def test_atr_trail_rejects_zero_atr(self):
        pos = make_position()
        try_add_tranche("atr_trail", pos, make_ctx(price=100.0, atr=2.0))

        ctx = make_ctx(price=110.0, atr=0.0)
        tranche, reason = try_add_tranche("atr_trail", pos, ctx)
        assert tranche is None
        assert "ATR" in reason


class TestHardLimits:
    def test_max_4_tranches(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))
        try_add_tranche("reclaim", pos, make_ctx(price=105.0, reclaim=True, higher_low=True))
        try_add_tranche("reclaim", pos, make_ctx(price=110.0, reclaim=True, higher_low=True))
        try_add_tranche("reclaim", pos, make_ctx(price=115.0, reclaim=True, higher_low=True))
        assert pos.tranche_count() == 4

        # 5th should be rejected
        tranche, reason = try_add_tranche("reclaim", pos, make_ctx(price=120.0, reclaim=True, higher_low=True))
        assert tranche is None
        assert "max" in reason.lower()

    def test_no_adding_to_losers(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))

        # Price drops (position losing)
        ctx = make_ctx(price=95.0, reclaim=True, higher_low=True)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is None
        assert "loser" in reason.lower()

    def test_stale_data_rejected(self):
        pos = make_position()
        ctx = make_ctx(price=100.0, ts=1000000, data_ts=1000000 - 600000)  # 10 min old
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is None
        assert "stale" in reason.lower()

    def test_correlated_exposure_rejected(self):
        pos = make_position()
        try_add_tranche("reclaim", pos, make_ctx(price=100.0))

        # Set very high correlated exposure
        ctx = make_ctx(price=105.0, reclaim=True, higher_low=True, corr_exp=50000.0)
        tranche, reason = try_add_tranche("reclaim", pos, ctx)
        assert tranche is None
        assert "correlated" in reason.lower()


# ─── Synthetic Data Generation Tests ──────────────────────────────────


class TestSyntheticData:
    def test_price_series_generation(self):
        prices = generate_synthetic_price_series(100.0, 50)
        assert len(prices) == 50
        assert all(p > 0 for p in prices)
        # Prices should be roughly around base price
        assert abs(prices[0] - 100.0) < 0.01

    def test_price_series_deterministic(self):
        prices1 = generate_synthetic_price_series(100.0, 50, seed=42)
        prices2 = generate_synthetic_price_series(100.0, 50, seed=42)
        assert prices1 == prices2

    def test_zone_scenarios_generation(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=10)
        assert len(scenarios) == 10
        for s in scenarios:
            assert "prices" in s
            assert "atr_values" in s
            assert "is_long" in s
            assert len(s["prices"]) > 10
            assert len(s["atr_values"]) == len(s["prices"])

    def test_zone_scenarios_win_loss_balance(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=100)
        winners = [s for s in scenarios if s["is_winner"]]
        # Should be approximately 55% winners
        assert 30 < len(winners) < 80


# ─── Snapshot Loading Tests ───────────────────────────────────────────


class TestSnapshotLoading:
    def test_load_from_nonexistent_dir(self):
        snapshots = load_captured_snapshots(Path("/nonexistent/dir"))
        assert snapshots == []

    def test_load_from_empty_dir(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            snapshots = load_captured_snapshots(Path(tmpdir))
            assert snapshots == []

    def test_load_valid_snapshot(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            snap = {
                "symbol": "BTC",
                "timestamp_ms": 1780222880030,
                "mark_price": 73769.5,
                "zones": [
                    {
                        "price": 110654.25,
                        "side_at_risk": "short",
                        "estimated_notional_usd": 130553.85,
                        "wallet_count": 0,
                        "distance_bps": 5000.0,
                        "confidence": 0.3,
                        "source_mix": ["oi_imbalance"],
                    }
                ],
            }
            path = Path(tmpdir) / "BTC_1780222880030.json"
            with open(path, "w") as f:
                json.dump(snap, f)

            snapshots = load_captured_snapshots(Path(tmpdir))
            assert len(snapshots) == 1
            assert snapshots[0]["symbol"] == "BTC"

    def test_load_malformed_snapshot(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / "BTC_invalid.json"
            with open(path, "w") as f:
                f.write("not valid json {{{")

            snapshots = load_captured_snapshots(Path(tmpdir))
            assert len(snapshots) == 0

    def test_get_base_prices(self):
        snapshots = [
            {"symbol": "BTC", "timestamp_ms": 1, "mark_price": 74000.0},
            {"symbol": "ETH", "timestamp_ms": 2, "mark_price": 2500.0},
            {"symbol": "BTC", "timestamp_ms": 3, "mark_price": 74500.0},
        ]
        prices = get_base_prices_from_snapshots(snapshots)
        assert prices["BTC"] == 74500.0  # Latest
        assert prices["ETH"] == 2500.0


# ─── Run Variant Tests ────────────────────────────────────────────────


class TestRunVariant:
    def test_none_variant_produces_trades(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=20)
        result = run_variant("none", scenarios)
        assert result.variant == "none"
        assert result.total_trades > 0

    def test_reclaim_variant_produces_trades(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=20)
        result = run_variant("reclaim", scenarios)
        assert result.variant == "reclaim"
        assert result.total_trades > 0

    def test_all_variants_produce_results(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=20)
        for variant in ["none", "reclaim", "retest", "profit_funded", "atr_trail"]:
            result = run_variant(variant, scenarios)
            assert result.variant == variant
            assert result.total_trades > 0
            assert result.win_count + result.loss_count == result.total_trades

    def test_results_are_deterministic(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=20)
        r1 = run_variant("reclaim", scenarios)
        r2 = run_variant("reclaim", scenarios)
        assert r1.total_trades == r2.total_trades
        assert abs(r1.net_pnl - r2.net_pnl) < 0.01
        assert abs(r1.sharpe_ratio - r2.sharpe_ratio) < 0.0001


# ─── Report Generation Tests ──────────────────────────────────────────


class TestReportGeneration:
    def test_report_contains_required_sections(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=10)
        results = [run_variant(v, scenarios) for v in ["none", "reclaim", "retest", "profit_funded", "atr_trail"]]
        report = generate_report(results, 10, ["BTC"])

        # Required sections per VAL-REPORTS-005
        assert "Variant Comparison Table" in report
        assert "Per-Variant Metrics" in report
        assert "Tranche Distribution Analysis" in report
        assert "Recommendation" in report
        assert "Does Pyramiding Improve Expectancy?" in report

    def test_report_contains_all_variants(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=10)
        results = [run_variant(v, scenarios) for v in ["none", "reclaim", "retest", "profit_funded", "atr_trail"]]
        report = generate_report(results, 10, ["BTC"])

        for variant in ["None", "Reclaim", "Retest", "Profit-Funded", "ATR Trail"]:
            assert variant in report

    def test_report_contains_metrics(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=10)
        results = [run_variant(v, scenarios) for v in ["none", "reclaim", "retest", "profit_funded", "atr_trail"]]
        report = generate_report(results, 10, ["BTC"])

        assert "Sharpe" in report
        assert "Sortino" in report
        assert "Calmar" in report
        assert "Drawdown" in report
        assert "Expectancy" in report

    def test_report_shows_expectancy_verdict(self):
        scenarios = generate_zone_scenarios(100.0, n_scenarios=10)
        results = [run_variant(v, scenarios) for v in ["none", "reclaim", "retest", "profit_funded", "atr_trail"]]
        report = generate_report(results, 10, ["BTC"])

        # Must contain verdict about whether pyramiding improves or harms
        assert "Verdict:" in report or "Improves?" in report


class TestAtomicWrite:
    def test_atomic_write_creates_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / "test.txt"
            atomic_write(path, "hello world")
            assert path.exists()
            with open(path) as f:
                assert f.read() == "hello world"

    def test_atomic_write_no_tmp_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / "test.txt"
            atomic_write(path, "hello world")
            assert not path.with_suffix(".tmp").exists()
