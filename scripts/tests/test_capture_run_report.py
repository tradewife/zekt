"""
Tests for the capture run report generator.

Validates that generate_capture_run_report.py correctly analyzes snapshot data
and produces a report with all required sections per VAL-REPORTS-001.
"""

import json
import os
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

# Import the module under test
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from generate_capture_run_report import (
    load_snapshots,
    compute_run_metadata,
    compute_per_cycle_zone_counts,
    compute_source_freshness_summary,
    compute_gap_detections,
    compute_data_quality_assessment,
    compute_confidence_distribution,
    compute_single_vs_multi_source,
    compute_source_coverage,
    generate_report,
)


def _make_snapshot(symbol: str, timestamp_ms: int, mark_price: float,
                   zones: list = None, depth_summary: dict = None,
                   source_freshness: dict = None, stale_sources: list = None,
                   funding_rate: float = None, local_high: float = None,
                   local_low: float = None, vwap: float = None,
                   capture_gap: dict = None,
                   imperial_mark_price: float = None) -> dict:
    """Helper to create a snapshot dict for testing."""
    snap = {
        "symbol": symbol,
        "timestamp_ms": timestamp_ms,
        "mark_price": mark_price,
        "zones": zones or [],
        "depth_summary": depth_summary,
        "funding_rate": funding_rate,
        "funding_rate_annual_pct": None,
        "imperial_funding_rate": None,
        "local_high": local_high,
        "local_low": local_low,
        "vwap": vwap,
        "imperial_mark_price": imperial_mark_price,
        "price_divergence_bps": None,
        "aggressive_flow": None,
        "source_freshness": source_freshness or {},
        "stale_sources": stale_sources or [],
    }
    if capture_gap is not None:
        snap["capture_gap"] = capture_gap
    return snap


def _make_zone(price: float, side_at_risk: str, notional: float,
               confidence: float, source_mix: list,
               wallet_count: int = 0, distance_bps: float = 500.0) -> dict:
    """Helper to create a zone dict for testing."""
    return {
        "price": price,
        "side_at_risk": side_at_risk,
        "estimated_notional_usd": notional,
        "wallet_count": wallet_count,
        "distance_bps": distance_bps,
        "confidence": confidence,
        "source_mix": source_mix,
    }


def _write_snapshots_to_dir(snapshots: list, output_dir: str) -> list:
    """Write snapshot dicts as JSON files to a directory. Returns list of file paths."""
    os.makedirs(output_dir, exist_ok=True)
    paths = []
    for snap in snapshots:
        fname = f"{snap['symbol']}_{snap['timestamp_ms']}.json"
        path = os.path.join(output_dir, fname)
        with open(path, "w") as f:
            json.dump(snap, f)
        paths.append(path)
    return paths


class TestLoadSnapshots(unittest.TestCase):
    """Tests for loading snapshot files from disk."""

    def test_load_from_empty_directory(self):
        """Loading from empty directory returns empty list."""
        with tempfile.TemporaryDirectory() as tmpdir:
            snapshots = load_snapshots(tmpdir)
            self.assertEqual(snapshots, [])

    def test_load_single_snapshot(self):
        """Loading a single snapshot file returns one snapshot."""
        snap = _make_snapshot("BTC", 1780222880030, 73769.5)
        with tempfile.TemporaryDirectory() as tmpdir:
            _write_snapshots_to_dir([snap], tmpdir)
            loaded = load_snapshots(tmpdir)
            self.assertEqual(len(loaded), 1)
            self.assertEqual(loaded[0]["symbol"], "BTC")
            self.assertEqual(loaded[0]["timestamp_ms"], 1780222880030)

    def test_load_multiple_snapshots_sorted_by_timestamp(self):
        """Snapshots are returned sorted by timestamp."""
        snap1 = _make_snapshot("BTC", 1780222460662, 73700.0)
        snap2 = _make_snapshot("BTC", 1780222880030, 73769.5)
        snap3 = _make_snapshot("ETH", 1780222802929, 2016.0)
        with tempfile.TemporaryDirectory() as tmpdir:
            _write_snapshots_to_dir([snap2, snap1, snap3], tmpdir)
            loaded = load_snapshots(tmpdir)
            self.assertEqual(len(loaded), 3)
            timestamps = [s["timestamp_ms"] for s in loaded]
            self.assertEqual(timestamps, sorted(timestamps))

    def test_load_ignores_non_json_files(self):
        """Non-JSON files are ignored."""
        with tempfile.TemporaryDirectory() as tmpdir:
            # Write a non-JSON file
            with open(os.path.join(tmpdir, "README.md"), "w") as f:
                f.write("not a snapshot")
            snapshots = load_snapshots(tmpdir)
            self.assertEqual(snapshots, [])

    def test_load_handles_malformed_json_gracefully(self):
        """Malformed JSON files are skipped."""
        with tempfile.TemporaryDirectory() as tmpdir:
            with open(os.path.join(tmpdir, "BTC_12345.json"), "w") as f:
                f.write("not valid json{{{")
            snapshots = load_snapshots(tmpdir)
            self.assertEqual(snapshots, [])


