"""
Tests for multi-symbol capture service validation.

VAL-CAPTURE-003: Ensure capture service produces at least one
LiquidationZoneSnapshot per symbol per cycle for BTC, ETH, SOL.
Verify snapshots validate correctly (non-empty symbol, valid timestamp,
positive notional, confidence 0-1). Zone validation rejects malformed data.

Tests cover:
- Multi-symbol capture produces 3 snapshots (BTC, ETH, SOL) per cycle
- Each snapshot has valid symbol, timestamp_ms, mark_price, zones array
- Snapshot validation rejects malformed data (empty symbol, negative
  timestamp, negative notional, confidence out of range)
- Zone confidence scoring for multi-source zones
- Per-symbol independence of capture cycle
- Distinct snapshot files per symbol
"""

import importlib
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

# Import the capture module (hyphenated filename)
_scripts_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts_dir)
lc = importlib.import_module("liquidation-capture")


class TestMultiSymbolCaptureCycle(unittest.TestCase):
    """VAL-CAPTURE-003: Each cycle produces 3 snapshots (BTC, ETH, SOL)."""

    def test_symbols_constant_includes_all_three(self):
        """SYMBOLS constant contains BTC, ETH, SOL."""
        self.assertIn("BTC", lc.SYMBOLS)
        self.assertIn("ETH", lc.SYMBOLS)
        self.assertIn("SOL", lc.SYMBOLS)
        self.assertEqual(len(lc.SYMBOLS), 3)

    def test_hl_symbol_mapping(self):
        """HL symbol mapping covers all three symbols."""
        for sym in ["BTC", "ETH", "SOL"]:
            self.assertIn(sym, lc.HL_SYMBOLS)
            self.assertEqual(lc.HL_SYMBOLS[sym], sym)

    def test_imperial_symbol_mapping(self):
        """Imperial symbol mapping covers all three symbols."""
        for sym in ["BTC", "ETH", "SOL"]:
            self.assertIn(sym, lc.IMPERIAL_SYMBOLS)
            self.assertEqual(lc.IMPERIAL_SYMBOLS[sym], f"{sym}-PERP")

    def test_capture_cycle_produces_three_snapshots(self):
        """Single capture cycle produces one snapshot per symbol."""
        with tempfile.TemporaryDirectory() as tmpdir:
            with patch.object(lc, "fetch_mark_prices", return_value={"BTC": 100000.0, "ETH": 3500.0, "SOL": 150.0}), \
                 patch.object(lc, "fetch_hl_funding_meta", return_value={"universe": [
                     {"name": "BTC", "funding": "0.0001"},
                     {"name": "ETH", "funding": "0.00005"},
                     {"name": "SOL", "funding": "0.0002"},
                 ]}), \
                 patch.object(lc, "fetch_imperial_oi", return_value=[]), \
                 patch.object(lc, "fetch_imperial_depth", return_value={}), \
                 patch.object(lc, "fetch_imperial_mark_prices", return_value={}), \
                 patch.object(lc, "fetch_imperial_funding", return_value={}), \
                 patch.object(lc, "fetch_hl_positions", return_value=[]), \
                 patch.object(lc, "fetch_hl_fills", return_value=[]), \
                 patch.object(lc, "fetch_hl_l2_book", return_value=None), \
                 patch.object(lc, "fetch_hl_candles", return_value=[]):

                stats = lc.run_capture_cycle(
                    config=lc.DEFAULT_CONFIG,
                    wallets=["0xtest"],
                    output_dir=tmpdir,
                    cycle_num=1,
                )

                # Should produce 3 snapshots
                self.assertEqual(stats["snapshots_written"], 3,
                                 "Should write 3 snapshots (BTC, ETH, SOL)")

                # Verify files exist for each symbol
                files = os.listdir(tmpdir)
                btc_files = [f for f in files if f.startswith("BTC_")]
                eth_files = [f for f in files if f.startswith("ETH_")]
                sol_files = [f for f in files if f.startswith("SOL_")]

                self.assertEqual(len(btc_files), 1, "Should have 1 BTC snapshot")
                self.assertEqual(len(eth_files), 1, "Should have 1 ETH snapshot")
                self.assertEqual(len(sol_files), 1, "Should have 1 SOL snapshot")

    def test_capture_cycle_snapshot_fields(self):
        """Each snapshot has valid symbol, timestamp_ms, mark_price, zones."""
        with tempfile.TemporaryDirectory() as tmpdir:
            with patch.object(lc, "fetch_mark_prices", return_value={"BTC": 100000.0, "ETH": 3500.0, "SOL": 150.0}), \
                 patch.object(lc, "fetch_hl_funding_meta", return_value=None), \
                 patch.object(lc, "fetch_imperial_oi", return_value=[]), \
                 patch.object(lc, "fetch_imperial_depth", return_value={}), \
                 patch.object(lc, "fetch_imperial_mark_prices", return_value={}), \
                 patch.object(lc, "fetch_imperial_funding", return_value={}), \
                 patch.object(lc, "fetch_hl_positions", return_value=[]), \
                 patch.object(lc, "fetch_hl_fills", return_value=[]), \
                 patch.object(lc, "fetch_hl_l2_book", return_value=None), \
                 patch.object(lc, "fetch_hl_candles", return_value=[]):

                lc.run_capture_cycle(
                    config=lc.DEFAULT_CONFIG,
                    wallets=[],
                    output_dir=tmpdir,
                    cycle_num=1,
                )

                # Read each snapshot and verify required fields
                for filename in os.listdir(tmpdir):
                    if not filename.endswith(".json"):
                        continue
                    filepath = os.path.join(tmpdir, filename)
                    with open(filepath, "r") as f:
                        snapshot = json.load(f)

                    # Required fields
                    self.assertIn("symbol", snapshot, f"{filename}: missing symbol")
                    self.assertIn("timestamp_ms", snapshot, f"{filename}: missing timestamp_ms")
                    self.assertIn("mark_price", snapshot, f"{filename}: missing mark_price")
                    self.assertIn("zones", snapshot, f"{filename}: missing zones")

                    # Valid values
                    self.assertTrue(len(snapshot["symbol"]) > 0, f"{filename}: empty symbol")
                    self.assertIn(snapshot["symbol"], ["BTC", "ETH", "SOL"],
                                  f"{filename}: unexpected symbol")
                    self.assertTrue(snapshot["timestamp_ms"] > 0, f"{filename}: invalid timestamp")
                    self.assertTrue(snapshot["mark_price"] > 0, f"{filename}: invalid mark_price")
                    self.assertIsInstance(snapshot["zones"], list, f"{filename}: zones must be array")

    def test_capture_cycle_snapshot_validates(self):
        """Each produced snapshot passes validate_snapshot()."""
        with tempfile.TemporaryDirectory() as tmpdir:
            with patch.object(lc, "fetch_mark_prices", return_value={"BTC": 100000.0, "ETH": 3500.0, "SOL": 150.0}), \
                 patch.object(lc, "fetch_hl_funding_meta", return_value=None), \
                 patch.object(lc, "fetch_imperial_oi", return_value=[]), \
                 patch.object(lc, "fetch_imperial_depth", return_value={}), \
                 patch.object(lc, "fetch_imperial_mark_prices", return_value={}), \
                 patch.object(lc, "fetch_imperial_funding", return_value={}), \
                 patch.object(lc, "fetch_hl_positions", return_value=[]), \
                 patch.object(lc, "fetch_hl_fills", return_value=[]), \
                 patch.object(lc, "fetch_hl_l2_book", return_value=None), \
                 patch.object(lc, "fetch_hl_candles", return_value=[]):

                lc.run_capture_cycle(
                    config=lc.DEFAULT_CONFIG,
                    wallets=[],
                    output_dir=tmpdir,
                    cycle_num=1,
                )

                for filename in os.listdir(tmpdir):
                    if not filename.endswith(".json"):
                        continue
                    filepath = os.path.join(tmpdir, filename)
                    with open(filepath, "r") as f:
                        snapshot = json.load(f)

                    self.assertTrue(
                        lc.validate_snapshot(snapshot),
                        f"{filename}: snapshot should pass validation",
                    )


