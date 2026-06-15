//! DEX swap event topic0 registry and decoding tables.

use alloy::primitives::{b256, B256};

use super::types::DexInfo;

/// All recognised DEX swap event families in priority order (most volume first).
pub static DEX_REGISTRY: &[DexInfo] = &[
    // --- Type A: Pool == event.address ---

    // 1. Uniswap V2 & forks (SushiSwap, PancakeSwap V2, Aerodrome, Velodrome, etc.)
    DexInfo {
        topic0: b256!("d78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822"),
        family: super::types::DexFamily::UniswapV2,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "Swap(address,uint256,uint256,uint256,uint256,address)",
        indexed_count: 2,
    },
    // 2. Uniswap V3 & forks (PancakeSwap V3, KyberSwap Elastic)
    DexInfo {
        topic0: b256!("c42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67"),
        family: super::types::DexFamily::UniswapV3,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "Swap(address,address,int256,int256,uint160,uint128,int24)",
        indexed_count: 2,
    },
    // 3. Uniswap V4 (via PoolManager singleton)
    DexInfo {
        topic0: b256!("40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f"),
        family: super::types::DexFamily::UniswapV4,
        pool_source: PoolSource::IndexedParam0,
        sender_in_event: true,
        event_sig: "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)",
        indexed_count: 2,
    },
    // 4. Curve Vyper legacy (TokenExchange with coin indices)
    DexInfo {
        topic0: b256!("8b3e96f2b889fa771c53c981b40daf005f63f637f1869f707052d15a3dd97140"),
        family: super::types::DexFamily::CurveVyper,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "TokenExchange(address,int128,uint256,int128,uint256)",
        indexed_count: 1,
    },
    // 5. Curve Swap Router (TokenExchange with explicit token addresses)
    DexInfo {
        topic0: b256!("bd3eb7bcfdd1721a4eb4f00d0df3ed91bd6f17222f82b2d7bce519d8cab3fe46"),
        family: super::types::DexFamily::CurveRouter,
        pool_source: PoolSource::IndexedParam2,
        sender_in_event: true,
        event_sig: "TokenExchange(address,address,address,address,address,uint256,uint256)",
        indexed_count: 3,
    },
    // 6. Balancer V2 (Vault emits Swap for all pools)
    DexInfo {
        topic0: b256!("2170c741c41531aec20e7c107c24eecfdd15e69c9bb0a8dd37b1840b9e0b207b"),
        family: super::types::DexFamily::BalancerV2,
        pool_source: PoolSource::IndexedParam0,
        sender_in_event: false,
        event_sig: "Swap(bytes32,address,address,uint256,uint256)",
        indexed_count: 1,
    },
    // 7. Balancer V3 (Vault emits Swap for all pools)
    DexInfo {
        topic0: b256!("0874b2d545cb271cdbda4e093020c452328b24af12382ed62c4d00f5c26709db"),
        family: super::types::DexFamily::BalancerV3,
        pool_source: PoolSource::IndexedParam0,
        sender_in_event: false,
        event_sig: "Swap(address,address,address,uint256,uint256,uint256,uint256)",
        indexed_count: 3,
    },
    // 8. DODO (SellBaseToken)
    DexInfo {
        topic0: b256!("d8648b6ac54162763c86fd54bf2005af8ecd2f9cb273a5775921fd7f91e17b2d"),
        family: super::types::DexFamily::Dodo,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "SellBaseToken(address,uint256,uint256)",
        indexed_count: 1,
    },
    // 8b. DODO (BuyBaseToken) — same family, different topic0
    DexInfo {
        topic0: b256!("e93ad76094f247c0dafc1c61adc2187de1ac2738f7a3b49cb20b2263420251a3"),
        family: super::types::DexFamily::Dodo,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "BuyBaseToken(address,uint256,uint256)",
        indexed_count: 1,
    },
    // 9. Maverick V1
    DexInfo {
        topic0: b256!("3b841dc9ab51e3104bda4f61b41e4271192d22cd19da5ee6e292dc8e2744f713"),
        family: super::types::DexFamily::MaverickV1,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "Swap(address,address,bool,bool,uint256,uint256,int32)",
        indexed_count: 0,
    },
    // 10. Maverick V2
    DexInfo {
        topic0: b256!("103ed084e94a44c8f5f6ba8e3011507c41063177e29949083c439777d8d63f60"),
        family: super::types::DexFamily::MaverickV2,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "PoolSwap(address,address,(uint256,bool,bool,int32),uint256,uint256)",
        indexed_count: 0,
    },
    // 11. Solidly & forks (Velodrome, Aerodrome, Ramses, etc.)
    DexInfo {
        topic0: b256!("b3e2773606abfd36b5bd91394b3a54d1398336c50b05baf7bf7a05efeffaf75b"),
        family: super::types::DexFamily::Solidly,
        pool_source: PoolSource::EventAddress,
        sender_in_event: true,
        event_sig: "Swap(address,address,uint256,uint256,uint256,uint256)",
        indexed_count: 2,
    },
    // 11. Ekubo Core (swap)
    DexInfo {
        topic0: b256!("d76ec32fbc3f07c70828b4f94343ee73279d0e8d4d2f28b018a4e67f37497753"),
        family: super::types::DexFamily::Ekubo,
        pool_source: PoolSource::EventAddress,
        sender_in_event: false,
        event_sig: "Swapped(address,int256,int256,uint160,uint24,int24)",
        indexed_count: 1,
    },
    // 11b. Ekubo Core pool event (liquidity/swap updates)
    DexInfo {
        topic0: b256!("704b3ab4a76158ad4d66625a2a43be81edbffd24630e8fde5174e97035370a07"),
        family: super::types::DexFamily::Ekubo,
        pool_source: PoolSource::EventAddress,
        sender_in_event: false,
        event_sig: "CoreEvent(?)",
        indexed_count: 0,
    },
    // 12. LiquidityBook (Joe V2 / TraderJoe LB)
    DexInfo {
        topic0: b256!("458f5fa412d0f69b08dd84872b0215675cc67bc1d5b6fd93300a1c3878b86196"),
        family: super::types::DexFamily::LiquidityBook,
        pool_source: PoolSource::EventAddress,
        sender_in_event: false,
        event_sig: "Swap(address,address,uint256,uint256)",
        indexed_count: 2,
    },
];

