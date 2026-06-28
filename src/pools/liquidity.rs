//! Liquidity snapshot fetching and touched-pool detection.

use std::collections::HashSet;

use alloy::primitives::{keccak256, Address, B256, U256};
use anyhow::{Context, Result};
use serde_json::json;

use crate::pools::pricing::{price_curve_pool_tvl, price_pool_tvl, price_v3_pool_tvl};
use crate::pools::types::{CurveState, Pool, PoolKind, PoolSnapshot, V3State};
use crate::rpc::RpcClient;

/// `getReserves()` function selector for UniV2.
const GET_RESERVES_SELECTOR: &[u8] = &[0x09, 0x02, 0xf1, 0xac];
/// `slot0()` function selector for UniV3.
const SLOT0_SELECTOR: &[u8] = &[0x38, 0x50, 0xc7, 0xbd];
/// `liquidity()` function selector for UniV3.
const LIQUIDITY_SELECTOR: &[u8] = &[0x1a, 0x68, 0x65, 0x02];

/// `Sync(uint112,uint112)` topic0 for UniV2.
fn sync_topic0() -> B256 {
    keccak256("Sync(uint112,uint112)")
}
/// UniV3 `Swap` topic0.
fn v3_swap_topic0() -> B256 {
    keccak256("Swap(address,address,int256,int256,uint160,uint128,int24)")
}
/// UniV3 `Mint` topic0.
fn v3_mint_topic0() -> B256 {
    keccak256("Mint(address,address,int24,int24,uint128,uint256,uint256)")
}
/// UniV3 `Burn` topic0.
fn v3_burn_topic0() -> B256 {
    keccak256("Burn(int24,int24,uint128,uint256,uint256)")
}
/// Curve `TokenExchange` topic0.
fn curve_exchange_topic0() -> B256 {
    keccak256("TokenExchange(address,int128,uint256,int128,uint256)")
}

/// Curve pool function selectors.
pub fn curve_a_selector() -> B256 { keccak256("A()") }
pub fn curve_fee_selector() -> B256 { keccak256("fee()") }
pub fn curve_balances_selector() -> B256 { keccak256("balances(uint256)") }

/// Fetch and update liquidity for all pools touched in `block_number`.
pub async fn update_touched_pools(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    pools: &[Pool],
    block_number: u64,
) -> Result<()> {
    update_touched_univ2_pools(client, db_pool, pools, block_number).await?;
    update_touched_univ3_pools(client, db_pool, pools, block_number).await?;
    update_touched_curve_pools(client, db_pool, pools, block_number).await?;
    Ok(())
}

