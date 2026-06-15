//! Ethereum RPC layer — single-call per block via `eth_dxgTraceBlockByNumber`.

use std::collections::HashSet;
use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::models::{Transfer, TxFlow};

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RpcClient {
    client: reqwest::Client,
    url: String,
}

impl RpcClient {
    pub fn new(rpc_url: &str) -> Result<Self> {
        let _: reqwest::Url = rpc_url.parse()
            .with_context(|| format!("invalid RPC URL: {}", rpc_url))?;
        Ok(Self { client: reqwest::Client::new(), url: rpc_url.to_string() })
    }

    async fn request<R: for<'de> Deserialize<'de>>(
        &self, method: &str, params: impl Serialize,
    ) -> Result<R> {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [params] });
        let resp_bytes = self.client.post(&self.url).json(&body).send().await?.bytes().await
            .with_context(|| format!("fetch {} response", method))?;
        let mut deserializer = serde_json::Deserializer::from_slice(&resp_bytes);
        deserializer.disable_recursion_limit();
        let resp: JsonRpcResponse = JsonRpcResponse::deserialize(&mut deserializer)
            .with_context(|| format!("parse {} response", method))?;
        if let Some(error) = resp.error {
            anyhow::bail!("RPC error {}: {}", error.code, error.message);
        }
        serde_json::from_value(resp.result)
            .with_context(|| format!("deserialize {} result", method))
    }

    pub async fn block_number(&self) -> anyhow::Result<u64> {
        let result: String = self.request("eth_blockNumber", "").await?;
        Ok(u64::from_str_radix(result.trim_start_matches("0x"), 16)?)
    }
}


// ---------------------------------------------------------------------------
// Block data fetch (1 RPC call)
// ---------------------------------------------------------------------------

/// Full block data: flows, pool set, and block metadata.
pub struct BlockData {
    pub flows: Vec<TxFlow>,
    pub coinbase: Address,
    /// Per-tx raw logs for classification.
    pub raw_logs: Vec<Vec<DxgLog>>,
}

/// Fetch block data using the DXG trace API.
pub async fn fetch_block(client: &RpcClient, block_number: u64) -> Result<BlockData> {
    let tag = format!("0x{:x}", block_number);
    let trace: DxgBlockTraceResponse = client
        .request("eth_dxgTraceBlockByNumber", tag)
        .await
        .with_context(|| format!("dxg trace block {}", block_number))?;

    let base_fee: u128 = if trace.block.base_fee.starts_with("0x") {
        u128::from_str_radix(trace.block.base_fee.trim_start_matches("0x"), 16)
            .unwrap_or(0)
    } else {
        trace.block.base_fee.parse::<u128>().unwrap_or(0)
    };

    let mut flows = Vec::with_capacity(trace.txs.len());
    let mut raw_logs = Vec::with_capacity(trace.txs.len());

    for (idx, tx) in trace.txs.iter().enumerate() {
        // Collect ALL logs: receipt-level + all internal call frames
        let mut all_logs = tx.receipt.logs.clone();
        collect_frame_logs(&tx.trace, &mut all_logs);

        raw_logs.push(tx.receipt.logs.clone());  // receipt-level only for classification

        // Extract unified transfers from all logs (deduplicated)
        let mut seen_transfers = HashSet::new();
        let transfers: Vec<Transfer> = all_logs.iter()
            .filter(|log| {
                if log.topics.len() < 3 || log.topics[0] != TRANSFER_TOPIC { return false; }
                let key = (log.address, log.topics[1], log.topics[2], log.data.clone());
                seen_transfers.insert(key)
            })
            .filter_map(decode_log_to_transfer)
            .collect();

        let tip: u128 = if tx.effective_tip_per_gas.starts_with("0x") {
            u128::from_str_radix(tx.effective_tip_per_gas.trim_start_matches("0x"), 16)
                .unwrap_or(0)
        } else {
            tx.effective_tip_per_gas.parse::<u128>().unwrap_or(0)
        };

        flows.push(TxFlow {
            tx_hash: tx.hash,
            tx_index: idx as u64,
            from: tx.from,
            to: tx.to,
            gas_used: tx.receipt.gas_used as u128,
            effective_gas_price: base_fee,
            effective_priority_fee: tip,
            success: tx.receipt.status == 1,
            transfers,
        });
    }

    debug!("block {}: {} txs, {} transfers",
        block_number, flows.len(),
        flows.iter().map(|f| f.transfers.len()).sum::<usize>());

    Ok(BlockData {
        flows,
        coinbase: trace.block.coinbase,
        raw_logs,
    })
}

// ---------------------------------------------------------------------------
// Log → Transfer decoding
// ---------------------------------------------------------------------------

const TRANSFER_TOPIC: B256 = B256::new(hex_literal::hex!(
    "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
));

fn decode_log_to_transfer(log: &DxgLog) -> Option<Transfer> {
    if log.topics.len() < 3 { return None; }
    if log.topics[0] != TRANSFER_TOPIC { return None; }
    let token = log.address;
    let from = Address::from_slice(&log.topics[1][12..]);
    let to = Address::from_slice(&log.topics[2][12..]);
    let amount = U256::from_be_slice(&hex_to_bytes(&log.data)?);
    Some(Transfer { token, from, to, amount })
}

fn hex_to_bytes(hex_str: &str) -> Option<Vec<u8>> {
    let s = hex_str.strip_prefix("0x")?;
    let mut bytes = vec![0u8; s.len() / 2];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        bytes[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some(bytes)
}

/// Collect all logs from nested call frames.
fn collect_frame_logs(frame: &DxgCallFrame, all_logs: &mut Vec<DxgLog>) {
    all_logs.extend(frame.logs.clone());
    for child in &frame.calls {
        collect_frame_logs(child, all_logs);
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: serde_json::Value,
    #[serde(default)]
    error: Option<JsonRpcError>,
}
#[derive(Deserialize)]
struct JsonRpcError { code: i64, message: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DxgBlockTraceResponse {
    block: DxgBlockInfo,
    txs: Vec<DxgBlockTx>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DxgBlockInfo {
    coinbase: Address,
    #[serde(default)]
    base_fee: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DxgBlockTx {
    hash: B256,
    from: Address,
    to: Option<Address>,
    #[serde(default)]
    #[serde(rename = "effectiveTipPerGas")]
    effective_tip_per_gas: String,
    trace: DxgCallFrame,
    receipt: DxgReceipt,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DxgReceipt {
    gas_used: u64,
    logs: Vec<DxgLog>,
    status: u8,
}

#[derive(Deserialize, Clone)]
pub(crate) struct DxgLog {
    pub(crate) address: Address,
    pub(crate) topics: Vec<B256>,
    pub(crate) data: String,
}

/// A single frame in a nested call trace.
#[derive(Deserialize, Clone)]
struct DxgCallFrame {
    #[serde(default)]
    logs: Vec<DxgLog>,
    #[serde(default)]
    calls: Vec<DxgCallFrame>,
}
