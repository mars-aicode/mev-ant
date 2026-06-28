//! TVL pricing for ranking Liquid Pools.
//!
//! Strategy: stablecoins pegged at $1; major volatile assets priced via reference
//! pools. For V1 we keep it simple: if a pool contains a stablecoin, use the
//! stablecoin side to estimate total TVL.

use alloy::primitives::{address, Address, U256};

/// Tokens treated as $1 stablecoins.
const STABLECOINS: &[Address] = &[
    address!("6B175474E89094C44Da98b954EedeAC495271d0F"), // DAI
    address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
    address!("dAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
    address!("853d955aCEf822Db058eb8505911ED77F175b99e"), // FRAX
    address!("f939E0A03FB07F59A73314E73794Be0E57ac1b4E"), // crvUSD
    address!("4c9EDD5852cd905f086C759E8383e09bff1E68B3"), // USDe
    address!("40D16FC0246aD3160Ccc09B8D0D3A2cD28aE6C2f"), // GHO
];

/// Estimate pool TVL in USD.
///
/// For V1: if one or both tokens are stablecoins, use the stablecoin reserve
/// amounts. If paired with a volatile token, double the stablecoin side as a
/// proxy for full pool TVL. If neither token is a stablecoin, return None.
pub fn price_pool_tvl(
    token0: Address,
    token1: Address,
    reserve0: U256,
    reserve1: U256,
    decimals0: u8,
    decimals1: u8,
) -> Option<f64> {
    let is_stable0 = STABLECOINS.iter().any(|a| *a == token0);
    let is_stable1 = STABLECOINS.iter().any(|a| *a == token1);

    match (is_stable0, is_stable1) {
        (true, true) => {
            let v0 = to_usd(reserve0, decimals0);
            let v1 = to_usd(reserve1, decimals1);
            Some(v0 + v1)
        }
        (true, false) => {
            let v0 = to_usd(reserve0, decimals0);
            Some(v0 * 2.0)
        }
        (false, true) => {
            let v1 = to_usd(reserve1, decimals1);
            Some(v1 * 2.0)
        }
        (false, false) => None,
    }
}

/// Estimate Curve stableswap pool TVL.
///
/// V1: treat all coins as stablecoins pegged at $1 and sum balances. This
/// is appropriate for the stableswap pools targeted in Issue 0003; crypto
/// pools will need a different pricing strategy later.
pub fn price_curve_pool_tvl(
    coins: &[Address],
    decimals: &[u8],
    balances: &[U256],
) -> Option<f64> {
    if coins.len() < 2 || decimals.len() != coins.len() || balances.len() != coins.len() {
        return None;
    }
    let mut tvl = 0.0;
    for (i, bal) in balances.iter().enumerate() {
        let dec = *decimals.get(i)?;
        tvl += to_usd(*bal, dec);
    }
    Some(tvl)
}

fn to_usd(amount: U256, decimals: u8) -> f64 {
    let s = amount.to_string();
    let f: f64 = s.parse().unwrap_or(0.0);
    f / 10f64.powi(decimals as i32)
}

/// Estimate V3 pool TVL from active liquidity and current sqrt price.
pub fn price_v3_pool_tvl(
    token0: Address,
    token1: Address,
    liquidity: U256,
    sqrt_price_x96: U256,
    decimals0: u8,
    decimals1: u8,
) -> Option<f64> {
    if sqrt_price_x96.is_zero() {
        return None;
    }
    let price = sqrt_price_to_price(sqrt_price_x96);
    // virtual reserves: x = L / sqrt(P), y = L * sqrt(P)
    let sqrt_p = price.sqrt();
    let reserve0 = liquidity / U256::from((sqrt_p * 1e18) as u128); // rough scaling
    let reserve1 = liquidity * U256::from((sqrt_p * 1e18) as u128) / U256::from(1_000_000_000_000_000_000u128);
    price_pool_tvl(token0, token1, reserve0, reserve1, decimals0, decimals1)
}

fn sqrt_price_to_price(sqrt_price_x96: U256) -> f64 {
    // price = (sqrtPriceX96 / 2^96)^2
    let q96 = 2f64.powi(96);
    let sp: f64 = sqrt_price_x96.to_string().parse().unwrap_or(0.0);
    (sp / q96).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn both_stablecoins() {
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let tvl = price_pool_tvl(
            usdc,
            dai,
            U256::from(1_000_000_000000u64), // 1,000,000 USDC
            U256::from(1_000_000_000000_000000_000000u128), // 1,000,000 DAI
            6,
            18,
        );
        assert!((tvl.unwrap() - 2_000_000.0).abs() < 1.0);
    }

    #[test]
    fn stable_volatile() {
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let tvl = price_pool_tvl(
            usdc,
            weth,
            U256::from(1_000_000_000000u64), // 1M USDC
            U256::from(500_000_000000_000000_000u128),
            6,
            18,
        );
        assert!((tvl.unwrap() - 2_000_000.0).abs() < 1.0);
    }

    #[test]
    fn volatile_only_no_tvl() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let wbtc = address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599");
        let tvl = price_pool_tvl(weth, wbtc, U256::from(1000), U256::from(50), 18, 8);
        assert!(tvl.is_none());
    }

    #[test]
    fn zero_reserves_collapse_to_zero() {
        // Even an "empty" pool should produce TVL=0, not None, if at
        // least one side is a stablecoin.
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let tvl = price_pool_tvl(usdc, weth, U256::ZERO, U256::ZERO, 6, 18);
        assert_eq!(tvl, Some(0.0));
    }

    #[test]
    fn usde_and_gho_are_stablecoins() {
        // The recently-added USDe and GHO should be priced at $1.
        let usde = address!("4c9EDD5852cd905f086C759E8383e09bff1E68B3");
        let gho = address!("40D16FC0246aD3160Ccc09B8D0D3A2cD28aE6C2f");
        let tvl_usde = price_pool_tvl(
            usde, usde, U256::from(1_000_000_000_000_000_000_000_000u128),
            U256::from(1_000_000_000_000_000_000_000_000u128), 18, 18,
        );
        let tvl_gho = price_pool_tvl(
            gho, gho, U256::from(1_000_000_000u64), U256::from(1_000_000_000u64), 6, 6,
        );
        // 1M USDe at $1 = 1,000,000 USD; 1B GHO units at 6 decimals = 1,000 USD.
        assert!((tvl_usde.unwrap() - 2_000_000.0).abs() < 1.0);
        assert!((tvl_gho.unwrap() - 2_000.0).abs() < 1.0);
    }

    #[test]
    fn curve_pool_with_3_coins_prices_all_coins() {
        // price_curve_pool_tvl accepts n-coin input; the V1 router limits
        // itself to 2-coin pools separately, but the pricing helper itself
        // sums every balance.
        let coins = vec![
            address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC (6)
            address!("dAC17F958D2ee523a2206206994597C13D831ec7"), // USDT (6)
            address!("6B175474E89094C44Da98b954EedeAC495271d0F"), // DAI  (18)
        ];
        // Use realistic magnitudes: 1M USDC, 1M USDT, 1M DAI.
        let bals = vec![
            U256::from(1_000_000_000_000u64),         // 1,000,000 USDC
            U256::from(1_000_000_000_000u64),         // 1,000,000 USDT
            U256::from(1_000_000_000_000_000_000_000_000u128), // 1,000,000 DAI
        ];
        let dec = vec![6u8, 6, 18];
        let tvl = price_curve_pool_tvl(&coins, &dec, &bals).expect("tvl");
        assert!((tvl - 3_000_000.0).abs() < 1.0);
    }

    #[test]
    fn mismatched_lengths_return_none() {
        let coins = vec![address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")];
        let dec = vec![6u8, 6];
        let bals = vec![U256::from(1u64)];
        assert!(price_curve_pool_tvl(&coins, &dec, &bals).is_none());
    }
}