class TestRunMetadata(unittest.TestCase):
    """Tests for run metadata computation."""

    def test_run_metadata_basic(self):
        """Run metadata includes start/end time, symbols, interval."""
        snap1 = _make_snapshot("BTC", 1780222460662, 73700.0)
        snap2 = _make_snapshot("BTC", 1780222880030, 73769.5)
        snap3 = _make_snapshot("ETH", 1780222802929, 2016.0)
        metadata = compute_run_metadata([snap1, snap2, snap3])
        self.assertEqual(metadata["start_ts_ms"], 1780222460662)
        self.assertEqual(metadata["end_ts_ms"], 1780222880030)
        self.assertIn("BTC", metadata["symbols"])
        self.assertIn("ETH", metadata["symbols"])
        self.assertGreater(metadata["duration_secs"], 0)
        self.assertIn("start_utc", metadata)
        self.assertIn("end_utc", metadata)

    def test_run_metadata_empty_snapshots(self):
        """Empty snapshot list produces safe defaults."""
        metadata = compute_run_metadata([])
        self.assertIsNone(metadata["start_ts_ms"])
        self.assertIsNone(metadata["end_ts_ms"])
        self.assertEqual(metadata["symbols"], [])
        self.assertEqual(metadata["duration_secs"], 0)

    def test_run_metadata_single_snapshot(self):
        """Single snapshot has zero duration."""
        snap = _make_snapshot("SOL", 1780222880030, 82.5)
        metadata = compute_run_metadata([snap])
        self.assertEqual(metadata["duration_secs"], 0)
        self.assertEqual(metadata["symbols"], ["SOL"])


class TestPerCycleZoneCounts(unittest.TestCase):
    """Tests for per-cycle zone count computation."""

    def test_per_cycle_counts(self):
        """Each cycle (grouped by timestamp) has correct zone counts."""
        snap1 = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),
        ])
        snap2 = _make_snapshot("ETH", 1000, 2016.0, zones=[
            _make_zone(3000, "short", 5000, 0.3, ["oi_imbalance"]),
        ])
        snap3 = _make_snapshot("BTC", 2000, 73800.0, zones=[
            _make_zone(110500, "short", 120000, 0.4, ["oi_imbalance"]),
            _make_zone(65000, "long", 50000, 0.5, ["hyperliquid_positions"]),
        ])
        counts = compute_per_cycle_zone_counts([snap1, snap2, snap3])
        # Two cycles: ts=1000 (2 zones total), ts=2000 (2 zones)
        self.assertEqual(len(counts), 2)
        # Find cycle at ts=1000
        cycle_1000 = [c for c in counts if c["timestamp_ms"] == 1000][0]
        self.assertEqual(cycle_1000["total_zones"], 2)
        self.assertEqual(cycle_1000["per_symbol"]["BTC"], 1)
        self.assertEqual(cycle_1000["per_symbol"]["ETH"], 1)

    def test_per_cycle_empty(self):
        """No snapshots produces empty list."""
        counts = compute_per_cycle_zone_counts([])
        self.assertEqual(counts, [])


