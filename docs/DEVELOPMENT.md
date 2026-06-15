# Development Guide

## Code Map

```
src/
├── main.rs          — CLI dispatch (peek/scan/export/serve)
├── config.rs        — Clap argument parsing + serve.toml config
├── api.rs           — REST API (Axum) for dashboard
├── rpc.rs           — RPC client (eth_dxgTraceBlockByNumber)
├── models.rs        — Core types: SandwichBundle, Transfer, TxFlow, etc.
├── classifier.rs    — Address classification (Pool/Token/Infra/Unknown)
├── db.rs            — PostgreSQL schema + insert/query
├── dex/
│   ├── registry.rs  — DEX Swap event topic0 lookup
│   └── types.rs     — DexInfo, DexFamily, PoolSource enums
└── detector/
    ├── mod.rs
    └── sandwich.rs  — Core detection pipeline
```

## Detection Pipeline

`detect_sandwiches()` orchestrates 3 rounds:

```
classify() → discover_executor_trades() → pair_trades() → post_process()
```

### Round 1: Classify + Filter

`classifier::classify()`: receipt-level logs → Pool/Token/Infra/Unknown.

### Round 2: Discovery + Pairing

`discover_executor_trades()`: per-tx, per-Unknown address, compute pool-involved token deltas. `pair_trades()`: group by same-executor, pair front/back trades.

### Round 3: Post-process

Dedup → validate → filter → resolve overlaps.

## Key Functions (sandwich.rs)

| Function | Role |
|----------|------|
| `discover_executor_trades` | Extract executor deltas from transfers |
| `pair_trades` | Pair same-executor trades → candidate bundles |
| `try_build_bundle` | Build + validate a single sandwich bundle |
| `is_consecutive` | Check gap txs between front/back |
| `share_pool` | Verify front/back share same DEX pool |
| `is_reversal` | Check token delta signs reverse |
| `trace_funder` | Trace capital source from executor |
| `compute_costs` | Gas + bribe costs for a bundle |
| `validate_bundles` | Post-hoc validation (pool same, funder consistency) |
| `filter_bundles` | Remove bundles where funder is a pool |
| `resolve_overlaps` | Keep non-overlapping, highest-profit bundles |

## Context (`Ctx`)

Block-level shared state passed to all helpers:

```rust
pub(crate) struct Ctx<'a> {
    block_number: u64,
    tx_flows: &'a [TxFlow],
    pool_set: &'a HashSet<Address>,
    unknown: &'a HashSet<Address>,
    coinbase: Address,
    supported_tokens: &'a [Address],
}
```

## Adding a new DEX

1. Add `DexInfo` entry in `src/dex/registry.rs` with topic0 + event signature
2. Select `PoolSource`:
   - `EventAddress` — pool = emitting contract
   - `IndexedParam0` — pool = first indexed topic (Balancer V2/V3)
   - `IndexedParam2` — pool = third indexed topic (Curve Router)
3. Rebuild — no other changes needed. Addresses emitting the new topic0
   will be classified as Pool automatically.

## Token Support

Add new supported tokens by updating `DEFAULT_TOKENS` in `src/main.rs`.
Profit calculation and victim detection use these tokens.

## Testing

```bash
cargo test                          # 18 unit/integration tests
cargo clippy -- -D warnings         # Lint check
```

Test coverage:
- `trace_funder`: direct sender, pool intermediary, flashloan
- `is_consecutive`: pool touch, supported token, attacker tx, rejection, empty gap
- `is_reversal`: flip sign, same sign, empty front
- `share_pool`: common pool, disjoint pools
- `i128_sat`: small value, max truncation
- `is_sup`: in list, not in list
- `try_build_bundle`: end-to-end simple WETH sandwich

## Deployment

See [deploy/INSTALL.md](../deploy/INSTALL.md) for full guide.

Files:
- `deploy/install.sh` — automated `systemd` installer
- `deploy/redeploy.sh` — quick rebuild + restart
- `deploy/dev.sh` — local dev (API + dashboard)
- `deploy/setup-db.sh` — create DB user + database
- `deploy/mev-ant.service` — hardened systemd unit
- `deploy/serve.toml.example` — config template

Binary location: `/usr/local/bin/mev-ant`
Config location: `/etc/mev-ant/serve.toml`
Dashboard static: `/var/lib/mev-ant/dashboard/`

## Continuous Scanning Service

`mev-ant serve --config serve.toml` runs two loops:

1. **Scanner**: reads `scan_state.next_block`, fetches block from RPC (respecting
   `delay_blocks` for reorg protection), detects sandwiches, stores in DB,
   advances `next_block`. Paused when `scan_state.enabled = false`.

2. **API**: Axum HTTP server on configurable port (default 6080).
   Endpoints in `src/api.rs`.

### Configuration (serve.toml)

```toml
rpc_url = "http://192.168.2.180:8547"
db_url = "postgres://..."
from_block = 0          # initial start (DB owns position after first run)
delay_blocks = 6        # reorg protection
api_port = 6080          # dashboard backend port
```

## Dashboard

Ant Design Pro (React) frontend in `dashboard/`:

```bash
cd dashboard
npm install --legacy-peer-deps
npm run dev     # dev server at :8000
npm run build   # production build
```

Pages:
- **Dashboard** — stats cards, pause/resume, recent sandwiches
- **Sandwiches** — ProTable with pagination, click row for detail modal
- **Attackers** — deduplicated attacker addresses with counts
- **Scan Config** — enabled toggle, jump to block

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/stats` | Aggregate sandwich statistics |
| GET | `/api/sandwiches?page=&pageSize=` | Paginated sandwich list |
| GET | `/api/sandwich?block_number=` | Full bundle detail |
| GET | `/api/attackers` | Known attackers with counts |
| GET | `/api/state` | Scanner status (enabled, next_block) |
| POST | `/api/state/pause` | Pause scanner |
| POST | `/api/state/resume` | Resume scanner |
| POST | `/api/state/jump` | Jump to block (`{"block_number": N}`) |

## Debugging

Set `RUST_LOG=debug` for per-block classification and pairing diagnostics.
Set `RUST_LOG=trace` for per-executor trade detail.

```bash
RUST_LOG=debug cargo run -- peek --from 25268438 --to 25268438
```