/// Fetch and update liquidity for *every* pool, regardless of whether it was
/// touched in the block. Used during the daily full refresh.
pub async fn update_all_pool_snapshots(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    pools: &[Pool],
    block_number: u64,
) -> Result<()> {
    for pool in pools {
        match pool.kind {
            // UniV2 forks (SushiSwap V2, FraxSwap) use the same getReserves interface.
            PoolKind::UniswapV2 | PoolKind::FraxSwap => {
                match fetch_univ2_reserves(client, pool.address).await {
                    Ok((reserve0, reserve1)) => {
                        let tvl_usd = price_pool_tvl(
                            pool.token0, pool.token1, reserve0, reserve1,
                            pool.token0_decimals, pool.token1_decimals,
                        );
                        let state = PoolSnapshot {
                            address: pool.address,
                            pool_id: pool.pool_id,
                            observed_at_block: block_number,
                            reserve0: Some(reserve0),
                            reserve1: Some(reserve1),
                            tvl_usd,
                            state: json!({}),
                        };
                        if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                            tracing::error!("upsert V2 state {}: {}", pool.key(), e);
                        }
                    }
                    Err(e) => tracing::error!("fetch V2 state {}: {}", pool.key(), e),
                }
            }
            // UniV3 forks (PancakeSwap V3) use the same slot0/liquidity interface.
            PoolKind::UniswapV3 | PoolKind::PancakeV3 => {
                let existing = crate::db::get_pool_snapshot(db_pool, pool.address, pool.pool_id).await.ok().flatten();
                let mut v3_state = existing.and_then(|s| serde_json::from_value(s.state).ok()).unwrap_or_else(|| V3State {
                    sqrt_price_x96: U256::ZERO,
                    tick: 0,
                    liquidity: U256::ZERO,
                    tick_spacing: 60,
                    ticks: Vec::new(),
                });
                match fetch_univ3_state(client, pool.address).await {
                    Ok((sqrt_price_x96, tick, liquidity)) => {
                        v3_state.sqrt_price_x96 = sqrt_price_x96;
                        v3_state.tick = tick;
                        v3_state.liquidity = liquidity;
                        let tvl_usd = price_v3_pool_tvl(
                            pool.token0, pool.token1, liquidity, sqrt_price_x96,
                            pool.token0_decimals, pool.token1_decimals,
                        );
                        let state = PoolSnapshot {
                            address: pool.address,
                            pool_id: pool.pool_id,
                            observed_at_block: block_number,
                            reserve0: None,
                            reserve1: None,
                            tvl_usd,
                            state: serde_json::to_value(&v3_state).unwrap_or_default(),
                        };
                        if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                            tracing::error!("upsert V3 state {}: {}", pool.key(), e);
                        }
                    }
                    Err(e) => tracing::error!("fetch V3 state {}: {}", pool.key(), e),
                }
            }
            PoolKind::CurveVyper | PoolKind::CurveRouter => {
                match fetch_curve_state(client, pool, block_number).await {
                    Ok(state) => {
                        if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                            tracing::error!("upsert Curve state {}: {}", pool.key(), e);
                        }
                    }
                    Err(e) => tracing::error!("fetch Curve state {}: {}", pool.key(), e),
                }
            }
            // Balancer V2/V3 and Fluid DEX need custom state fetchers (not in V1).
            _ => {}
        }
    }
    Ok(())
}

async fn update_touched_univ2_pools(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    pools: &[Pool],
    block_number: u64,
) -> Result<()> {
    let univ2_pools: Vec<&Pool> = pools
        .iter()
        .filter(|p| matches!(p.kind, PoolKind::UniswapV2 | PoolKind::FraxSwap))
        .collect();
    if univ2_pools.is_empty() {
        return Ok(());
    }

    let addresses: Vec<String> = univ2_pools.iter().map(|p| format!("{:?}", p.address)).collect();
    let filter = json!({
        "fromBlock": format!("0x{:x}", block_number),
        "toBlock": format!("0x{:x}", block_number),
        "address": addresses,
        "topics": [format!("{:?}", sync_topic0())]
    });

    let logs = client.get_logs(filter).await.context("fetch touched UniV2 Sync logs")?;
    let touched: HashSet<Address> = logs.into_iter().map(|log| log.address).collect();

    for pool in univ2_pools {
        if !touched.contains(&pool.address) {
            continue;
        }
        match fetch_univ2_reserves(client, pool.address).await {
            Ok((reserve0, reserve1)) => {
                let tvl_usd = price_pool_tvl(pool.token0, pool.token1, reserve0, reserve1, pool.token0_decimals, pool.token1_decimals);
                let state = PoolSnapshot {
                    address: pool.address,
                    pool_id: pool.pool_id,
                    observed_at_block: block_number,
                    reserve0: Some(reserve0),
                    reserve1: Some(reserve1),
                    tvl_usd,
                    state: json!({}),
                };
                if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                    tracing::error!("upsert pool state {}: {}", pool.key(), e);
                }
            }
            Err(e) => {
                tracing::error!("fetch reserves for {}: {}", pool.key(), e);
            }
        }
    }

    Ok(())
}

