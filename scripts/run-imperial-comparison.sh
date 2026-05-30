#!/bin/bash
# Run backtests for all 10 blueprint strategies × 3 markets × 2 cost modes
# Results saved as individual JSON files for comparison table generation

set -euo pipefail

STRATEGIES=(
    "blueprint-scalper"
    "blueprint-mean-revert"
    "blueprint-cluster-002"
    "blueprint-cluster-003"
    "blueprint-cluster-005"
    "blueprint-cluster-006"
    "blueprint-cluster-007"
    "blueprint-cluster-008"
    "blueprint-cluster-009"
    "blueprint-hft-market-maker"
)

START_DATE="2026-04-01"
MARKETS="BTC,SOL,ETH"
BALANCE=1000
INTERVAL="5m"
OUTDIR="data/backtest-imperial-comparison"

mkdir -p "$OUTDIR"

for COST_MODE in "flash-only" "imperial-route-oracle"; do
    echo "=========================================="
    echo "  Cost mode: $COST_MODE"
    echo "=========================================="
    for STRAT in "${STRATEGIES[@]}"; do
        echo ""
        echo ">>> Running $STRAT ($COST_MODE) ..."
        OUTFILE="$OUTDIR/${STRAT}__${COST_MODE}.json"
        if [ -f "$OUTFILE" ]; then
            echo "    Already exists: $OUTFILE (skipping)"
            continue
        fi
        ./target/release/zekt \
            --backtest \
            --strategies "$STRAT" \
            --markets "$MARKETS" \
            --backtest-start "$START_DATE" \
            --backtest-interval "$INTERVAL" \
            --paper-balance "$BALANCE" \
            --cost-mode "$COST_MODE" \
            2>&1 | grep -E "(Total Net|Final balance|Sharpe|cost_mode|imperial-route|Route cost|BACKTEST RESULTS|Starting Balance)" || true

        # Copy summary to named file
        if [ -f "data/backtest-results/summary.json" ]; then
            cp "data/backtest-results/summary.json" "$OUTFILE"
            echo "    Saved: $OUTFILE"
        else
            echo "    WARNING: No summary.json produced for $STRAT ($COST_MODE)"
        fi
    done
done

echo ""
echo "All backtests complete. Results in $OUTDIR/"
ls -la "$OUTDIR/"
