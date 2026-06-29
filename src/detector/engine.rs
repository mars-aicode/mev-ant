//! Detection engine — classify, discover, pair, and post-process.
//!
//! This module is the private implementation behind `detect_sandwiches`.
//! It is intentionally one deep module so that the public surface stays
//! a single function and internal data structures (e.g. `ExecutorTrade`)
//! do not leak out of the detector.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, B256, I256, U256};
use tracing::debug;

use crate::detector::funder::{FunderResolver, WETH};
use crate::detector::Ctx;
use crate::models::{PoolId, SandwichBundle, TokenDelta, Transfer, TxFlow, ETH_TRANSFER_ADDR};

/// Trade signature for an executor in one tx.
#[derive(Debug, Clone)]
pub(crate) struct ExecutorTrade {
    pub(crate) tx_index: u64,
    pub(crate) executor: Address,
    /// Net pool-touching token deltas (positive = received from pool)
    pub(crate) deltas: HashMap<Address, i128>,
    /// Pool identities this executor touched in this tx.
    pub(crate) pools: HashSet<PoolId>,
    /// tx.from and tx.to
    pub(crate) from: Address,
    pub(crate) to: Option<Address>,
}

// ============================================================================
// Stage 1: discover executor trades
// ============================================================================

pub(crate) fn discover_executor_trades(
    ctx: &Ctx,
    flows: &[&TxFlow],
) -> Vec<ExecutorTrade> {
    let mut trades: Vec<ExecutorTrade> = Vec::new();

    for flow in flows {
        let mut exec_deltas: HashMap<Address, HashMap<Address, i128>> = HashMap::new();

        for t in &flow.transfers {
            let to_pool = ctx.pool_set.contains(&t.to);
            let from_pool = ctx.pool_set.contains(&t.from);
            if !to_pool && !from_pool { continue; }

            let amt = i128_sat(t.amount);
            if ctx.unknown.contains(&t.from) {
                *exec_deltas.entry(t.from).or_default().entry(t.token).or_default() -= amt;
            }
            if ctx.unknown.contains(&t.to) {
                *exec_deltas.entry(t.to).or_default().entry(t.token).or_default() += amt;
            }
        }

        // Add tx.from / tx.to as stubs with empty deltas when not already tracked
        let known_exec: HashSet<Address> = exec_deltas.keys().copied().collect();
        let tx_pool_set = ctx.tx_pools.get(flow.tx_index as usize)
            .cloned()
            .unwrap_or_default();
        if let Some(to) = flow.to {
            if ctx.unknown.contains(&to) && !known_exec.contains(&to) {
                trades.push(ExecutorTrade {
                    tx_index: flow.tx_index, executor: to,
                    deltas: HashMap::new(), pools: tx_pool_set.clone(),
                    from: flow.from, to: flow.to,
                });
            }
        }
        if ctx.unknown.contains(&flow.from) && !known_exec.contains(&flow.from) {
            trades.push(ExecutorTrade {
                tx_index: flow.tx_index, executor: flow.from,
                deltas: HashMap::new(), pools: tx_pool_set,
                from: flow.from, to: flow.to,
            });
        }

        for (executor, deltas) in exec_deltas {
            let pools = ctx.tx_pools.get(flow.tx_index as usize)
                .cloned()
                .unwrap_or_default();
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

// ============================================================================
// Stage 2: pair trades into candidate bundles
// ============================================================================

pub(crate) fn pair_trades(ctx: &Ctx, trades: Vec<ExecutorTrade>) -> Vec<SandwichBundle> {
    let mut trades_by_exec: HashMap<Address, Vec<&ExecutorTrade>> = HashMap::new();
    for t in &trades { trades_by_exec.entry(t.executor).or_default().push(t); }

    let mut bundles: Vec<SandwichBundle> = Vec::new();

    for (executor, exec_trades) in &trades_by_exec {
        for i in 0..exec_trades.len() {
            let front = &exec_trades[i];
            for back in &exec_trades[(i+1)..] {
                if front.tx_index >= back.tx_index { continue; }

                if !is_consecutive(ctx, front.tx_index, back.tx_index, front.from, front.to.unwrap_or(Address::ZERO)) { continue; }
                if front.pools.is_disjoint(&back.pools) { continue; }

                if let Some(b) = try_build_bundle(
                    ctx, front.tx_index, back.tx_index,
                    front.from, front.to.unwrap_or(Address::ZERO),
                    &front.deltas, &back.deltas, *executor, None,
                    back.from, front.to,
                ) {
                    bundles.push(b);
                }
            }
        }
    }

    bundles
}

#[allow(clippy::too_many_arguments)]
fn try_build_bundle(
    ctx: &Ctx,
    front_tx: u64, back_tx: u64,
    initiator: Address, target: Address,
    front_deltas: &HashMap<Address, i128>,
    back_deltas: &HashMap<Address, i128>,
    executor: Address,
    funder_hint: Option<Address>,
    back_initiator: Address,
    front_target: Option<Address>,
) -> Option<SandwichBundle> {
    if !is_reversal(front_deltas, back_deltas) { return None; }
    let mut pm: HashMap<Address, i128> = HashMap::new();
    for (t, d) in front_deltas { *pm.entry(*t).or_default() += d; }
    for (t, d) in back_deltas { *pm.entry(*t).or_default() += d; }
    let profit: Vec<TokenDelta> = pm.into_iter()
        .filter(|(t, n)| *n > 0 && is_sup(*t, ctx.supported_tokens))
        .map(|(token, net)| TokenDelta { token, amount: I256::from_raw(U256::from(net as u128)) })
        .collect();
    if profit.is_empty() {
        if ctx.block_number == 25305868 { eprintln!("  -> profit empty"); }
        return None;
    }

    let ff = ctx.tx_flows.iter().find(|f| f.tx_index == front_tx)?;
    let bf = ctx.tx_flows.iter().find(|f| f.tx_index == back_tx)?;
    let funder = funder_hint.or_else(|| FunderResolver::new(ctx, ff, executor).resolve()).unwrap_or(executor);

    // Flashloan guard: same-token round-trip only counts when counterparty is non-pool.
    if funder == executor {
        let senders: HashSet<(Address, Address)> = ff.transfers.iter()
            .filter(|t| t.to == executor
                && !ctx.pool_set.contains(&t.from)
                && t.from != Address::ZERO)
            .map(|t| (t.from, t.token))
            .collect();
        let repays_same = ff.transfers.iter().any(|t|
            t.from == executor
            && !ctx.pool_set.contains(&t.to)
            && t.to != Address::ZERO
            && senders.contains(&(t.to, t.token)));
        if repays_same { return None; }
    }

    let attacker = funder;
    let victims: Vec<u64> = (front_tx + 1..back_tx)
        .filter(|&v| {
            let Some(vf) = ctx.tx_flows.iter().find(|f| f.tx_index == v) else { return false; };
            if vf.from == initiator || vf.from == attacker || vf.from == executor { return false; }
            if vf.to == front_target && front_target.is_some() { return false; }
            let mut va: HashMap<Address, HashMap<Address, i128>> = HashMap::new();
            for t in &vf.transfers {
                if !ctx.pool_set.contains(&t.from) && !ctx.pool_set.contains(&t.to) { continue; }
                let amt = i128_sat(t.amount);
                *va.entry(t.from).or_default().entry(t.token).or_default() -= amt;
                *va.entry(t.to).or_default().entry(t.token).or_default() += amt;
            }
            if va.is_empty() { return true; }
            let fp = ctx.tx_pools.get(front_tx as usize).cloned().unwrap_or_default();
            let vp = ctx.tx_pools.get(v as usize).cloned().unwrap_or_default();
            if fp.is_disjoint(&vp) { return false; }

            let net_match = va.values().any(|ad| {
                ad.iter().any(|(tok, d)| {
                    d.signum() != 0 && front_deltas.get(tok).is_some_and(|fd| fd.signum() == d.signum())
                })
            });
            if net_match { return true; }

            let front_sold: HashSet<Address> = ff.transfers.iter()
                .filter(|t| t.from == executor && ctx.pool_set.contains(&t.to))
                .map(|t| t.token)
                .collect();
            let front_bought: HashSet<Address> = ff.transfers.iter()
                .filter(|t| t.to == executor && ctx.pool_set.contains(&t.from))
                .map(|t| t.token)
                .collect();
            va.values().any(|ad| {
                ad.iter().any(|(tok, d)| {
                    (*d > 0 && front_sold.contains(tok)) || (*d < 0 && front_bought.contains(tok))
                })
            })
        })
        .collect();
    if victims.is_empty() { return None; }

    let victim_tx_hashes: Vec<B256> = victims.iter()
        .filter_map(|&v| ctx.tx_flows.iter().find(|f| f.tx_index == v).map(|f| f.tx_hash))
        .collect();
    let (gas, bribe, expense) = compute_costs(ctx, front_tx, back_tx, attacker, executor, initiator);

    Some(SandwichBundle {
        block_number: ctx.block_number, front_tx_index: front_tx, back_tx_index: back_tx,
        victim_tx_indices: victims,
        victim_tx_hashes,
        attacked_pool: PoolId::Contract(Address::ZERO),
        auxiliary_pools: vec![], attacker,
        frontrun_transfers: ff.transfers.clone(), backrun_transfers: bf.transfers.clone(),
        victim_transfers: vec![], profit,
        gas_cost_wei: gas, coinbase_bribe: bribe, expense_wei: expense,
        funder, executor, initiator, back_initiator,
        target,
        coinbase: ctx.coinbase,
        front_tx_hash: ff.tx_hash,
        back_tx_hash: bf.tx_hash,
    })
}

// ============================================================================
// Stage 3: post-process
// ============================================================================

pub(crate) fn post_process(
    ctx: &Ctx,
    bundles: Vec<SandwichBundle>,
    blacklist: &[Address],
) -> Vec<SandwichBundle> {
    let bundles = dedup_bundles(bundles);
    let mut bundles = validate_bundles(ctx, bundles);
    filter_bundles(ctx, &mut bundles, blacklist);
    resolve_overlaps(bundles, ctx.block_number)
}

fn dedup_bundles(bundles: Vec<SandwichBundle>) -> Vec<SandwichBundle> {
    let mut dedup: HashMap<(u64, u64), SandwichBundle> = HashMap::new();
    for b in bundles {
        let key = (b.front_tx_index, b.back_tx_index);
        if let Some(existing) = dedup.get(&key) {
            if profit_sum(&b) > profit_sum(existing) { dedup.insert(key, b); }
        } else { dedup.insert(key, b); }
    }
    dedup.into_values().collect()
}

fn profit_sum(b: &SandwichBundle) -> u128 {
    b.profit.iter().map(|p| p.amount.into_sign_and_abs().1.to::<u128>()).sum()
}

fn validate_bundles(
    ctx: &Ctx,
    bundles: Vec<SandwichBundle>,
) -> Vec<SandwichBundle> {
    bundles.into_iter().filter_map(|mut b| {
        let ff = ctx.tx_flows.iter().find(|f| f.tx_index == b.front_tx_index)?;
        let bf = ctx.tx_flows.iter().find(|f| f.tx_index == b.back_tx_index)?;

        let front_dp: Vec<&Transfer> = ff.transfers.iter()
            .filter(|t| ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to)).collect();
        let back_dp: Vec<&Transfer> = bf.transfers.iter()
            .filter(|t| ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to)).collect();

        let exec_f = front_dp.iter().any(|t| t.from == b.executor || t.to == b.executor);
        let exec_b = back_dp.iter().any(|t| t.from == b.executor || t.to == b.executor);
        if !exec_f && !exec_b { return None; }

        let fps = ctx.tx_pools.get(ff.tx_index as usize).cloned().unwrap_or_default();
        let bps = ctx.tx_pools.get(bf.tx_index as usize).cloned().unwrap_or_default();
        if fps.is_disjoint(&bps) { return None; }

        let same_targ = ff.to.is_some() && ff.to == bf.to;
        if ff.from != bf.from && !same_targ {
            let bfunder = FunderResolver::new(ctx, bf, b.executor).resolve();
            if let Some(bfu) = bfunder {
                if b.funder != bfu && b.funder != b.executor && bfu != b.executor
                    && !ctx.pool_set.contains(&b.funder) && !ctx.pool_set.contains(&bfu) { return None; }
            }
        }

        if b.profit.is_empty() {
            let ts: HashSet<Address> = ctx.supported_tokens.iter().copied().collect();
            let mut tn: HashMap<Address, i128> = HashMap::new();
            for t in front_dp.iter().chain(back_dp.iter()) {
                if !ts.contains(&t.token) { continue; }
                let a = i128_sat(t.amount);
                if t.from == b.executor { *tn.entry(t.token).or_default() -= a; }
                if t.to == b.executor { *tn.entry(t.token).or_default() += a; }
            }
            let recomp: Vec<TokenDelta> = tn.into_iter().filter_map(|(t, n)| {
                if n > 0 { Some(TokenDelta { token: t, amount: I256::from_raw(U256::from(n as u128)) }) }
                else { None }
            }).collect();
            if recomp.is_empty() { return None; }
            b.profit = recomp;
        }

        let rs: HashSet<Address> = [b.attacker, b.executor, b.initiator].iter()
            .filter(|a| **a != Address::ZERO).copied().collect();
        b.victim_tx_indices.retain(|&vtx| {
            let Some(vf) = ctx.tx_flows.iter().find(|f| f.tx_index == vtx) else { return false; };
            if rs.contains(&vf.from) { return false; }
            vf.transfers.iter().any(|t|
                (ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to)) || is_sup(t.token, ctx.supported_tokens))
        });
        b.victim_tx_hashes = b.victim_tx_indices.iter()
            .filter_map(|&vi| ctx.tx_flows.iter().find(|f| f.tx_index == vi).map(|f| f.tx_hash))
            .collect();

        let vpools: HashSet<PoolId> = b.victim_tx_indices.iter()
            .filter_map(|&vi| ctx.tx_pools.get(vi as usize))
            .flat_map(|s| s.iter().cloned())
            .collect();
        b.attacked_pool = fps.intersection(&bps)
            .filter(|p| vpools.contains(*p))
            .next()
            .cloned()
            .unwrap_or_else(|| {
                // Fallback: pick any shared pool between front and back.
                fps.intersection(&bps).next().cloned()
                    .unwrap_or(PoolId::Contract(Address::ZERO))
            });

        b.frontrun_transfers = ff.transfers.clone();
        b.backrun_transfers = bf.transfers.clone();
        b.victim_transfers = ctx.tx_flows.iter()
            .filter(|f| b.victim_tx_indices.contains(&f.tx_index))
            .flat_map(|f| f.transfers.clone())
            .collect();
        Some(b)
    }).collect()
}

