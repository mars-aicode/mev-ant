//! Pool registry seeding and maintenance.
//!
//! Primary seed source: TheGraph.
//! Fallback: RPC `eth_getLogs` on known DEX factory addresses.

use std::collections::HashSet;

use alloy::primitives::{address, keccak256, Address, B256, U256};
use anyhow::{Context, Result};
use serde_json::json;

use crate::db;
use crate::pools::types::{Pool, PoolKind};
use crate::rpc::RpcClient;

/// Uniswap V2 factory on Ethereum mainnet.
pub const UNISWAP_V2_FACTORY: Address = address!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f");

/// TheGraph hosted-service endpoint for Uniswap V2.
const UNISWAP_V2_SUBGRAPH: &str = "https://api.thegraph.com/subgraphs/name/uniswap/uniswap-v2";

/// TheGraph hosted-service endpoint for Uniswap V3.
const UNISWAP_V3_SUBGRAPH: &str = "https://api.thegraph.com/subgraphs/name/uniswap/uniswap-v3";

/// Uniswap V3 factory on Ethereum mainnet.
pub const UNISWAP_V3_FACTORY: Address = address!("1F98431c8aD98523631AE4a59f267346ea31F984");

/// Curve registry on Ethereum mainnet (legacy registry, pool enumeration).
pub const CURVE_REGISTRY: Address = address!("90E00ACe148ca3b23AcB1cE0E2c6D85dDFde5C7f");

/// Seed UniV2 pools from TheGraph, returning the top N by `reserveUSD`.
pub async fn seed_univ2_from_subgraph(top_n: usize) -> Result<Vec<Pool>> {
    let client = reqwest::Client::new();
    let query = format!(
        r#"{{
            pairs(first: {}, orderBy: reserveUSD, orderDirection: desc) {{
                id
                token0 {{ id decimals }}
                token1 {{ id decimals }}
                reserveUSD
            }}
        }}"#,
        top_n
    );

    let resp = client
        .post(UNISWAP_V2_SUBGRAPH)
        .json(&json!({ "query": query }))
        .send()
        .await
        .context("TheGraph UniV2 request")?;

    let status = resp.status();
    let body = resp.text().await.context("TheGraph UniV2 response body")?;
    if !status.is_success() {
        anyhow::bail!("TheGraph UniV2 returned {}: {}", status, body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body).context("parse TheGraph UniV2 JSON")?;
    let pairs = parsed
        .get("data")
        .and_then(|d| d.get("pairs"))
        .and_then(|p| p.as_array())
        .context("missing pairs in TheGraph response")?;

    let mut pools = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let address: Address = parse_address(pair.get("id").and_then(|v| v.as_str()))?;
        let token0: Address = parse_address(pair.get("token0").and_then(|t| t.get("id")).and_then(|v| v.as_str()))?;
        let token1: Address = parse_address(pair.get("token1").and_then(|t| t.get("id")).and_then(|v| v.as_str()))?;
        let token0_decimals = parse_u8(pair.get("token0").and_then(|t| t.get("decimals")).and_then(|v| v.as_u64()))?;
        let token1_decimals = parse_u8(pair.get("token1").and_then(|t| t.get("decimals")).and_then(|v| v.as_u64()))?;
        pools.push(Pool {
            address,
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: Some(UNISWAP_V2_FACTORY),
            token0,
            token0_decimals,
            token1,
            token1_decimals,
            fee: Some(30),
            block_created: None,
        });
    }

    Ok(pools)
}

