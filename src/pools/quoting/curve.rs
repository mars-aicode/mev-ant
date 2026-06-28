//! Exact output quoting for Curve stableswap pools.
//!
//! V1 targets 2-coin stableswap pools. The solver works in normalised (human)
//! units using `f64`, which is accurate enough for routing and avoids the
//! overflow complications of a full integer Newton-Raphson implementation.

use alloy::primitives::{Address, U256};

use crate::pools::types::{CurveState, Pool, PoolSnapshot};

/// Max iterations for the Newton-Raphson solvers.
const MAX_ITER: usize = 64;
/// Convergence tolerance (normalised units).
const TOL: f64 = 1e-9;

/// Quote output for a given input through a Curve stableswap pool.
///
/// `token_in` and `token_out` must both be coins of the pool. V1 supports only
/// 2-coin pools (`n_coins == 2`).
pub fn quote(
    _pool: &Pool,
    state: &PoolSnapshot,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
) -> Option<U256> {
    let curve: CurveState = serde_json::from_value(state.state.clone()).ok()?;
    if curve.n_coins != 2 || curve.coins.len() != 2 {
        return None;
    }

    let i = curve.coins.iter().position(|t| *t == token_in)?;
    let j = curve.coins.iter().position(|t| *t == token_out)?;
    if i == j {
        return None;
    }

    let a = u256_to_f64(curve.a);
    let fee = u256_to_f64(curve.fee) / 1e10;

    let decimals_in = curve.decimals.get(i).copied().unwrap_or(18) as i32;
    let decimals_out = curve.decimals.get(j).copied().unwrap_or(18) as i32;

    let x: Vec<f64> = curve
        .balances
        .iter()
        .enumerate()
        .map(|(idx, b)| to_f64(*b, curve.decimals.get(idx).copied().unwrap_or(18)))
        .collect();

    let dx = to_f64(amount_in, decimals_in as u8);
    if dx <= 0.0 || x.iter().any(|v| *v <= 0.0) {
        return None;
    }

    let d = compute_d(a, &x)?;
    let mut x_after = x.clone();
    x_after[i] += dx;

    let y = compute_y(a, &x_after, j, d)?;
    if y >= x[j] {
        return None;
    }

    let dy = x[j] - y;
    let dy_after_fee = dy * (1.0 - fee);
    if dy_after_fee <= 0.0 {
        return None;
    }

    Some(from_f64(dy_after_fee, decimals_out as u8))
}

/// Compute the stableswap invariant `D` for balances `x`.
///
/// Solves `A * n^n * sum(x_i) + D = A * n^n * D + D^(n+1) / (n^n * prod(x_i))`.
fn compute_d(a: f64, x: &[f64]) -> Option<f64> {
    let n = x.len() as f64;
    let ann = a * n.powi(x.len() as i32);
    let sum: f64 = x.iter().sum();
    let prod: f64 = x.iter().product();

    if sum == 0.0 {
        return Some(0.0);
    }

    let mut d = sum;
    for _ in 0..MAX_ITER {
        let d_prev = d;
        // d_p = D^(n+1) / (n^n * prod(x_i))
        let d_p = d.powi(x.len() as i32 + 1) / (n.powi(x.len() as i32) * prod);
        let num = ann * sum + d_p * n;
        let den = ann - 1.0 + d_p * (n + 1.0) / d;
        d = num / den;
        if (d - d_prev).abs() < TOL {
            return Some(d);
        }
    }
    None
}

/// Compute the new balance of coin `j` after adding input to coin `i` while
/// keeping the invariant `D` constant.
fn compute_y(a: f64, x: &[f64], j: usize, d: f64) -> Option<f64> {
    let n = x.len() as f64;
    let ann = a * n.powi(x.len() as i32);
    let sum_others: f64 = x.iter().enumerate().filter(|(idx, _)| *idx != j).map(|(_, v)| *v).sum();
    let prod_others: f64 = x.iter().enumerate().filter(|(idx, _)| *idx != j).map(|(_, v)| *v).product();

    // Initial guess: remove the average share from coin j.
    let mut y = (d - sum_others) / (n - 1.0);
    if y <= 0.0 {
        y = d / n;
    }

    for _ in 0..MAX_ITER {
        let y_prev = y;
        // f(y) = ann*(sum_others + y) + d - ann*d - d^(n+1)/(n^n * prod_others * y)
        let k = ann * (sum_others + y) + d - ann * d;
        let rhs = d.powi(x.len() as i32 + 1) / (n.powi(x.len() as i32) * prod_others * y);
        let f = k - rhs;
        // f'(y) = ann + d^(n+1)/(n^n * prod_others * y^2)
        let df = ann + d.powi(x.len() as i32 + 1) / (n.powi(x.len() as i32) * prod_others * y * y);
        if df == 0.0 {
            return None;
        }
        y = y - f / df;
        if (y - y_prev).abs() < TOL {
            return Some(y);
        }
    }
    None
}

fn u256_to_f64(amount: U256) -> f64 {
    amount.to_string().parse().unwrap_or(0.0)
}

fn to_f64(amount: U256, decimals: u8) -> f64 {
    let s = amount.to_string();
    let f: f64 = s.parse().unwrap_or(0.0);
    f / 10f64.powi(decimals as i32)
}

fn from_f64(amount: f64, decimals: u8) -> U256 {
    let scaled = amount * 10f64.powi(decimals as i32);
    U256::from(scaled as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, B256, U256};

    use crate::pools::types::{CurveState, Pool, PoolKind};

    fn pool() -> Pool {
        Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::CurveVyper,
            factory: None,
            token0: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            token0_decimals: 6,
            token1: address!("dAC17F958D2ee523a2206206994597C13D831ec7"),
            token1_decimals: 6,
            fee: Some(4),
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
            state: serde_json::to_value(&CurveState {
                n_coins: 2,
                a: U256::from(2000),
                fee: U256::from(4_000_000),
                coins: vec![pool().token0, pool().token1],
                decimals: vec![6, 6],
                balances: vec![U256::from(10_000_000_000000u64), U256::from(10_000_000_000000u64)],
            })
            .unwrap(),
        }
    }

    #[test]
    fn basic_stableswap_quote() {
        let p = pool();
        let s = state();
        let out = quote(&p, &s, p.token0, p.token1, U256::from(1_000_000)).unwrap();
        // 1 USDC in should yield slightly less than 1 USDT out after the fee.
        assert!(out > U256::from(999_000));
        assert!(out < U256::from(1_000_000));
    }

    #[test]
    fn reverse_stableswap_quote() {
        let p = pool();
        let s = state();
        let out = quote(&p, &s, p.token1, p.token0, U256::from(1_000_000)).unwrap();
        assert!(out > U256::from(999_000));
        assert!(out < U256::from(1_000_000));
    }
}
