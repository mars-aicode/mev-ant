//! Round 2a: Discover executor trade signatures from pool-involved transfers.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, U256};
use tracing::debug;

use crate::detector::sandwich::Ctx;
use crate::models::TxFlow;

/// Trade signature for an executor in one tx.
#[derive(Debug, Clone)]
pub(crate) struct ExecutorTrade {
    pub(crate) tx_index: u64,
    pub(crate) executor: Address,
    /// Net pool-touching token deltas (positive = received from pool)
    pub(crate) deltas: HashMap<Address, i128>,
    /// Pool addresses this executor touched
    #[allow(dead_code)]
    pub(crate) pools: HashSet<Address>,
    /// tx.from and tx.to
    pub(crate) from: Address,
    pub(crate) to: Option<Address>,
}

/// Discover executor trade signatures from pool-involved transfers.
pub(crate) fn discover_executor_trades(
    ctx: &Ctx,
    flows: &[&TxFlow],
    classified: &crate::classifier::Classified,
) -> Vec<ExecutorTrade> {
    let mut trades: Vec<ExecutorTrade> = Vec::new();

    for flow in flows {
        let mut exec_deltas: HashMap<Address, HashMap<Address, i128>> = HashMap::new();
        let mut exec_pools: HashMap<Address, HashSet<Address>> = HashMap::new();

        for t in &flow.transfers {
            let to_pool = ctx.pool_set.contains(&t.to);
            let from_pool = ctx.pool_set.contains(&t.from);
            if !to_pool && !from_pool { continue; }

            let amt = i128_sat(t.amount);
            if classified.unknown.contains(&t.from) {
                *exec_deltas.entry(t.from).or_default().entry(t.token).or_default() -= amt;
                if to_pool { exec_pools.entry(t.from).or_default().insert(t.to); }
                if from_pool { exec_pools.entry(t.from).or_default().insert(t.from); }
            }
            if classified.unknown.contains(&t.to) {
                *exec_deltas.entry(t.to).or_default().entry(t.token).or_default() += amt;
                if to_pool { exec_pools.entry(t.to).or_default().insert(t.to); }
                if from_pool { exec_pools.entry(t.to).or_default().insert(t.from); }
            }
        }

        // Add tx.from / tx.to as stubs with empty deltas when not already tracked
        let known_exec: HashSet<Address> = exec_deltas.keys().copied().collect();
        if let Some(to) = flow.to {
            if classified.unknown.contains(&to) && !known_exec.contains(&to) {
                trades.push(ExecutorTrade {
                    tx_index: flow.tx_index, executor: to,
                    deltas: HashMap::new(), pools: HashSet::new(),
                    from: flow.from, to: flow.to,
                });
            }
        }
        if classified.unknown.contains(&flow.from) && !known_exec.contains(&flow.from) {
            trades.push(ExecutorTrade {
                tx_index: flow.tx_index, executor: flow.from,
                deltas: HashMap::new(), pools: HashSet::new(),
                from: flow.from, to: flow.to,
            });
        }

        for (executor, deltas) in exec_deltas {
            let pools = exec_pools.remove(&executor).unwrap_or_default();
            trades.push(ExecutorTrade {
                tx_index: flow.tx_index,
                executor,
                deltas,
                pools,
                from: flow.from,
                to: flow.to,
            });
        }
    }

    debug!("block {}: {} executor trades", ctx.block_number, trades.len());
    tracing::trace!("executor trades: {:?}", trades.iter().map(|t| (t.tx_index, t.executor, t.deltas.len())).collect::<Vec<_>>());
    trades
}

/// WETH mainnet address — shared across stages.
pub(crate) const WETH: Address = Address::new(hex_literal::hex!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));

/// Check if token is in supported list (shared across stages).
pub(crate) fn is_sup(token: Address, supported: &[Address]) -> bool { supported.contains(&token) }

/// Saturating U256 → i128 conversion (shared across stages).
pub(crate) fn i128_sat(v: U256) -> i128 {
    let b = v.to_be_bytes::<32>();
    let high = i128::from_be_bytes(b[..16].try_into().unwrap_or([0;16]));
    if high != 0 { i128::MAX } else {
        (u128::from_be_bytes(b[16..].try_into().unwrap_or([0;16]))).min(i128::MAX as u128) as i128
    }
}
