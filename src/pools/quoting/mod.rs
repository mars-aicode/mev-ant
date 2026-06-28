//! Exact output quoting for DEX pools.

use alloy::primitives::{Address, U256};

use crate::pools::types::{Pool, PoolKind, PoolSnapshot};

pub mod curve;
pub mod univ2;
pub mod univ3;

/// Quote the output of swapping `amount_in` of `token_in` through `pool`.
/// Returns None if the pool kind is not supported for exact quoting.
pub fn quote_exact_output(
    pool: &Pool,
    state: &PoolSnapshot,
    token_in: Address,
    amount_in: U256,
) -> Option<U256> {
    match pool.kind {
        // UniV2 forks (SushiSwap V2, FraxSwap) share the same k=x*y math.
        PoolKind::UniswapV2 | PoolKind::FraxSwap => {
            univ2::quote(pool, state, token_in, amount_in)
        }
        // UniV3 forks (PancakeSwap V3) share the same concentrated-liquidity math.
        PoolKind::UniswapV3 | PoolKind::PancakeV3 => {
            univ3::quote(pool, state, token_in, amount_in)
        }
        PoolKind::CurveVyper | PoolKind::CurveRouter => {
            let token_out = if token_in == pool.token0 {
                pool.token1
            } else if token_in == pool.token1 {
                pool.token0
            } else {
                return None;
            };
            curve::quote(pool, state, token_in, token_out, amount_in)
        }
        // Balancer, Fluid, and other exotic pool kinds need custom quoters.
        _ => None,
    }
}
