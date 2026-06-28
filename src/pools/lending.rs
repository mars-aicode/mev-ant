//! Lending market tracking for liquidation and collateral-swap MEV strategies.
//!
//! This module is intentionally separate from the DEX registry. Lending
//! markets are characterised by per-asset supply/borrow rates and an
//! available-liquidity figure; they are *not* swap paths and must not appear
//! in the V1 routing graph.
//!
//! V1 covers Aave V3 only. The reserve list is enumerated by calling
//! `getReservesList()` on the Pool, and per-reserve rates are fetched via
//! the single-word helpers `getReserveCurrent{Liquidity,VariableBorrow,StableBorrow}Rate`.
//! Touched-market detection uses the `ReserveDataUpdated` event topic.

use alloy::primitives::{keccak256, Address, B256};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::rpc::RpcClient;

/// Aave V3 Pool proxy on Ethereum mainnet.
pub const AAVE_V3_POOL: Address =
    alloy::primitives::address!("87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2");

/// `getReservesList() external view returns (address[])`
#[allow(dead_code)]
const GET_RESERVES_LIST_SELECTOR: [u8; 4] = [0xd1, 0x94, 0x6d, 0xbc];

/// `getReserveData(address) external view returns (DataTypes.ReserveData memory)`
///
/// Returns the full reserve struct. The single-word rate helpers
/// (`getReserveCurrent*Rate`) were not present in Aave V3 v3.0 and are
/// unreliable on older proxies; using the full struct is portable.
const GET_RESERVE_DATA_SELECTOR: [u8; 4] = [0x35, 0xea, 0x6a, 0x75];

/// Field offsets inside the ABI-encoded `ReserveData` struct, in 32-byte words.
/// Field order matches the Aave V3 source: configuration, liquidityIndex,
/// currentLiquidityRate, variableBorrowIndex, currentVariableBorrowRate,
/// currentStableBorrowRate, lastUpdateTimestamp+id, aTokenAddress, ...
#[allow(dead_code)]
const FIELD_CONFIG: usize = 0;
#[allow(dead_code)]
const FIELD_LIQUIDITY_INDEX: usize = 1;
const FIELD_LIQUIDITY_RATE: usize = 2;
#[allow(dead_code)]
const FIELD_VARIABLE_BORROW_INDEX: usize = 3;
const FIELD_VARIABLE_BORROW_RATE: usize = 4;
const FIELD_STABLE_BORROW_RATE: usize = 5;
#[allow(dead_code)]
const FIELD_LAST_UPDATE: usize = 6;
#[allow(dead_code)]
const FIELD_ATOKEN_ADDRESS: usize = 7;
#[allow(dead_code)]
const FIELD_STABLE_DEBT_ADDRESS: usize = 8;
#[allow(dead_code)]
const FIELD_VARIABLE_DEBT_ADDRESS: usize = 9;
#[allow(dead_code)]
const FIELD_INTEREST_STRATEGY: usize = 10;
#[allow(dead_code)]
const FIELD_ACCRUED_TREASURY: usize = 11;
/// Number of words in the static part of the struct (before any trailing
/// padding the implementation may append).
#[allow(dead_code)]
const RESERVE_DATA_WORDS_V3_2: usize = 15;

/// `ReserveDataUpdated(address,uint256,uint256,uint256,uint256,uint256)`
pub fn reserve_data_updated_topic0() -> B256 {
    keccak256("ReserveDataUpdated(address,uint256,uint256,uint256,uint256,uint256)")
}

/// Protocol identifier for a lending market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LendingProtocol {
    AaveV3,
}

impl LendingProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            LendingProtocol::AaveV3 => "aave_v3",
        }
    }
}

/// Snapshot of a single lending market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingMarket {
    /// Address identifying the market. For Aave V3 this is the underlying
    /// asset address (each asset is one market).
    pub market_address: Address,
    pub protocol: LendingProtocol,
    /// The underlying token this market lends/borrows.
    pub underlying_asset: Address,
    /// Unbacked underlying-token balance available to borrow.
    /// `None` when the source (aToken/balanceOf) was unavailable.
    pub available_liquidity: Option<alloy::primitives::U256>,
    /// Per-second supply rate in ray (1e27). `None` if the call failed.
    pub supply_rate_ray: Option<alloy::primitives::U256>,
    /// Per-second variable borrow rate in ray (1e27).
    pub variable_borrow_rate_ray: Option<alloy::primitives::U256>,
    /// Per-second stable borrow rate in ray (1e27). May be zero or `None`
    /// for assets that don't support stable-rate borrowing.
    pub stable_borrow_rate_ray: Option<alloy::primitives::U256>,
}

