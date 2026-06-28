# Issue 0004 — Shared Pool Identity for Vault-Style Protocols

## Goal
Create a shared pool-address/ID resolution module and fix classifier/detector support for Balancer V2/V3 and Uniswap V4.

## Acceptance Criteria
- [x] `src/pools/identity.rs` exposes `resolve_swap_log(log) -> ResolvedPool { address, pool_id, kind }`.
- [x] Module decodes `IndexedParam0` for Balancer V2/V3 and Uniswap V4 `Swap` events (and `IndexedParam2` for Curve Router).
- [x] Fix `UNISWAP_V4_POOLMANAGER` constant in `src/dex/registry.rs` to match canonical mainnet address.
- [x] `src/classifier.rs` uses `pools::identity` and emits `PoolId::Param` for vault-style pools.
- [x] `src/detector/engine.rs` handles `PoolId::Param` when matching attacked pools and collecting touched pools.
- [x] Existing sandwich integration tests still pass.
- [x] Unit tests cover V2, Balancer V2, and UniV4 identity resolution.

## Dependencies
- Issue 0001 (establishes `src/pools/` structure).

## Notes / Risks
- This touches core sandwich-detection logic; regression tests must pass before merge.
- The classifier change affects how Balancer/UniV4 swaps are represented in existing bundles.
