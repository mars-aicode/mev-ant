# ADR: Separate background job for liquidity updates

Status: accepted
Date: 2026-06-18
Scope: liquidity-registry architecture — concurrency, replay isolation

## Context

The liquidity-registry and routing feature (Issues 0001–0008) needs to
keep `pool_state` and `liquid_pools` current as the chain advances. The
workload has a fundamentally different shape from the sandwich scanner:

- **Sandwich scanner** reads one block at a time, classifies the
  transfers, and writes zero or more sandwich bundles. The result
  depends only on the block contents.
- **Liquidity job** reads per-pool state (reserves, slot0+liquidity,
  Curve `A()`/`balances`) via multicall, and writes it keyed by
  `(address, pool_id, block_number)`. The result is the union of a
  per-block "touched pools" log filter and an on-demand `eth_call`.

Running both workloads in the same loop on the same connection pool
creates two problems:

1. **Replay coupling.** A sandwich-scanner replay (delete
   `sandwiches` and reset the cursor) should not invalidate the
   liquidity snapshots the dashboard is reading. A single combined
   cursor would force a choice: either the replay also wipes pool
   state, or the replay leaves pool state stale because the cursor
   advances backwards.
2. **RPC starvation.** Per-block touched-pool detection fans out to
   one `eth_getLogs` per DEX family plus one multicall per touched
   pool. Sandwich detection is one `eth_dxgTraceBlockByNumber` per
   block. Sharing a single in-flight RPC budget would let the
   liquidity loop dominate under heavy block throughput.

## Decision

Liquidity updates run in a **separate tokio task** with its own
`liquidity_job_state` cursor, its own `RpcClient`, and its own
`liquidity_poll_interval_secs` schedule. The sandwich scanner remains
unaware of the liquidity job.

- Cursor storage: `liquidity_job_state(id, next_block, last_full_refresh_at)`.
- Refresh: per-block touched pools + daily full refresh from TheGraph
  + on-chain state fetch.
- API exposure: `/api/liquid-pools` and `/api/routes` read from the
  pool tables and are unaffected by scanner replay.

## Consequences

Positive:

- **Replay isolation.** A scanner replay only touches `sandwiches`,
  `sandwich_attackers`, and `blocks_scanned` from `from_block` onward.
  The liquidity cursor and pool state tables are not modified. The
  dashboard continues to show the latest pool state while the scanner
  re-scans historical blocks.
- **Independent failure domains.** TheGraph outages on the daily
  refresh leave the per-block touched updates running; RPC failures
  during touched updates leave the daily refresh schedule intact. The
  two paths log separately so operators can tell which one degraded.
- **Independent configuration.** `liquidity_poll_interval_secs` and
  `liquidity_top_n` can be tuned without affecting scanner cadence.

Negative:

- **Two DB pools** would have been nice for full isolation, but
  `max_connections(10)` is sufficient for the combined workload
  because each path issues short-lived queries. We accept the
  shared pool.
- **Lending markets and DEX pools are tracked separately by the job**
  but read from the same DB. The lending update path is gated by
  `lending_enabled` (default true) so operators can disable it
  without disabling the DEX path.

## Alternatives considered

- **Combined cursor.** Simpler conceptually, but a scanner replay
  would force a choice between wiping pool state (broken dashboard) or
  skipping past stale data (broken replay). Rejected.
- **PostgreSQL LISTEN/NOTIFY for pool state changes.** Adds
  complexity for marginal benefit; the job already polls at a
  configurable interval. Rejected for V1.