async fn update_touched_univ3_pools(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    pools: &[Pool],
    block_number: u64,
) -> Result<()> {
    let univ3_pools: Vec<&Pool> = pools
        .iter()
        .filter(|p| matches!(p.kind, PoolKind::UniswapV3 | PoolKind::PancakeV3))
        .collect();
    if univ3_pools.is_empty() {
        return Ok(());
    }

    let addresses: Vec<String> = univ3_pools.iter().map(|p| format!("{:?}", p.address)).collect();
    let filter = json!({
        "fromBlock": format!("0x{:x}", block_number),
        "toBlock": format!("0x{:x}", block_number),
        "address": addresses,
        "topics": [[
            format!("{:?}", v3_swap_topic0()),
            format!("{:?}", v3_mint_topic0()),
            format!("{:?}", v3_burn_topic0())
        ]]
    });

    let logs = client.get_logs(filter).await.context("fetch touched UniV3 logs")?;
    let touched: HashSet<Address> = logs.into_iter().map(|log| log.address).collect();

    for pool in univ3_pools {
        if !touched.contains(&pool.address) {
            continue;
        }
        let existing = crate::db::get_pool_snapshot(db_pool, pool.address, pool.pool_id).await.ok().flatten();
        let mut v3_state = existing.and_then(|s| serde_json::from_value(s.state).ok()).unwrap_or_else(|| V3State {
            sqrt_price_x96: U256::ZERO,
            tick: 0,
            liquidity: U256::ZERO,
            tick_spacing: 60,
            ticks: Vec::new(),
        });

        match fetch_univ3_state(client, pool.address).await {
            Ok((sqrt_price_x96, tick, liquidity)) => {
                v3_state.sqrt_price_x96 = sqrt_price_x96;
                v3_state.tick = tick;
                v3_state.liquidity = liquidity;
                let tvl_usd = price_v3_pool_tvl(
                    pool.token0, pool.token1, liquidity, sqrt_price_x96,
                    pool.token0_decimals, pool.token1_decimals,
                );
                let state = PoolSnapshot {
                    address: pool.address,
                    pool_id: pool.pool_id,
                    observed_at_block: block_number,
                    reserve0: None,
                    reserve1: None,
                    tvl_usd,
                    state: serde_json::to_value(&v3_state).unwrap_or_default(),
                };
                if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                    tracing::error!("upsert pool state {}: {}", pool.key(), e);
                }
            }
            Err(e) => {
                tracing::error!("fetch V3 state for {}: {}", pool.key(), e);
            }
        }
    }

    Ok(())
}

async fn update_touched_curve_pools(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    pools: &[Pool],
    block_number: u64,
) -> Result<()> {
    let curve_pools: Vec<&Pool> = pools
        .iter()
        .filter(|p| matches!(p.kind, PoolKind::CurveVyper | PoolKind::CurveRouter))
        .collect();
    if curve_pools.is_empty() {
        return Ok(());
    }

    let addresses: Vec<String> = curve_pools.iter().map(|p| format!("{:?}", p.address)).collect();
    let filter = json!({
        "fromBlock": format!("0x{:x}", block_number),
        "toBlock": format!("0x{:x}", block_number),
        "address": addresses,
        "topics": [format!("{:?}", curve_exchange_topic0())]
    });

    let logs = client.get_logs(filter).await.context("fetch touched Curve logs")?;
    let touched: HashSet<Address> = logs.into_iter().map(|log| log.address).collect();

    for pool in curve_pools {
        if !touched.contains(&pool.address) {
            continue;
        }
        match fetch_curve_state(client, pool, block_number).await {
            Ok(state) => {
                if let Err(e) = crate::db::upsert_pool_snapshot(db_pool, &state).await {
                    tracing::error!("upsert pool state {}: {}", pool.key(), e);
                }
            }
            Err(e) => {
                tracing::error!("fetch Curve state for {}: {}", pool.key(), e);
            }
        }
    }

    Ok(())
}