class TestSnapshotValidationMultiSymbol(unittest.TestCase):
    """VAL-CAPTURE-003: Snapshot validation for multi-symbol scenarios."""

    def test_valid_btc_snapshot(self):
        """BTC snapshot with valid fields passes validation."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": int(time.time() * 1000),
            "mark_price": 100_000.0,
            "zones": [
                {
                    "price": 95_000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 500_000.0,
                    "confidence": 0.75,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        self.assertTrue(lc.validate_snapshot(snapshot))

    def test_valid_eth_snapshot(self):
        """ETH snapshot with valid fields passes validation."""
        snapshot = {
            "symbol": "ETH",
            "timestamp_ms": int(time.time() * 1000),
            "mark_price": 3_500.0,
            "zones": [],
        }
        self.assertTrue(lc.validate_snapshot(snapshot))

    def test_valid_sol_snapshot(self):
        """SOL snapshot with valid fields passes validation."""
        snapshot = {
            "symbol": "SOL",
            "timestamp_ms": int(time.time() * 1000),
            "mark_price": 150.0,
            "zones": [
                {
                    "price": 140.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 200_000.0,
                    "confidence": 0.6,
                    "source_mix": ["hyperliquid_positions", "oi_imbalance"],
                },
                {
                    "price": 160.0,
                    "side_at_risk": "short",
                    "estimated_notional_usd": 150_000.0,
                    "confidence": 0.5,
                    "source_mix": ["depth_fragility"],
                },
            ],
        }
        self.assertTrue(lc.validate_snapshot(snapshot))

    def test_all_three_symbols_pass_validation(self):
        """All three target symbols produce valid snapshots."""
        symbols_and_prices = [("BTC", 100_000.0), ("ETH", 3_500.0), ("SOL", 150.0)]
        for sym, price in symbols_and_prices:
            snapshot = {
                "symbol": sym,
                "timestamp_ms": int(time.time() * 1000),
                "mark_price": price,
                "zones": [
                    {
                        "price": price * 0.95,
                        "side_at_risk": "long",
                        "estimated_notional_usd": 100_000.0,
                        "confidence": 0.5,
                        "source_mix": ["hyperliquid_positions"],
                    }
                ],
            }
            self.assertTrue(
                lc.validate_snapshot(snapshot),
                f"Snapshot for {sym} should pass validation",
            )


class TestZoneValidationRejectsMalformed(unittest.TestCase):
    """VAL-CAPTURE-003: Zone validation rejects malformed data."""

    def test_negative_notional_rejected(self):
        """Zone with negative notional is rejected."""
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
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_confidence_above_one_rejected(self):
        """Zone with confidence > 1.0 is rejected."""
        snapshot = {
            "symbol": "ETH",
            "timestamp_ms": 1000000,
            "mark_price": 3500.0,
            "zones": [
                {
                    "price": 3400.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": 1.5,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_confidence_below_zero_rejected(self):
        """Zone with confidence < 0.0 is rejected."""
        snapshot = {
            "symbol": "SOL",
            "timestamp_ms": 1000000,
            "mark_price": 150.0,
            "zones": [
                {
                    "price": 140.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": -0.1,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_confidence_boundary_values_accepted(self):
        """Confidence at exactly 0.0 and 1.0 is accepted."""
        # Confidence = 0.0
        snapshot_zero = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": 0.0,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        self.assertTrue(lc.validate_snapshot(snapshot_zero))

        # Confidence = 1.0
        snapshot_one = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": 1.0,
                    "source_mix": ["hyperliquid_positions"],
                }
            ],
        }
        self.assertTrue(lc.validate_snapshot(snapshot_one))

    def test_empty_symbol_rejected(self):
        """Snapshot with empty symbol is rejected."""
        snapshot = {
            "symbol": "",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_missing_symbol_rejected(self):
        """Snapshot missing symbol field is rejected."""
        snapshot = {
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_negative_timestamp_rejected(self):
        """Snapshot with negative timestamp is rejected."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": -1,
            "mark_price": 50000.0,
            "zones": [],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_invalid_side_at_risk_rejected(self):
        """Zone with invalid side_at_risk value is rejected.

        Note: The Python validator allows empty string ("") for side_at_risk
        as some zones may not have a side classification. Only truly invalid
        values like "invalid", "LONG", "SHORT", "both" are rejected.
        """
        for bad_side in ["invalid", "LONG", "SHORT", "both"]:
            snapshot = {
                "symbol": "BTC",
                "timestamp_ms": 1000000,
                "mark_price": 50000.0,
                "zones": [
                    {
                        "price": 49000.0,
                        "side_at_risk": bad_side,
                        "estimated_notional_usd": 100.0,
                        "confidence": 0.5,
                        "source_mix": ["hyperliquid_positions"],
                    }
                ],
            }
            self.assertFalse(
                lc.validate_snapshot(snapshot),
                f"side_at_risk='{bad_side}' should be rejected",
            )

    def test_multiple_zones_one_bad_rejects_all(self):
        """If any zone is invalid, the entire snapshot is rejected."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 100.0,
                    "confidence": 0.5,
                    "source_mix": ["hyperliquid_positions"],
                },
                {
                    "price": 48000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": -50.0,  # Bad!
                    "confidence": 0.5,
                    "source_mix": ["hyperliquid_positions"],
                },
            ],
        }
        self.assertFalse(lc.validate_snapshot(snapshot))

    def test_zero_notional_accepted(self):
        """Zone with zero notional is accepted."""
        snapshot = {
            "symbol": "BTC",
            "timestamp_ms": 1000000,
            "mark_price": 50000.0,
            "zones": [
                {
                    "price": 49000.0,
                    "side_at_risk": "long",
                    "estimated_notional_usd": 0.0,
                    "confidence": 0.0,
                    "source_mix": [],
                }
            ],
        }
        self.assertTrue(lc.validate_snapshot(snapshot))


class TestPersistMultiSymbolSnapshots(unittest.TestCase):
    """VAL-CAPTURE-003: Snapshot persistence for multi-symbol capture."""

    def test_persist_three_symbols_distinct_files(self):
        """Persisting snapshots for BTC, ETH, SOL creates 3 distinct files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            now_ms = int(time.time() * 1000)
            symbols = [("BTC", 100_000.0), ("ETH", 3_500.0), ("SOL", 150.0)]

            paths = []
            for sym, mark in symbols:
                snapshot = {
                    "symbol": sym,
                    "timestamp_ms": now_ms,
                    "mark_price": mark,
                    "zones": [],
                }
                path = lc.persist_snapshot(snapshot, tmpdir)
                paths.append(path)

            # 3 unique files
            self.assertEqual(len(paths), 3)
            unique_paths = set(paths)
            self.assertEqual(len(unique_paths), 3, "all paths should be unique")

            # Each file exists
            for p in paths:
                self.assertTrue(os.path.exists(p), f"file should exist: {p}")

            # Verify contents
            for p in paths:
                with open(p, "r") as f:
                    data = json.load(f)
                self.assertIn(data["symbol"], ["BTC", "ETH", "SOL"])

    def test_persist_atomic_write_no_partial_files(self):
        """No .tmp files remain after successful write."""
        with tempfile.TemporaryDirectory() as tmpdir:
            snapshot = {
                "symbol": "BTC",
                "timestamp_ms": int(time.time() * 1000),
                "mark_price": 100_000.0,
                "zones": [],
            }
            lc.persist_snapshot(snapshot, tmpdir)

            # No .tmp files should exist
            tmp_files = [f for f in os.listdir(tmpdir) if f.endswith(".tmp")]
            self.assertEqual(len(tmp_files), 0, "no .tmp files should remain")

            # Final JSON file should exist
            json_files = [f for f in os.listdir(tmpdir) if f.endswith(".json")]
            self.assertEqual(len(json_files), 1)


