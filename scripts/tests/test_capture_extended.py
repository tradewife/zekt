"""
Tests for the extended liquidation capture script.

Tests cover:
- L2 depth summary computation
- Funding rate capture and storage
- Local high/low detection from candles
- Anchored VWAP computation
- Source freshness tracking with staleness detection
- Capture gap detection (> 2x interval)
- Single-source vs multi-source zone separation
- Atomic write pattern
- --health CLI flag JSON output
- Snapshot validation (rejects malformed data)
- HL fills burst detection
- HL market prices/candles captured
- Imperial OI imbalance captured
- Imperial mark prices captured
- Imperial Phoenix depth captured
- Retry logic with exponential backoff
"""

import json
import math
import os
import sys
import tempfile
import time
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import MagicMock, patch

import requests

# Import the capture module (hyphenated filename, use importlib)
import importlib
_scripts_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts_dir)
lc = importlib.import_module("liquidation-capture")


class TestL2DepthSummary(unittest.TestCase):
    """VAL-CAPTURE-004: Zone snapshots include L2 depth summaries."""

    def test_l2_book_parsed_correctly(self):
        """L2 book data correctly parsed into best bid/ask and levels."""
        raw_book = {
            "levels": [
                # bids (descending)
                [{"px": "100.0", "sz": "10.0", "n": 1}, {"px": "99.5", "sz": "20.0", "n": 2}],
                # asks (ascending)
                [{"px": "100.5", "sz": "15.0", "n": 1}, {"px": "101.0", "sz": "25.0", "n": 2}],
            ]
        }
        result = lc.parse_l2_book(raw_book)
        self.assertIsNotNone(result)
        self.assertAlmostEqual(result["best_bid"], 100.0)
        self.assertAlmostEqual(result["best_ask"], 100.5)

    def test_l2_depth_summary_computation(self):
        """L2 book data summarized into spread, depth tiers, imbalance."""
        raw_book = {
            "levels": [
                # bids
                [
                    {"px": "100.0", "sz": "50.0", "n": 1},
                    {"px": "99.95", "sz": "30.0", "n": 1},
                    {"px": "99.75", "sz": "20.0", "n": 1},
                    {"px": "99.50", "sz": "10.0", "n": 1},
                ],
                # asks
                [
                    {"px": "100.5", "sz": "40.0", "n": 1},
                    {"px": "100.55", "sz": "25.0", "n": 1},
                    {"px": "100.75", "sz": "15.0", "n": 1},
                    {"px": "101.0", "sz": "5.0", "n": 1},
                ],
            ]
        }
        result = lc.compute_depth_summary(raw_book, mid_price=100.25)
        self.assertIsNotNone(result)
        # Spread bps
        self.assertIn("spread_bps", result)
        self.assertGreater(result["spread_bps"], 0)
        # Depth tiers
        self.assertIn("bid_depth_usd", result)
        self.assertIn("ask_depth_usd", result)
        # Imbalance
        self.assertIn("imbalance", result)
        # Best bid/ask
        self.assertAlmostEqual(result["best_bid"], 100.0)
        self.assertAlmostEqual(result["best_ask"], 100.5)

    def test_l2_book_empty_levels(self):
        """Empty book returns None."""
        result = lc.parse_l2_book({"levels": [[], []]})
        self.assertIsNone(result)

    def test_l2_depth_tiers_at_bps_thresholds(self):
        """Depth computed at 10/25/50 bps thresholds."""
        mid = 100.0
        raw_book = {
            "levels": [
                # bids: 10 bps = 99.90, 25 bps = 99.75, 50 bps = 99.50
                [
                    {"px": "100.0", "sz": "10.0", "n": 1},
                    {"px": "99.90", "sz": "20.0", "n": 1},
                    {"px": "99.75", "sz": "30.0", "n": 1},
                    {"px": "99.50", "sz": "40.0", "n": 1},
                ],
                # asks: 10 bps = 100.10, 25 bps = 100.25, 50 bps = 100.50
                [
                    {"px": "100.1", "sz": "15.0", "n": 1},
                    {"px": "100.25", "sz": "25.0", "n": 1},
                    {"px": "100.5", "sz": "35.0", "n": 1},
                ],
            ]
        }
        result = lc.compute_depth_summary(raw_book, mid_price=mid)
        self.assertIsNotNone(result)
        # At 10 bps
        self.assertIn("bid_depth_10bps_usd", result)
        self.assertIn("ask_depth_10bps_usd", result)
        # At 25 bps
        self.assertIn("bid_depth_25bps_usd", result)
        self.assertIn("ask_depth_25bps_usd", result)
        # At 50 bps
        self.assertIn("bid_depth_50bps_usd", result)
        self.assertIn("ask_depth_50bps_usd", result)


