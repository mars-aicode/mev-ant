//! Shared pool-address / pool-ID resolution from swap events.
//!
//! This module is the single source of truth for "what is the pool in this log?"
//! It is used by both the sandwich classifier and the liquidity registry.

#![allow(dead_code)]

use alloy::primitives::{Address, B256};

use crate::dex::registry::{lookup_topic0, PoolSource};
use crate::pools::types::PoolKind;
use crate::rpc::DxgLog;

/// Resolved pool identity from a swap log.
#[derive(Debug, Clone)]
pub struct ResolvedPool {
    /// Contract used to interact with the pool (pool itself for UniV2/V3,
    /// Vault/PoolManager for Balancer/UniV4).
    pub address: Address,
    /// Bytes32 pool ID for vault-style protocols.
    pub pool_id: Option<B256>,
    pub kind: PoolKind,
}

/// Resolve the pool identity for a swap log.
///
/// For UniV2/V3-style pools, the pool is the log emitter.
/// For Balancer/UniV4 vault-style pools, the pool ID is the first indexed topic
/// and the interaction address is the Vault/PoolManager singleton.
pub fn resolve_swap_log(log: &DxgLog) -> Option<ResolvedPool> {
    let info = lookup_topic0(log.topics.first().copied()?)?;
    let kind = dex_family_to_pool_kind(info.family);

    let (address, pool_id) = match info.pool_source {
        PoolSource::EventAddress => (log.address, None),
        PoolSource::IndexedParam0 => {
            let id = log.topics.get(1).copied()?;
            (log.address, Some(id))
        }
        PoolSource::IndexedParam2 => {
            let id = log.topics.get(3).copied()?;
            (log.address, Some(id))
        }
    };

    Some(ResolvedPool {
        address,
        pool_id,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn log(address: Address, topics: Vec<B256>) -> DxgLog {
        DxgLog { address, topics, data: "0x".to_string() }
    }

    #[test]
    fn resolves_uniswap_v2_by_event_address() {
        let swap_topic = B256::new(hex_literal::hex!(
            "d78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822"
        ));
        let pool = address!("0000000000000000000000000000000000000001");
        let resolved = resolve_swap_log(&log(pool, vec![swap_topic])).unwrap();
        assert_eq!(resolved.address, pool);
        assert!(resolved.pool_id.is_none());
        assert_eq!(resolved.kind, PoolKind::UniswapV2);
    }

    #[test]
    fn resolves_balancer_v2_pool_id() {
        let swap_topic = B256::new(hex_literal::hex!(
            "2170c741c41531aec20e7c107c24eecfdd15e69c9bb0a8dd37b1840b9e0b207b"
        ));
        let vault = crate::dex::registry::BALANCER_V2_VAULT;
        let pool_id = B256::new(hex_literal::hex!(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));
        let resolved = resolve_swap_log(&log(vault, vec![swap_topic, pool_id])).unwrap();
        assert_eq!(resolved.address, vault);
        assert_eq!(resolved.pool_id, Some(pool_id));
        assert_eq!(resolved.kind, PoolKind::BalancerV2);
    }

    #[test]
    fn resolves_uniswap_v4_pool_id() {
        let swap_topic = B256::new(hex_literal::hex!(
            "40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f"
        ));
        let manager = crate::dex::registry::UNISWAP_V4_POOLMANAGER;
        let pool_id = B256::new(hex_literal::hex!(
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        ));
        let resolved = resolve_swap_log(&log(manager, vec![swap_topic, pool_id, B256::ZERO])).unwrap();
        assert_eq!(resolved.address, manager);
        assert_eq!(resolved.pool_id, Some(pool_id));
        assert_eq!(resolved.kind, PoolKind::UniswapV4);
    }
}

fn dex_family_to_pool_kind(family: crate::dex::types::DexFamily) -> PoolKind {
    use crate::dex::types::DexFamily;
    match family {
        DexFamily::UniswapV2 => PoolKind::UniswapV2,
        DexFamily::UniswapV3 => PoolKind::UniswapV3,
        DexFamily::UniswapV4 => PoolKind::UniswapV4,
        DexFamily::CurveVyper => PoolKind::CurveVyper,
        DexFamily::CurveRouter => PoolKind::CurveRouter,
        DexFamily::BalancerV2 => PoolKind::BalancerV2,
        DexFamily::BalancerV3 => PoolKind::BalancerV3,
        DexFamily::Dodo => PoolKind::Dodo,
        DexFamily::MaverickV1 => PoolKind::MaverickV1,
        DexFamily::MaverickV2 => PoolKind::MaverickV2,
        DexFamily::Solidly => PoolKind::Solidly,
        DexFamily::Ekubo => PoolKind::Ekubo,
        DexFamily::LiquidityBook => PoolKind::LiquidityBook,
        DexFamily::Fluid => PoolKind::Fluid,
    }
}