/// How to extract the pool identity from a swap log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolSource {
    /// Pool is the log's emitting contract address.
    EventAddress,
    /// Pool is the first indexed topic.
    IndexedParam0,
    /// Pool is the third indexed topic.
    IndexedParam2,
}

/// Look up a `DexInfo` by topic0 (event signature hash).
pub fn lookup_topic0(topic0: B256) -> Option<&'static DexInfo> {
    DEX_REGISTRY.iter().find(|d| d.topic0 == topic0)
}

// ---------------------------------------------------------------------------
// Known pool contract addresses (for Type B — vault/manager contracts)
// ---------------------------------------------------------------------------

/// Balancer V2 Vault address on Ethereum mainnet.
pub const BALANCER_V2_VAULT: alloy::primitives::Address =
    alloy::primitives::address!("BA12222222228d8Ba445958a75a0704d566BF2C8");

/// Uniswap V4 PoolManager address on Ethereum mainnet.
#[allow(dead_code)]
pub const UNISWAP_V4_POOLMANAGER: alloy::primitives::Address =
    alloy::primitives::address!("000000000004444c5dc75Cb358380D2e08dE62B0");

/// All known vault/manager contracts (Type B emitters).
#[allow(dead_code)]
pub const TYPE_B_EMITTERS: &[alloy::primitives::Address] =
    &[BALANCER_V2_VAULT, UNISWAP_V4_POOLMANAGER];
