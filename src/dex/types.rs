//! DEX type definitions shared across the swap decoder.

use alloy::primitives::B256;
use serde::{Deserialize, Serialize};

/// Metadata for a single DEX swap event family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexInfo {
    /// Keccak-256 hash of the event signature (topic0).
    pub topic0: B256,
    /// DEX family identifier.
    pub family: DexFamily,
    /// How to extract the pool identity from the log.
    pub pool_source: super::registry::PoolSource,
    /// Whether the swap initiator address is included in the event params.
    pub sender_in_event: bool,
    /// Human-readable event signature for debugging.
    pub event_sig: &'static str,
    /// Number of indexed parameters (topics beyond topic0).
    pub indexed_count: usize,
}

/// DEX family identifier for decoding strategy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DexFamily {
    UniswapV2,
    UniswapV3,
    UniswapV4,
    CurveVyper,
    CurveRouter,
    BalancerV2,
    BalancerV3,
    Dodo,
    MaverickV1,
    MaverickV2,
    Ekubo,
    LiquidityBook,
    Solidly,
}