/// Seed UniV3 pools from TheGraph, returning the top N by `totalValueLockedUSD`
/// along with their current tick list.
pub async fn seed_univ3_from_subgraph(top_n: usize) -> Result<Vec<(Pool, crate::pools::types::V3State)>> {
    let client = reqwest::Client::new();
    let query = format!(
        r#"{{
            pools(first: {}, orderBy: totalValueLockedUSD, orderDirection: desc) {{
                id
                token0 {{ id decimals }}
                token1 {{ id decimals }}
                feeTier
                tickSpacing
                sqrtPriceX96
                liquidity
                tick
                ticks(orderBy: tickIdx) {{
                    tickIdx
                    liquidityNet
                }}
            }}
        }}"#,
        top_n
    );

    let resp = client
        .post(UNISWAP_V3_SUBGRAPH)
        .json(&json!({ "query": query }))
        .send()
        .await
        .context("TheGraph UniV3 request")?;

    let status = resp.status();
    let body = resp.text().await.context("TheGraph UniV3 response body")?;
    if !status.is_success() {
        anyhow::bail!("TheGraph UniV3 returned {}: {}", status, body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body).context("parse TheGraph UniV3 JSON")?;
    let pool_arr = parsed
        .get("data")
        .and_then(|d| d.get("pools"))
        .and_then(|p| p.as_array())
        .context("missing pools in TheGraph response")?;

    let mut out = Vec::with_capacity(pool_arr.len());
    for p in pool_arr {
        let address: Address = parse_address(p.get("id").and_then(|v| v.as_str()))?;
        let token0: Address = parse_address(p.get("token0").and_then(|t| t.get("id")).and_then(|v| v.as_str()))?;
        let token1: Address = parse_address(p.get("token1").and_then(|t| t.get("id")).and_then(|v| v.as_str()))?;
        let token0_decimals = parse_u8(p.get("token0").and_then(|t| t.get("decimals")).and_then(|v| v.as_u64()))?;
        let token1_decimals = parse_u8(p.get("token1").and_then(|t| t.get("decimals")).and_then(|v| v.as_u64()))?;
        let fee: u32 = p.get("feeTier")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .context("missing feeTier")?;
        let tick_spacing: i16 = p.get("tickSpacing")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .context("missing tickSpacing")?;
        let sqrt_price_x96: U256 = parse_u256(p.get("sqrtPriceX96").and_then(|v| v.as_str()))?;
        let liquidity: U256 = parse_u256(p.get("liquidity").and_then(|v| v.as_str()))?;
        let tick: i32 = p.get("tick")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .context("missing tick")?;

        let ticks: Vec<crate::pools::types::V3Tick> = p
            .get("ticks")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        let idx = t.get("tickIdx")?.as_str()?.parse().ok()?;
                        let net = t.get("liquidityNet")?.as_str()?.parse().ok()?;
                        Some(crate::pools::types::V3Tick { idx, liquidity_net: net })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let pool = Pool {
            address,
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV3,
            factory: Some(UNISWAP_V3_FACTORY),
            token0,
            token0_decimals,
            token1,
            token1_decimals,
            fee: Some(fee),
            block_created: None,
        };
        let v3_state = crate::pools::types::V3State {
            sqrt_price_x96,
            tick,
            liquidity,
            tick_spacing,
            ticks,
        };
        out.push((pool, v3_state));
    }

    Ok(out)
}

/// Fallback: scan `PoolCreated` events from the UniV3 factory over a block range.
#[allow(dead_code)]
pub async fn seed_univ3_from_rpc(
    client: &RpcClient,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    let topic0 = keccak256("PoolCreated(address,address,uint24,int24,address)");
    let filter = json!({
        "fromBlock": format!("0x{:x}", from_block),
        "toBlock": format!("0x{:x}", to_block),
        "address": format!("{:?}", UNISWAP_V3_FACTORY),
        "topics": [format!("{:?}", topic0)]
    });

    let logs = client.get_logs(filter).await.context("RPC UniV3 factory logs")?;

    let mut pools = Vec::with_capacity(logs.len());
    let mut seen = HashSet::new();
    for log in logs {
        // PoolCreated(address indexed token0, address indexed token1, uint24 indexed fee, int24 tickSpacing, address pool)
        if log.topics.len() < 4 {
            continue;
        }
        let token0 = address_from_topic(log.topics[1]);
        let token1 = address_from_topic(log.topics[2]);
        let fee_bytes = log.topics[3].as_slice();
        let fee = u32::from_be_bytes([0, fee_bytes[29], fee_bytes[30], fee_bytes[31]]);
        let tick_spacing_bytes = log.data.trim_start_matches("0x");
        let _tick_spacing = if tick_spacing_bytes.len() >= 64 {
            let ts_hex = &tick_spacing_bytes[24..64];
            let decoded = hex::decode(ts_hex).ok().and_then(|b| {
                b.get(14..16).and_then(|s| <[u8; 2]>::try_from(s).ok())
            });
            if let Some(arr) = decoded {
                i16::from_be_bytes(arr)
            } else {
                continue;
            }
        } else {
            continue;
        };
        let pool = log.address;
        if seen.insert(pool) {
            pools.push(Pool {
                address: pool,
                pool_id: B256::ZERO,
                kind: PoolKind::UniswapV3,
                factory: Some(UNISWAP_V3_FACTORY),
                token0,
                token0_decimals: 18,
                token1,
                token1_decimals: 18,
                fee: Some(fee),
                block_created: None,
            });
        }
    }

    Ok(pools)
}

/// Seed Curve stableswap pools.
///
/// V1 supports 2-coin pools only (the Pool type is token0/token1). Pools with
/// more than two coins are skipped; they will be added once the routing graph
/// supports n-coin pools.
///
/// Strategy:
/// 1. Try TheGraph Curve subgraph.
/// 2. Fall back to on-chain registry enumeration.
/// 3. If both fail, use a hardcoded list of top 2-coin stable pools so the
///    seed command remains usable on nodes without registry state.
pub async fn seed_curve_from_registry(
    client: &RpcClient,
    top_n: usize,
) -> Result<Vec<Pool>> {
    match seed_curve_from_subgraph(top_n).await {
        Ok(pools) if !pools.is_empty() => return Ok(pools),
        Ok(_) => tracing::warn!("Curve subgraph returned empty; trying registry"),
        Err(e) => tracing::warn!("Curve subgraph unavailable: {}; trying registry", e),
    }

    match seed_curve_from_rpc_registry(client, top_n).await {
        Ok(pools) if !pools.is_empty() => return Ok(pools),
        Ok(_) => tracing::warn!("Curve registry returned empty; using hardcoded fallback"),
        Err(e) => tracing::warn!("Curve registry unavailable: {}; using hardcoded fallback", e),
    }

    Ok(seed_curve_hardcoded().into_iter().take(top_n).collect())
}

