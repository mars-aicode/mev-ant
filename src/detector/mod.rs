//! MEV detector: find sandwich bundles in decoded block traces.
//!
//! The detector exposes a single public function: `detect_sandwiches`.
//! Everything else — classification, executor discovery, pairing,
//! post-processing, and funder resolution — is internal to this module.

use std::collections::HashSet;
use alloy::primitives::Address;
use tracing::debug;

use crate::models::{PoolId, SandwichBundle, TxFlow};

pub(crate) mod engine;
pub(crate) mod funder;

#[cfg(test)]
mod tests;

/// Block-level context shared across all detection functions.
pub(crate) struct Ctx<'a> {
    pub(crate) block_number: u64,
    pub(crate) tx_flows: &'a [TxFlow],
    /// Addresses that are pools or routers (used for transfer-level detection).
    pub(crate) pool_set: &'a HashSet<Address>,
    /// Per-tx pool identities, indexed by `tx_index`.
    pub(crate) tx_pools: Vec<HashSet<PoolId>>,
    pub(crate) lending_set: &'a HashSet<Address>,
    pub(crate) unknown: &'a HashSet<Address>,
    pub(crate) coinbase: Address,
    pub(crate) supported_tokens: &'a [Address],
}

/// Detect sandwich bundles in a single block.
///
/// This is the only public entry point to the detector. It runs three
/// stages internally:
///   1. Classify addresses and filter txs with ≥2 Transfer events.
///   2. Discover executor trade signatures and pair them into bundles.
///   3. Post-process: deduplicate, validate, filter, and resolve overlaps.
///
/// The classifier is injected so tests can plug in fixtures or stubs.
pub fn detect_sandwiches<C: crate::classifier::Classifier>(
    classifier: &C,
    block_number: u64,
    tx_flows: &[TxFlow],
    raw_logs: &[Vec<crate::rpc::DxgLog>],
    coinbase: Address,
    blacklist: &[Address],
    supported_tokens: &[Address],
) -> Vec<SandwichBundle> {
    let classified = classifier.classify(tx_flows, raw_logs);

    // Exclude reverted transactions: their trace-captured transfers
    // never materialised on chain and would produce phantom profit.
    let flows: Vec<&TxFlow> = tx_flows.iter()
        .filter(|f| f.success && f.transfers.len() >= 2)
        .collect();
    if flows.len() < 2 { return vec![]; }
    debug!("block {}: {} txs after filter ({} total)", block_number, flows.len(), tx_flows.len());

    let pool_set = &classified.pool_or_router;
    let lending_set = &classified.lending_set;
    let unknown = &classified.unknown;

    // Augment swap-event-derived pool identities with transfer-derived pools.
    // This keeps detection robust when swap logs live in internal call frames
    // or when a pool is recognised only by fund-flow heuristics.
    let mut tx_pools = classified.tx_pools.clone();
    for (tx_idx, flow) in tx_flows.iter().enumerate() {
        if tx_idx >= tx_pools.len() {
            tx_pools.push(HashSet::new());
        }
        for t in &flow.transfers {
            if classified.pool_or_router.contains(&t.from) {
                tx_pools[tx_idx].insert(PoolId::Contract(t.from));
            }
            if classified.pool_or_router.contains(&t.to) {
                tx_pools[tx_idx].insert(PoolId::Contract(t.to));
            }
        }
    }

    let ctx = Ctx {
        block_number,
        tx_flows,
        pool_set,
        tx_pools,
        lending_set,
        unknown,
        coinbase,
        supported_tokens,
    };

    let trades = engine::discover_executor_trades(&ctx, &flows, &classified);
    let bundles = engine::pair_trades(&ctx, trades);

    debug!("block {}: {} bundles after pairing", block_number, bundles.len());
    let bundles = engine::post_process(&ctx, bundles, blacklist);

    debug!("block {} final sandwiches: {}", block_number, bundles.len());
    bundles
}