fn filter_bundles(
    ctx: &Ctx,
    bundles: &mut Vec<SandwichBundle>,
    blacklist: &[Address],
) {
    bundles.retain(|b| {
        !blacklist.contains(&b.funder)
    });
    for b in bundles.iter_mut() {
        let role_set: HashSet<Address> = [b.funder, b.executor, b.initiator, b.target]
            .into_iter().filter(|a| *a != Address::ZERO).collect();
        b.victim_tx_indices.retain(|&vtx| {
            let Some(vf) = ctx.tx_flows.iter().find(|f| f.tx_index == vtx) else { return false; };
            if role_set.contains(&vf.from) { return false; }
            vf.transfers.iter().any(|t|
                (ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to))
                || is_sup(t.token, ctx.supported_tokens)
            )
        });
    }
    bundles.retain(|b| !b.victim_tx_indices.is_empty());
}

fn resolve_overlaps(mut bundles: Vec<SandwichBundle>, block_number: u64) -> Vec<SandwichBundle> {
    bundles.sort_by(|a, b| profit_sum(b).cmp(&profit_sum(a)));
    let mut result = Vec::new();
    for b in bundles {
        let overlaps = result.iter().any(|e: &SandwichBundle|
            b.front_tx_index <= e.back_tx_index && b.back_tx_index >= e.front_tx_index);
        if !overlaps { result.push(b); }
    }
    debug!("block {} final sandwiches: {}", block_number, result.len());
    result
}