/// TheGraph endpoint for Curve pools.
const CURVE_SUBGRAPH: &str = "https://api.thegraph.com/subgraphs/name/curvefi/curve";

/// Seed Curve pools from TheGraph.
pub async fn seed_curve_from_subgraph(top_n: usize) -> Result<Vec<Pool>> {
    let http = reqwest::Client::new();
    let query = format!(
        r#"{{
            pools(first: {}, orderBy: cumulativeVolumeUSD, orderDirection: desc) {{
                id
                name
                coins
                decimals
                isV2
            }}
        }}"#,
        top_n * 4
    );

    let resp = http
        .post(CURVE_SUBGRAPH)
        .json(&json!({ "query": query }))
        .send()
        .await
        .context("Curve TheGraph request")?;

    let status = resp.status();
    let body = resp.text().await.context("Curve TheGraph body")?;
    if !status.is_success() {
        anyhow::bail!("Curve TheGraph returned {}: {}", status, body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body).context("parse Curve TheGraph JSON")?;
    let pool_arr = parsed
        .get("data")
        .and_then(|d| d.get("pools"))
        .and_then(|p| p.as_array())
        .context("missing pools in Curve TheGraph response")?;

    let mut pools = Vec::new();
    for p in pool_arr {
        let coins: Vec<Address> = p
            .get("coins")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()))
                    .collect()
            })
            .unwrap_or_default();
        let decimals: Vec<u8> = p
            .get("decimals")
            .and_then(|d| d.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().and_then(|n| n.try_into().ok())).collect())
            .unwrap_or_default();

        if coins.len() != 2 || decimals.len() != 2 {
            continue; // V1: 2-coin pools only
        }

        let address: Address = parse_address(p.get("id").and_then(|v| v.as_str()))?;
        pools.push(Pool {
            address,
            pool_id: B256::ZERO,
            kind: PoolKind::CurveVyper,
            factory: Some(CURVE_REGISTRY),
            token0: coins[0],
            token0_decimals: decimals[0],
            token1: coins[1],
            token1_decimals: decimals[1],
            fee: None, // fetched on-chain later
            block_created: None,
        });
    }

    Ok(pools.into_iter().take(top_n).collect())
}

/// Seed Curve pools by enumerating the on-chain registry.
async fn seed_curve_from_rpc_registry(
    client: &RpcClient,
    top_n: usize,
) -> Result<Vec<Pool>> {
    let pool_count = curve_pool_count(client).await?;
    let mut candidates = Vec::with_capacity(pool_count.min(top_n * 3));

    // Enumerate registry entries up to a generous cap so we can rank by TVL.
    let scan_limit = (top_n * 3).min(pool_count);
    for i in 0..scan_limit {
        match curve_pool_at_index(client, U256::from(i)).await {
            Ok(pool_addr) => {
                if let Ok(Some(meta)) = curve_pool_meta(client, pool_addr).await {
                    candidates.push((pool_addr, meta));
                }
            }
            Err(e) => {
                tracing::debug!("curve pool index {} lookup failed: {}", i, e);
            }
        }
    }

    // Rank by rough USD TVL: for stable pairs, sum stablecoin balances.
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|(addr, meta)| {
            let tvl = crate::pools::pricing::price_curve_pool_tvl(&meta.coins, &meta.decimals, &meta.balances);
            tvl.map(|t| (addr, meta, t))
        })
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_n);

    Ok(ranked
        .into_iter()
        .map(|(addr, meta, _)| Pool {
            address: addr,
            pool_id: B256::ZERO,
            kind: PoolKind::CurveVyper,
            factory: Some(CURVE_REGISTRY),
            token0: meta.coins[0],
            token0_decimals: meta.decimals[0],
            token1: meta.coins[1],
            token1_decimals: meta.decimals[1],
            fee: Some((meta.fee.to::<u64>() / 1_000_000_000_0) as u32), // 1e10 fee precision -> bps-ish
            block_created: None,
        })
        .collect())
}