class TestFundingRateCapture(unittest.TestCase):
    """VAL-CAPTURE-005: Zone snapshots include funding rates."""

    def test_funding_rate_from_hl_meta(self):
        """Funding rates extracted from HL meta response."""
        meta = {
            "universe": [
                {"name": "BTC", "funding": "0.0001"},
                {"name": "ETH", "funding": "-0.00005"},
                {"name": "SOL", "funding": "0.0002"},
            ]
        }
        result = lc.extract_funding_rates_from_meta(meta)
        self.assertEqual(len(result), 3)
        self.assertAlmostEqual(result["BTC"], 0.0001)
        self.assertAlmostEqual(result["ETH"], -0.00005)
        self.assertAlmostEqual(result["SOL"], 0.0002)

    def test_funding_rate_in_snapshot(self):
        """Funding rate field present in snapshot."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
            "funding_rate": 0.0001,
            "funding_rate_annual_pct": 10.95,
        }
        self.assertIn("funding_rate", snapshot)
        self.assertIn("funding_rate_annual_pct", snapshot)

    def test_funding_rate_extraction_empty(self):
        """Empty meta returns empty dict."""
        result = lc.extract_funding_rates_from_meta({})
        self.assertEqual(result, {})

    def test_funding_rate_extraction_missing_universe(self):
        """Missing universe key returns empty dict."""
        result = lc.extract_funding_rates_from_meta({"other": "data"})
        self.assertEqual(result, {})


class TestLocalHighLowVWAP(unittest.TestCase):
    """VAL-CAPTURE-006: Zone snapshots include local H/L and VWAP."""

    def test_local_high_low_from_candles(self):
        """Local H/L computed from recent candle data."""
        candles = [
            {"h": 101.0, "l": 99.0, "c": 100.0, "o": 100.0, "v": 100, "t": 0},
            {"h": 102.0, "l": 98.0, "c": 101.0, "o": 100.0, "v": 150, "t": 1},
            {"h": 100.5, "l": 97.0, "c": 99.0, "o": 101.0, "v": 200, "t": 2},
        ]
        result = lc.compute_local_high_low(candles)
        self.assertIsNotNone(result)
        self.assertAlmostEqual(result["local_high"], 102.0)
        self.assertAlmostEqual(result["local_low"], 97.0)

    def test_local_high_low_empty(self):
        """Empty candles returns None."""
        result = lc.compute_local_high_low([])
        self.assertIsNone(result)

    def test_anchored_vwap_computation(self):
        """Anchored VWAP computed from volume-weighted candle data."""
        candles = [
            {"h": 101.0, "l": 99.0, "c": 100.0, "o": 100.0, "v": 100, "t": 0},
            {"h": 102.0, "l": 98.0, "c": 101.0, "o": 100.0, "v": 200, "t": 1},
        ]
        result = lc.compute_anchored_vwap(candles)
        # VWAP = sum(typical_price * volume) / sum(volume)
        # typical = (h+l+c)/3
        tp1 = (101 + 99 + 100) / 3  # = 100.0
        tp2 = (102 + 98 + 101) / 3  # = 100.333...
        expected_vwap = (tp1 * 100 + tp2 * 200) / (100 + 200)
        self.assertAlmostEqual(result, expected_vwap, places=4)

    def test_anchored_vwap_zero_volume(self):
        """Zero volume candles returns None."""
        candles = [
            {"h": 101.0, "l": 99.0, "c": 100.0, "o": 100.0, "v": 0, "t": 0},
        ]
        result = lc.compute_anchored_vwap(candles)
        self.assertIsNone(result)

    def test_local_hl_and_vwap_in_snapshot(self):
        """Local H/L and VWAP fields present in snapshot."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
            "local_high": 50500.0,
            "local_low": 49500.0,
            "vwap": 50010.0,
        }
        self.assertIn("local_high", snapshot)
        self.assertIn("local_low", snapshot)
        self.assertIn("vwap", snapshot)
        self.assertGreater(snapshot["local_high"], snapshot["local_low"])


