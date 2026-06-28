//! Exact output quoting for Uniswap V3 concentrated-liquidity pools.
//!
//! V1 implements single-tick quoting: it assumes the trade stays within the
//! current tick range. This is exact for small trades and a close approximation
//! for most routing decisions. Multi-tick traversal is left as a follow-up.

use alloy::primitives::{Address, U256};

use crate::pools::types::{Pool, PoolSnapshot, V3State};

const Q96: u128 = 1 << 96;

/// Quote output for a given input through a UniV3 pool.
/// `token_in` must be either `pool.token0` or `pool.token1`.
pub fn quote(
    pool: &Pool,
    state: &PoolSnapshot,
    token_in: Address,
    amount_in: U256,
) -> Option<U256> {
    let v3: V3State = serde_json::from_value(state.state.clone()).ok()?;
    let zero_for_one = token_in == pool.token0;
    let fee_pips = pool.fee? as u128;
    let liquidity: u128 = v3.liquidity.try_into().ok()?;
    if liquidity == 0 || v3.sqrt_price_x96.is_zero() {
        return None;
    }

    // Apply fee: amount_in * (1_000_000 - fee_pips) / 1_000_000
    let amount_in_after_fee = amount_in * U256::from(1_000_000 - fee_pips) / U256::from(1_000_000);

    Some(if zero_for_one {
        // token0 -> token1, price decreases.
        // Δx = amount_in_after_fee
        // 1/√P_next = 1/√P_current + Δx / (L * Q96)
        let sqrt_p_current = v3.sqrt_price_x96;
        // 1 / sqrt_p_next = (sqrt_p_current + term) / (sqrt_p_current * term) ... no.
        // 1/√P_next = 1/√P_current + Δx/(L*Q96)
        // √P_next = 1 / (1/√P_current + Δx/(L*Q96))
        //        = L*Q96 / (L*Q96/√P_current + Δx)
        let sqrt_p_next_num = U256::from(liquidity) * U256::from(Q96);
        let sqrt_p_next_den = sqrt_p_next_num / sqrt_p_current + amount_in_after_fee;
        let sqrt_p_next = sqrt_p_next_num / sqrt_p_next_den;
        // Δy = L * (√P_current - √P_next) / Q96
        U256::from(liquidity) * (sqrt_p_current - sqrt_p_next) / U256::from(Q96)
    } else {
        // token1 -> token0, price increases.
        // Δy = amount_in_after_fee
        // √P_next = √P_current + Δy * Q96 / L
        let sqrt_p_current = v3.sqrt_price_x96;
        let sqrt_p_next = sqrt_p_current + amount_in_after_fee * U256::from(Q96) / U256::from(liquidity);
        // Δx = L * (1/√P_next - 1/√P_current) * Q96
        //    = L * Q96 * (√P_current - √P_next) / (√P_current * √P_next)
        // Since √P_next > √P_current, this is negative. We want absolute value.
        let delta = U256::from(liquidity) * U256::from(Q96) * (sqrt_p_next - sqrt_p_current);
        delta / sqrt_p_current / sqrt_p_next
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, B256, U256};

    use crate::pools::types::{Pool, PoolKind, PoolSnapshot};

    fn pool() -> Pool {
        Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV3,
            factory: None,
            token0: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            token0_decimals: 6,
            token1: address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            token1_decimals: 18,
            fee: Some(500),
            block_created: None,
        }
    }

    fn state() -> PoolSnapshot {
        PoolSnapshot {
            address: Address::ZERO,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: None,
            reserve1: None,
            tvl_usd: None,
            state: serde_json::to_value(&V3State {
                // sqrt(2000) * 2^96  ~  WETH/USDC where WETH ~= $2000
                sqrt_price_x96: U256::from(3_543_000_000_000_000_000_000_000_000_000u128),
                tick: 0,
                liquidity: U256::from(1_000_000_000_000_000_000u128),
                tick_spacing: 10,
                ticks: vec![],
            })
            .unwrap(),
        }
    }

    #[test]
    fn basic_v3_quote() {
        let p = pool();
        let s = state();
        let out = quote(&p, &s, p.token0, U256::from(1_000_000)).unwrap();
        assert!(out > U256::ZERO);
    }

    #[test]
    fn reverse_v3_quote() {
        let p = pool();
        let s = state();
        let out = quote(&p, &s, p.token1, U256::from(1_000_000_000_000_000_000u128)).unwrap();
        assert!(out > U256::ZERO);
    }
}
