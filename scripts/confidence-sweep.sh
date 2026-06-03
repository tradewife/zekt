#!/bin/bash
# Confidence Threshold Sweep for Liquidation Zone Strategies
# Runs 5 thresholds × 4 strategies = 20 runs
set -euo pipefail

BINARY="./target/release/zekt"
SNAPSHOT_DIR="data/liquidation-zones/"
OUTPUT_DIR="data/confidence-sweep-results"
RESULTS_JSON="$OUTPUT_DIR/sweep_results.json"

mkdir -p "$OUTPUT_DIR"

# Thresholds to sweep
THRESHOLDS=(0.25 0.30 0.35 0.40 0.45)

# Strategy configs: name|confidence_field
STRATEGIES=(
    "cascade-continuation|confidence_min"
    "sweep-reclaim|min_confidence"
    "liquidity-memory-fisher|min_confidence"
    "liquidation-zone-arbiter|min_zone_confidence"
)

echo "=== Confidence Threshold Sweep ==="
echo "Strategies: ${#STRATEGIES[@]}"
echo "Thresholds: ${#THRESHOLDS[@]}"
echo "Total runs: $((${#STRATEGIES[@]} * ${#THRESHOLDS[@]}))"
echo ""

# Initialize results JSON
echo '[]' > "$RESULTS_JSON"

run_sweep() {
    local strategy="$1"
    local confidence_field="$2"
    local threshold="$3"
    
    local safe_strategy="${strategy//-/_}"
    local output_prefix="$OUTPUT_DIR/${safe_strategy}_conf_${threshold}"
    
    # Build param override JSON with enabled:true and the confidence field
    local param_override="{\"enabled\": true, \"${confidence_field}\": ${threshold}}"
    
    echo "Running: $strategy with ${confidence_field}=${threshold}"
    echo "  Override: $param_override"
    
    # Run the replay
    local output_file="$output_prefix.txt"
    $BINARY --liquidation-replay \
        --strategy "$strategy" \
        --snapshot-dir "$SNAPSHOT_DIR" \
        --starting-balance 1000 \
        --param-override "$param_override" \
        > "$output_file" 2>&1 || true
    
    echo "  Output saved to: $output_file"
}

total=0
for strategy_entry in "${STRATEGIES[@]}"; do
    IFS='|' read -r strategy confidence_field <<< "$strategy_entry"
    for threshold in "${THRESHOLDS[@]}"; do
        total=$((total + 1))
        echo "--- Run $total ---"
        run_sweep "$strategy" "$confidence_field" "$threshold"
        echo ""
    done
done

echo "=== Sweep complete: $total runs ==="
echo "Results in: $OUTPUT_DIR/"