class TestSourceFreshnessSummary(unittest.TestCase):
    """Tests for source freshness summary."""

    def test_freshness_with_source_data(self):
        """Sources with timestamps are reported."""
        snap = _make_snapshot("BTC", 1000, 73700.0,
                             source_freshness={
                                 "hyperliquid_mark": 1000,
                                 "hyperliquid_l2_book": 1000,
                                 "imperial_oi": 1000,
                             })
        summary = compute_source_freshness_summary([snap])
        self.assertIn("hyperliquid_mark", summary)
        self.assertEqual(summary["hyperliquid_mark"]["snapshot_count"], 1)

    def test_freshness_empty(self):
        """Empty snapshots produce empty summary."""
        summary = compute_source_freshness_summary([])
        self.assertEqual(summary, {})

    def test_freshness_tracks_latest_timestamp(self):
        """Latest timestamp per source is tracked."""
        snap1 = _make_snapshot("BTC", 1000, 73700.0,
                              source_freshness={"hyperliquid_mark": 1000})
        snap2 = _make_snapshot("BTC", 2000, 73800.0,
                              source_freshness={"hyperliquid_mark": 2000})
        summary = compute_source_freshness_summary([snap1, snap2])
        self.assertEqual(summary["hyperliquid_mark"]["latest_ts_ms"], 2000)


class TestGapDetections(unittest.TestCase):
    """Tests for capture gap detection."""

    def test_no_gaps_when_close_together(self):
        """No gaps when snapshots are within 2x interval."""
        snap1 = _make_snapshot("BTC", 1000, 73700.0)
        snap2 = _make_snapshot("BTC", 2000, 73800.0)
        gaps = compute_gap_detections([snap1, snap2], interval_secs=30)
        self.assertEqual(gaps, [])

    def test_gap_detected_when_large_jump(self):
        """Gap detected when time between snapshots exceeds 2x interval."""
        snap1 = _make_snapshot("BTC", 1000, 73700.0)
        snap2 = _make_snapshot("BTC", 300_000, 73800.0)  # 300s gap, interval=30s => 10x
        gaps = compute_gap_detections([snap1, snap2], interval_secs=30)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0]["from_ts_ms"], 1000)
        self.assertEqual(gaps[0]["to_ts_ms"], 300000)
        self.assertGreater(gaps[0]["gap_ratio"], 2.0)

    def test_gap_with_empty_snapshots(self):
        """Empty list produces no gaps."""
        gaps = compute_gap_detections([], interval_secs=30)
        self.assertEqual(gaps, [])

    def test_gap_between_capture_sessions(self):
        """Large gap between two capture sessions is detected."""
        # Session 1: timestamps 1000-2000
        # Session 2: timestamps 1_000_000-1_001_000
        snaps = [
            _make_snapshot("BTC", 1000, 73700.0),
            _make_snapshot("BTC", 2000, 73750.0),
            _make_snapshot("BTC", 1_000_000, 73800.0),
            _make_snapshot("BTC", 1_001_000, 73850.0),
        ]
        gaps = compute_gap_detections(snaps, interval_secs=30)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0]["from_ts_ms"], 2000)
        self.assertEqual(gaps[0]["to_ts_ms"], 1_000_000)


class TestDataQualityAssessment(unittest.TestCase):
    """Tests for data quality assessment."""

    def test_quality_with_enriched_data(self):
        """Enriched snapshots have higher quality scores."""
        snap = _make_snapshot("BTC", 1000, 73700.0,
                             depth_summary={"spread_bps": 0.1},
                             funding_rate=0.0001,
                             local_high=74000,
                             local_low=73500,
                             vwap=73800,
                             source_freshness={"hyperliquid_mark": 1000})
        assessment = compute_data_quality_assessment([snap])
        self.assertEqual(assessment["total_snapshots"], 1)
        self.assertEqual(assessment["snapshots_with_depth"], 1)
        self.assertEqual(assessment["snapshots_with_funding"], 1)
        self.assertEqual(assessment["snapshots_with_local_hl"], 1)
        self.assertEqual(assessment["snapshots_with_vwap"], 1)

    def test_quality_with_minimal_data(self):
        """Old format snapshots without enrichment are tracked."""
        snap = _make_snapshot("BTC", 1000, 73700.0)  # No enrichment
        assessment = compute_data_quality_assessment([snap])
        self.assertEqual(assessment["total_snapshots"], 1)
        self.assertEqual(assessment["snapshots_with_depth"], 0)
        self.assertEqual(assessment["snapshots_with_funding"], 0)

    def test_quality_empty(self):
        """Empty list produces zero counts."""
        assessment = compute_data_quality_assessment([])
        self.assertEqual(assessment["total_snapshots"], 0)


