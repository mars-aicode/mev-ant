# Plan: Liquidity Registry & Routing

## Context

This is a new capability, distinct from sandwich detection. The goal is to collect
on-chain DEX liquidity, maintain it in real time, and expose a routing API for
MEV strategies (arbitrage, sandwich routing, collateral swaps, etc.).

Domain terms are defined in `CONTEXT.md`:
- **Liquid Pool**: global top 1,000 pools by TVL.
- **Pool Address**: contract used to interact with the pool.
- **Pool ID**: bytes32 identifier for vault-style pools (Balancer/UniV4).
- **Pool Snapshot**: latest per-pool state, updated per block.
- **Route**: multi-hop path A→B through Liquid Pools.
- **Lending Market**: tracked separately, not in V1 routing graph.

## Decisions

| Decision | Choice |
|---|---|
| Ranking metric | Top 1,000 pools by TVL, global across Ethereum mainnet. |
| TVL pricing | Stablecoins pegged at $1; WETH/WBTC priced via on-chain reference pools. No external oracles. |
| Seed source | TheGraph primary; RPC factory-event indexing fallback for protocols without a subgraph. |
| Real-time updates | Per block, only for pools touched by state-changing events. |
| Full refresh / re-rank | Daily, synchronized with TheGraph re-seed. |
| Pool identity | Store both `Pool Address` (interaction contract) and `Pool ID` (bytes32, null for UniV2/V3). |
| Pool-ID extraction | Shared module `src/pools/identity.rs` used by classifier and liquidity job. |
| Liquidity job | Separate background job with its own block cursor, independent of the sandwich scanner, so replays do not corrupt pool state. |
| Exact quoting V1 | UniV2, UniV3, Curve — implemented from scratch, verified against pinned mainnet swaps. |
| Other protocols in routing graph | Included as Liquid Pools, but routes through them are annotated `quote_confidence: estimated`. |
| Lending protocols | Tracked separately for liquidation/collateral-swap MEV; not included in V1 routing graph. |
| Pathfinding | DFS with max 3 hops, cycle prevention, intermediate tokens restricted to a static whitelist. |
| API output | Route metadata only; no execution calldata in V1. |
| Tests | Offline unit tests for graph/ranking/quoting; live-Reth integration tests for RPC state fetching and exact quoting. |
| First-run failure | Fail loudly if no DB data exists and TheGraph/fallback is unreachable. Subsequent restarts use existing DB. |

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         TheGraph / RPC fallback                      │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ seed / daily re-seed
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  src/pools/registry.rs                                               │
│  - fetch/parse pool list                                             │
│  - write to `pools` table                                            │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Liquidity Job (src/pools/job.rs)                                    │
│  - independent RPC cursor                                            │
│  - per block: detect touched Liquid Pools via logs                   │
│  - multicall reserves / state                                        │
│  - upsert `pool_state`                                               │
│  - daily: full refresh + re-rank `liquid_pools`                      │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  src/pools/graph.rs + src/pools/routing.rs                           │
│  - build token-pool graph from `liquid_pools`                        │
│  - DFS pathfinding, rank by liquidity/output/fee/hops                │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  src/services.rs + src/api.rs                                        │
│  - /api/liquid-pools                                                 │
│  - /api/liquid-pools/:pool                                           │
│  - /api/routes                                                       │
└─────────────────────────────────────────────────────────────────────┘
```

## Modules to create / modify

### New modules

- `src/pools/mod.rs` — public facade.
- `src/pools/registry.rs` — TheGraph/RPC seeding, pool list management.
- `src/pools/identity.rs` — shared pool-address/ID resolution from swap events.
- `src/pools/liquidity.rs` — snapshot fetching (reserves, protocol state, TVL).
- `src/pools/pricing.rs` — TVL pricing (stable peg + reference pools).
- `src/pools/graph.rs` — token-pool graph.
- `src/pools/routing.rs` — pathfinding and route ranking.
- `src/pools/job.rs` — background liquidity job.
- `src/pools/quoting/mod.rs` — exact quoting trait + UniV2/V3/Curve implementations.
- `src/pools/tests.rs` — offline unit tests.

### Modified modules

- `src/classifier.rs` — use `pools::identity` to emit `PoolId::Param` for Balancer/UniV4.
- `src/detector/engine.rs` — handle `PoolId::Param` when matching attacked pools.
- `src/services.rs` — add `LiquidPoolService`, `RoutingService`.
- `src/api.rs` — add `/api/liquid-pools`, `/api/liquid-pools/:pool`, `/api/routes`.
- `src/db.rs` — add migration for `pools`, `pool_state`, `liquid_pools` tables.

## Database schema

```sql
-- All discovered pools.
CREATE TABLE pools (
    address BYTEA NOT NULL,              -- interaction contract
    pool_id BYTEA NOT NULL DEFAULT '',   -- bytes32; empty for UniV2/V3
    kind TEXT NOT NULL,                  -- DexFamily
    factory BYTEA,
    token0 BYTEA NOT NULL,
    token1 BYTEA NOT NULL,
    fee INTEGER,
    block_created BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (address, pool_id)
);