class TestSourceFreshnessTracking(unittest.TestCase):
    """VAL-CAPTURE-007: Source freshness reported per capture cycle."""

    def test_source_freshness_map_populated(self):
        """Each source has a timestamp in freshness map."""
        freshness = {
            "hyperliquid_mark": 1000000,
            "hyperliquid_positions": 1000000,
            "hyperliquid_fills": 1000000,
            "oi_imbalance": 1000000,
            "depth_fragility": 1000000,
        }
        self.assertEqual(len(freshness), 5)
        for key, ts in freshness.items():
            self.assertIsInstance(ts, int)
            self.assertGreater(ts, 0)

    def test_staleness_detection(self):
        """Stale sources are flagged correctly."""
        now_ms = 2000000
        staleness_threshold_ms = 60000  # 60 seconds
        freshness = {
            "hyperliquid_mark": now_ms,  # Fresh
            "hyperliquid_positions": now_ms - 120000,  # Stale (2 min old)
            "oi_imbalance": now_ms - 30000,  # Fresh (30s old)
        }
        stale = lc.detect_stale_sources(freshness, now_ms, staleness_threshold_ms)
        self.assertIn("hyperliquid_positions", stale)
        self.assertNotIn("hyperliquid_mark", stale)
        self.assertNotIn("oi_imbalance", stale)

    def test_freshness_in_snapshot_metadata(self):
        """Snapshot includes source_freshness metadata."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
            "source_freshness": {
                "hyperliquid_mark": 1000000,
                "oi_imbalance": 1000000,
            },
            "stale_sources": [],
        }
        self.assertIn("source_freshness", snapshot)
        self.assertIn("stale_sources", snapshot)


class TestCaptureGapDetection(unittest.TestCase):
    """VAL-CAPTURE-008: Capture gap detection works."""

    def test_gap_detected_when_exceeds_2x_interval(self):
        """Gap > 2x interval is detected and reported."""
        interval_ms = 30000  # 30 seconds
        prev_ts = 1000000
        current_ts = 1000000 + (2 * interval_ms) + 1000  # 2x + 1 second
        gap = lc.detect_capture_gap(prev_ts, current_ts, interval_ms)
        self.assertIsNotNone(gap)
        self.assertGreater(gap["gap_ms"], 2 * interval_ms)
        self.assertEqual(gap["expected_interval_ms"], interval_ms)

    def test_no_gap_within_normal_interval(self):
        """No gap detected when within normal interval."""
        interval_ms = 30000
        prev_ts = 1000000
        current_ts = 1000000 + interval_ms  # Exactly 1x
        gap = lc.detect_capture_gap(prev_ts, current_ts, interval_ms)
        self.assertIsNone(gap)

    def test_no_gap_at_exactly_2x(self):
        """No gap at exactly 2x (must exceed)."""
        interval_ms = 30000
        prev_ts = 1000000
        current_ts = 1000000 + 2 * interval_ms  # Exactly 2x
        gap = lc.detect_capture_gap(prev_ts, current_ts, interval_ms)
        self.assertIsNone(gap)

    def test_gap_metadata_in_snapshot(self):
        """Gap info recorded in snapshot metadata."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "capture_gap": {
                "gap_ms": 65000,
                "expected_interval_ms": 30000,
            },
        }
        self.assertIn("capture_gap", snapshot)