/// Fetch full Curve stableswap state for a 2-coin pool.
pub async fn fetch_curve_state(
    client: &RpcClient,
    pool: &Pool,
    block_number: u64,
) -> Result<PoolSnapshot> {
    let a = curve_call_u256(client, pool.address, &curve_a_selector().0[..4]).await?;
    let fee = curve_call_u256(client, pool.address, &curve_fee_selector().0[..4]).await?;

    let mut balances = Vec::with_capacity(2);
    for i in 0..2u8 {
        let mut data = curve_balances_selector().0[..4].to_vec();
        data.extend_from_slice(&U256::from(i).to_be_bytes::<32>());
        let bal = curve_call_u256(client, pool.address, &data).await?;
        balances.push(bal);
    }

    let tvl_usd = price_curve_pool_tvl(
        &[pool.token0, pool.token1],
        &[pool.token0_decimals, pool.token1_decimals],
        &balances,
    );

    let curve_state = CurveState {
        n_coins: 2,
        a,
        fee,
        balances,
        coins: vec![pool.token0, pool.token1],
        decimals: vec![pool.token0_decimals, pool.token1_decimals],
    };

    Ok(PoolSnapshot {
        address: pool.address,
        pool_id: pool.pool_id,
        observed_at_block: block_number,
        reserve0: None,
        reserve1: None,
        tvl_usd,
        state: serde_json::to_value(&curve_state).unwrap_or_default(),
    })
}

async fn curve_call_u256(client: &RpcClient, pool: Address, selector: &[u8]) -> Result<U256> {
    let result = client
        .call(pool, alloy::primitives::Bytes::from(selector.to_vec()))
        .await
        .context("curve eth_call")?;
    let bytes = hex::decode(result.trim_start_matches("0x")).context("decode curve call")?;
    if bytes.len() < 32 {
        anyhow::bail!("curve call response too short: {} bytes", bytes.len());
    }
    Ok(U256::from_be_slice(&bytes[0..32]))
}

/// Fetch full reserves for a UniV2 pool.
pub async fn fetch_univ2_reserves(client: &RpcClient, pool: Address) -> Result<(U256, U256)> {
    let data = alloy::primitives::Bytes::from_static(GET_RESERVES_SELECTOR);
    let result = client.call(pool, data).await.context("getReserves call")?;
    let bytes = hex::decode(result.trim_start_matches("0x")).context("decode getReserves")?;
    if bytes.len() < 64 {
        anyhow::bail!("getReserves response too short: {} bytes", bytes.len());
    }
    let reserve0 = U256::from_be_slice(&bytes[0..32]);
    let reserve1 = U256::from_be_slice(&bytes[32..64]);
    Ok((reserve0, reserve1))
}

/// Fetch slot0 + liquidity for a UniV3 pool.
pub async fn fetch_univ3_state(client: &RpcClient, pool: Address) -> Result<(U256, i32, U256)> {
    let slot0_data = alloy::primitives::Bytes::from_static(SLOT0_SELECTOR);
    let slot0_result = client.call(pool, slot0_data).await.context("slot0 call")?;
    let slot0_bytes = hex::decode(slot0_result.trim_start_matches("0x")).context("decode slot0")?;
    if slot0_bytes.len() < 32 {
        anyhow::bail!("slot0 response too short: {} bytes", slot0_bytes.len());
    }
    let sqrt_price_x96 = U256::from_be_slice(&slot0_bytes[0..32]);
    let tick = parse_int24(&slot0_bytes[29..32]);

    let liq_data = alloy::primitives::Bytes::from_static(LIQUIDITY_SELECTOR);
    let liq_result = client.call(pool, liq_data).await.context("liquidity call")?;
    let liq_bytes = hex::decode(liq_result.trim_start_matches("0x")).context("decode liquidity")?;
    if liq_bytes.len() < 32 {
        anyhow::bail!("liquidity response too short: {} bytes", liq_bytes.len());
    }
    let liquidity = U256::from_be_slice(&liq_bytes[0..32]);

    Ok((sqrt_price_x96, tick, liquidity))
}

fn parse_int24(bytes: &[u8]) -> i32 {
    let mut buf = [0u8; 4];
    buf[1..4].copy_from_slice(bytes);
    let raw = i32::from_be_bytes(buf);
    if raw & 0x800000 != 0 {
        raw | !0x7fffff
    } else {
        raw
    }
}
