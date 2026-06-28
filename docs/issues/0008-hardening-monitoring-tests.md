# Issue 0008 — Hardening, Monitoring, and Tests

## Goal
Make the liquidity/routing feature production-ready with comprehensive tests and observability.

## Acceptance Criteria
- [x] Add integration tests for pool-state fetching at pinned blocks for UniV2/V3/Curve.
  - `univ2_weth_usdc_reserves_at_25_300_000` (UniV2 `getReserves`)
  - existing `univ3_quote_weth_usdc` exercises `slot0` + `liquidity` reads
  - existing `curve_quote_frax_usdc` exercises Curve `A()`/`fee()`/`balances(i)`
- [x] Add integration tests for routing API on well-known token pairs (e.g., WETH→USDC).
  - `routing_finds_weth_usdc_via_known_pool` builds a `TokenGraph` from a live WETH/USDC V3 pool and runs `find_routes` with a known `amount_in`.
- [x] Add unit tests for multi-hop pathfinding edge cases: cycles, dead ends, max-hop limit.
  - `cycle_prevented`, `dead_end_no_route`, `max_hops_respected`, `route_to_self_without_pool_returns_empty`, `intermediate_token_not_whitelisted_blocks_route`.
- [x] Add unit tests for TVL pricing logic.
  - `zero_reserves_collapse_to_zero`, `usde_and_gho_are_stablecoins`, `curve_pool_with_3_coins_prices_all_coins`, `mismatched_lengths_return_none`.
- [x] Add metrics/logging for job cursor lag, RPC failures, TheGraph failures, refresh duration.
  - `LiquidityJob::tick` emits a structured `liquidity_job_tick` log per iteration with `cursor`, `blocks_processed`, `rpc_failures`, `thegraph_failures`, `refresh_duration_ms`, `tick_duration_ms`.
  - TheGraph failures bump `thegraph_failures` and log at WARN; the daily refresh still retried the next tick.
- [x] Document failure modes: first-run seed failure, TheGraph outage, RPC rate limiting.
  - `docs/failure-modes.md` covers all of the above plus replay isolation, empty-registry behaviour, Aave V3 helper reverts, available-liquidity limitation, lending-vs-routing separation.
- [x] Update `docs/adr/` if any architecture decisions meet ADR criteria (e.g., separate liquidity job).
  - `docs/adr/0003-separate-liquidity-job.md`.

## Dependencies
- Issues 0001–0006.

## Notes / Risks
- Live tests require `MEV_ANT_RPC_URL` or the default Reth node; they should fail loudly when unreachable.
- Avoid committed fixtures; use pinned mainnet blocks.
