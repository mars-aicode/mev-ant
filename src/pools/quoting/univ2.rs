//! Exact output quoting for Uniswap V2 constant-product pools.

use alloy::primitives::{Address, U256};

use crate::pools::types::{Pool, PoolSnapshot};

/// Constant-product fee numerator. UniV2 takes 0.3% = 30 bps.
const FEE_NUMERATOR: u128 = 997;
const FEE_DENOMINATOR: u128 = 1000;

/// Quote output for a given input through a UniV2 pool.
///
/// `token_in` must be either `pool.token0` or `pool.token1`.
pub fn quote(
    pool: &Pool,
    state: &PoolSnapshot,
    token_in: Address,
    amount_in: U256,
) -> Option<U256> {
    let (reserve_in, reserve_out) = if token_in == pool.token0 {
        (state.reserve0?, state.reserve1?)
    } else if token_in == pool.token1 {
        (state.reserve1?, state.reserve0?)
    } else {
        return None;
    };

    Some(get_amount_out(amount_in, reserve_in, reserve_out))
}

/// `amountOut = (amountIn * 997 * reserveOut) / (reserveIn * 1000 + amountIn * 997)`
fn get_amount_out(amount_in: U256, reserve_in: U256, reserve_out: U256) -> U256 {
    let amount_in_with_fee = amount_in * U256::from(FEE_NUMERATOR);
    let numerator = amount_in_with_fee * reserve_out;
    let denominator = reserve_in * U256::from(FEE_DENOMINATOR) + amount_in_with_fee;
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn pool() -> Pool {
        Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: alloy::primitives::B256::ZERO,
            kind: crate::pools::types::PoolKind::UniswapV2,
            factory: None,
            token0: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            token0_decimals: 6,
            token1: address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            token1_decimals: 18,
            fee: Some(30),
            block_created: None,
        }
    }

    fn state(reserve0: u128, reserve1: u128) -> PoolSnapshot {
        PoolSnapshot {
            address: pool().address,
            pool_id: alloy::primitives::B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(reserve0)),
            reserve1: Some(U256::from(reserve1)),
            tvl_usd: None,
            state: serde_json::json!({}),
        }
    }

    #[test]
    fn basic_swap() {
        let p = pool();
        let s = state(1_000_000_000_000, 500_000_000_000_000_000_000_000);
        let out = quote(&p, &s, p.token0, U256::from(1_000_000)).unwrap();
        // 1 USDC in, expect ~0.0004985 WETH out
        assert!(out > U256::from(498_000_000_000_000_000u64));
        assert!(out < U256::from(499_000_000_000_000_000u64));
    }
}
