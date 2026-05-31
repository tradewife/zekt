#!/usr/bin/env bash
# Install zekt-liquidation-capture as a systemd user service.
#
# Usage:
#   ./scripts/install-liquidation-capture-service.sh [--enable] [--start]
#
# Flags:
#   --enable   Enable the service to start at login (default: off)
#   --start    Start the service immediately after installing (default: off)
#
# The service unit file is copied to ~/.config/systemd/user/ and daemon is
# reloaded.  The service runs scripts/liquidation-capture.py in a continuous
# loop with Restart=on-failure and RestartSec=10.
#
# Logs are available via:
#   journalctl --user -u zekt-liquidation-capture.service -f
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
UNIT_NAME="zekt-liquidation-capture.service"
UNIT_SRC="$SCRIPT_DIR/$UNIT_NAME"
UNIT_DEST_DIR="$HOME/.config/systemd/user"
UNIT_DEST="$UNIT_DEST_DIR/$UNIT_NAME"

ENABLE=false
START=false

for arg in "$@"; do
    case "$arg" in
        --enable) ENABLE=true ;;
        --start)  START=true ;;
        -h|--help)
            echo "Usage: $0 [--enable] [--start]"
            echo ""
            echo "Install zekt-liquidation-capture as a systemd user service."
            echo ""
            echo "Options:"
            echo "  --enable   Enable the service to start at login"
            echo "  --start    Start the service immediately"
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

echo "=== Zekt Liquidation Capture Service Installer ==="
echo ""

# Pre-flight checks
if [[ ! -f "$UNIT_SRC" ]]; then
    echo "ERROR: Unit file not found at $UNIT_SRC" >&2
    exit 1
fi

if ! command -v systemctl &>/dev/null; then
    echo "ERROR: systemctl not found. systemd user services are not available." >&2
    exit 1
fi

# Verify the user systemd directory exists and systemctl --user works
if ! systemctl --user status &>/dev/null; then
    echo "ERROR: systemd user session not available." >&2
    echo "Hint: Ensure you have a valid login session (loginctl)." >&2
    exit 1
fi

# Verify the capture script exists
CAPTURE_SCRIPT="$PROJECT_DIR/scripts/liquidation-capture.py"
if [[ ! -f "$CAPTURE_SCRIPT" ]]; then
    echo "ERROR: Capture script not found at $CAPTURE_SCRIPT" >&2
    exit 1
fi

# Verify Python3 is available
if ! command -v python3 &>/dev/null; then
    echo "ERROR: python3 not found." >&2
    exit 1
fi

# Ensure data directories exist
mkdir -p "$PROJECT_DIR/data/liquidation-zones"
mkdir -p "$PROJECT_DIR/data/liquidity-memory"

# Create watchlist file if missing
WATCHLIST="$PROJECT_DIR/data/watchlist.json"
if [[ ! -f "$WATCHLIST" ]]; then
    echo '{"wallets": []}' > "$WATCHLIST"
    echo "Created empty watchlist at $WATCHLIST"
fi

# Install unit file
echo "Installing service unit..."
mkdir -p "$UNIT_DEST_DIR"
cp "$UNIT_SRC" "$UNIT_DEST"
echo "  Copied: $UNIT_SRC -> $UNIT_DEST"

# Reload systemd daemon
echo "Reloading systemd daemon..."
systemctl --user daemon-reload

# Verify unit is recognized
if systemctl --user list-unit-files | grep -q "$UNIT_NAME"; then
    echo "  Unit recognized by systemd"
else
    echo "ERROR: Unit not recognized after daemon-reload" >&2
    exit 1
fi

# Optionally enable
if $ENABLE; then
    echo "Enabling service to start at login..."
    systemctl --user enable "$UNIT_NAME"
    echo "  Enabled"
fi

# Optionally start
if $START; then
    echo "Starting service..."
    systemctl --user start "$UNIT_NAME"
    sleep 2
    if systemctl --user is-active --quiet "$UNIT_NAME"; then
        echo "  Service started successfully (active)"
    else
        echo "WARNING: Service may not have started correctly." >&2
        systemctl --user status "$UNIT_NAME" || true
    fi
fi

echo ""
echo "=== Installation Complete ==="
echo ""
echo "Service:  $UNIT_NAME"
echo "Unit:     $UNIT_DEST"
echo "Project:  $PROJECT_DIR"
echo ""
echo "Commands:"
echo "  Start:   systemctl --user start $UNIT_NAME"
echo "  Stop:    systemctl --user stop $UNIT_NAME"
echo "  Status:  systemctl --user status $UNIT_NAME"
echo "  Logs:    journalctl --user -u $UNIT_NAME -f"
echo "  Enable:  systemctl --user enable $UNIT_NAME"
echo "  Disable: systemctl --user disable $UNIT_NAME"
