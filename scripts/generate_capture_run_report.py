#!/usr/bin/env python3
"""
Capture Run Report Generator

Analyzes liquidation zone snapshot data from data/liquidation-zones/ and produces
a comprehensive markdown report with all required sections per VAL-REPORTS-001:
  - Run metadata (start/end time, symbols, interval)
  - Per-cycle zone counts
  - Source freshness summary
  - Gap detections
  - Data quality assessment
  - Confidence distribution
  - Single-source vs multi-source zone counts
  - Source coverage

Usage:
    python3 scripts/generate_capture_run_report.py [--snapshot-dir DIR] [--output PATH] [--interval-secs N]
"""

import argparse
import json
import logging
import math
import os
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%Y-%m-%dT%H:%M:%S",
)
log = logging.getLogger("capture-run-report")


# ---------------------------------------------------------------------------
# Snapshot Loading
# ---------------------------------------------------------------------------

def load_snapshots(snapshot_dir: str) -> List[dict]:
    """Load all snapshot JSON files from a directory, sorted by timestamp.

    Malformed files are skipped gracefully.
    """
    if not os.path.isdir(snapshot_dir):
        log.warning("Snapshot directory does not exist: %s", snapshot_dir)
        return []

    snapshots = []
    for fname in sorted(os.listdir(snapshot_dir)):
        if not fname.endswith(".json"):
            continue
        fpath = os.path.join(snapshot_dir, fname)
        try:
            with open(fpath) as f:
                snap = json.load(f)
            if "symbol" in snap and "timestamp_ms" in snap:
                snapshots.append(snap)
            else:
                log.warning("Skipping %s: missing required fields", fname)
        except (json.JSONDecodeError, OSError) as e:
            log.warning("Skipping %s: %s", fname, e)

    # Sort by timestamp
    snapshots.sort(key=lambda s: s.get("timestamp_ms", 0))
    return snapshots


# ---------------------------------------------------------------------------
# Run Metadata
# ---------------------------------------------------------------------------

def compute_run_metadata(snapshots: List[dict]) -> dict:
    """Compute run metadata: start/end time, symbols, duration, interval."""
    if not snapshots:
        return {
            "start_ts_ms": None,
            "end_ts_ms": None,
            "start_utc": "N/A",
            "end_utc": "N/A",
            "duration_secs": 0,
            "duration_human": "0s",
            "symbols": [],
            "snapshot_count": 0,
        }

    timestamps = [s["timestamp_ms"] for s in snapshots]
    start_ts = min(timestamps)
    end_ts = max(timestamps)
    duration_ms = end_ts - start_ts
    duration_secs = duration_ms / 1000.0

    symbols = sorted(set(s["symbol"] for s in snapshots))

    start_utc = _ms_to_utc(start_ts)
    end_utc = _ms_to_utc(end_ts)

    # Estimate interval from consecutive timestamps
    intervals = []
    for i in range(1, len(snapshots)):
        gap = snapshots[i]["timestamp_ms"] - snapshots[i - 1]["timestamp_ms"]
        if gap > 0:
            intervals.append(gap / 1000.0)
    median_interval = _median(intervals) if intervals else 0

    return {
        "start_ts_ms": start_ts,
        "end_ts_ms": end_ts,
        "start_utc": start_utc,
        "end_utc": end_utc,
        "duration_secs": duration_secs,
        "duration_human": _format_duration(duration_secs),
        "symbols": symbols,
        "snapshot_count": len(snapshots),
        "estimated_interval_secs": median_interval,
    }


# ---------------------------------------------------------------------------
# Per-Cycle Zone Counts
# ---------------------------------------------------------------------------

def compute_per_cycle_zone_counts(snapshots: List[dict]) -> List[dict]:
    """Group snapshots by timestamp into cycles, count zones per symbol per cycle."""
    if not snapshots:
        return []

    # Group by timestamp
    by_ts = defaultdict(list)
    for snap in snapshots:
        by_ts[snap["timestamp_ms"]].append(snap)

    cycles = []
    for ts in sorted(by_ts.keys()):
        group = by_ts[ts]
        per_symbol = {}
        total_zones = 0
        for snap in group:
            zone_count = len(snap.get("zones", []))
            per_symbol[snap["symbol"]] = zone_count
            total_zones += zone_count

        cycles.append({
            "timestamp_ms": ts,
            "timestamp_utc": _ms_to_utc(ts),
            "symbols": sorted(snap["symbol"] for snap in group),
            "total_zones": total_zones,
            "per_symbol": per_symbol,
        })

    return cycles


