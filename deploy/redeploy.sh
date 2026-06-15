#!/bin/bash
set -euo pipefail

BIN_SRC="./target/release/mev-ant"
BIN_DST="/usr/local/bin/mev-ant"
DASHBOARD_SRC="./dashboard/dist"
SERVICE="mev-ant"

echo "=== mev-ant quick update ==="

# 1. Build Rust
echo "Building Rust..."
cargo build --release

# 2. Build dashboard
if [ -d ./dashboard ]; then
    echo "Building dashboard..."
    cd dashboard
    npm run build -- --legacy-peer-deps 2>/dev/null || npm run build 2>/dev/null
    cd ..
fi

# 3. Stop service
echo "Stopping $SERVICE..."
sudo systemctl stop "$SERVICE" 2>/dev/null || true

# 4. Copy binary
echo "Copying binary..."
sudo cp "$BIN_SRC" "$BIN_DST"

# 5. Copy dashboard
echo "Copying dashboard..."
sudo mkdir -p /var/lib/mev-ant/dashboard
sudo cp -r "$DASHBOARD_SRC"/* /var/lib/mev-ant/dashboard/ 2>/dev/null || echo "  (no dashboard dist)"

# 6. Start service
echo "Starting $SERVICE..."
sudo systemctl start "$SERVICE"

# 7. Status
sleep 1
sudo systemctl status "$SERVICE" --no-pager -l | head -10

echo ""
echo "=== Done ==="
