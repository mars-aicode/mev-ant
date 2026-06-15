#!/bin/bash
set -euo pipefail

BIN_SRC="./target/release/mev-ant"
BIN_DST="/usr/local/bin/mev-ant"
CONF_DST="/etc/mev-ant"
DASHBOARD_SRC="./dashboard/dist"
DASHBOARD_DST="/var/lib/mev-ant/dashboard"
SERVICE_SRC="./deploy/mev-ant.service"
SERVICE_DST="/etc/systemd/system/mev-ant.service"
USER="mevant"

echo "=== mev-ant installer ==="

# 1. Check binary
if [ ! -f "$BIN_SRC" ]; then
    echo "Error: binary not found at $BIN_SRC. Run 'cargo build --release' first."
    exit 1
fi

# 2. Check config
if [ ! -f ./serve.toml ]; then
    echo "Error: serve.toml not found. Copy from deploy/serve.toml.example and edit."
    exit 1
fi

# 3. Build dashboard
if [ -d ./dashboard ] && [ ! -f "$DASHBOARD_SRC/index.html" ]; then
    echo "Building dashboard..."
    cd dashboard && npm install --legacy-peer-deps --silent && npm run build --silent && cd ..
fi

# 4. Create user
if ! id -u "$USER" >/dev/null 2>&1; then
    echo "Creating user $USER..."
    sudo useradd -r -s /bin/false "$USER"
fi

# 5. Install binary
echo "Installing binary to $BIN_DST..."
sudo cp "$BIN_SRC" "$BIN_DST"
sudo chmod 755 "$BIN_DST"

# 6. Install config
echo "Installing config to $CONF_DST..."
sudo mkdir -p "$CONF_DST"
sudo cp ./serve.toml "$CONF_DST/serve.toml"
sudo chown -R "$USER:$USER" "$CONF_DST"

# 7. Install dashboard
if [ -f "$DASHBOARD_SRC/index.html" ]; then
    echo "Installing dashboard to $DASHBOARD_DST..."
    sudo mkdir -p "$DASHBOARD_DST"
    sudo cp -r "$DASHBOARD_SRC"/* "$DASHBOARD_DST"/
    sudo chown -R "$USER:$USER" "$DASHBOARD_DST"
fi

# 8. Install systemd service
echo "Installing systemd service..."
sudo cp "$SERVICE_SRC" "$SERVICE_DST"
sudo systemctl daemon-reload
sudo systemctl enable --now mev-ant.service

# 9. Check status
sleep 2
sudo systemctl status mev-ant.service --no-pager
echo ""
echo "=== Done ==="
echo "API/Dashboard: http://localhost:$(grep api_port serve.toml | grep -oP '\d+')"
echo "Logs: journalctl -u mev-ant -f"