# ---------------------------------------------------------------------------
# Source Freshness Summary
# ---------------------------------------------------------------------------

def compute_source_freshness_summary(snapshots: List[dict]) -> Dict[str, dict]:
    """Compute source freshness summary across all snapshots."""
    source_data = defaultdict(lambda: {"count": 0, "latest_ts_ms": 0, "snapshot_count": 0})

    for snap in snapshots:
        freshness = snap.get("source_freshness", {})
        for source, ts in freshness.items():
            source_data[source]["count"] += 1
            source_data[source]["snapshot_count"] += 1
            if ts > source_data[source]["latest_ts_ms"]:
                source_data[source]["latest_ts_ms"] = ts

    # Convert to output format
    result = {}
    for source, data in source_data.items():
        result[source] = {
            "snapshot_count": data["snapshot_count"],
            "latest_ts_ms": data["latest_ts_ms"],
            "latest_utc": _ms_to_utc(data["latest_ts_ms"]) if data["latest_ts_ms"] > 0 else "N/A",
        }

    return result


# ---------------------------------------------------------------------------
# Gap Detections
# ---------------------------------------------------------------------------

def compute_gap_detections(snapshots: List[dict], interval_secs: float = 30) -> List[dict]:
    """Detect capture gaps where time between consecutive snapshots exceeds 2x interval."""
    if len(snapshots) < 2:
        return []

    interval_ms = interval_secs * 1000
    threshold_ms = 2 * interval_ms
    gaps = []

    for i in range(1, len(snapshots)):
        prev_ts = snapshots[i - 1]["timestamp_ms"]
        curr_ts = snapshots[i]["timestamp_ms"]
        gap_ms = curr_ts - prev_ts

        if gap_ms > threshold_ms:
            gap_secs = gap_ms / 1000.0
            gaps.append({
                "from_ts_ms": prev_ts,
                "to_ts_ms": curr_ts,
                "from_utc": _ms_to_utc(prev_ts),
                "to_utc": _ms_to_utc(curr_ts),
                "gap_ms": gap_ms,
                "gap_secs": gap_secs,
                "gap_human": _format_duration(gap_secs),
                "gap_ratio": gap_ms / interval_ms if interval_ms > 0 else 0,
            })

    return gaps


# ---------------------------------------------------------------------------
# Data Quality Assessment
# ---------------------------------------------------------------------------

def compute_data_quality_assessment(snapshots: List[dict]) -> dict:
    """Assess data quality: enrichment completeness, validation."""
    empty_result = {
        "total_snapshots": 0,
        "snapshots_with_depth": 0,
        "snapshots_with_funding": 0,
        "snapshots_with_local_hl": 0,
        "snapshots_with_vwap": 0,
        "snapshots_with_source_freshness": 0,
        "snapshots_with_imperial_mark": 0,
        "fully_enriched_snapshots": 0,
        "enrichment_rate_pct": 0,
        "stale_source_events": 0,
        "validation_issues": [],
    }
    if not snapshots:
        return empty_result

    with_depth = sum(1 for s in snapshots if s.get("depth_summary"))
    with_funding = sum(1 for s in snapshots if s.get("funding_rate") is not None)
    with_local_hl = sum(1 for s in snapshots if s.get("local_high") is not None and s.get("local_low") is not None)
    with_vwap = sum(1 for s in snapshots if s.get("vwap") is not None)
    with_freshness = sum(1 for s in snapshots if s.get("source_freshness"))
    with_imperial = sum(1 for s in snapshots if s.get("imperial_mark_price") is not None)

    # Check for stale sources
    stale_count = sum(len(s.get("stale_sources", [])) for s in snapshots)

    # Check validation issues
    issues = []
    for snap in snapshots:
        zones = snap.get("zones", [])
        for z in zones:
            conf = z.get("confidence", 0)
            if conf < 0 or conf > 1:
                issues.append(f"{snap['symbol']}@{snap['timestamp_ms']}: confidence {conf} out of range")
            notional = z.get("estimated_notional_usd", 0)
            if notional < 0:
                issues.append(f"{snap['symbol']}@{snap['timestamp_ms']}: negative notional {notional}")

    # A snapshot is "enriched" if it has at least depth + funding + local_hl + vwap
    enriched = sum(1 for s in snapshots
                   if s.get("depth_summary") and s.get("funding_rate") is not None
                   and s.get("local_high") and s.get("vwap"))

    total = len(snapshots)
    return {
        "total_snapshots": total,
        "snapshots_with_depth": with_depth,
        "snapshots_with_funding": with_funding,
        "snapshots_with_local_hl": with_local_hl,
        "snapshots_with_vwap": with_vwap,
        "snapshots_with_source_freshness": with_freshness,
        "snapshots_with_imperial_mark": with_imperial,
        "fully_enriched_snapshots": enriched,
        "enrichment_rate_pct": (enriched / total * 100) if total > 0 else 0,
        "stale_source_events": stale_count,
        "validation_issues": issues,
    }