class TestConfidenceDistribution(unittest.TestCase):
    """Tests for confidence distribution computation."""

    def test_distribution_buckets(self):
        """Confidence scores are bucketed correctly."""
        snap = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.1, ["oi_imbalance"]),
            _make_zone(105000, "short", 80000, 0.35, ["oi_imbalance"]),
            _make_zone(65000, "long", 50000, 0.6, ["hyperliquid_positions"]),
            _make_zone(66000, "long", 30000, 0.8, ["hyperliquid_positions", "oi_imbalance"]),
        ])
        dist = compute_confidence_distribution([snap])
        self.assertEqual(dist["total_zones"], 4)
        self.assertAlmostEqual(dist["mean"], 0.4625, places=3)
        self.assertEqual(dist["buckets"]["low_0_0.3"], 1)
        self.assertEqual(dist["buckets"]["moderate_0.3_0.5"], 1)
        self.assertEqual(dist["buckets"]["good_0.5_0.7"], 1)
        self.assertEqual(dist["buckets"]["high_0.7_1.0"], 1)

    def test_distribution_empty(self):
        """No zones produces zero totals."""
        snap = _make_snapshot("BTC", 1000, 73700.0, zones=[])
        dist = compute_confidence_distribution([snap])
        self.assertEqual(dist["total_zones"], 0)

    def test_distribution_no_snapshots(self):
        """Empty snapshot list produces zero totals."""
        dist = compute_confidence_distribution([])
        self.assertEqual(dist["total_zones"], 0)


class TestSingleVsMultiSource(unittest.TestCase):
    """Tests for single-source vs multi-source zone counts."""

    def test_single_vs_multi(self):
        """Zones classified by source count correctly."""
        snap = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),  # single
            _make_zone(65000, "long", 50000, 0.6, ["hyperliquid_positions", "oi_imbalance"]),  # multi
            _make_zone(66000, "long", 30000, 0.5, ["oi_imbalance"]),  # single
        ])
        result = compute_single_vs_multi_source([snap])
        self.assertEqual(result["single_source_count"], 2)
        self.assertEqual(result["multi_source_count"], 1)
        self.assertEqual(result["total_zones"], 3)

    def test_single_vs_multi_empty(self):
        """Empty data produces zeros."""
        result = compute_single_vs_multi_source([])
        self.assertEqual(result["single_source_count"], 0)
        self.assertEqual(result["multi_source_count"], 0)

    def test_multi_source_higher_confidence(self):
        """Multi-source zones have higher mean confidence."""
        snap = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),
            _make_zone(65000, "long", 50000, 0.7, ["hyperliquid_positions", "oi_imbalance"]),
        ])
        result = compute_single_vs_multi_source([snap])
        self.assertGreater(result["multi_source_mean_confidence"],
                           result["single_source_mean_confidence"])


class TestSourceCoverage(unittest.TestCase):
    """Tests for source coverage computation."""

    def test_source_coverage(self):
        """Source coverage tracks which sources contributed zones."""
        snap = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),
            _make_zone(65000, "long", 50000, 0.6, ["hyperliquid_positions", "oi_imbalance"]),
        ])
        coverage = compute_source_coverage([snap])
        self.assertIn("oi_imbalance", coverage)
        self.assertEqual(coverage["oi_imbalance"]["zone_count"], 2)
        self.assertIn("hyperliquid_positions", coverage)
        self.assertEqual(coverage["hyperliquid_positions"]["zone_count"], 1)

    def test_source_coverage_empty(self):
        """Empty data produces empty coverage."""
        coverage = compute_source_coverage([])
        self.assertEqual(coverage, {})

    def test_source_coverage_tracks_symbols(self):
        """Source coverage includes which symbols contributed."""
        snap_btc = _make_snapshot("BTC", 1000, 73700.0, zones=[
            _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),
        ])
        snap_eth = _make_snapshot("ETH", 1000, 2016.0, zones=[
            _make_zone(3000, "short", 5000, 0.3, ["oi_imbalance"]),
        ])
        coverage = compute_source_coverage([snap_btc, snap_eth])
        self.assertIn("BTC", coverage["oi_imbalance"]["symbols"])
        self.assertIn("ETH", coverage["oi_imbalance"]["symbols"])


