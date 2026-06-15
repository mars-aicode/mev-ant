# mev-ant

Historical sandwich MEV scanner for Ethereum mainnet. Detects sandwich bundles from on-chain data via a custom `eth_dxgTraceBlockByNumber` RPC endpoint.

## Quick Start

```bash
# Build
cargo build --release

# Scan blocks and print to console
./target/release/mev-ant peek --from 25268438 --to 25268438

# Scan blocks and store in PostgreSQL
./target/release/mev-ant scan --pg-url "postgres://user:pass@192.168.2.185/mev" \
    --from 25268438 --to 25268438

# Export stored results
./target/release/mev-ant export --pg-url "..." --limit 100

# Start continuous scanning service + management API
./target/release/mev-ant serve --config serve.toml
```

## Requirements

- **RPC**: Self-hosted Reth node with `eth_dxgTraceBlockByNumber` support (`http://192.168.2.180:8547`)
- **PostgreSQL**: For `scan` and `export` commands (`192.168.2.185`)
- **Rust**: 1.75+

## CLI

### `peek` — Print sandwiches to console

| Flag | Description |
|------|-------------|
| `--from` | Start block (required) |
| `--to` | End block (required) |
| `--format` | Output mode: `summary`, `json`, `csv` |
| `--rpc` | RPC endpoint URL |

### `scan` — Store sandwiches in PostgreSQL

Same flags as `peek`, plus:
| Flag | Description |
|------|-------------|
| `--pg-url` | PostgreSQL connection string |

### `export` — Query stored sandwiches

| Flag | Description |
|------|-------------|
| `--pg-url` | PostgreSQL connection string |
| `--limit` | Max rows to return |
| `--format` | `json` or `csv` |

### `serve` — Continuous scanning service

Reads configuration from a TOML file and runs indefinitely:
- Scans blocks one at a time behind chain tip (delay_blocks for reorg protection)
- Stores results in PostgreSQL
- Exposes REST API on configurable port for dashboard

See [serve.toml](serve.toml) for configuration options.

## Management Dashboard

Ant Design Pro frontend in `dashboard/`. Connects to the `serve` API.

```bash
cd dashboard
npm install --legacy-peer-deps
npm run dev    # dev server on :8000
npm run build  # production build
```

## Sandwich Detection

Detects 3-transaction sandwich bundles (frontrun → victim → backrun) using Transfer event deltas and Swap event pool classification. Supports 12+ DEX families.

Profit tracked in supported tokens: WETH, USDC, USDT, DAI, WBTC.

See [CONTEXT.md](CONTEXT.md) for full glossary and algorithm details.

## Database Schema

| Table | Purpose |
|-------|---------|
| `sandwiches` | Per-bundle detection results (profit, costs, actors) |
| `sandwich_attackers` | Discovered attacker actor sets |
| `blocks_scanned` | Tracking which blocks have been scanned |
| `scan_state` | Singleton row for service state (next_block, enabled) |

## Architecture

Source modules:
- `detector/sandwich.rs` — Core detection pipeline
- `classifier.rs` — Address classification (Pool/Token/Infra/Unknown)
- `dex/registry.rs` — DEX Swap event topic0 registry
- `rpc.rs` — RPC client for `eth_dxgTraceBlockByNumber`
- `api.rs` — REST API for management dashboard (Axum)
- `db.rs` — PostgreSQL schema + insert/query
- `models.rs` — Data types (SandwichBundle, Transfer, etc.)
- `config.rs` — CLI argument parsing + serve TOML config
- `main.rs` — Entry point, command dispatch

## Deployment

See [deploy/INSTALL.md](deploy/INSTALL.md) for full deployment guide.

```bash
# Automated (systemd)
cargo build --release
cd dashboard && npm run build && cd ..
cp deploy/serve.toml.example serve.toml  # edit rpc_url + db_url
sudo ./deploy/install.sh

# Manual
sudo cp target/release/mev-ant /usr/local/bin/
sudo cp deploy/mev-ant.service /etc/systemd/system/
sudo systemctl enable --now mev-ant
```

Deploy files:
- `deploy/install.sh` — automated installer
- `deploy/mev-ant.service` — systemd unit (unprivileged, read-only fs)
- `deploy/INSTALL.md` — detailed guide + uninstall + upgrade
- `deploy/serve.toml.example` — config template