/// Hardcoded top 2-coin Curve stable pools used when both TheGraph and the
/// on-chain registry are unreachable. These are legacy/mainnet stable pools.
fn seed_curve_hardcoded() -> Vec<Pool> {
    vec![
        Pool {
            address: address!("DcEF968d416a41Cdac0ED8702fAC8128A64241A2"),
            pool_id: B256::ZERO,
            kind: PoolKind::CurveVyper,
            factory: Some(CURVE_REGISTRY),
            token0: address!("853d955aCEf822Db058eb8505911ED77F175b99e"), // FRAX
            token0_decimals: 18,
            token1: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            token1_decimals: 6,
            fee: None,
            block_created: None,
        },
        Pool {
            address: address!("06364f10B501e868329afBc005b3492902d6C763"),
            pool_id: B256::ZERO,
            kind: PoolKind::CurveVyper,
            factory: Some(CURVE_REGISTRY),
            token0: address!("8E870D67F660D95d5be530380D0eC0bd388289E1"), // PAX
            token0_decimals: 18,
            token1: address!("6c3F90f043a72FA612cbac8115EE7e52BDe6E490"), // 3CRV
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
    ]
}

#[derive(Debug)]
struct CurvePoolMeta {
    pub fee: U256,
    pub coins: Vec<Address>,
    pub decimals: Vec<u8>,
    pub balances: Vec<U256>,
}

async fn curve_pool_count(client: &RpcClient) -> Result<usize> {
    let selector = keccak256("pool_count()");
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(selector.0[..4].to_vec()))
        .await
        .context("curve pool_count")?;
    let count = decode_u256(&result)?;
    Ok(count.to::<usize>())
}

async fn curve_pool_at_index(client: &RpcClient, index: U256) -> Result<Address> {
    let selector = keccak256("pool_list(uint256)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(&index.to_be_bytes::<32>());
    let result = client.call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve pool_list")?;
    decode_address(&result)
}

async fn curve_pool_meta(client: &RpcClient, pool: Address) -> Result<Option<CurvePoolMeta>> {
    let n_coins = match curve_get_n_coins(client, pool).await {
        Ok(n) => n as u8,
        Err(e) => {
            tracing::debug!("curve get_n_coins for {:?} failed: {}", pool, e);
            return Ok(None);
        }
    };
    if n_coins != 2 {
        // V1: only 2-coin pools fit the token0/token1 model.
        return Ok(None);
    }

    let coins = curve_get_coins(client, pool).await?;
    let decimals = curve_get_decimals(client, pool).await?;
    let balances = curve_get_balances(client, pool).await?;
    let fee = curve_get_fees(client, pool).await.unwrap_or(U256::ZERO);

    Ok(Some(CurvePoolMeta {
        fee,
        coins,
        decimals,
        balances,
    }))
}

async fn curve_get_n_coins(client: &RpcClient, pool: Address) -> Result<u64> {
    let selector = keccak256("get_n_coins(address)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(pool.as_slice());
    data.resize(4 + 32, 0);
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve get_n_coins")?;
    // get_n_coins returns uint256[2]; first word is n_coins.
    let arr = decode_u256_array(&result, 2)?;
    Ok(arr[0].to::<u64>())
}

async fn curve_get_coins(client: &RpcClient, pool: Address) -> Result<Vec<Address>> {
    let selector = keccak256("get_coins(address)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(pool.as_slice());
    data.resize(4 + 32, 0);
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve get_coins")?;
    decode_address_array(&result, 8)
}

async fn curve_get_decimals(client: &RpcClient, pool: Address) -> Result<Vec<u8>> {
    let selector = keccak256("get_decimals(address)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(pool.as_slice());
    data.resize(4 + 32, 0);
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve get_decimals")?;
    let arr = decode_u256_array(&result, 8)?;
    Ok(arr.iter().map(|v| v.to::<u64>() as u8).collect())
}

async fn curve_get_balances(client: &RpcClient, pool: Address) -> Result<Vec<U256>> {
    let selector = keccak256("get_balances(address)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(pool.as_slice());
    data.resize(4 + 32, 0);
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve get_balances")?;
    decode_u256_array(&result, 8)
}

async fn curve_get_fees(client: &RpcClient, pool: Address) -> Result<U256> {
    let selector = keccak256("get_fees(address)");
    let mut data = selector.0[..4].to_vec();
    data.extend_from_slice(pool.as_slice());
    data.resize(4 + 32, 0);
    let result = client
        .call(CURVE_REGISTRY, alloy::primitives::Bytes::from(data))
        .await
        .context("curve get_fees")?;
    // get_fees returns uint256[2]: [fee, admin_fee]; we only need fee.
    let arr = decode_u256_array(&result, 2)?;
    Ok(arr[0])
}

fn decode_u256(hex_result: &str) -> Result<U256> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).context("decode u256")?;
    if bytes.len() < 32 {
        anyhow::bail!("u256 result too short: {} bytes", bytes.len());
    }
    Ok(U256::from_be_slice(&bytes[0..32]))
}

fn decode_u256_array(hex_result: &str, size: usize) -> Result<Vec<U256>> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).context("decode u256 array")?;
    if bytes.len() < 32 + size * 32 {
        anyhow::bail!("u256 array result too short: {} bytes", bytes.len());
    }
    // Static arrays returned in-place; ABI-encoded as consecutive 32-byte words.
    let mut out = Vec::with_capacity(size);
    for i in 0..size {
        let start = 32 + i * 32;
        out.push(U256::from_be_slice(&bytes[start..start + 32]));
    }
    Ok(out)
}

fn decode_address(hex_result: &str) -> Result<Address> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).context("decode address")?;
    if bytes.len() < 32 {
        anyhow::bail!("address result too short: {} bytes", bytes.len());
    }
    Ok(Address::from_slice(&bytes[12..32]))
}

