#!/usr/bin/env bash
# Manual start script for the Zekt Liquidation Zone Capture service.
#
# Usage:
#   ./scripts/start-liquidation-capture.sh [OPTIONS]
#
# This script runs the capture script directly (outside of systemd) for
# manual / ad-hoc capture runs.  For durable background operation, use
# the systemd service instead:
#   systemctl --user start zekt-liquidation-capture.service
#
# Options:
#   --cycles N          Number of capture cycles (default: 1)
#   --interval-secs S   Seconds between cycles (default: 30)
#   --output-dir DIR    Snapshot output directory (default: data/liquidation-zones)
#   --wallets-file FILE Wallet list JSON (default: data/watchlist.json)
#   --daemon            Run continuously (equivalent to --cycles 999999)
#   --health            Print health/status JSON and exit
#   -h, --help          Show this help message
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

CYCLES=""
INTERVAL=""
OUTPUT_DIR=""
WALLETS_FILE=""
DAEMON=false
HEALTH=false
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cycles)
            CYCLES="$2"
            shift 2
            ;;
        --interval-secs)
            INTERVAL="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --wallets-file)
            WALLETS_FILE="$2"
            shift 2
            ;;
        --daemon)
            DAEMON=true
            shift
            ;;
        --health)
            HEALTH=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Run the Zekt Liquidation Zone Capture script manually."
            echo ""
            echo "Options:"
            echo "  --cycles N          Number of capture cycles (default: 1)"
            echo "  --interval-secs S   Seconds between cycles (default: 30)"
            echo "  --output-dir DIR    Snapshot output directory"
            echo "  --wallets-file FILE Wallet list JSON"
            echo "  --daemon            Run continuously (infinite cycles)"
            echo "  --health            Print health/status JSON and exit"
            echo "  -h, --help          Show this help message"
            echo ""
            echo "For durable background operation, use the systemd service:"
            echo "  systemctl --user start zekt-liquidation-capture.service"
            exit 0
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

# Ensure we're in the project directory
cd "$PROJECT_DIR"

# Ensure data directories exist
mkdir -p data/liquidation-zones
mkdir -p data/liquidity-memory

# Build command
CMD=(python3 scripts/liquidation-capture.py)

if $HEALTH; then
    CMD+=(--health)
    exec "${CMD[@]}"
fi

if $DAEMON; then
    CYCLES=999999
fi

if [[ -n "$CYCLES" ]]; then
    CMD+=(--cycles "$CYCLES")
fi

if [[ -n "$INTERVAL" ]]; then
    CMD+=(--interval-secs "$INTERVAL")
fi

if [[ -n "$OUTPUT_DIR" ]]; then
    CMD+=(--output-dir "$OUTPUT_DIR")
fi

if [[ -n "$WALLETS_FILE" ]]; then
    CMD+=(--wallets-file "$WALLETS_FILE")
fi

CMD+=("${EXTRA_ARGS[@]}")

echo "=== Zekt Liquidation Zone Capture ==="
echo "Command: ${CMD[*]}"
echo "Working directory: $(pwd)"
echo "PID: $$"
echo "Started at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "Press Ctrl+C to stop"
echo ""

exec "${CMD[@]}"
