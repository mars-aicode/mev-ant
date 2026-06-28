//! Domain types for the liquidity registry and routing feature.

use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

/// A discovered DEX pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    /// Contract used to interact with the pool.
    /// For UniV2/V3 this is the pool itself; for Balancer/UniV4 this is the Vault/PoolManager.
    pub address: Address,
    /// Bytes32 pool ID for vault-style protocols; empty for UniV2/V3.
    pub pool_id: B256,
    /// DEX family.
    pub kind: PoolKind,
    /// Factory that created the pool (if applicable).
    pub factory: Option<Address>,
    pub token0: Address,
    pub token0_decimals: u8,
    pub token1: Address,
    pub token1_decimals: u8,
    /// Fee in basis points or protocol-specific units.
    pub fee: Option<u32>,
    /// Block at which the pool was created.
    pub block_created: Option<u64>,
}

impl Pool {
    /// Canonical string key for this pool.
    pub fn key(&self) -> String {
        if self.pool_id.is_zero() {
            format!("{:?}", self.address)
        } else {
            format!("{:?}:{:?}", self.address, self.pool_id)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolKind {
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
    Solidly,
    Ekubo,
    LiquidityBook,
    Fluid,
    FraxSwap,
    PancakeV3,
    Bancor,
    Unknown,
}

impl std::fmt::Display for PoolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PoolKind::UniswapV2 => "uniswap_v2",
            PoolKind::UniswapV3 => "uniswap_v3",
            PoolKind::UniswapV4 => "uniswap_v4",
            PoolKind::CurveVyper => "curve_vyper",
            PoolKind::CurveRouter => "curve_router",
            PoolKind::BalancerV2 => "balancer_v2",
            PoolKind::BalancerV3 => "balancer_v3",
            PoolKind::Dodo => "dodo",
            PoolKind::MaverickV1 => "maverick_v1",
            PoolKind::MaverickV2 => "maverick_v2",
            PoolKind::Solidly => "solidly",
            PoolKind::Ekubo => "ekubo",
            PoolKind::LiquidityBook => "liquidity_book",
            PoolKind::Fluid => "fluid",
            PoolKind::FraxSwap => "frax_swap",
            PoolKind::PancakeV3 => "pancake_v3",
            PoolKind::Bancor => "bancor",
            PoolKind::Unknown => "unknown",
        };
        write!(f, "{}", s)
    }
}

/// Latest pool snapshot. A point-in-time record of a pool's reserves, prices,
/// and derived TVL; only the latest snapshot per pool is retained for routing.
/// Snapshots are produced per block for pools touched by state-changing events
/// (Swap, Mint, Burn, Sync), with a daily full refresh of all Liquid Pools
/// synchronized with the TheGraph re-seed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolSnapshot {
    pub address: Address,
    pub pool_id: B256,
    pub observed_at_block: u64,
    pub reserve0: Option<U256>,
    pub reserve1: Option<U256>,
    pub tvl_usd: Option<f64>,
    /// Protocol-specific state used for exact quoting.
    pub state: serde_json::Value,
}

/// A single Uniswap V3 tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V3Tick {
    pub idx: i32,
    pub liquidity_net: i128,
}

/// Uniswap V3 state stored in `PoolSnapshot.state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V3State {
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub liquidity: U256,
    pub tick_spacing: i16,
    pub ticks: Vec<V3Tick>,
}

/// Curve stableswap pool state stored in `PoolSnapshot.state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurveState {
    pub n_coins: u8,
    pub a: U256,
    pub fee: U256,
    pub balances: Vec<U256>,
    pub coins: Vec<Address>,
    pub decimals: Vec<u8>,
}

/// A single hop in a route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    pub pool_address: Address,
    pub pool_id: B256,
    pub kind: PoolKind,
    pub token_in: Address,
    pub token_out: Address,
    pub fee: u32,
}

/// A candidate route from token A to token B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub hops: Vec<Hop>,
    pub hop_count: usize,
    pub total_fee_bps: u64,
    pub total_output: Option<U256>,
    pub min_pool_tvl_usd: f64,
    pub quote_confidence: QuoteConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteConfidence {
    Exact,
    Estimated,
}

/// Sort key for `GET /api/routes?sort=<mode>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSortMode {
    Output,
    Fee,
    Tvl,
    Confidence,
    Hops,
}

impl Default for RouteSortMode {
    fn default() -> Self {
        Self::Output
    }
}