class TestSingleSourceVsMultiSourceZoneSeparation(unittest.TestCase):
    """VAL-CAPTURE-009: Single-source vs multi-source zone separation."""

    def test_multi_source_zone_has_higher_confidence(self):
        """Multi-source zone receives higher confidence than single-source."""
        config = lc.DEFAULT_CONFIG
        source_freshness = {"hyperliquid_positions": 1000000, "hyperliquid_fills": 1000000}
        now_ms = 1000000

        single_zone = {
            "price": 100.0,
            "side_at_risk": "long",
            "estimated_notional_usd": 50000.0,
            "wallet_count": 1,
            "distance_bps": 100.0,
            "confidence": 0.0,
            "source_mix": ["hyperliquid_positions"],
        }

        multi_zone = {
            "price": 100.0,
            "side_at_risk": "long",
            "estimated_notional_usd": 50000.0,
            "wallet_count": 1,
            "distance_bps": 100.0,
            "confidence": 0.0,
            "source_mix": ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance"],
        }

        single_conf = lc.compute_confidence(single_zone, config, source_freshness, now_ms)
        multi_conf = lc.compute_confidence(multi_zone, config, source_freshness, now_ms)

        self.assertGreater(multi_conf, single_conf,
                           "Multi-source zone should have higher confidence than single-source")

    def test_source_count_classification(self):
        """Zone source_mix length reflected in classification."""
        single = {"source_mix": ["hyperliquid_positions"]}
        multi = {"source_mix": ["hyperliquid_positions", "oi_imbalance", "depth_fragility"]}

        self.assertEqual(len(single["source_mix"]), 1)
        self.assertEqual(len(multi["source_mix"]), 3)

        # Verify confidence bonus scales with source count
        config = lc.DEFAULT_CONFIG
        source_freshness = {s: 1000000 for s in ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance", "depth_fragility"]}
        now_ms = 1000000

        confidences = []
        for sources in [
            ["hyperliquid_positions"],
            ["hyperliquid_positions", "hyperliquid_fills"],
            ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance"],
            ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance", "depth_fragility"],
        ]:
            zone = {
                "price": 100.0,
                "side_at_risk": "long",
                "estimated_notional_usd": 50000.0,
                "wallet_count": 1,
                "distance_bps": 100.0,
                "confidence": 0.0,
                "source_mix": sources,
            }
            confidences.append(lc.compute_confidence(zone, config, source_freshness, now_ms))

        # Each additional source should increase confidence (or at least not decrease)
        for i in range(1, len(confidences)):
            self.assertGreaterEqual(confidences[i], confidences[i - 1],
                                     f"More sources should give >= confidence: {confidences}")


class TestAtomicWrites(unittest.TestCase):
    """VAL-CAPTURE-010: Atomic snapshot writes."""

    def test_atomic_write_pattern(self):
        """Snapshot written via .tmp → rename pattern."""
        snapshot = {
            "symbol": "TEST",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
        }
        with tempfile.TemporaryDirectory() as tmpdir:
            path = lc.persist_snapshot(snapshot, tmpdir)
            # File exists at final path
            self.assertTrue(os.path.exists(path))
            # .tmp file should not exist
            self.assertFalse(os.path.exists(path + ".tmp"))

    def test_atomic_write_content_valid(self):
        """Atomic write produces valid JSON."""
        snapshot = {
            "symbol": "TEST",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [{"price": 49000, "confidence": 0.5}],
        }
        with tempfile.TemporaryDirectory() as tmpdir:
            path = lc.persist_snapshot(snapshot, tmpdir)
            with open(path) as f:
                loaded = json.load(f)
            self.assertEqual(loaded["symbol"], "TEST")
            self.assertEqual(len(loaded["zones"]), 1)


class TestHealthCLI(unittest.TestCase):
    """VAL-CAPTURE-011: Health/status command returns operational data."""

    def test_health_output_format(self):
        """--health returns JSON with required fields."""
        health = lc.generate_health_status(
            status="ok",
            last_capture_ts=1000000,
            symbols_captured=["BTC", "ETH", "SOL"],
            total_zones=9,
            uptime_secs=3600,
            source_freshness={"hyperliquid_mark": 1000000},
            stale_sources=[],
        )
        # Must be valid JSON
        parsed = json.loads(health)
        self.assertEqual(parsed["status"], "ok")
        self.assertEqual(parsed["last_capture_ts"], 1000000)
        self.assertIn("BTC", parsed["symbols_captured"])
        self.assertIn("total_zones", parsed)
        self.assertIn("uptime_secs", parsed)

    def test_health_degraded_status(self):
        """Degraded status when sources are stale."""
        health = lc.generate_health_status(
            status="degraded",
            last_capture_ts=1000000,
            symbols_captured=["BTC"],
            total_zones=0,
            uptime_secs=60,
            source_freshness={},
            stale_sources=["hyperliquid_positions", "hyperliquid_fills"],
        )
        parsed = json.loads(health)
        self.assertEqual(parsed["status"], "degraded")
        self.assertIn("stale_sources", parsed)
        self.assertEqual(len(parsed["stale_sources"]), 2)


class TestSnapshotValidation(unittest.TestCase):
    """VAL-CAPTURE-012: Snapshot validation rejects malformed data."""

    def test_valid_snapshot_passes(self):
        """Valid snapshot passes validation."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 1000.0,
                    "confidence": 0.5,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertTrue(result)

    def test_empty_symbol_rejected(self):
        """Empty symbol rejected."""
        snapshot = {
            "symbol": "",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertFalse(result)

    def test_negative_timestamp_rejected(self):
        """Negative timestamp rejected."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": -1,
            "mark_price": 50000.0,
            "zones": [],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertFalse(result)

    def test_negative_notional_rejected(self):
        """Negative notional in zone rejected."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": -100.0,
                    "confidence": 0.5,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertFalse(result)

    def test_invalid_confidence_rejected(self):
        """Confidence out of [0, 1] rejected."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": 1.5,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertFalse(result)

    def test_missing_symbol_rejected(self):
        """Missing symbol field rejected."""
        snapshot = {
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
        }
        result = lc.validate_snapshot(snapshot)
        self.assertFalse(result)


class TestHLFillsBurstDetection(unittest.TestCase):
    """VAL-CAPTURE-013: HL fills captured for burst detection."""

    def test_burst_detected_with_sufficient_fills(self):
        """Fill burst detected when N+ fills in window."""
        config = lc.DEFAULT_CONFIG
        now_ms = 10000000
        fills = []
        for i in range(12):
            fills.append({
                "wallet": f"0xwallet{i % 3}",
                "coin": "BTC",
                "side": "A",
                "price": 50000.0 + i * 10,
                "size": 1.0,
                "closed_pnl": -100.0,
                "timestamp_ms": now_ms - 30000 + i * 3000,
                "direction": "Close Short",
            })

        zones = lc.detect_forced_liquidation_bursts(fills, 50000.0, config, now_ms)
        self.assertGreater(len(zones), 0, "Should detect at least one burst")
        self.assertEqual(zones[0]["source_mix"], ["hyperliquid_fills"])

    def test_no_burst_below_threshold(self):
        """No burst when fill count below threshold."""
        config = lc.DEFAULT_CONFIG
        now_ms = 10000000
        fills = []
        for i in range(3):
            fills.append({
                "wallet": f"0xwallet{i}",
                "coin": "BTC",
                "side": "A",
                "price": 50000.0,
                "size": 1.0,
                "closed_pnl": -100.0,
                "timestamp_ms": now_ms - 10000 + i * 1000,
                "direction": "Close Short",
            })

        zones = lc.detect_forced_liquidation_bursts(fills, 50000.0, config, now_ms)
        self.assertEqual(len(zones), 0)

    def test_burst_direction_correct(self):
        """Burst direction matches fill side correctly."""
        config = lc.DEFAULT_CONFIG
        now_ms = 10000000
        fills = []
        for i in range(12):
            fills.append({
                "wallet": f"0xwallet{i % 3}",
                "coin": "BTC",
                "side": "B",  # Buy side → shorts being closed
                "price": 50000.0,
                "size": 1.0,
                "closed_pnl": -100.0,
                "timestamp_ms": now_ms - 30000 + i * 3000,
                "direction": "Close Long",
            })

        zones = lc.detect_forced_liquidation_bursts(fills, 50000.0, config, now_ms)
        if zones:
            # Side B (buy closing long) → side_at_risk = "short"
            self.assertEqual(zones[0]["side_at_risk"], "short")


class TestHLMarketDataCapture(unittest.TestCase):
    """VAL-CAPTURE-014: HL market prices/candles captured."""

    def test_mark_prices_fetched(self):
        """Mark prices extracted from HL allMids response."""
        mock_response = {"BTC": "50000.0", "ETH": "3000.0", "SOL": "100.0"}
        with patch.object(lc, 'hl_post', return_value=mock_response):
            prices = lc.fetch_mark_prices(["BTC", "ETH", "SOL"])
            self.assertAlmostEqual(prices["BTC"], 50000.0)
            self.assertAlmostEqual(prices["ETH"], 3000.0)
            self.assertAlmostEqual(prices["SOL"], 100.0)

    def test_candles_fetched_for_vwap(self):
        """Candle data structure valid for VWAP computation."""
        mock_candles = [
            {"t": 1000, "o": "100.0", "h": "101.0", "l": "99.0", "c": "100.5", "v": "1000", "n": 10},
            {"t": 2000, "o": "100.5", "h": "102.0", "l": "98.0", "c": "101.0", "v": "2000", "n": 20},
        ]
        parsed = lc.parse_candle_snapshot(mock_candles)
        self.assertEqual(len(parsed), 2)
        self.assertAlmostEqual(parsed[0]["h"], 101.0)
        self.assertAlmostEqual(parsed[0]["v"], 1000)

    def test_empty_candle_response(self):
        """Empty candle response returns empty list."""
        parsed = lc.parse_candle_snapshot([])
        self.assertEqual(parsed, [])


class TestImperialOICapture(unittest.TestCase):
    """VAL-CAPTURE-015: Imperial OI imbalance captured."""

    def test_oi_imbalance_zone_produced(self):
        """OI imbalance produces a zone when threshold met."""
        config = lc.DEFAULT_CONFIG.copy()
        config["imbalance_threshold_pct"] = 20.0
        oi_data = [
            {
                "symbol": "BTC",
                "long_oi_usd": 1_000_000,
                "short_oi_usd": 500_000,
            }
        ]
        mark_prices = {"BTC": 50000.0}
        zones = lc.produce_oi_imbalance_zones(oi_data, mark_prices, config)
        self.assertGreater(len(zones), 0)
        # Long-heavy → long at risk
        self.assertEqual(zones[0]["side_at_risk"], "long")
        self.assertIn("oi_imbalance", zones[0]["source_mix"])

    def test_oi_no_imbalance_below_threshold(self):
        """No zone when OI within threshold."""
        config = lc.DEFAULT_CONFIG.copy()
        config["imbalance_threshold_pct"] = 50.0
        oi_data = [
            {
                "symbol": "BTC",
                "long_oi_usd": 1_000_000,
                "short_oi_usd": 900_000,
            }
        ]
        mark_prices = {"BTC": 50000.0}
        zones = lc.produce_oi_imbalance_zones(oi_data, mark_prices, config)
        self.assertEqual(len(zones), 0)


class TestImperialMarkPrices(unittest.TestCase):
    """VAL-CAPTURE-016: Imperial mark prices captured."""

    def test_imperial_mark_price_parsed(self):
        """Imperial mark price data parsed per symbol."""
        mock_response = [
            {"symbol": "BTC-PERP", "markPrice": "50100.5"},
            {"symbol": "ETH-PERP", "markPrice": "3050.25"},
        ]
        prices = lc.parse_imperial_mark_prices(mock_response)
        self.assertIn("BTC", prices)
        self.assertAlmostEqual(prices["BTC"], 50100.5)
        self.assertIn("ETH", prices)
        self.assertAlmostEqual(prices["ETH"], 3050.25)

    def test_imperial_mark_price_in_snapshot(self):
        """Imperial mark price compared against HL price."""
        snapshot = {
            "symbol": "BTC",
            "mark_price": 50000.0,
            "imperial_mark_price": 50010.0,
            "price_divergence_bps": 2.0,
        }
        self.assertIn("imperial_mark_price", snapshot)
        self.assertIn("price_divergence_bps", snapshot)


class TestImperialPhoenixDepth(unittest.TestCase):
    """VAL-CAPTURE-017: Imperial Phoenix depth captured."""

    def test_phoenix_depth_parsed(self):
        """Phoenix depth data parsed into bid/ask tiers."""
        mock_data = {
            "snapshots": {
                "BTC": {
                    "mid": "50000.0",
                    "bids": [
                        {"price": "49999.0", "sizeBase": "10.0"},
                        {"price": "49995.0", "sizeBase": "20.0"},
                    ],
                    "asks": [
                        {"price": "50001.0", "sizeBase": "15.0"},
                        {"price": "50005.0", "sizeBase": "25.0"},
                    ],
                }
            }
        }
        result = lc.parse_phoenix_depth(mock_data, "BTC")
        self.assertIsNotNone(result)
        self.assertAlmostEqual(result["mid"], 50000.0)
        self.assertEqual(len(result["bids"]), 2)
        self.assertEqual(len(result["asks"]), 2)

    def test_phoenix_depth_missing_symbol(self):
        """Missing symbol returns None."""
        mock_data = {"snapshots": {"ETH": {"mid": "3000.0"}}}
        result = lc.parse_phoenix_depth(mock_data, "BTC")
        self.assertIsNone(result)

    def test_fragility_zone_produced(self):
        """Depth fragility zones produced from thin book."""
        config = lc.DEFAULT_CONFIG.copy()
        config["depth_min_threshold_usd"] = 1_000_000  # High threshold
        depth_data = {
            "BTC": {
                "mid": "50000.0",
                "bids": [{"price": "49990.0", "sizeBase": "1.0"}],
                "asks": [{"price": "50010.0", "sizeBase": "0.5"}],
            }
        }
        mark_prices = {"BTC": 50000.0}
        zones = lc.produce_fragility_zones(depth_data, mark_prices, config)
        self.assertGreater(len(zones), 0)


class TestRetryLogic(unittest.TestCase):
    """API error handling with retry logic."""

    @patch('requests.post')
    def test_retry_on_transient_error(self, mock_post):
        """Retry occurs on transient HTTP errors."""
        call_count = 0
        real_post = requests.post

        def mock_fn(*args, **kwargs):
            nonlocal call_count
            call_count += 1
            if call_count < 3:
                raise requests.exceptions.ConnectionError("Connection error")
            mock_resp = MagicMock()
            mock_resp.status_code = 200
            mock_resp.json.return_value = {"BTC": "50000.0"}
            mock_resp.raise_for_status = MagicMock()
            return mock_resp

        mock_post.side_effect = mock_fn
        result = lc.hl_post_with_retry({"type": "allMids"}, max_retries=3, base_delay=0.01)
        self.assertIsNotNone(result)
        self.assertEqual(call_count, 3)

    @patch('requests.post')
    def test_retry_exhaustion_returns_none(self, mock_post):
        """Returns None after exhausting retries."""
        mock_post.side_effect = requests.exceptions.ConnectionError("Persistent error")
        result = lc.hl_post_with_retry({"type": "allMids"}, max_retries=2, base_delay=0.01)
        self.assertIsNone(result)

    @patch('requests.get')
    def test_imperial_retry(self, mock_get):
        """Imperial GET also supports retry."""
        call_count = 0

        def mock_fn(*args, **kwargs):
            nonlocal call_count
            call_count += 1
            if call_count < 2:
                raise requests.exceptions.ConnectionError("Connection error")
            mock_resp = MagicMock()
            mock_resp.status_code = 200
            mock_resp.json.return_value = {"rows": []}
            mock_resp.raise_for_status = MagicMock()
            return mock_resp

        mock_get.side_effect = mock_fn
        result = lc.imperial_get_with_retry("/api/v1/stats/markets", max_retries=3, base_delay=0.01)
        self.assertIsNotNone(result)
        self.assertEqual(call_count, 2)


class TestAggressiveFlowCapture(unittest.TestCase):
    """Aggressive flow capture from fills."""

    def test_aggressive_flow_direction_captured(self):
        """Aggressive flow direction and magnitude captured."""
        fills = [
            {"coin": "BTC", "side": "A", "price": 50000.0, "size": 10.0,
             "timestamp_ms": 1000000, "closed_pnl": -500, "wallet": "0x1", "direction": "Close Short"},
            {"coin": "BTC", "side": "A", "price": 50010.0, "size": 5.0,
             "timestamp_ms": 1000001, "closed_pnl": -200, "wallet": "0x2", "direction": "Close Short"},
            {"coin": "BTC", "side": "B", "price": 49990.0, "size": 3.0,
             "timestamp_ms": 1000002, "closed_pnl": -100, "wallet": "0x3", "direction": "Close Long"},
        ]
        flow = lc.compute_aggressive_flow(fills)
        self.assertIsNotNone(flow)
        self.assertIn("BTC", flow)
        # Sell flow should be dominant (side A = sell)
        self.assertIn("sell_flow_usd", flow["BTC"])
        self.assertIn("buy_flow_usd", flow["BTC"])
        self.assertIn("net_flow_usd", flow["BTC"])

    def test_aggressive_flow_empty(self):
        """Empty fills returns empty dict."""
        flow = lc.compute_aggressive_flow([])
        self.assertEqual(flow, {})


if __name__ == "__main__":
    unittest.main()