impl LendingMarket {
    pub fn protocol_str(&self) -> &'static str {
        self.protocol.as_str()
    }
}

/// Enumerate the current Aave V3 reserve list at the given block.
#[allow(dead_code)]
pub async fn aave_v3_reserves_list(
    client: &RpcClient,
    pool: Address,
    block: u64,
) -> Result<Vec<Address>> {
    let block_tag = format!("0x{:x}", block);
    let data = alloy::primitives::Bytes::from_static(&GET_RESERVES_LIST_SELECTOR);
    let hex = client
        .call_at(pool, data, &block_tag)
        .await
        .context("aave_v3 getReservesList")?;
    decode_address_array(&hex)
}

/// Fetch the rate triple for a single Aave V3 reserve at the given block.
///
/// Returns a `LendingMarket` with all rate fields populated and the
/// `available_liquidity` field left `None` (V1 omits the aToken balance
/// fetch; it can be added by following the aToken address from
/// `getReserveData`).
pub async fn aave_v3_reserve_state(
    client: &RpcClient,
    pool: Address,
    asset: Address,
    block: u64,
) -> Result<LendingMarket> {
    let block_tag = format!("0x{:x}", block);

    let data = call_get_reserve_data(client, pool, asset, &block_tag).await?;
    let bytes = hex::decode(data.trim_start_matches("0x"))
        .context("decode aave_v3 getReserveData")?;

    let supply = left_aligned_u128(&bytes, FIELD_LIQUIDITY_RATE);
    let var = left_aligned_u128(&bytes, FIELD_VARIABLE_BORROW_RATE);
    let stable = left_aligned_u128(&bytes, FIELD_STABLE_BORROW_RATE);

    Ok(LendingMarket {
        market_address: asset,
        protocol: LendingProtocol::AaveV3,
        underlying_asset: asset,
        available_liquidity: None,
        supply_rate_ray: supply,
        variable_borrow_rate_ray: var,
        stable_borrow_rate_ray: stable,
    })
}

async fn call_get_reserve_data(
    client: &RpcClient,
    pool: Address,
    asset: Address,
    block_tag: &str,
) -> Result<String> {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&GET_RESERVE_DATA_SELECTOR);
    data.resize(4 + 32, 0);
    data[4 + 12..4 + 32].copy_from_slice(asset.as_slice());
    client
        .call_at(pool, alloy::primitives::Bytes::from(data), block_tag)
        .await
        .context("aave_v3 getReserveData")
}

/// Extract a left-aligned uint128 from `bytes` at `word_index` (in 32-byte words).
///
/// Aave stores `uint128` values left-aligned in a 32-byte slot, so the
/// integer value is `word >> 128`.
fn left_aligned_u128(bytes: &[u8], word_index: usize) -> Option<alloy::primitives::U256> {
    let start = word_index * 32;
    let end = start + 32;
    if bytes.len() < end {
        return None;
    }
    let mut word = [0u8; 32];
    word.copy_from_slice(&bytes[start..end]);
    Some(alloy_2_u256(&word) >> 128)
}

/// Fetch state for every reserve currently in the Aave V3 reserve list.
#[allow(dead_code)]
pub async fn aave_v3_fetch_all(
    client: &RpcClient,
    pool: Address,
    block: u64,
) -> Result<Vec<LendingMarket>> {
    let reserves = aave_v3_reserves_list(client, pool, block).await?;
    let mut out = Vec::with_capacity(reserves.len());
    for asset in reserves {
        match aave_v3_reserve_state(client, pool, asset, block).await {
            Ok(m) => out.push(m),
            Err(e) => tracing::warn!("aave_v3 state for {}: {}", asset, e),
        }
    }
    Ok(out)
}

