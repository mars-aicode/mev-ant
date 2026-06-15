//! Round 3: Post-process bundles — dedup, validate, filter, resolve overlaps.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, I256, U256};

use crate::detector::building::trace_funder;
use crate::detector::discovery::{i128_sat, is_sup};
use crate::detector::sandwich::Ctx;
use tracing::debug;

use crate::models::{PoolId, SandwichBundle, TokenDelta, Transfer, TxFlow};

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

// ——— Stage steps ———

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

        // Executor must touch a pool in at least one direction
        let exec_f = front_dp.iter().any(|t| t.from == b.executor || t.to == b.executor);
        let exec_b = back_dp.iter().any(|t| t.from == b.executor || t.to == b.executor);
        if !exec_f && !exec_b { return None; }

        // Same-pool recheck
        let fps: HashSet<Address> = front_dp.iter()
            .filter(|t| ctx.pool_set.contains(&t.from)).map(|t| t.from)
            .chain(front_dp.iter().filter(|t| ctx.pool_set.contains(&t.to)).map(|t| t.to)).collect();
        let bps: HashSet<Address> = back_dp.iter()
            .filter(|t| ctx.pool_set.contains(&t.from)).map(|t| t.from)
            .chain(back_dp.iter().filter(|t| ctx.pool_set.contains(&t.to)).map(|t| t.to)).collect();
        if fps.is_disjoint(&bps) { return None; }

        // Funder consistency
        let same_targ = ff.to.is_some() && ff.to == bf.to;
        if ff.from != bf.from && !same_targ {
            let bfunder = trace_funder(ctx, bf, b.executor);
            if let Some(bfu) = bfunder {
                if b.funder != bfu && b.funder != b.executor && bfu != b.executor
                    && !ctx.pool_set.contains(&b.funder) && !ctx.pool_set.contains(&bfu) { return None; }
            }
        }

        // Recompute profit if empty
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

        // Retain valid victims
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

        // Attacked pool: triple intersection (front executor ∩ victim ∩ back executor)
        let fpools = exec_pool_set(ff, b.executor, ctx.pool_set);
        let bpools = exec_pool_set(bf, b.executor, ctx.pool_set);
        let vpools: HashSet<Address> = b.victim_tx_indices.iter()
            .filter_map(|&vi| ctx.tx_flows.iter().find(|f| f.tx_index == vi))
            .flat_map(|vf| vf.transfers.iter()
                .filter(|t| ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to))
                .flat_map(|t| [t.from, t.to])
                .filter(|a| ctx.pool_set.contains(a)))
            .collect();
        b.attacked_pool = fpools.intersection(&bpools)
            .filter(|p| vpools.contains(*p))
            .next()
            .map(|a| PoolId::Contract(*a))
            .unwrap_or_else(|| {
                front_dp.iter().find_map(|t| {
                    if t.from == b.executor && ctx.pool_set.contains(&t.to) { Some(PoolId::Contract(t.to)) }
                    else if t.to == b.executor && ctx.pool_set.contains(&t.from) { Some(PoolId::Contract(t.from)) }
                    else { None }
                }).unwrap_or(PoolId::Contract(Address::ZERO))
            });

        // Collect all transfers for display
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

/// Pools an executor touches in one tx.
fn exec_pool_set(tx: &TxFlow, executor: Address, pool_set: &HashSet<Address>) -> HashSet<Address> {
    tx.transfers.iter()
        .filter(|t| t.from == executor || t.to == executor)
        .filter(|t| pool_set.contains(&t.from) || pool_set.contains(&t.to))
        .flat_map(|t| [t.from, t.to])
        .filter(|a| pool_set.contains(a))
        .collect()
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