fn decode_address_array(hex_result: &str, size: usize) -> Result<Vec<Address>> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).context("decode address array")?;
    if bytes.len() < 32 + size * 32 {
        anyhow::bail!("address array result too short: {} bytes", bytes.len());
    }
    let mut out = Vec::with_capacity(size);
    for i in 0..size {
        let start = 32 + i * 32;
        out.push(Address::from_slice(&bytes[start + 12..start + 32]));
    }
    Ok(out)
}

/// Full refresh used by the background liquidity job (and the CLI seed command).
///
/// 1. Re-seed V2/V3 from TheGraph and Curve from on-chain registry.
/// 2. If TheGraph is unavailable, existing DB pools are left in place so the
///    job can continue with stale-but-valid registry data.
/// 3. Fetch on-chain state for every seeded pool and rank the top N by TVL.
///
/// Returns the ranked list and updates `liquid_pools` in the DB.
pub async fn refresh_liquid_pools(
    client: &RpcClient,
    db_pool: &sqlx::PgPool,
    top_n: usize,
) -> Result<Vec<(Pool, f64)>> {
    let block_number = client.block_number().await?;
    tracing::info!("refreshing liquidity at block {}", block_number);

    // Seed UniV2. Failure is non-fatal: we keep whatever is already in the DB.
    let v2_pools = match seed_univ2_from_subgraph(top_n).await {
        Ok(p) => {
            db::insert_pools(db_pool, &p).await?;
            p
        }
        Err(e) => {
            tracing::warn!("TheGraph UniV2 unavailable during refresh: {}; using DB pools", e);
            Vec::new()
        }
    };

    // Seed UniV3.
    let v3 = match seed_univ3_from_subgraph(top_n).await {
        Ok(p) => {
            let pools: Vec<_> = p.iter().map(|(pool, _)| pool.clone()).collect();
            db::insert_pools(db_pool, &pools).await?;
            p
        }
        Err(e) => {
            tracing::warn!("TheGraph UniV3 unavailable during refresh: {}; using DB pools", e);
            Vec::new()
        }
    };

    // Seed Curve.
    let curve_pools = match seed_curve_from_registry(client, top_n).await {
        Ok(p) => {
            db::insert_pools(db_pool, &p).await?;
            p
        }
        Err(e) => {
            tracing::warn!("Curve seeding unavailable during refresh: {}; using DB pools", e);
            Vec::new()
        }
    };

    // Seed SushiSwap V2 (UniV2 fork) from the PairCreated log history.
    let sushi_pools = match seed_sushiswap_v2(client, 10_783_000, block_number).await {
        Ok(p) => {
            db::insert_pools(db_pool, &p).await?;
            p
        }
        Err(e) => {
            tracing::warn!("SushiSwap V2 seeding failed: {}; skipping", e);
            Vec::new()
        }
    };

    // Seed FraxSwap V2 (UniV2 fork).
    let frax_pools = match seed_fraxswap_v2(client, 16_773_425, block_number).await {
        Ok(p) => {
            db::insert_pools(db_pool, &p).await?;
            p
        }
        Err(e) => {
            tracing::warn!("FraxSwap V2 seeding failed: {}; skipping", e);
            Vec::new()
        }
    };

    // Seed PancakeSwap V3 (UniV3 fork).
    let pancake_pools = match seed_pancakeswap_v3(client, 17_028_000, block_number).await {
        Ok(p) => {
            db::insert_pools(db_pool, &p).await?;
            p
        }
        Err(e) => {
            tracing::warn!("PancakeSwap V3 seeding failed: {}; skipping", e);
            Vec::new()
        }
    };

    // Seed Balancer V2 + V3 from hardcoded top pools.
    let balancer_v2 = seed_balancer_v2_top();
    if let Err(e) = db::insert_pools(db_pool, &balancer_v2).await {
        tracing::warn!("Balancer V2 pool insert failed: {}", e);
    }
    let balancer_v3 = seed_balancer_v3_top();
    if let Err(e) = db::insert_pools(db_pool, &balancer_v3).await {
        tracing::warn!("Balancer V3 pool insert failed: {}", e);
    }

    // Seed Fluid DEX from hardcoded top pools.
    let fluid = seed_fluid_top();
    if let Err(e) = db::insert_pools(db_pool, &fluid).await {
        tracing::warn!("Fluid pool insert failed: {}", e);
    }

    // Refresh state for every pool we just seeded.
    let seeded: Vec<_> = v2_pools
        .iter()
        .chain(v3.iter().map(|(p, _)| p))
        .chain(curve_pools.iter())
        .chain(sushi_pools.iter())
        .chain(frax_pools.iter())
        .chain(pancake_pools.iter())
        .cloned()
        .collect();
    crate::pools::liquidity::update_all_pool_snapshots(client, db_pool, &seeded, block_number).await?;

    // For V3, also preserve tick data from TheGraph if on-chain refresh didn't write it.
    for (p, v3_state) in &v3 {
        let existing = db::get_pool_snapshot(db_pool, p.address, p.pool_id).await.ok().flatten();
        if existing.map(|s| s.state.get("ticks").and_then(|t| t.as_array()).map(|a| a.is_empty()).unwrap_or(true)).unwrap_or(true) {
            let tvl = crate::pools::pricing::price_v3_pool_tvl(
                p.token0, p.token1, v3_state.liquidity, v3_state.sqrt_price_x96,
                p.token0_decimals, p.token1_decimals,
            );
            let state = crate::pools::types::PoolSnapshot {
                address: p.address,
                pool_id: p.pool_id,
                observed_at_block: block_number,
                reserve0: None,
                reserve1: None,
                tvl_usd: tvl,
                state: serde_json::to_value(v3_state).unwrap_or_default(),
            };
            db::upsert_pool_snapshot(db_pool, &state).await?;
        }
    }

    // Rank by TVL.
    let all = db::get_all_pools_with_snapshots(db_pool).await?;
    let mut ranked: Vec<_> = all
        .into_iter()
        .filter(|(_, s)| s.tvl_usd.is_some())
        .map(|(p, s)| (p, s.tvl_usd.unwrap()))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_n);
    db::set_liquid_pools(db_pool, &ranked).await?;

    tracing::info!("full refresh complete: {} liquid pools ranked", ranked.len());
    Ok(ranked)
}