-- Latest state per pool.
CREATE TABLE pool_state (
    address BYTEA NOT NULL,
    pool_id BYTEA NOT NULL DEFAULT '',
    block_number BIGINT NOT NULL,
    reserve0 NUMERIC,
    reserve1 NUMERIC,
    tvl_usd NUMERIC,
    state JSONB,                         -- protocol-specific quoting data
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (address, pool_id)
);

-- Current top-1,000 Liquid Pools.
CREATE TABLE liquid_pools (
    address BYTEA NOT NULL,
    pool_id BYTEA NOT NULL DEFAULT '',
    rank INTEGER NOT NULL,
    tvl_usd NUMERIC NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (address, pool_id)
);

CREATE INDEX idx_pools_tokens ON pools(token0, token1);
CREATE INDEX idx_pool_state_tvl ON pool_state(tvl_usd DESC);
```

## API specification

### `GET /api/liquid-pools`

List current top-1,000 Liquid Pools.

Query params:
- `limit` (default 100, max 1000)
- `token` — filter pools containing a token. Match is exact-address against
  `token0` or `token1`. Malformed addresses return `400 Bad Request`.

Response:
```json
{
  "pools": [
    {
      "rank": 1,
      "address": "0x...",
      "pool_id": "0x...",
      "kind": "uniswap_v3",
      "token0": "0x...",
      "token1": "0x...",
      "fee": 3000,
      "tvl_usd": 150000000,
      "reserve0": "...",
      "reserve1": "..."
    }
  ]
}
```

### `GET /api/liquid-pools/:pool`

Single pool state for a Liquid Pool (top-1,000 by TVL). `:pool` format is one of:

- `0x<address>` (40 hex chars) — for non-vault pools (UniV2/V3, FraxSwap, PancakeSwap V3, etc.). `pool_id` is treated as `B256::ZERO`.
- `0x<address>:0x<poolId>` (`0x` + 40 hex + `:` + `0x` + 64 hex) — for vault-style pools (Balancer V2/V3, UniV4) where multiple pools share a single Pool Address.

The `0x` prefix is optional on both halves (consistent with the rest of the API; `alloy::Address::parse` and `B256::parse` accept both).

Responses:
- `200 OK` with `{"pool": { ... }}` (single `LiquidPoolRow`, same shape as one element of the list endpoint).
- `400 Bad Request` for malformed paths (empty, non-hex, wrong length, malformed `poolId` after `:`).
- `404 Not Found` when the pool is not in the `liquid_pools` table (i.e., not in the current top-1,000 ranking).

### `GET /api/pools/:pool`

Single pool state for any Tracked Pool — i.e., any pool registered in the
`pools` table, regardless of TVL ranking. Use this for the dashboard link
from a sandwich's `attacked_pool` (which can reference any pool the
classifier saw, not just top-1,000).

Same `:pool` format as `/api/liquid-pools/:pool`. Same response codes
(`200` / `400` / `404`). Response shape is a `TrackedPoolRow` (identical
fields to `LiquidPoolRow` minus `rank`).

### `GET /api/routes`

Find routes between two tokens.

Query params:
- `from` (required)
- `to` (required)
- `amount` (optional; required for output-amount ranking)
- `max_hops` (default 3, max 4)
- `min_tvl_usd` (optional filter)

Response:
```json
{
  "from": "0x...",
  "to": "0x...",
  "routes": [
    {
      "hops": [
        {
          "pool_address": "0x...",
          "pool_id": "0x...",
          "kind": "uniswap_v3",
          "token_in": "0x...",
          "token_out": "0x...",
          "fee": 3000
        }
      ],
      "hop_count": 1,
      "total_fee_bps": 30,
      "total_output": "123456789",
      "min_pool_tvl_usd": 5000000,
      "quote_confidence": "exact"
    }
  ]
}
```

## V1 protocol coverage

### Seeding (registry)

| Protocol | Seed method |
|---|---|
| Curve | TheGraph / RPC fallback |
| Uniswap V2 | TheGraph / RPC fallback |
| Uniswap V3 | TheGraph / RPC fallback |
| Uniswap V4 | TheGraph / RPC fallback |
| Balancer V2/V3 | TheGraph / RPC fallback |
| Fluid DEX | RPC fallback (verify subgraph availability) |
| Frax Swap | RPC fallback (verify subgraph availability) |
| SushiSwap / PancakeSwap | Covered by UniV2/UniV3 signatures if subgraphs unavailable |

### Exact quoting

| Protocol | V1 exact quoting |
|---|---|
| UniV2 | ✅ |
| UniV3 | ✅ |
| Curve | ✅ |
| Others | estimated / not ranked by output |

## Implementation phases

### Phase 0 — Foundation
- Create `src/pools/identity.rs` and wire classifier/detector to use `PoolId::Param`.
- Fix Uniswap V4 PoolManager address in `src/dex/registry.rs`.
- Add DB migration for `pools`, `pool_state`, `liquid_pools`.

### Phase 1 — Registry & seeding
- Implement `src/pools/registry.rs`.
- TheGraph seed for UniV2/V3/V4, Curve, Balancer.
- RPC factory-event fallback.
- Command or startup routine to perform initial seed.

### Phase 2 — Liquidity snapshots
- Implement `src/pools/liquidity.rs` + `src/pools/pricing.rs`.
- Implement touched-pool detection per block.
- Implement daily full refresh.
- Build `liquid_pools` ranking.

### Phase 3 — Quoting
- Implement `src/pools/quoting/` for UniV2/V3/Curve.
- Integration tests against pinned mainnet blocks.

### Phase 4 — Routing API
- Implement `src/pools/graph.rs` + `src/pools/routing.rs`.
- Add `/api/routes`, `/api/liquid-pools`, `/api/liquid-pools/:pool`.

### Phase 5 — Background job
- Implement `src/pools/job.rs` as a separate job with its own cursor.
- Wire into `main.rs` startup alongside the sandwich scanner.

### Phase 6 — Expansion
- Add exact quoting for Balancer, Fluid, Frax, etc.
- Add lending-market tracking (separate from routing).

## Testing strategy

- **Unit tests** (offline):
  - Graph pathfinding on small synthetic token graphs.
  - TVL pricing with hardcoded token prices.
  - Quoting math with hardcoded pool states.
- **Integration tests** (live Reth, fail loudly if unreachable):
  - Seed/fetch a known pool's state at a pinned block.
  - Verify UniV2/V3/Curve exact quote against a real historical swap.
  - Verify routing returns expected paths for well-known token pairs.

## Open questions / follow-ups

1. Confirm TheGraph subgraph URLs/IDs for each V1 protocol. — Resolved: subgraph URLs are pinned in `src/pools/registry.rs`. Hardcoded top-pool lists cover protocols without hosted subgraphs (Balancer V2/V3, Fluid).
2. Decide whether to ship a static bootstrap file to remove the first-run TheGraph dependency. — Resolved: `src/pools/bootstrap.rs` ships `load_bootstrap(path)`, wired into `seed-pools` via `--bootstrap <file>`. The bootstrap is additive (`ON CONFLICT DO NOTHING`) and versioned (`version: 1`). The daily refresh in the Liquidity Job still re-seeds from TheGraph.
3. Define the exact intermediate-token whitelist (start with WETH, WBTC, USDC, USDT, DAI, FRAX, crvUSD, USDe, GHO). — Resolved: `INTERMEDIATE_WHITELIST` in `src/pools/routing.rs` matches the proposed set.
4. Decide whether this plan should be captured in an ADR for the "separate liquidity job" decision. — Resolved: `docs/adr/0003-separate-liquidity-job.md`.

## Bootstrap file format

```json
{
  "version": 1,
  "pools": [
    {
      "address": "0x...",
      "pool_id": "0x0000...",
      "kind": "uniswap_v2",
      "factory": "0x...",
      "token0": "0x...",
      "token0_decimals": 18,
      "token1": "0x...",
      "token1_decimals": 6,
      "fee": 30,
      "block_created": 12345
    }
  ]
}
```

`kind` is the same `PoolKind` enum the registry uses elsewhere (snake_case via
`#[serde(rename_all = "snake_case")]` on the enum). `pool_id` is the empty
bytes32 for non-vault pools. `factory` and `block_created` are optional.