// ============================================================================
// Helpers
// ============================================================================

#[cfg(test)]
pub(crate) fn share_pool(ctx: &Ctx, ftx: u64, btx: u64) -> bool {
    let fp = ctx.tx_pools.get(ftx as usize).cloned().unwrap_or_default();
    let bp = ctx.tx_pools.get(btx as usize).cloned().unwrap_or_default();
    !fp.is_disjoint(&bp)
}

fn is_reversal(front: &HashMap<Address, i128>, back: &HashMap<Address, i128>) -> bool {
    front.iter().any(|(t, fd)| {
        if *t == ETH_TRANSFER_ADDR || *t == WETH { return false; }
        back.get(t).is_some_and(|bd| fd.signum() * bd.signum() < 0)
    })
}

fn is_consecutive(ctx: &Ctx, front_tx: u64, back_tx: u64, initiator: Address, target: Address) -> bool {
    for tx in (front_tx + 1)..back_tx {
        let Some(f) = ctx.tx_flows.iter().find(|fx| fx.tx_index == tx) else { return false; };
        if f.transfers.is_empty() { continue; }
        let same_attacker = f.from == initiator
            || (f.to == Some(target) && target != Address::ZERO);
        let has_pool = f.transfers.iter().any(|t|
            ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to));
        let has_token = f.transfers.iter().any(|t| is_sup(t.token, ctx.supported_tokens));
        if !same_attacker && !has_pool && !has_token { return false; }
    }
    true
}