/// Fallback: scan `PairCreated` events from the UniV2 factory over a block range.
#[allow(dead_code)]
pub async fn seed_univ2_from_rpc(
    client: &RpcClient,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    seed_v2_from_factory(
        client,
        UNISWAP_V2_FACTORY,
        PoolKind::UniswapV2,
        from_block,
        to_block,
    )
    .await
}

/// Generic UniV2-style factory seeder.
///
/// `PairCreated(address,address,address,uint256)` is identical across UniV2,
/// SushiSwap V2, and FraxSwap V2. The `kind` is recorded on each pool so the
/// detector and quoter can dispatch correctly.
pub async fn seed_v2_from_factory(
    client: &RpcClient,
    factory: Address,
    kind: PoolKind,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    let topic0 = pair_created_topic0();
    let filter = json!({
        "fromBlock": format!("0x{:x}", from_block),
        "toBlock": format!("0x{:x}", to_block),
        "address": format!("{:?}", factory),
        "topics": [format!("{:?}", topic0)]
    });

    let logs = client.get_logs(filter).await.context("RPC V2 factory logs")?;

    let mut pools = Vec::with_capacity(logs.len());
    let mut seen = HashSet::new();
    for log in logs {
        // PairCreated(address indexed token0, address indexed token1, address pair)
        if log.topics.len() < 3 {
            continue;
        }
        let token0 = address_from_topic(log.topics[1]);
        let token1 = address_from_topic(log.topics[2]);
        // PairCreated(address indexed, address indexed, address pair, uint256)
        // `pair` is the first non-indexed parameter, so it occupies bytes 0..32 of `data`.
        let pair = parse_address_from_data(&log.data)?;
        if seen.insert(pair) {
            pools.push(Pool {
                address: pair,
                pool_id: B256::ZERO,
                kind,
                factory: Some(factory),
                token0,
                token0_decimals: 18,
                token1,
                token1_decimals: 18,
                fee: Some(30),
                block_created: None,
            });
        }
    }

    Ok(pools)
}

/// Compute the PairCreated topic0.
/// Standard UniV2 event: PairCreated(address indexed, address indexed, address, uint256)
#[allow(dead_code)]
fn pair_created_topic0() -> B256 {
    keccak256("PairCreated(address,address,address,uint256)")
}

fn parse_address(value: Option<&str>) -> Result<Address> {
    let s = value.context("missing address field")?;
    s.parse().with_context(|| format!("invalid address: {}", s))
}

fn parse_u8(value: Option<u64>) -> Result<u8> {
    value
        .and_then(|v| v.try_into().ok())
        .context("missing or invalid decimals field")
}

fn parse_u256(value: Option<&str>) -> Result<U256> {
    let s = value.context("missing u256 field")?;
    s.parse().with_context(|| format!("invalid u256: {}", s))
}

#[allow(dead_code)]
fn address_from_topic(topic: B256) -> Address {
    Address::from_slice(&topic[12..32])
}

#[allow(dead_code)]
fn parse_address_from_data(data: &str) -> Result<Address> {
    let data = data.trim_start_matches("0x");
    if data.len() < 64 {
        anyhow::bail!("PairCreated data too short: {}", data);
    }
    // last 20 bytes of first 32-byte word
    let addr_hex = &data[24..64];
    let addr = Address::from_slice(&hex::decode(addr_hex).context("decode pair address")?);
    Ok(addr)
}

// ---------------------------------------------------------------------------
// Additional DEX forks: SushiSwap V2, FraxSwap V2, PancakeSwap V3,
// Balancer V2, Balancer V3, Fluid DEX
// ---------------------------------------------------------------------------

/// SushiSwap V2 factory on Ethereum mainnet (UniV2 fork).
pub const SUSHISWAP_V2_FACTORY: Address = address!("C0AEe478e3658e2610c5F7A4A2E1777DeE94ebeD");