# ---------------------------------------------------------------------------
# Confidence Distribution
# ---------------------------------------------------------------------------

def compute_confidence_distribution(snapshots: List[dict]) -> dict:
    """Compute confidence distribution across all zones."""
    confidences = []
    for snap in snapshots:
        for zone in snap.get("zones", []):
            confidences.append(zone.get("confidence", 0))

    if not confidences:
        return {
            "total_zones": 0,
            "mean": 0,
            "median": 0,
            "min": 0,
            "max": 0,
            "std": 0,
            "buckets": {
                "low_0_0.3": 0,
                "moderate_0.3_0.5": 0,
                "good_0.5_0.7": 0,
                "high_0.7_1.0": 0,
            },
        }

    mean_conf = sum(confidences) / len(confidences)
    variance = sum((c - mean_conf) ** 2 for c in confidences) / len(confidences)
    std_conf = math.sqrt(variance)

    buckets = {"low_0_0.3": 0, "moderate_0.3_0.5": 0, "good_0.5_0.7": 0, "high_0.7_1.0": 0}
    for c in confidences:
        if c < 0.3:
            buckets["low_0_0.3"] += 1
        elif c < 0.5:
            buckets["moderate_0.3_0.5"] += 1
        elif c < 0.7:
            buckets["good_0.5_0.7"] += 1
        else:
            buckets["high_0.7_1.0"] += 1

    return {
        "total_zones": len(confidences),
        "mean": round(mean_conf, 4),
        "median": round(_median(confidences), 4),
        "min": round(min(confidences), 4),
        "max": round(max(confidences), 4),
        "std": round(std_conf, 4),
        "buckets": buckets,
    }


# ---------------------------------------------------------------------------
# Single-Source vs Multi-Source Zones
# ---------------------------------------------------------------------------

def compute_single_vs_multi_source(snapshots: List[dict]) -> dict:
    """Compute counts and stats for single-source vs multi-source zones."""
    single_confs = []
    multi_confs = []

    for snap in snapshots:
        for zone in snap.get("zones", []):
            source_count = len(zone.get("source_mix", []))
            conf = zone.get("confidence", 0)
            if source_count <= 1:
                single_confs.append(conf)
            else:
                multi_confs.append(conf)

    total = len(single_confs) + len(multi_confs)

    return {
        "single_source_count": len(single_confs),
        "multi_source_count": len(multi_confs),
        "total_zones": total,
        "single_source_pct": (len(single_confs) / total * 100) if total > 0 else 0,
        "multi_source_pct": (len(multi_confs) / total * 100) if total > 0 else 0,
        "single_source_mean_confidence": round(sum(single_confs) / len(single_confs), 4) if single_confs else 0,
        "multi_source_mean_confidence": round(sum(multi_confs) / len(multi_confs), 4) if multi_confs else 0,
    }


# ---------------------------------------------------------------------------
# Source Coverage
# ---------------------------------------------------------------------------

