# Issue 0001 — Tracer Bullet: UniV2 Liquidity Registry + Routing

## Goal
Prove the entire liquidity pipeline end-to-end using only Uniswap V2 pools.

## Acceptance Criteria
- [ ] DB migration creates `pools`, `pool_state`, `liquid_pools` tables.
- [ ] `src/pools/registry.rs` can seed Uniswap V2 pools from TheGraph.
- [ ] RPC fallback can scan `PairCreated` events from the UniV2 factory if TheGraph fails.
- [ ] `src/pools/liquidity.rs` fetches `getReserves()` for touched UniV2 pools each block.
- [ ] `src/pools/pricing.rs` computes `tvl_usd` using stablecoin peg + reference-pool pricing.
- [ ] `GET /api/liquid-pools` returns the current top-1,000 UniV2 pools with rank, TVL, reserves.
- [ ] `GET /api/routes?from=...&to=...&amount=...` returns direct UniV2 routes with exact output.
- [ ] Offline unit tests for graph pathfinding and quoting math.
- [ ] Live integration test fetches a known UniV2 pool state at a pinned block.

## Dependencies
- None (this is the first slice).

## Notes / Risks
- Keep this slice focused on UniV2 only. Do not add UniV3, Curve, or vault-style pools yet.
- TheGraph seed failure on first run should fail loudly, matching existing integration-test philosophy.
