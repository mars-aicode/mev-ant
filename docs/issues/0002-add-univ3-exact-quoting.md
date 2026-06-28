# Issue 0002 — Add Uniswap V3 Exact Quoting

## Goal
Extend the liquidity pipeline to support Uniswap V3 pools with exact output quoting.

## Acceptance Criteria
- [x] `src/pools/registry.rs` seeds Uniswap V3 pools from TheGraph (or RPC factory fallback).
- [x] `src/pools/liquidity.rs` fetches V3 state: `slot0()` (sqrtPriceX96, tick, observationIndex), `liquidity()`, and fee.
- [x] `src/pools/quoting/univ3.rs` implements exact output quoting **within the current tick range**.
- [x] `GET /api/routes` can return multi-hop routes that mix UniV2 and UniV3 pools.
- [x] Routes through UniV3 are annotated `quote_confidence: exact`.
- [x] Live integration test verifies a UniV3 quote against a pinned block.
- [ ] Full multi-tick traversal for large V3 trades (follow-up).

## Dependencies
- Issue 0001 (tracer bullet UniV2 end-to-end).

## Notes / Risks
- Full V3 TVL calculation requires iterating ticks; for ranking we can use a liquidity-around-current-price approximation.
- Tick math must match on-chain behavior exactly; use pinned-block integration tests as the source of truth.