def compute_source_coverage(snapshots: List[dict]) -> Dict[str, dict]:
    """Compute which sources contributed zones and their statistics."""
    source_stats = defaultdict(lambda: {"zone_count": 0, "total_notional": 0.0, "symbols": set()})

    for snap in snapshots:
        sym = snap["symbol"]
        for zone in snap.get("zones", []):
            for source in zone.get("source_mix", []):
                source_stats[source]["zone_count"] += 1
                source_stats[source]["total_notional"] += zone.get("estimated_notional_usd", 0)
                source_stats[source]["symbols"].add(sym)

    # Convert sets to sorted lists
    result = {}
    for source, data in sorted(source_stats.items()):
        result[source] = {
            "zone_count": data["zone_count"],
            "total_notional_usd": round(data["total_notional"], 2),
            "symbols": sorted(data["symbols"]),
        }

    return result


# ---------------------------------------------------------------------------
# Report Generation
# ---------------------------------------------------------------------------

def generate_report(snapshots: List[dict], interval_secs: float = 30) -> str:
    """Generate the full capture run report as markdown."""
    metadata = compute_run_metadata(snapshots)
    cycle_counts = compute_per_cycle_zone_counts(snapshots)
    freshness = compute_source_freshness_summary(snapshots)
    gaps = compute_gap_detections(snapshots, interval_secs)
    quality = compute_data_quality_assessment(snapshots)
    confidence = compute_confidence_distribution(snapshots)
    single_multi = compute_single_vs_multi_source(snapshots)
    coverage = compute_source_coverage(snapshots)

    now_utc = datetime.now(timezone.utc).isoformat()

    # Build report
    lines = []
    lines.append("# Liquidation Zone Capture Run Report")
    lines.append("")
    lines.append(f"**Generated:** {now_utc}")
    lines.append(f"**Assertion:** VAL-REPORTS-001")
    lines.append("")

    # --- Run Metadata ---
    lines.append("## Run Metadata")
    lines.append("")
    lines.append("| Field | Value |")
    lines.append("|-------|-------|")
    lines.append(f"| Start Time | {metadata['start_utc']} |")
    lines.append(f"| End Time | {metadata['end_utc']} |")
    lines.append(f"| Duration | {metadata['duration_human']} ({metadata['duration_secs']:.1f}s) |")
    lines.append(f"| Symbols | {', '.join(metadata['symbols']) if metadata['symbols'] else 'N/A'} |")
    lines.append(f"| Estimated Interval | {metadata.get('estimated_interval_secs', 0):.1f}s |")
    lines.append(f"| Total Snapshots | {metadata['snapshot_count']} |")
    lines.append(f"| Snapshot Directory | `data/liquidation-zones/` |")
    lines.append("")

    # --- Per-Cycle Zone Counts ---
    lines.append("## Per-Cycle Zone Counts")
    lines.append("")

    if cycle_counts:
        # Get all symbols
        all_symbols = sorted(set(sym for c in cycle_counts for sym in c["per_symbol"]))
        header = "| Timestamp (UTC) | " + " | ".join(all_symbols) + " | Total |"
        sep = "|" + "|".join(["---"] * (len(all_symbols) + 2)) + "|"
        lines.append(header)
        lines.append(sep)
        for cycle in cycle_counts:
            ts_short = cycle["timestamp_utc"][:19] if cycle["timestamp_utc"] != "N/A" else "N/A"
            sym_vals = [str(cycle["per_symbol"].get(s, 0)) for s in all_symbols]
            row = f"| {ts_short} | " + " | ".join(sym_vals) + f" | {cycle['total_zones']} |"
            lines.append(row)

        # Summary
        total_zones_all = sum(c["total_zones"] for c in cycle_counts)
        lines.append("")
        lines.append(f"**Total cycles:** {len(cycle_counts)}")
        lines.append(f"**Total zones across all cycles:** {total_zones_all}")
        lines.append(f"**Mean zones per cycle:** {total_zones_all / len(cycle_counts):.1f}" if cycle_counts else "")
    else:
        lines.append("No capture cycles found.")
    lines.append("")

    # --- Source Freshness Summary ---
    lines.append("## Source Freshness Summary")
    lines.append("")
    if freshness:
        lines.append("| Source | Snapshots | Latest Timestamp (UTC) |")
        lines.append("|--------|-----------|----------------------|")
        for source, data in sorted(freshness.items()):
            lines.append(f"| {source} | {data['snapshot_count']} | {data['latest_utc']} |")
    else:
        lines.append("No source freshness data available (snapshots lack source_freshness field).")
    lines.append("")

    # --- Gap Detections ---
    lines.append("## Gap Detections")
    lines.append("")
    if gaps:
        lines.append(f"**{len(gaps)} capture gap(s) detected** (exceeding 2× interval of {interval_secs}s)")
        lines.append("")
        lines.append("| From (UTC) | To (UTC) | Duration | Gap Ratio |")
        lines.append("|------------|----------|----------|-----------|")
        for gap in gaps:
            from_short = gap["from_utc"][:19] if gap["from_utc"] != "N/A" else "N/A"
            to_short = gap["to_utc"][:19] if gap["to_utc"] != "N/A" else "N/A"
            lines.append(f"| {from_short} | {to_short} | {gap['gap_human']} | {gap['gap_ratio']:.1f}× |")
    else:
        lines.append("No capture gaps detected. All snapshots are within the expected interval.")
    lines.append("")

    # --- Data Quality Assessment ---
    lines.append("## Data Quality Assessment")
    lines.append("")
    total = quality["total_snapshots"]
    lines.append("| Metric | Count | Rate |")
    lines.append("|--------|-------|------|")
    lines.append(f"| Total Snapshots | {total} | — |")
    lines.append(f"| With L2 Depth Summary | {quality['snapshots_with_depth']} | {quality['snapshots_with_depth']/total*100:.0f}% |" if total else "| With L2 Depth Summary | 0 | — |")
    lines.append(f"| With Funding Rate | {quality['snapshots_with_funding']} | {quality['snapshots_with_funding']/total*100:.0f}% |" if total else "| With Funding Rate | 0 | — |")
    lines.append(f"| With Local H/L | {quality['snapshots_with_local_hl']} | {quality['snapshots_with_local_hl']/total*100:.0f}% |" if total else "| With Local H/L | 0 | — |")
    lines.append(f"| With VWAP | {quality['snapshots_with_vwap']} | {quality['snapshots_with_vwap']/total*100:.0f}% |" if total else "| With VWAP | 0 | — |")
    lines.append(f"| With Source Freshness | {quality['snapshots_with_source_freshness']} | {quality['snapshots_with_source_freshness']/total*100:.0f}% |" if total else "| With Source Freshness | 0 | — |")
    lines.append(f"| With Imperial Mark Price | {quality['snapshots_with_imperial_mark']} | {quality['snapshots_with_imperial_mark']/total*100:.0f}% |" if total else "| With Imperial Mark Price | 0 | — |")
    lines.append(f"| Fully Enriched (depth+fund+H/L+VWAP) | {quality['fully_enriched_snapshots']} | {quality['enrichment_rate_pct']:.0f}% |")
    lines.append(f"| Stale Source Events | {quality['stale_source_events']} | — |")

    if quality["validation_issues"]:
        lines.append("")
        lines.append("### Validation Issues")
        for issue in quality["validation_issues"]:
            lines.append(f"- {issue}")
    lines.append("")

    # --- Confidence Distribution ---
    lines.append("## Confidence Distribution")
    lines.append("")
    lines.append(f"**Total zones scored:** {confidence['total_zones']}")
    if confidence["total_zones"] > 0:
        lines.append(f"**Mean confidence:** {confidence['mean']:.4f}")
        lines.append(f"**Median confidence:** {confidence['median']:.4f}")
        lines.append(f"**Std deviation:** {confidence['std']:.4f}")
        lines.append(f"**Range:** [{confidence['min']:.4f}, {confidence['max']:.4f}]")
        lines.append("")
        lines.append("| Bucket | Count | Percentage |")
        lines.append("|--------|-------|------------|")
        total_z = confidence["total_zones"]
        for bucket_name, count in confidence["buckets"].items():
            label = bucket_name.replace("low_0_0.3", "Low [0.0, 0.3)") \
                               .replace("moderate_0.3_0.5", "Moderate [0.3, 0.5)") \
                               .replace("good_0.5_0.7", "Good [0.5, 0.7)") \
                               .replace("high_0.7_1.0", "High [0.7, 1.0]")
            pct = count / total_z * 100 if total_z > 0 else 0
            lines.append(f"| {label} | {count} | {pct:.1f}% |")
    else:
        lines.append("No zones with confidence scores available.")
    lines.append("")

    # --- Single-Source vs Multi-Source Zones ---
    lines.append("## Single-Source vs Multi-Source Zones")
    lines.append("")
    sm = single_multi
    lines.append(f"**Single-source zones:** {sm['single_source_count']} ({sm['single_source_pct']:.1f}%)")
    lines.append(f"**Multi-source zones:** {sm['multi_source_count']} ({sm['multi_source_pct']:.1f}%)")
    if sm['single_source_count'] > 0:
        lines.append(f"**Single-source mean confidence:** {sm['single_source_mean_confidence']:.4f}")
    if sm['multi_source_count'] > 0:
        lines.append(f"**Multi-source mean confidence:** {sm['multi_source_mean_confidence']:.4f}")
    if sm['multi_source_count'] > 0 and sm['single_source_count'] > 0:
        delta = sm['multi_source_mean_confidence'] - sm['single_source_mean_confidence']
        lines.append(f"**Multi-source confidence premium:** +{delta:.4f}")
    lines.append("")

    # --- Source Coverage ---
    lines.append("## Source Coverage")
    lines.append("")
    if coverage:
        lines.append("| Source | Zone Count | Total Notional (USD) | Symbols |")
        lines.append("|--------|------------|---------------------|---------|")
        for source, data in coverage.items():
            lines.append(f"| {source} | {data['zone_count']} | ${data['total_notional_usd']:,.2f} | {', '.join(data['symbols'])} |")
    else:
        lines.append("No source coverage data available.")
    lines.append("")

    # --- Footer ---
    lines.append("---")
    lines.append(f"*Report generated by `scripts/generate_capture_run_report.py`*")
    lines.append(f"*Snapshot source: `data/liquidation-zones/`*")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _ms_to_utc(ts_ms: int) -> str:
    """Convert milliseconds timestamp to UTC ISO string."""
    if ts_ms is None or ts_ms == 0:
        return "N/A"
    try:
        dt = datetime.fromtimestamp(ts_ms / 1000.0, tz=timezone.utc)
        return dt.isoformat()
    except (OSError, ValueError, OverflowError):
        return "N/A"