class TestConfidenceScoringMultiSymbol(unittest.TestCase):
    """VAL-CAPTURE-003: Confidence scoring for multi-source zones."""

    def test_multi_source_confidence_higher_than_single(self):
        """Zones with multiple sources get higher confidence."""
        config = lc.DEFAULT_CONFIG
        now_ms = int(time.time() * 1000)
        freshness = {
            "hyperliquid_positions": now_ms,
            "hyperliquid_fills": now_ms,
            "oi_imbalance": now_ms,
        }

        single_zone = {
            "price": 95000.0,
            "side_at_risk": "long",
            "estimated_notional_usd": 500_000.0,
            "wallet_count": 10,
            "distance_bps": 500.0,
            "confidence": 0.0,
            "source_mix": ["hyperliquid_positions"],
        }
        multi_zone = {
            "price": 95000.0,
            "side_at_risk": "long",
            "estimated_notional_usd": 500_000.0,
            "wallet_count": 10,
            "distance_bps": 500.0,
            "confidence": 0.0,
            "source_mix": ["hyperliquid_positions", "hyperliquid_fills", "oi_imbalance"],
        }

        single_conf = lc.compute_confidence(single_zone, config, freshness, now_ms)
        multi_conf = lc.compute_confidence(multi_zone, config, freshness, now_ms)

        self.assertGreater(multi_conf, single_conf,
                           "multi-source zone should have higher confidence")
        self.assertGreaterEqual(single_conf, 0.0)
        self.assertLessEqual(single_conf, 1.0)
        self.assertGreaterEqual(multi_conf, 0.0)
        self.assertLessEqual(multi_conf, 1.0)

    def test_confidence_in_valid_range(self):
        """Confidence is always in [0, 1] regardless of inputs."""
        config = lc.DEFAULT_CONFIG
        now_ms = int(time.time() * 1000)

        # Test with extreme values
        zones = [
            # Very high notional, many sources
            {
                "price": 95000.0, "side_at_risk": "long",
                "estimated_notional_usd": 100_000_000.0,
                "wallet_count": 1000, "distance_bps": 500.0,
                "confidence": 0.0,
                "source_mix": ["hyperliquid_positions", "hyperliquid_fills",
                                "oi_imbalance", "depth_fragility"],
            },
            # Very low notional, single source, stale
            {
                "price": 95000.0, "side_at_risk": "long",
                "estimated_notional_usd": 100.0,
                "wallet_count": 1, "distance_bps": 500.0,
                "confidence": 0.0,
                "source_mix": ["hyperliquid_positions"],
            },
        ]

        freshness_stale = {"hyperliquid_positions": now_ms - 300_000}  # 5 min stale
        freshness_fresh = {
            "hyperliquid_positions": now_ms,
            "hyperliquid_fills": now_ms,
            "oi_imbalance": now_ms,
            "depth_fragility": now_ms,
        }

        for zone in zones:
            for freshness in [freshness_stale, freshness_fresh]:
                conf = lc.compute_confidence(zone, config, freshness, now_ms)
                self.assertGreaterEqual(conf, 0.0, "confidence >= 0.0")
                self.assertLessEqual(conf, 1.0, "confidence <= 1.0")


if __name__ == "__main__":
    unittest.main()