class TestGenerateReport(unittest.TestCase):
    """Integration tests for full report generation."""

    def test_report_contains_all_required_sections(self):
        """Report contains all required sections per VAL-REPORTS-001."""
        snapshots = [
            _make_snapshot("BTC", 1780222460662, 73700.0,
                          zones=[_make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"])],
                          depth_summary={"spread_bps": 0.1, "bid_depth_usd": 5000000, "ask_depth_usd": 4500000},
                          funding_rate=0.0001,
                          local_high=74000, local_low=73500, vwap=73800,
                          source_freshness={"hyperliquid_mark": 1780222460662}),
            _make_snapshot("BTC", 1780222880030, 73769.5,
                          zones=[_make_zone(110500, "short", 120000, 0.4, ["oi_imbalance"])],
                          depth_summary={"spread_bps": 0.1, "bid_depth_usd": 5200000, "ask_depth_usd": 4800000},
                          funding_rate=0.0001,
                          local_high=74100, local_low=73550, vwap=73900,
                          source_freshness={"hyperliquid_mark": 1780222880030}),
            _make_snapshot("ETH", 1780222460662, 2016.0,
                          zones=[_make_zone(3000, "short", 5000, 0.3, ["oi_imbalance"])],
                          source_freshness={"hyperliquid_mark": 1780222460662}),
        ]

        report = generate_report(snapshots, interval_secs=30)

        # Check required sections (VAL-REPORTS-001)
        self.assertIn("# Liquidation Zone Capture Run Report", report)
        self.assertIn("## Run Metadata", report)
        self.assertIn("## Per-Cycle Zone Counts", report)
        self.assertIn("## Source Freshness Summary", report)
        self.assertIn("## Gap Detections", report)
        self.assertIn("## Data Quality Assessment", report)
        self.assertIn("## Confidence Distribution", report)
        self.assertIn("## Single-Source vs Multi-Source Zones", report)
        self.assertIn("## Source Coverage", report)

    def test_report_file_generation(self):
        """Report can be written to disk via atomic write."""
        snapshots = [
            _make_snapshot("BTC", 1000, 73700.0,
                          zones=[_make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"])]),
        ]
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = os.path.join(tmpdir, "liquidation-capture-run.md")
            report = generate_report(snapshots, interval_secs=30)
            # Atomic write
            tmp_path = output_path + ".tmp"
            with open(tmp_path, "w") as f:
                f.write(report)
            os.rename(tmp_path, output_path)
            self.assertTrue(os.path.exists(output_path))
            with open(output_path) as f:
                content = f.read()
            self.assertIn("# Liquidation Zone Capture Run Report", content)

    def test_report_handles_empty_data(self):
        """Report generates cleanly with no snapshot data."""
        report = generate_report([], interval_secs=30)
        self.assertIn("# Liquidation Zone Capture Run Report", report)
        self.assertIn("Total Snapshots | 0", report)

    def test_report_shows_zone_counts(self):
        """Report includes zone count data."""
        snapshots = [
            _make_snapshot("BTC", 1000, 73700.0,
                          zones=[
                              _make_zone(110000, "short", 100000, 0.3, ["oi_imbalance"]),
                              _make_zone(65000, "long", 50000, 0.6, ["hyperliquid_positions"]),
                          ]),
        ]
        report = generate_report(snapshots, interval_secs=30)
        self.assertIn("2", report)  # 2 zones


if __name__ == "__main__":
    unittest.main()