def _median(values: list) -> float:
    """Compute median of a list of numbers."""
    if not values:
        return 0.0
    s = sorted(values)
    mid = len(s) // 2
    if len(s) % 2 == 0 and mid > 0:
        return (s[mid - 1] + s[mid]) / 2
    return s[mid]


def _format_duration(secs: float) -> str:
    """Format seconds into human-readable duration."""
    if secs < 60:
        return f"{secs:.0f}s"
    elif secs < 3600:
        mins = secs / 60
        return f"{mins:.1f}min"
    elif secs < 86400:
        hours = secs / 3600
        return f"{hours:.1f}h"
    else:
        days = secs / 86400
        return f"{days:.1f}d"


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Generate capture run report")
    parser.add_argument("--snapshot-dir", default="data/liquidation-zones",
                        help="Directory containing snapshot JSON files")
    parser.add_argument("--output", default="data/liquidation-capture-run.md",
                        help="Output report path")
    parser.add_argument("--interval-secs", type=int, default=30,
                        help="Expected capture interval in seconds (for gap detection)")
    args = parser.parse_args()

    log.info("Loading snapshots from %s", args.snapshot_dir)
    snapshots = load_snapshots(args.snapshot_dir)
    log.info("Loaded %d snapshots", len(snapshots))

    if not snapshots:
        log.warning("No snapshots found. Generating report with empty data.")

    report = generate_report(snapshots, interval_secs=args.interval_secs)

    # Atomic write
    output_dir = os.path.dirname(args.output)
    if output_dir:
        os.makedirs(output_dir, exist_ok=True)

    tmp_path = args.output + ".tmp"
    with open(tmp_path, "w") as f:
        f.write(report)
    os.rename(tmp_path, args.output)

    log.info("Report written to %s (%d lines, %d chars)",
             args.output, report.count("\n"), len(report))

    # Print summary
    metadata = compute_run_metadata(snapshots)
    confidence = compute_confidence_distribution(snapshots)
    print(f"\nReport: {args.output}")
    print(f"Snapshots: {metadata['snapshot_count']}")
    print(f"Symbols: {', '.join(metadata['symbols'])}")
    print(f"Duration: {metadata['duration_human']}")
    print(f"Total zones: {confidence['total_zones']}")
    print(f"Mean confidence: {confidence['mean']:.4f}")


if __name__ == "__main__":
    main()