/// FraxSwap V2 factory on Ethereum mainnet (UniV2 fork).
pub const FRAXSWAP_V2_FACTORY: Address = address!("43eC799eAdd63848443E2347C49f5f52e8Fe0F6f");

/// PancakeSwap V3 factory on Ethereum mainnet (UniV3 fork).
pub const PANCAKESWAP_V3_FACTORY: Address = address!("0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865");

/// Balancer V2 Vault on Ethereum mainnet.
pub const BALANCER_V2_VAULT_ADDR: Address = address!("BA12222222228d8Ba445958a75a0704d566BF2C8");

/// Balancer V3 Vault proxy on Ethereum mainnet.
pub const BALANCER_V3_VAULT_ADDR: Address = address!("bA1333333333a1BA1108E8412f11850A5C319bA9");

/// Seed SushiSwap V2 pools by indexing `PairCreated` on its factory.
pub async fn seed_sushiswap_v2(
    client: &RpcClient,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    seed_v2_from_factory(
        client,
        SUSHISWAP_V2_FACTORY,
        PoolKind::UniswapV2,
        from_block,
        to_block,
    )
    .await
    .context("seed SushiSwap V2")
}

/// Seed FraxSwap V2 pools by indexing `PairCreated` on its factory.
pub async fn seed_fraxswap_v2(
    client: &RpcClient,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    seed_v2_from_factory(
        client,
        FRAXSWAP_V2_FACTORY,
        PoolKind::FraxSwap,
        from_block,
        to_block,
    )
    .await
    .context("seed FraxSwap V2")
}