/// Detect markets touched in a block via the `ReserveDataUpdated` event,
/// then fetch the latest state for each touched underlying asset.
pub async fn update_touched_aave_v3(
    client: &RpcClient,
    pool: Address,
    block: u64,
) -> Result<Vec<LendingMarket>> {
    let topic0 = reserve_data_updated_topic0();
    let filter = serde_json::json!({
        "fromBlock": format!("0x{:x}", block),
        "toBlock": format!("0x{:x}", block),
        "address": format!("{:?}", pool),
        "topics": [format!("{:?}", topic0)]
    });
    let logs = client
        .get_logs(filter)
        .await
        .context("aave_v3 ReserveDataUpdated logs")?;

    let mut touched: std::collections::HashSet<Address> = std::collections::HashSet::new();
    for log in logs {
        if log.topics.len() < 2 {
            continue;
        }
        touched.insert(Address::from_slice(&log.topics[1][12..32]));
    }
    if touched.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(touched.len());
    for asset in touched {
        match aave_v3_reserve_state(client, pool, asset, block).await {
            Ok(m) => out.push(m),
            Err(e) => tracing::warn!("aave_v3 touched state for {}: {}", asset, e),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ABI-decoding helpers
// ---------------------------------------------------------------------------

/// Decode `getReservesList()` output: ABI-encoded dynamic `address[]` with a
/// 32-byte length prefix, then 32-byte aligned address words.
#[allow(dead_code)]
fn decode_address_array(hex_result: &str) -> Result<Vec<Address>> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x"))
        .context("decode address array")?;
    if bytes.len() < 32 {
        anyhow::bail!("address array result too short: {} bytes", bytes.len());
    }
    let len = alloy_2_u256(&bytes[0..32]).to::<usize>();
    let need = 32 + len * 32;
    if bytes.len() < need {
        anyhow::bail!("address array truncated: want {} bytes, got {}", need, bytes.len());
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let start = 32 + i * 32 + 12; // skip 12-byte left padding
        let end = start + 20;
        out.push(Address::from_slice(&bytes[start..end]));
    }
    Ok(out)
}

fn alloy_2_u256(bytes: &[u8]) -> alloy::primitives::U256 {
    debug_assert!(bytes.len() == 32);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    alloy::primitives::U256::from_be_bytes(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn topic0_matches_canonical() {
        // keccak256("ReserveDataUpdated(address,uint256,uint256,uint256,uint256,uint256)")
        let expected = B256::new(hex_literal::hex!(
            "804c9b842b2748a22bb64b345453a3de7ca54a6ca45ce00d415894979e22897a"
        ));
        assert_eq!(reserve_data_updated_topic0(), expected);
    }

    #[test]
    fn left_aligned_u128_decodes() {
        // Left-aligned uint128: 100 lives in the high 16 bytes (bytes 14..16).
        // 100 = 0x64. Set bytes 14 and 15 to 0x00, 0x64 — but since 100 < 256,
        // byte 14 = 0x00, byte 15 = 0x64. So bytes[14] = 0, bytes[15] = 100.
        let mut bytes = [0u8; 32];
        bytes[15] = 100;
        let v = super::left_aligned_u128(&bytes, 0).expect("decode");
        assert_eq!(v, alloy::primitives::U256::from(100u64));
    }

    #[test]
    fn decode_address_array_parses() {
        // Encoded: 2 addresses (0x...aa1, 0x...bb2)
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u256_be(2u64));
        bytes.extend_from_slice(&[0u8; 12]);
        bytes.extend_from_slice(&hex_literal::hex!("0000000000000000000000000000000000000aa1"));
        bytes.extend_from_slice(&[0u8; 12]);
        bytes.extend_from_slice(&hex_literal::hex!("0000000000000000000000000000000000000bb2"));
        let hex_str = format!("0x{}", hex::encode(&bytes));
        let out = decode_address_array(&hex_str).expect("decode");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], address!("0000000000000000000000000000000000000aa1"));
        assert_eq!(out[1], address!("0000000000000000000000000000000000000bb2"));
    }

    fn u256_be(v: u64) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[24..32].copy_from_slice(&v.to_be_bytes());
        out
    }
}
