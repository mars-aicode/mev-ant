# Deployment Guide

## Prerequisites

- Debian/Ubuntu Linux with systemd
- Rust toolchain (for building)
- PostgreSQL (accessible from target machine)
- Reth node with `eth_dxgTraceBlockByNumber` support
- Node.js (for building dashboard)

### 0. Database setup

```bash
./deploy/setup-db.sh
# Prompts for mevant password, creates user + mev_ant database
# Then update serve.toml with the password
```

```bash
# Build
cargo build --release
cd dashboard && npm run build && cd ..

# Copy + edit config
cp deploy/serve.toml.example serve.toml
# Edit serve.toml: set rpc_url, db_url

# Install (as root or with sudo)
sudo ./deploy/install.sh
```

## Manual Install

### 1. Build

```bash
cargo build --release
cd dashboard && npm install --legacy-peer-deps && npm run build && cd ..
```

### 2. Create user

```bash
sudo useradd -r -s /bin/false mevant
```

### 3. Install binary

```bash
sudo cp target/release/mev-ant /usr/local/bin/
sudo chmod 755 /usr/local/bin/mev-ant
```

### 4. Install config

```bash
sudo mkdir -p /etc/mev-ant
sudo cp serve.toml /etc/mev-ant/serve.toml
sudo chown -R mevant:mevant /etc/mev-ant
```

### 5. Install dashboard (optional)

```bash
sudo mkdir -p /var/lib/mev-ant/dashboard
sudo cp -r dashboard/dist/* /var/lib/mev-ant/dashboard/
sudo chown -R mevant:mevant /var/lib/mev-ant
```

### 6. Install systemd service

```bash
sudo cp deploy/mev-ant.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now mev-ant
```

## Verify

```bash
# Check service status
systemctl status mev-ant

# View logs
journalctl -u mev-ant -f

# Test API
curl http://localhost:6080/api/stats

# Dashboard
open http://localhost:6080
```

## Configuration

See [deploy/serve.toml.example](serve.toml.example) for all options:

| Field | Description | Default |
|-------|-------------|---------|
| `rpc_url` | Reth RPC endpoint | — |
| `db_url` | PostgreSQL connection string | — |
| `from_block` | Initial scan position (DB takes over) | 0 |
| `delay_blocks` | Lag behind chain tip | 6 |
| `api_port` | Dashboard API port | 6080 |

## Uninstall

```bash
sudo systemctl stop mev-ant
sudo systemctl disable mev-ant
sudo rm /etc/systemd/system/mev-ant.service
sudo rm /usr/local/bin/mev-ant
sudo rm -rf /etc/mev-ant /var/lib/mev-ant
sudo userdel mevant
```

## Upgrading

```bash
# Rebuild
cargo build --release
cd dashboard && npm run build && cd ..

# Deploy new binary
sudo systemctl stop mev-ant
sudo cp target/release/mev-ant /usr/local/bin/
sudo cp -r dashboard/dist/* /var/lib/mev-ant/dashboard/
sudo systemctl start mev-ant
```
