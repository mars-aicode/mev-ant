# Liquidity / Routing Failure Modes

This document captures the known failure paths for the liquidity-registry
and routing feature (Issues 0001–0008) and how the system handles each.

## First-run seed failure

**Symptom:** `mev-ant seed-pools` returns an error before any pool is inserted.

**Causes:**
- TheGraph hosted-service endpoints for UniV2, UniV3, and Curve are
  unreachable or rate-limited.
- Reth returns an error for `getReservesList`, `getReserves`, or any of
  the seed-time `eth_call` requests.

**Behaviour:**
- The `seed-pools` CLI fails loudly (`anyhow::bail!` if UniV2 returned
  zero pools, since the registry cannot start empty).
- `LiquidityJob` spawned in `run_serve` is independent of the CLI and
  will retry on its own schedule; a failed first run leaves
  `liquidity_job_state` empty and the job initialises the cursor to the
  current chain head, then proceeds to daily refreshes.

**Operator action:** inspect logs for `thegraph_unavailable` or
`refresh_liquid_pools` errors. Once TheGraph recovers, re-run
`mev-ant seed-pools` or wait for the next daily refresh.

## TheGraph outage mid-run

**Symptom:** Daily refresh fails; pool counts in `liquid_pools` go stale.

**Behaviour:**
- `refresh_liquid_pools` logs `liquidity job full refresh failed` at
  WARN level and bumps `thegraph_failures` in the per-tick log.
- Existing pool state is **left in place** so routing continues to work
  against the previous snapshot.
- The job retries on its next tick (default every 12s); the refresh
  only fires once per day even on multiple attempts.
- Curve fallback: when TheGraph returns empty, the seed helper
  falls back to the on-chain registry, and finally to a small hardcoded
  list of FRAX/USDC and PAX/3CRV pools.

## RPC rate limiting

**Symptom:** `eth_call` or `eth_getLogs` returns HTTP 429 or `RPC error -32005`.

**Behaviour:**
- The job logs the error at WARN level and increments `rpc_failures`.
- The per-block loop `break`s on a hard touched-pool failure so the
  cursor does not advance past the failing block; the next tick retries.
- The `max_connections(10)` cap on the sqlx pool caps DB concurrency,
  not RPC. RPC is currently a single `reqwest::Client` per process;
  under high block throughput, multiple job ticks may issue concurrent
  RPC calls.

**Operator action:** if the Reth node is rate-limiting, throttle
`liquidity_poll_interval_secs` upward in `serve.toml` to 30s or 60s.

## Sandwich-scanner replay

**Symptom:** An operator triggers `mev-ant replay <from_block>`.

**Behaviour:**
- Replay deletes rows from `sandwiches`, `sandwich_attackers`, and
  `blocks_scanned` from `from_block` onward, then resets the sandwich
  scanner cursor to `from_block`. The liquidity job cursor and the
  `pool_state` and `lending_markets` tables are **not** touched.
- The liquidity job continues independently. While the sandwich
  scanner re-scans historical blocks, the liquidity snapshot for those
  blocks remains the most recent post-block state (e.g., a re-scanned
  block 25,300,000 reads the live pool state, not a back-in-time
  snapshot).

## Empty registry

**Symptom:** `GET /api/liquid-pools` returns an empty list.

**Causes:**
- `mev-ant seed-pools` was never run, and the background job has not
  yet completed its first daily refresh.
- A daily refresh ran but every pool failed state-fetching (RPC down).

**Behaviour:**
- `TokenGraph::new(vec![])` produces an empty graph; route queries
  return `[]` cleanly.
- `update_touched_pools` is a no-op when `pools.is_empty()`.

## Touched-pool detection misses

**Symptom:** A pool that traded in a block has stale state at the next tick.

**Cause:** Touched detection filters logs by the pool's `address` and a
small set of event topics (`Sync`, `Swap`, `Mint`, `Burn`, `TokenExchange`).
Vault-style protocols (Balancer V2, UniV4) emit a single event from the
Vault/PoolManager for many underlying pools; the per-block touched set
in the detector is augmented by transfer counterparty addresses (see
`src/detector/mod.rs`), but a non-vault pool whose event signature
isn't in `DEX_REGISTRY` will be missed.

**Operator action:** add the missing topic0 to `src/dex/registry.rs`
with the correct `family` and `pool_source`.

## Aave V3 single-word rate helper reverts

**Symptom:** `aave_v3_reserves_and_rates` integration test fails with
`execution reverted` on the rate helpers.

**Status:** Mitigated. `src/pools/lending.rs` uses the full
`getReserveData(address)` ABI-encoded struct and decodes the rate
fields at known word offsets. The single-word rate helpers
(`getReserveCurrentLiquidityRate` etc.) are not present in older
Aave V3 v3.0 implementations and are not relied on.

## Available liquidity is always None

**Symptom:** `GET /api/lending-markets` shows `available_liquidity: null`.

**Status:** Known V1 limitation. Reading the aToken's underlying balance
requires following the `aTokenAddress` field from `getReserveData` and
issuing another `eth_call` (or `eth_call` to the AaveProtocolDataProvider).
Both add latency and a second point of failure. Deferred to a follow-up
issue.

## Lending and routing are intentionally separate

**Symptom:** A natural-language query like "swap WETH for USDC using Aave
collateral" returns no routes.

**Status:** By design. `/api/routes` operates on the DEX graph only;
`/api/lending-markets` exposes Aave V3 state. Combining them (borrow →
swap → repay) is V2 scope and would need a separate path-finder that
treats lending rates as edge weights and tracks collateralisation
health factors.