fn compute_costs(
    ctx: &Ctx,
    front_tx: u64, back_tx: u64,
    attacker: Address, executor: Address, initiator: Address,
) -> (u128, u128, u128) {
    let gas_cost: u128 = ctx.tx_flows.iter()
        .filter(|f| f.tx_index == front_tx || f.tx_index == back_tx)
        .map(|f| f.gas_used.saturating_mul(f.effective_gas_price.saturating_add(f.effective_priority_fee)))
        .sum();
    let cb_prio: u128 = ctx.tx_flows.iter()
        .filter(|f| f.tx_index == front_tx || f.tx_index == back_tx)
        .map(|f| f.gas_used.saturating_mul(f.effective_priority_fee))
        .sum();
    let roles: HashSet<Address> = [attacker, executor, initiator].iter()
        .filter(|a| **a != Address::ZERO).copied().collect();
    let direct_eth: u128 = [front_tx, back_tx].iter()
        .flat_map(|&tx| ctx.tx_flows.iter().filter(move |f| f.tx_index == tx))
        .flat_map(|f| f.transfers.iter())
        .filter(|t| t.to == ctx.coinbase && t.token == ETH_TRANSFER_ADDR)
        .filter(|t| roles.contains(&t.from))
        .map(|t| amount_u128(t.amount))
        .sum();
    (gas_cost, cb_prio.saturating_add(direct_eth), gas_cost.saturating_add(direct_eth))
}

pub(crate) fn is_sup(token: Address, supported: &[Address]) -> bool { supported.contains(&token) }

pub(crate) fn i128_sat(v: U256) -> i128 {
    let b = v.to_be_bytes::<32>();
    let high = i128::from_be_bytes(b[..16].try_into().unwrap_or([0;16]));
    if high != 0 { i128::MAX } else {
        (u128::from_be_bytes(b[16..].try_into().unwrap_or([0;16]))).min(i128::MAX as u128) as i128
    }
}

pub(crate) fn amount_u128(v: U256) -> u128 {
    let b = v.to_be_bytes::<32>();
    u128::from_be_bytes(b[16..].try_into().unwrap_or([0; 16]))
}
