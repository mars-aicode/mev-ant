# Issue 0003 — Add Curve Exact Quoting

## Goal
Extend the liquidity pipeline to support Curve stable/cryptoswap pools with exact output quoting.

## Acceptance Criteria
- [x] `src/pools/registry.rs` seeds Curve pools from TheGraph, with on-chain registry fallback, and a hardcoded fallback of top 2-coin pools.
- [x] `src/pools/liquidity.rs` fetches Curve stableswap state: `A`, `fee`, `balances(i)`, and coin addresses.
- [ ] `src/pools/liquidity.rs` fetches Curve cryptoswap state (`gamma`, `D`) — deferred.
- [x] `src/pools/quoting/curve.rs` implements stableswap invariant output calculation.
- [x] `GET /api/routes` can return multi-hop routes that include Curve pools.
- [x] Routes through Curve are annotated `quote_confidence: exact`.
- [x] Live integration test verifies a Curve quote against on-chain `get_dy` at a pinned block.
- [ ] Expand beyond 2-coin pools once the routing graph supports n-coin pools.

## Dependencies
- Issue 0001.

## Notes / Risks
- Curve has multiple pool types (stableswap, crypto, stableswap-ng, tricrypto). Start with the most common stableswap variant.
- Newton-Raphson solver must converge robustly; test edge cases (very small/large trades).
