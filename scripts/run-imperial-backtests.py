#!/usr/bin/env python3
"""Run backtests for all strategies × cost modes and collect results."""

import json
import os
import subprocess
import sys
from pathlib import Path

STRATEGIES = [
    "blueprint-scalper",
    "blueprint-mean-revert",
    "blueprint-cluster-002",
    "blueprint-cluster-003",
    "blueprint-cluster-005",
    "blueprint-cluster-006",
    "blueprint-cluster-007",
    "blueprint-cluster-008",
    "blueprint-cluster-009",
    "blueprint-hft-market-maker",
]

COST_MODES = ["flash-only", "imperial-route-oracle"]
MARKETS = "BTC,SOL,ETH"
START_DATE = "2026-04-01"
INTERVAL = "5m"
BALANCE = "1000"
OUTDIR = Path("data/backtest-imperial-comparison")

def main():
    OUTDIR.mkdir(parents=True, exist_ok=True)
    results = {}

    for cost_mode in COST_MODES:
        for strategy in STRATEGIES:
            label = f"{strategy}__{cost_mode}"
            outfile = OUTDIR / f"{label}.json"

            if outfile.exists():
                print(f"[SKIP] {label} (already exists)")
                results[label] = json.loads(outfile.read_text())
                continue

            print(f"\n{'='*60}")
            print(f"  Running: {strategy} ({cost_mode})")
            print(f"{'='*60}")

            cmd = [
                "./target/release/zekt",
                "--backtest",
                "--strategies", strategy,
                "--markets", MARKETS,
                "--backtest-start", START_DATE,
                "--backtest-interval", INTERVAL,
                "--paper-balance", BALANCE,
                "--cost-mode", cost_mode,
            ]

            try:
                result = subprocess.run(
                    cmd,
                    capture_output=True,
                    text=True,
                    timeout=300,  # 5 minute timeout per backtest
                )

                # Read summary.json
                summary_path = Path("data/backtest-results/summary.json")
                if summary_path.exists():
                    data = json.loads(summary_path.read_text())
                    # Verify the strategy matches
                    cells = data.get("cells", [])
                    if cells:
                        cell_strategies = set(c["strategy"] for c in cells)
                        if strategy not in cell_strategies:
                            print(f"  WARNING: summary.json has wrong strategy: {cell_strategies} (expected {strategy})")
                            # Write stderr for debugging
                            print(f"  stderr: {result.stderr[-500:] if result.stderr else 'none'}")
                            print(f"  Retrying...")
                            # Remove stale summary and retry
                            summary_path.unlink()
                            result = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
                            if summary_path.exists():
                                data = json.loads(summary_path.read_text())
                                cells = data.get("cells", [])
                                cell_strategies = set(c["strategy"] for c in cells)
                                if strategy not in cell_strategies:
                                    print(f"  STILL WRONG after retry: {cell_strategies}")
                                    continue

                    # Save to named file
                    outfile.write_text(json.dumps(data, indent=2))
                    total_trades = data.get("total_trades", 0)
                    net_pnl = data.get("total_net_pnl", 0)
                    total_fees = data.get("total_fees", 0)
                    print(f"  OK: {total_trades} trades, net_pnl=${net_pnl:.2f}, fees=${total_fees:.2f}")
                    results[label] = data
                else:
                    print(f"  ERROR: No summary.json produced")
                    print(f"  stdout: {result.stdout[-500:] if result.stdout else 'none'}")
                    print(f"  stderr: {result.stderr[-500:] if result.stderr else 'none'}")

            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT after 300s")
            except Exception as e:
                print(f"  ERROR: {e}")

    print(f"\n\nCompleted. {len(results)} results collected.")
    return results

if __name__ == "__main__":
    main()
