# DEX TVL Ranking & Pool-ID Support Gap

Date: 2026-06-17
Data source: DefiLlama `/protocols` API (`https://api.llama.fi/protocols`)
Scope: Ethereum mainnet only (`chainTvls.Ethereum`)

## Summary

- The top 4 DEXes on Ethereum mainnet by TVL are already covered by the existing classifier: **Curve DEX**, **Uniswap V3**, **Uniswap V4**, **Uniswap V2**.
- The largest genuinely unsupported protocol is **Fluid DEX** (~$129M ETH TVL).
- **Uniswap V4 is NOT actually supported end-to-end** despite being listed in `src/dex/registry.rs` and documented in `docs/protocols/protocol-uniswap-v4.md`.
- The same architectural gap affects **Balancer V2/V3**: the classifier recognizes the Vault event but never extracts the `bytes32` pool ID.

## Ethereum-mainnet DEX ranking by TVL

| Rank | Protocol | ETH TVL | Total TVL | Status |
|---:|---|---:|---:|---|
| 1 | Curve DEX | $1,374,781,914 | $1,455,238,843 | ✅ supported |
| 2 | Uniswap V3 | $851,187,941 | $1,493,612,218 | ✅ supported |
| 3 | Uniswap V4 | $730,539,547 | $899,653,533 | ⚠️ listed, not fully implemented |
| 4 | Uniswap V2 | $611,418,627 | $735,349,850 | ✅ supported |
| 5 | Fluid DEX | $128,531,389 | $249,000,593 | ❌ candidate |
| 6 | Balancer V3 | $75,478,377 | $99,405,298 | ⚠️ listed, pool-ID extraction missing |
| 7 | IDEX V1 | $28,701,682 | $28,701,682 | ❌ candidate (orderbook) |
| 8 | PancakeSwap AMM V3 | $27,403,573 | $291,392,035 | ⚠️ likely covered by UniV3 event sig |
| 9 | SushiSwap V3 | $23,610,796 | $40,639,011 | ⚠️ likely covered by UniV3 event sig |
| 10 | Balancer V2 | $21,132,671 | $27,372,386 | ⚠️ listed, pool-ID extraction missing |
| 11 | SushiSwap | $20,569,224 | $33,971,827 | ⚠️ likely covered by UniV2 event sig |
| 12 | Bancor V3 | $17,324,684 | $17,324,684 | ❌ candidate |
| 13 | Thorchain DEX | $11,457,713 | $53,973,976 | ❌ candidate (cross-chain) |
| 14 | Loopring | $9,073,008 | $9,073,122 | ❌ candidate (rollup/orderbook) |
| 15 | Bancor V2.1 | $8,479,017 | $8,479,017 | ❌ candidate |
| 16 | Balancer V1 | $6,192,624 | $6,192,624 | ❌ candidate |
| 17 | Frax Swap | $5,671,440 | $10,913,909 | ❌ candidate |
| 18 | ShibaSwap V1 | $4,811,111 | $4,816,572 | ⚠️ likely covered by UniV2 event sig |
| 19 | Ekubo | $4,347,912 | $23,481,280 | ✅ supported |
| 20 | Uniswap V1 | $2,950,529 | $2,950,529 | ❌ candidate |
| 21 | DODO AMM | $2,553,564 | $11,299,749 | ✅ supported |
| 22 | Clipper | $459,606 | $717,842 | ❌ candidate |
| 23 | Saddle Finance | $795k | $812k | ❌ candidate |
| 24 | KyberSwap Classic | $598,171 | $1,002,187 | ❌ candidate |

Notes:
- "likely covered by UniV2/UniV3 event sig" means the protocol forks Uniswap and emits the same `Swap` topic0, so the current classifier should already mark its pools correctly.
- The list is truncated at ~$500k ETH TVL. Lower-value protocols exist but were omitted.

## Why Uniswap V4 is not actually supported

`src/dex/registry.rs` registers the Uniswap V4 `Swap` topic0 and sets `pool_source: PoolSource::IndexedParam0`:

```rust
// Uniswap V4 (via PoolManager singleton)
DexInfo {
    topic0: b256!("40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f"),
    family: DexFamily::UniswapV4,
    pool_source: PoolSource::IndexedParam0,
    event_sig: "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)",
    ...
}
```

However:

1. `src/classifier.rs` only checks `lookup_topic0(t0).is_some()` and inserts `kinds.insert(addr, AddressKind::Pool)` where `addr = log.address`. For Uniswap V4, `log.address` is the **PoolManager singleton**, not the actual pool. All V4 swaps would appear to touch the same address.
2. `src/detector/engine.rs` builds `PoolId::Contract(...)` from transfer counter-parties and checks membership in `ctx.pool_set`, which is address-based. It never decodes the `bytes32` pool ID from the event or constructs `PoolId::Param(...)`.
3. The `UNISWAP_V4_POOLMANAGER` constant in `src/dex/registry.rs` (`0x000000000004444c5dc75Cb358380D2e08dE62B0`) **does not match** the documented address in `docs/protocols/protocol-uniswap-v4.md` (`0x000000000004444c5dc75cB358380D2e3dE08A90`). The documented address appears to be the canonical mainnet PoolManager.

The same problem exists for **Balancer V2/V3**:
- The classifier marks the Balancer Vault as the pool.
- The real pool identity is the `bytes32 poolId` emitted in the `Swap` event.
- The detector never extracts or uses `PoolId::Param(poolId)`.

`src/models.rs` already defines `PoolId::Param(B256)` to represent such pools, but no code path currently produces it.

## Recommended actions

### Short term
1. **Fix the Uniswap V4 PoolManager address** in `src/dex/registry.rs`.
2. **Clarify support status** in `docs/protocols/protocol-uniswap-v4.md`: currently the classifier can recognize the event but cannot attribute swaps to individual V4 pools.

### Medium term
3. **Implement vault-style pool-ID extraction**:
   - Decode `IndexedParam0` from `Swap` events for Uniswap V4 and Balancer V2/V3.
   - Map the extracted `bytes32` ID to the pool's token pair (V4: read `PoolKey` from PoolManager state or cache; Balancer: call `Vault.getPoolTokens(poolId)`).
   - Pass `PoolId::Param(id)` through the detector so `attacked_pool` and `auxiliary_pools` are accurate.
4. **Add Fluid DEX** — it is the largest unsupported AMM by ETH TVL and has a distinct event signature.
5. **Add Bancor V3** and **Frax Swap** — next-largest distinct AMMs.

### Lower priority
6. **Uniswap V1**, **Balancer V1**, **Clipper**, **Saddle**, **KyberSwap Classic** — add for completeness, but TVL is relatively small.
7. **Skip orderbook/aggregator/cross-chain protocols** (IDEX, Loopring, Thorchain, 1inch, CoW, etc.) for sandwich detection; they do not provide the same pool-manipulation surface.