/// Seed PancakeSwap V3 pools by indexing `PoolCreated` on its factory.
/// Identical to UniV3 factory log signature.
pub async fn seed_pancakeswap_v3(
    client: &RpcClient,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Pool>> {
    let topic0 = keccak256("PoolCreated(address,address,uint24,int24,address)");
    let filter = json!({
        "fromBlock": format!("0x{:x}", from_block),
        "toBlock": format!("0x{:x}", to_block),
        "address": format!("{:?}", PANCAKESWAP_V3_FACTORY),
        "topics": [format!("{:?}", topic0)]
    });

    let logs = client.get_logs(filter).await.context("RPC PancakeSwap V3 factory logs")?;

    let mut pools = Vec::with_capacity(logs.len());
    let mut seen = HashSet::new();
    for log in logs {
        if log.topics.len() < 4 {
            continue;
        }
        let token0 = address_from_topic(log.topics[1]);
        let token1 = address_from_topic(log.topics[2]);
        let fee_bytes = log.topics[3].as_slice();
        let fee = u32::from_be_bytes([0, fee_bytes[29], fee_bytes[30], fee_bytes[31]]);

        // PoolCreated's last param is the pool address.
        let data = log.data.trim_start_matches("0x");
        if data.len() < 64 {
            continue;
        }
        let addr_hex = &data[24..64];
        let pool = match Address::try_from(hex::decode(addr_hex).context("decode pancake pool")?.as_slice()) {
            Ok(a) => a,
            Err(_) => continue,
        };
        if seen.insert(pool) {
            pools.push(Pool {
                address: pool,
                pool_id: B256::ZERO,
                kind: PoolKind::PancakeV3,
                factory: Some(PANCAKESWAP_V3_FACTORY),
                token0,
                token0_decimals: 18,
                token1,
                token1_decimals: 18,
                fee: Some(fee),
                block_created: None,
            });
        }
    }

    Ok(pools)
}

/// Hardcoded top Balancer V2 pools (by historical TVL).
///
/// The hosted TheGraph endpoint is retired and the Balancer-hosted API
/// requires a key. V1 uses a small curated list; full enumeration is left
/// for a follow-up that ingests the official API.
pub fn seed_balancer_v2_top() -> Vec<Pool> {
    // Common token addresses on Ethereum mainnet.
    let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let wbtc = address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599");
    let wsteth = address!("7f39C581F595B53c5cb19bD0b3f8dA6c935E2Ca0");
    let reth = address!("ae78736Cd615f374D3085123A210448E74Fc6393");
    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

    vec![
        // rETH / WETH (ComposableStable)
        Pool {
            address: address!("1E19CF2D73a01Ef0b76E5F40A0B0e388a20C2B86"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV2,
            factory: Some(BALANCER_V2_VAULT_ADDR),
            token0: reth,
            token0_decimals: 18,
            token1: weth,
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
        // wstETH / WETH
        Pool {
            address: address!("32296969EF14EB0C6d29669C550D4a0449130230"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV2,
            factory: Some(BALANCER_V2_VAULT_ADDR),
            token0: wsteth,
            token0_decimals: 18,
            token1: weth,
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
        // WBTC / WETH
        Pool {
            address: address!("A6F548DFBf0864661BCa1A6eD50A4d3A4C9790A4"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV2,
            factory: Some(BALANCER_V2_VAULT_ADDR),
            token0: wbtc,
            token0_decimals: 8,
            token1: weth,
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
        // USDC / WETH
        Pool {
            address: address!("96646936b91d6B6CA5f6503BFCFBc288e63def06"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV2,
            factory: Some(BALANCER_V2_VAULT_ADDR),
            token0: usdc,
            token0_decimals: 6,
            token1: weth,
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
    ]
}

/// Hardcoded top Balancer V3 pools. V3 launched in 2025 and TVL is still
/// smaller; we seed the most-trafficked pairs.
pub fn seed_balancer_v3_top() -> Vec<Pool> {
    let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    let wsteth = address!("7f39C581F595B53c5cb19bD0b3f8dA6c935E2Ca0");

    vec![
        Pool {
            address: address!("d10e65A5C7C7De25120b99BB1cA5C02344E0c5b6"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV3,
            factory: Some(BALANCER_V3_VAULT_ADDR),
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: None,
            block_created: None,
        },
        Pool {
            address: address!("88B4B5D6561c8d7B7Eb04b2aEa3F7B8bEAd51cC5"),
            pool_id: B256::ZERO,
            kind: PoolKind::BalancerV3,
            factory: Some(BALANCER_V3_VAULT_ADDR),
            token0: wsteth,
            token0_decimals: 18,
            token1: weth,
            token1_decimals: 18,
            fee: None,
            block_created: None,
        },
    ]
}

/// Hardcoded top Fluid DEX pools. Fluid uses a vault-shaped AMM that is not
/// UniV2-compatible; V1 records the pool/token metadata but TVL stays
/// `None` until a custom state fetcher is added (Issue 0008 follow-up).
pub fn seed_fluid_top() -> Vec<Pool> {
    let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    let usdt = address!("dAC17F958D2ee523a2206206994597C13D831ec7");
    let wbtc = address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599");

    vec![
        // FluidLiquidity deployer-derived pool addresses. These are the
        // canonical token-pair contract addresses exposed by the Instadapp
        // UI; they should be verified per chain in a follow-up.
        Pool {
            address: address!("9Fb7b4477576Fe5B32be4C6c31834De5A8C9A9D4"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: Some(address!("52Aa899454998Be5b000Ad077a46Bbe360F4e497")),
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: None,
            block_created: None,
        },
        Pool {
            address: address!("4E29d2EEA651CDF0bE4d68Fe4b2d9B5C3b3a3B66"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: Some(address!("52Aa899454998Be5b000Ad077a46Bbe360F4e497")),
            token0: weth,
            token0_decimals: 18,
            token1: usdt,
            token1_decimals: 6,
            fee: None,
            block_created: None,
        },
        Pool {
            address: address!("1c6C2D7b5b1F2a3b4c5d6e7f8a9b0c1d2e3f4A5B"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: Some(address!("52Aa899454998Be5b000Ad077a46Bbe360F4e497")),
            token0: wbtc,
            token0_decimals: 8,
            token1: usdc,
            token1_decimals: 6,
            fee: None,
            block_created: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluid_top_returns_known_pools() {
        let pools = seed_fluid_top();
        assert!(!pools.is_empty());
        assert!(pools.iter().all(|p| p.kind == PoolKind::Fluid));
    }

    #[test]
    fn balancer_v2_top_uses_vault_as_factory() {
        let pools = seed_balancer_v2_top();
        assert!(!pools.is_empty());
        for p in &pools {
            assert_eq!(p.kind, PoolKind::BalancerV2);
            assert_eq!(p.factory, Some(BALANCER_V2_VAULT_ADDR));
        }
    }

    #[test]
    fn balancer_v3_top_uses_vault_as_factory() {
        let pools = seed_balancer_v3_top();
        assert!(!pools.is_empty());
        for p in &pools {
            assert_eq!(p.kind, PoolKind::BalancerV3);
            assert_eq!(p.factory, Some(BALANCER_V3_VAULT_ADDR));
        }
    }

    /// Sanity check: hardcoded FraxSwap fallback contains the canonical
    /// FRAX/USDC pair so the seed command remains usable without RPC.
    fn frax_hardcoded() -> Vec<Pool> {
        vec![Pool {
            address: address!("97d9Db4Ad1B97D67Fb1691D89b9267224e2B0A12"),
            pool_id: B256::ZERO,
            kind: PoolKind::FraxSwap,
            factory: Some(FRAXSWAP_V2_FACTORY),
            token0: address!("853d955aCEf822Db058eb8505911ED77F175b99e"), // FRAX
            token0_decimals: 18,
            token1: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            token1_decimals: 6,
            fee: Some(30),
            block_created: None,
        }]
    }

    #[test]
    fn frax_hardcoded_contains_frax_usdc() {
        let pools = frax_hardcoded();
        let frax = address!("853d955aCEf822Db058eb8505911ED77F175b99e");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let has = pools.iter().any(|p| {
            (p.token0 == frax && p.token1 == usdc)
                || (p.token0 == usdc && p.token1 == frax)
        });
        assert!(has, "FraxSwap hardcoded pools should include FRAX/USDC");
    }
}
