# Issue 0007 — Lending Market Liquidity Tracking

## Goal
Track lending markets (Aave, Compound, Morpho Blue) as a separate liquidity type for liquidation and collateral-swap MEV strategies.

## Acceptance Criteria
- [x] New table `lending_markets` stores market address, protocol, underlying asset, available liquidity (nullable), supply rate, variable borrow rate, stable borrow rate.
- [x] Module `src/pools/lending.rs` fetches market state per block via `getReserveData` decoding (V1 supports Aave V3 only; portable across v3.x because the struct is version-agnostic enough for the rate fields we read).
- [x] Touched-market detection: `update_touched_aave_v3` filters `ReserveDataUpdated` logs from the Pool and re-fetches state for touched reserves.
- [x] Admin endpoint `GET /api/lending-markets` lists tracked markets (`LendingService` + axum route).
- [x] Lending markets are **not** included in `/api/routes` — V1 has no path that pulls lending entries into the routing graph.
- [x] Live integration test `aave_v3_reserves_and_rates_at_25_300_000` calls `getReservesList` + `getReserveData` for USDC at block 25,300,000 and decodes rates successfully.
- [x] Per-block lending update is integrated into the `LiquidityJob` background loop, gated by `lending_enabled` (default true).

## Dependencies
- Issue 0006 (background job pattern).

## Notes / Risks
- This is intentionally separate from DEX routing. Mixing loans and swaps in one route model is V2 scope.
