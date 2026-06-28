# Issue 0005 — Expand Registry to Remaining Top Protocols

## Goal
Add the remaining high-TVL protocols to the pool registry and routing graph.

## Acceptance Criteria
- [x] Seed Balancer V2/V3 pools (hardcoded top pools; TheGraph hosted service is retired and the Balancer API requires a key — left as follow-up).
- [x] Seed Fluid DEX pools from hardcoded top addresses (vault-shaped AMM; custom quoter is follow-up).
- [x] Seed FraxSwap V2 pools via `PairCreated` factory indexing.
- [x] Seed SushiSwap V2 pools via `PairCreated` factory indexing (UniV2 fork; quoter reuses the UniV2 formula).
- [x] Seed PancakeSwap V3 pools via `PoolCreated` factory indexing (UniV3 fork; quoter reuses the UniV3 formula).
- [x] `GET /api/liquid-pools` covers the global top 1,000 across all seeded protocols.
- [x] Routes through Balancer/Fluid are included in pathfinding and annotated `quote_confidence: estimated` until custom quoters are added.
- [x] Daily TheGraph re-seed + re-rank works end-to-end (driven by `LiquidityJob`).
- [x] New unit tests cover fork-kind routing and Balancer/Fluid hardcoded seeds.

## Dependencies
- Issue 0001 (registry pipeline).
- Issue 0004 (Balancer/UniV4 identity).

## Notes / Risks
- Some protocols may lack public subgraphs; RPC fallback must be implemented per protocol.
- "Estimated" routes should still be ranked by liquidity and hop count.
