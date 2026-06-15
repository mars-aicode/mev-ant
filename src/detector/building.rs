//! Round 2b: Pair executor trades into candidate sandwich bundles.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, B256, I256, U256};
use crate::detector::discovery::{ExecutorTrade, i128_sat, is_sup, WETH};
use crate::detector::sandwich::Ctx;
use crate::models::{PoolId, SandwichBundle, TokenDelta, TxFlow, ETH_TRANSFER_ADDR};

/// Pair executor trades by same executor into sandwich bundles.
pub(crate) fn pair_trades(ctx: &Ctx, trades: Vec<ExecutorTrade>) -> Vec<SandwichBundle> {
    // Group trades by executor address
    let mut trades_by_exec: HashMap<Address, Vec<&ExecutorTrade>> = HashMap::new();
    for t in &trades { trades_by_exec.entry(t.executor).or_default().push(t); }

    let mut bundles: Vec<SandwichBundle> = Vec::new();

    for (executor, exec_trades) in &trades_by_exec {
        for i in 0..exec_trades.len() {
            let front = &exec_trades[i];
            for back in &exec_trades[(i+1)..] {
                if front.tx_index >= back.tx_index { continue; }

                if !is_consecutive(ctx, front.tx_index, back.tx_index, front.from, front.to.unwrap_or(Address::ZERO)) { continue; }
                if !share_pool(ctx, front.tx_index, back.tx_index) { continue; }

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
    // Profit
    let mut pm: HashMap<Address, i128> = HashMap::new();
    for (t, d) in front_deltas { *pm.entry(*t).or_default() += d; }
    for (t, d) in back_deltas { *pm.entry(*t).or_default() += d; }
    let profit: Vec<TokenDelta> = pm.into_iter()
        .filter(|(t, n)| *n > 0 && is_sup(*t, ctx.supported_tokens))
        .map(|(token, net)| TokenDelta { token, amount: I256::from_raw(U256::from(net as u128)) })
        .collect();
    if profit.is_empty() { return None; }

    let ff = ctx.tx_flows.iter().find(|f| f.tx_index == front_tx)?;
    let bf = ctx.tx_flows.iter().find(|f| f.tx_index == back_tx)?;
    let funder = funder_hint.or_else(|| trace_funder(ctx, ff, executor)).unwrap_or(executor);
    // Flashloan: self-funded executor round-trips same token to same address
    if funder == executor {
        let senders: HashSet<(Address, Address)> = ff.transfers.iter()
            .filter(|t| t.to == executor).map(|t| (t.from, t.token)).collect();
        let repays_same = ff.transfers.iter().any(|t|
            t.from == executor && senders.contains(&(t.to, t.token)));
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
            if !share_pool(ctx, front_tx, v) { return false; }
            va.values().any(|ad| {
                ad.iter().any(|(tok, d)| {
                    d.signum() != 0 && front_deltas.get(tok).is_some_and(|fd| fd.signum() == d.signum())
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
        liquidity_pools: vec![], attacker,
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

// ——— Helpers ———

/// Check two txs share at least one pool address.
fn share_pool(ctx: &Ctx, ftx: u64, btx: u64) -> bool {
    let pools = |tx: u64| -> HashSet<Address> {
        ctx.tx_flows.iter().find(|f| f.tx_index == tx)
            .map(|ff| ff.transfers.iter()
                .filter(|t| ctx.pool_set.contains(&t.from) || ctx.pool_set.contains(&t.to))
                .flat_map(|t| [t.from, t.to].into_iter().filter(|a| ctx.pool_set.contains(a)))
                .collect())
            .unwrap_or_default()
    };
    !pools(ftx).is_disjoint(&pools(btx))
}

/// Reversal requires at least one non-ETH/WETH token to flip sign between
/// front and back. ETH↔WETH is wrapping (no price deviation), not a trade.
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

/// Trace funder from front tx. Also used by post-process for funder consistency check.
///
/// Resolves the executor's pool-out token back to its risk-capital source, in
/// priority order:
///
/// 1. **Direct inbound** — funder sent the pool-out token to the executor in
///    this tx. Pool-intermediary variant: funder → pool → executor (the pool
///    forwards the executor's funding token).
/// 2. **Wrap** — pool-out is WETH, no WETH inbound, but executor received ETH
///    from a funder (wrapped to WETH inside the executor). Only applies when
///    the ETH inbound is sufficient to cover the WETH pool-out at 1:1 — dust
///    ETH from an initiator (gas money) is ignored.
/// 3. **Borrow** — executor sent token C to a lending platform and received
///    pool-out from the same platform. The capital source is whoever put C
///    into the executor's balance (with wrap fallback for C).
/// 4. **Pre-balance** — none of the above applies; the executor already had
///    the pool-out token. Returns `None` so the caller falls back to the
///    executor (self-funded).
///
/// After a candidate is found in any case, a round-trip check rejects it if
/// the executor sends the same token back to the candidate in the same tx
/// (flashloan / round-trip funding).
///
/// Falls back to the legacy pool-intermediary pattern (executor has no
/// pool-outbound, only pool-sourced inbound) for degenerate cases.
pub(crate) fn trace_funder(ctx: &Ctx, ff: &TxFlow, executor: Address) -> Option<Address> {
    // Sort + dedup for deterministic resolution order across runs.
    let mut pool_out_tokens: Vec<Address> = ff.transfers.iter()
        .filter(|t| t.from == executor && ctx.pool_set.contains(&t.to))
        .map(|t| t.token)
        .collect();
    pool_out_tokens.sort();
    pool_out_tokens.dedup();

    for &pool_out in &pool_out_tokens {
        // Case 1: direct inbound of the pool-out token
        if let Some(f) = resolve_direct_inbound(ctx, ff, executor, pool_out, ctx.pool_set) {
            if !is_round_trip(ff, executor, f, pool_out) { return Some(f); }
        }
        // Case 2: wrap — pool-out is WETH, executor received ETH.
        // Only valid if the ETH inbound is large enough to cover the WETH out.
        if pool_out == WETH && eth_sufficient_for_wrap(ctx, ff, executor) {
            if let Some(f) = resolve_eth_inbound(ctx, ff, executor) {
                if !is_round_trip(ff, executor, f, ETH_TRANSFER_ADDR) { return Some(f); }
            }
        }
        // Case 3: borrow from a lending platform
        if let Some(f) = resolve_borrow_funder(ctx, ff, executor, pool_out) {
            if !is_round_trip(ff, executor, f, pool_out) { return Some(f); }
        }
    }

    // Fallback: executor has no pool-outbound. If its only inbound is from a
    // pool, trace that pool's inbound to find the upstream funder. This
    // preserves the legacy `trace_funder_pool_intermediary` pattern.
    if pool_out_tokens.is_empty() {
        if let Some((f, funded_token)) = resolve_pool_intermediary(ctx, ff, executor) {
            if !is_round_trip(ff, executor, f, funded_token) { return Some(f); }
        }
    }

    None
}

/// Reject candidates the executor pays back in the same tx (flashloan /
/// round-trip). The funder is whoever put risk capital in; a same-tx return
/// of the same token means the capital was never at risk. Token-scoped: a
/// profit-share or fee rebate in a different token doesn't disqualify.
fn is_round_trip(ff: &TxFlow, executor: Address, candidate: Address, token: Address) -> bool {
    ff.transfers.iter().any(|t| t.from == executor && t.to == candidate && t.token == token)
}

/// True when the executor's total ETH inbound (from non-pool, non-lending,
/// non-zero sources) is at least the WETH sent to the pool. A 1:1 wrap
/// requires the inbound to cover the outbound. Dust ETH from an initiator
/// paying gas fails this and falls through to pre-balance.
fn eth_sufficient_for_wrap(ctx: &Ctx, ff: &TxFlow, executor: Address) -> bool {
    let total_eth_in: u128 = ff.transfers.iter()
        .filter(|t| t.to == executor
            && t.token == ETH_TRANSFER_ADDR
            && t.from != Address::ZERO
            && !ctx.pool_set.contains(&t.from)
            && !ctx.lending_set.contains(&t.from))
        .map(|t| amount_u128(t.amount))
        .sum();
    let weth_out: u128 = ff.transfers.iter()
        .filter(|t| t.from == executor
            && ctx.pool_set.contains(&t.to)
            && t.token == WETH)
        .map(|t| amount_u128(t.amount))
        .sum();
    if weth_out == 0 { return true; }
    total_eth_in >= weth_out
}

/// Find the largest inbound to the executor of `token` from an Unknown
/// (not Pool, Lending, Token, Router, Infra) non-zero source, but only if
/// the inbound actually covers the executor's outbound of that token to
/// `outbound_target_set` (a pool in case 1, a lending platform in the
/// borrow case). Insufficient inbound is gas/dust, not trade capital —
/// fall through to pre-balance / wrap / borrow cases.
fn resolve_direct_inbound(
    ctx: &Ctx,
    ff: &TxFlow,
    executor: Address,
    token: Address,
    outbound_target_set: &HashSet<Address>,
) -> Option<Address> {
    // Executor must have sent this token to a target in the set for the
    // direct-inbound funding case to apply.
    let out_total: u128 = ff.transfers.iter()
        .filter(|t| t.from == executor
            && outbound_target_set.contains(&t.to)
            && t.token == token)
        .map(|t| amount_u128(t.amount))
        .sum();
    if out_total == 0 { return None; }

    // Sum all direct inbounds of `token` from Unknown. This is the
    // candidate-funder pool.
    let inbound_total: u128 = ff.transfers.iter()
        .filter(|t| t.to == executor
            && t.token == token
            && t.from != Address::ZERO
            && ctx.unknown.contains(&t.from))
        .map(|t| amount_u128(t.amount))
        .sum();

    // Sufficiency: the inbound must cover the outbound. Anything less is
    // gas/dust (e.g., 25301044's 59 wei of initiator gas vs 0.00675 ETH
    // pool-out, or 3807's 0.0000675 ETH vs 1.13 WETH).
    if inbound_total < out_total {
        return None;
    }

    ff.transfers.iter()
        .filter(|t| t.to == executor
            && t.token == token
            && t.from != Address::ZERO
            && ctx.unknown.contains(&t.from))
        .max_by_key(|t| amount_u128(t.amount))
        .map(|t| t.from)
}

/// Case 2: find the largest ETH inbound to the executor from an Unknown
/// non-zero source. The executor wraps ETH to WETH internally.
fn resolve_eth_inbound(ctx: &Ctx, ff: &TxFlow, executor: Address) -> Option<Address> {
    ff.transfers.iter()
        .filter(|t| t.to == executor
            && t.token == ETH_TRANSFER_ADDR
            && t.from != Address::ZERO
            && ctx.unknown.contains(&t.from))
        .max_by_key(|t| amount_u128(t.amount))
        .map(|t| t.from)
}

/// Case 3: the executor sent some token C to a lending platform L and
/// received `pool_out` from L in the same tx. The funder is whoever put C
/// into the executor's balance (with wrap fallback for C if C is WETH).
fn resolve_borrow_funder(
    ctx: &Ctx,
    ff: &TxFlow,
    executor: Address,
    pool_out: Address,
) -> Option<Address> {
    for t_out in ff.transfers.iter()
        .filter(|t| t.from == executor
            && ctx.lending_set.contains(&t.to)
            && t.token != pool_out)
    {
        let has_borrow = ff.transfers.iter().any(|t_borrow|
            t_borrow.to == executor
            && t_borrow.from == t_out.to
            && t_borrow.token == pool_out);
        if !has_borrow { continue; }
        // Borrow pair: executor sent C to L, L sent pool_out to executor.
        // Find funder of C: try direct inbound of C (collateral → lending
        // platform) first, then wrap if C is WETH.
        if let Some(f) = resolve_direct_inbound(ctx, ff, executor, t_out.token, ctx.lending_set) {
            return Some(f);
        }
        if t_out.token == WETH {
            if let Some(f) = resolve_eth_inbound(ctx, ff, executor) {
                return Some(f);
            }
        }
    }
    None
}

/// Fallback: executor's only inbound is from a pool. Trace that pool's
/// inbound to find the upstream funder. Returns `(funder, funded_token)` so
/// the caller can run a token-scoped round-trip check. Mirrors the legacy
/// `trace_funder_pool_intermediary` test pattern.
fn resolve_pool_intermediary(
    ctx: &Ctx,
    ff: &TxFlow,
    executor: Address,
) -> Option<(Address, Address)> {
    // First branch: executor received token T directly from an Unknown
    // address. The inbound must cover the executor's outbound of T to be
    // the actual funder — anything less is gas dust (e.g., 25304912's 99 wei
    // from the initiator EOA, where the real capital was the executor's
    // pre-balance WETH). If the inbound is insufficient, fall through to
    // the pool-intermediary branch below.
    if let Some((from, token, inbound_total)) = ff.transfers.iter()
        .filter(|t| t.to == executor
            && t.from != Address::ZERO
            && ctx.unknown.contains(&t.from))
        .fold(None::<(Address, Address, u128)>, |acc, t| {
            let amt = amount_u128(t.amount);
            match acc {
                None => Some((t.from, t.token, amt)),
                Some((_, _, prev_amt)) if amt > prev_amt => Some((t.from, t.token, amt)),
                Some(prev) => Some(prev),
            }
        })
    {
        let out_total: u128 = ff.transfers.iter()
            .filter(|t| t.from == executor && t.token == token)
            .map(|t| amount_u128(t.amount))
            .sum();
        if inbound_total >= out_total && out_total > 0 {
            return Some((from, token));
        }
        // fall through to pool-intermediary branch
    }
    // Inbound from a pool → trace pool's inbound
    let from_pool = ff.transfers.iter()
        .filter(|t| t.to == executor && ctx.pool_set.contains(&t.from))
        .max_by_key(|t| amount_u128(t.amount));
    if let Some(t) = from_pool {
        let pool = t.from;
        let token = t.token;
        if let Some(t) = ff.transfers.iter()
            .filter(|t| t.to == pool
                && t.from != Address::ZERO
                && ctx.unknown.contains(&t.from))
            .max_by_key(|t| amount_u128(t.amount))
        {
            return Some((t.from, token));
        }
    }
    None
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

fn amount_u128(v: U256) -> u128 {
    let b = v.to_be_bytes::<32>();
    u128::from_be_bytes(b[16..].try_into().unwrap_or([0;16]))
}
