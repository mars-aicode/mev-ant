//! Address classification — Pool, Router, Token identification from events + fund flow.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, B256};
use tracing::debug;

use crate::dex::registry::lookup_topic0;
use crate::models::{PoolId, TxFlow};
use crate::pools::identity::resolve_swap_log;

// ---------------------------------------------------------------------------
// Classification result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// A DEX pool — converts token A → token B.
    Pool,
    /// A router/aggregator — receives token A, forwards token A elsewhere.
    Router,
    /// An ERC20/ERC721 token contract.
    Token,
    /// Recognized infrastructure (blacklisted).
    Infra,
    /// A lending platform — holds collateral, issues borrows. Used to walk
    /// the funding chain past the borrow intermediary to the real funder.
    Lending,
    /// Not yet classified — address without recognized events.
    #[allow(dead_code)]
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Classified {
    /// All classified addresses by kind.
    #[allow(dead_code)]
    pub kinds: HashMap<Address, AddressKind>,
    /// Pool + Router addresses (for transfer-level pool/router detection).
    pub pool_or_router: HashSet<Address>,
    /// Actual pool identities discovered from swap events.
    /// For vault-style protocols this includes `PoolId::Param(bytes32)` IDs.
    #[allow(dead_code)]
    pub pools: HashSet<PoolId>,
    /// Per-tx pool identities, indexed by `tx_index`. Mirrors the logs passed
    /// to the classifier so the detector uses the same source of truth.
    pub tx_pools: Vec<HashSet<PoolId>>,
    /// Token addresses.
    #[allow(dead_code)]
    pub tokens: HashSet<Address>,
    /// Lending platform addresses (Aave, Compound, Maker, etc.).
    pub lending_set: HashSet<Address>,
    /// Addresses still unknown after classification.
    pub unknown: HashSet<Address>,
}

const TRANSFER_TOPIC: B256 = B256::new(hex_literal::hex!(
    "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
));
const APPROVAL_TOPIC: B256 = B256::new(hex_literal::hex!(
    "8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"
));

// ---------------------------------------------------------------------------
// Classifier trait
// ---------------------------------------------------------------------------

/// A classifier turns decoded block traces into address role sets.
pub trait Classifier {
    fn classify(&self, tx_flows: &[TxFlow], raw_logs_per_tx: &[Vec<crate::rpc::DxgLog>]) -> Classified;
}

// ---------------------------------------------------------------------------
// Default classifier
// ---------------------------------------------------------------------------

/// The production classifier. It combines known event signatures, blacklist
/// /lending overrides, and fund-flow heuristics.
#[derive(Clone, Copy)]
pub struct DefaultClassifier<'a> {
    blacklist: &'a [Address],
    lending_addresses: &'a [Address],
}

impl<'a> DefaultClassifier<'a> {
    pub fn new(blacklist: &'a [Address], lending_addresses: &'a [Address]) -> Self {
        Self { blacklist, lending_addresses }
    }
}

impl<'a> Classifier for DefaultClassifier<'a> {
    fn classify(&self, tx_flows: &[TxFlow], raw_logs_per_tx: &[Vec<crate::rpc::DxgLog>]) -> Classified {
        classify_impl(tx_flows, raw_logs_per_tx, self.blacklist, self.lending_addresses)
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

fn classify_impl(
    tx_flows: &[TxFlow],
    all_logs_per_tx: &[Vec<crate::rpc::DxgLog>],
    blacklist: &[Address],
    lending_addresses: &[Address],
) -> Classified {
    let mut kinds: HashMap<Address, AddressKind> = HashMap::new();
    let mut lending_set: HashSet<Address> = HashSet::new();
    let mut pools: HashSet<PoolId> = HashSet::new();
    let mut tx_pools: Vec<HashSet<PoolId>> = Vec::with_capacity(all_logs_per_tx.len());

    // --- Tier 1: Recognized events ---
    for tx_logs in all_logs_per_tx {
        let mut this_tx_pools = HashSet::new();
        for log in tx_logs {
            let addr = log.address;
            if log.topics.is_empty() { continue; }
            let t0 = log.topics[0];

            // Pool/Router events (highest priority — a contract can be both pool and token)
            if lookup_topic0(t0).is_some() {
                kinds.insert(addr, AddressKind::Pool);
                if let Some(resolved) = resolve_swap_log(log) {
                    let pid = resolved
                        .pool_id
                        .map(PoolId::Param)
                        .unwrap_or(PoolId::Contract(resolved.address));
                    pools.insert(pid.clone());
                    this_tx_pools.insert(pid);
                }
            } else if t0 == TRANSFER_TOPIC || t0 == APPROVAL_TOPIC {
                // Fallback: ERC20 Token (only if not already Pool)
                kinds.entry(addr).or_insert(AddressKind::Token);
            }
            // Other event → unknown for now, will be analyzed in tier 2
        }
        tx_pools.push(this_tx_pools);
    }

    // --- Tier 1b: Blacklist → Infra (only if not already classified as Pool) ---
    for &bl in blacklist {
        if !matches!(kinds.get(&bl), Some(AddressKind::Pool)) {
            kinds.insert(bl, AddressKind::Infra);
        }
    }

    // --- Tier 1c: Lending platforms → Lending (only if not already Pool/Infra) ---
    for &la in lending_addresses {
        lending_set.insert(la);
        if !matches!(kinds.get(&la), Some(AddressKind::Pool) | Some(AddressKind::Infra)) {
            kinds.entry(la).or_insert(AddressKind::Lending);
        }
    }

    // --- Tier 2: Fund flow analysis for non-classified addresses ---
    let mut candidates: HashSet<Address> = HashSet::new();
    for flow in tx_flows {
        for t in &flow.transfers {
            if !kinds.contains_key(&t.from) {
                candidates.insert(t.from);
            }
            if !kinds.contains_key(&t.to) {
                candidates.insert(t.to);
            }
        }
    }

    classify_by_fund_flow(tx_flows, &candidates, &mut kinds);

    // --- Build output sets ---
    let mut pool_or_router = HashSet::new();
    let mut tokens = HashSet::new();
    let mut unknown = HashSet::new();
    for (&addr, kind) in &kinds {
        match kind {
            AddressKind::Pool | AddressKind::Router => { pool_or_router.insert(addr); }
            AddressKind::Token => { tokens.insert(addr); }
            AddressKind::Unknown => { unknown.insert(addr); }
            AddressKind::Infra | AddressKind::Lending => {}
        }
    }

    // Also collect addresses that appear in transfers but weren't classified
    for flow in tx_flows {
        for t in &flow.transfers {
            if !kinds.contains_key(&t.from) {
                unknown.insert(t.from);
            }
            if !kinds.contains_key(&t.to) {
                unknown.insert(t.to);
            }
        }
    }

    // from/to of txs also
    for flow in tx_flows {
        if !kinds.contains_key(&flow.from) {
            unknown.insert(flow.from);
        }
        if let Some(to) = flow.to {
            if !kinds.contains_key(&to) {
                unknown.insert(to);
            }
        }
    }

    debug!(
        "classify: pools={} tokens={} lending={} unknown={}",
        pool_or_router.len(),
        tokens.len(),
        lending_set.len(),
        unknown.len()
    );

    Classified {
        kinds,
        pool_or_router,
        pools,
        tx_pools,
        tokens,
        lending_set,
        unknown,
    }
}

fn classify_by_fund_flow(
    tx_flows: &[TxFlow],
    candidates: &HashSet<Address>,
    kinds: &mut HashMap<Address, AddressKind>,
) {
    for flow in tx_flows {
        let relevant: Vec<Address> = candidates.iter()
            .filter(|a| flow.transfers.iter().any(|t| t.from == **a || t.to == **a))
            .copied()
            .collect();

        for &addr in &relevant {
            if kinds.contains_key(&addr) { continue; }
            if addr == Address::ZERO { continue; }

            let received: Vec<(Address, u128)> = flow.transfers.iter()
                .filter(|t| t.to == addr)
                .map(|t| (t.token, amount_as_u128(t.amount)))
                .collect();
            let sent: Vec<(Address, u128)> = flow.transfers.iter()
                .filter(|t| t.from == addr)
                .map(|t| (t.token, amount_as_u128(t.amount)))
                .collect();

            if !received.is_empty() && !sent.is_empty() {
                let sent_tokens: HashSet<Address> = sent.iter().map(|(t, _)| *t).collect();
                let recv_tokens: HashSet<Address> = received.iter().map(|(t, _)| *t).collect();
                if sent_tokens.is_disjoint(&recv_tokens) {
                    kinds.insert(addr, AddressKind::Pool);
                } else if sent_tokens == recv_tokens {
                    let mut sent_amt: HashMap<Address, u128> = HashMap::new();
                    let mut recv_amt: HashMap<Address, u128> = HashMap::new();
                    for (t, a) in &sent { *sent_amt.entry(*t).or_default() += *a; }
                    for (t, a) in &received { *recv_amt.entry(*t).or_default() += *a; }
                    if sent_amt == recv_amt {
                        kinds.insert(addr, AddressKind::Router);
                    }
                }
            }
        }
    }
}

fn amount_as_u128(v: alloy::primitives::U256) -> u128 {
    let bytes = v.to_be_bytes::<32>();
    u128::from_be_bytes(bytes[16..].try_into().unwrap_or([0; 16]))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, B256, U256};
    use crate::models::{Transfer, TxFlow};

    fn addr(s: &str) -> Address {
        let p = format!("0x{:0>40}", s.trim_start_matches("0x"));
        p.parse().unwrap()
    }

    fn mock_tx(index: u64, transfers: Vec<Transfer>) -> TxFlow {
        TxFlow {
            tx_hash: B256::ZERO, tx_index: index, from: Address::ZERO,
            to: None, transfers, gas_used: 0, effective_gas_price: 0,
            effective_priority_fee: 0, success: true,
        }
    }

    fn transfer(from: Address, to: Address, token: Address, amount: u64) -> Transfer {
        Transfer { from, to, token, amount: U256::from(amount) }
    }

    fn log(address: Address, topic0: B256) -> crate::rpc::DxgLog {
        crate::rpc::DxgLog { address, topics: vec![topic0], data: "0x".to_string() }
    }

    fn classifier() -> DefaultClassifier<'static> {
        DefaultClassifier::new(&[], &[])
    }

    #[test]
    fn swap_event_classifies_pool() {
        let pool = addr("0xaaaa");
        // Uniswap V2 Swap topic0
        let swap_topic = B256::new(hex_literal::hex!(
            "d78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822"
        ));
        let txs = vec![mock_tx(0, vec![])];
        let logs = vec![vec![log(pool, swap_topic)]];

        let c = classifier().classify(&txs, &logs);
        assert!(c.pool_or_router.contains(&pool));
        assert!(c.kinds.get(&pool) == Some(&AddressKind::Pool));
    }

    #[test]
    fn transfer_event_classifies_token() {
        let token = addr("0xbbbb");
        let txs = vec![mock_tx(0, vec![])];
        let logs = vec![vec![log(token, TRANSFER_TOPIC)]];

        let c = classifier().classify(&txs, &logs);
        assert!(c.tokens.contains(&token));
        assert!(c.kinds.get(&token) == Some(&AddressKind::Token));
    }

    #[test]
    fn pool_overrides_token() {
        let pool = addr("0xcccc");
        // Swap topic for a known DEX event from registry
        let swap_topic = B256::new(hex_literal::hex!(
            "c42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67"
        ));
        let txs = vec![mock_tx(0, vec![])];
        let logs = vec![vec![
            log(pool, TRANSFER_TOPIC),
            log(pool, swap_topic),
        ]];

        let c = classifier().classify(&txs, &logs);
        assert!(c.pool_or_router.contains(&pool));
        assert!(c.kinds.get(&pool) == Some(&AddressKind::Pool));
    }

    #[test]
    fn blacklist_marks_infra() {
        let infra = addr("0xdddd");
        let txs = vec![mock_tx(0, vec![])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let blacklist = [infra];
        let classifier = DefaultClassifier::new(&blacklist, &[]);
        let c = classifier.classify(&txs, &logs);
        assert!(c.kinds.get(&infra) == Some(&AddressKind::Infra));
        assert!(!c.pool_or_router.contains(&infra));
    }

    #[test]
    fn lending_address_added_to_lending_set() {
        let lending = addr("0xeeee");
        let txs = vec![mock_tx(0, vec![])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let lending_addrs = [lending];
        let classifier = DefaultClassifier::new(&[], &lending_addrs);
        let c = classifier.classify(&txs, &logs);
        assert!(c.lending_set.contains(&lending));
        assert!(c.kinds.get(&lending) == Some(&AddressKind::Lending));
    }

    #[test]
    fn fund_flow_router_same_token_equal_amounts() {
        let router = addr("0x1111");
        let token = addr("0x2222");
        let user = addr("0x3333");
        let pool = addr("0x4444");

        // Router receives 100 token from user and sends 100 token to pool
        let txs = vec![mock_tx(0, vec![
            transfer(user, router, token, 100),
            transfer(router, pool, token, 100),
        ])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let c = classifier().classify(&txs, &logs);
        assert!(c.pool_or_router.contains(&router));
        assert!(c.kinds.get(&router) == Some(&AddressKind::Router));
    }

    #[test]
    fn fund_flow_pool_different_tokens() {
        let pool = addr("0x5555");
        let token_in = addr("0x6666");
        let token_out = addr("0x7777");
        let user = addr("0x8888");

        let txs = vec![mock_tx(0, vec![
            transfer(user, pool, token_in, 100),
            transfer(pool, user, token_out, 50),
        ])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let c = classifier().classify(&txs, &logs);
        assert!(c.pool_or_router.contains(&pool));
        assert!(c.kinds.get(&pool) == Some(&AddressKind::Pool));
    }

    #[test]
    fn fund_flow_eth_sender_classified_as_pool_by_flow() {
        // Fund-flow analysis classifies ETH senders as Pool
        // (WETH→ETH exchange pattern = different tokens).
        // The DETECTOR layer (detect_sandwiches) cleans this up
        // by demoting coinbase-ETH senders back to Unknown,
        // because a DEX pool never sends ETH to the block's coinbase.
        let funder = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let coinbase = addr("0xcccc");
        let weth = addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let eth = crate::models::ETH_TRANSFER_ADDR;

        let txs = vec![mock_tx(0, vec![
            transfer(funder, executor, weth, 100),
        ]), mock_tx(1, vec![
            transfer(executor, funder, weth, 105),
            transfer(funder, coinbase, eth, 1),
        ])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![], vec![]];

        let c = classifier().classify(&txs, &logs);
        // Fund-flow still classifies as Pool — this is expected.
        // The detector layer's cleanup will handle it.
        assert!(c.pool_or_router.contains(&funder));
    }

    #[test]
    fn zero_address_skipped_as_router() {
        let token = addr("0x9999");
        let user = addr("0xaaaa");

        // Mint appears as zero -> user transfer
        let txs = vec![mock_tx(0, vec![
            transfer(Address::ZERO, user, token, 100),
        ])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let c = classifier().classify(&txs, &logs);
        assert!(!c.pool_or_router.contains(&Address::ZERO));
        assert!(c.unknown.contains(&Address::ZERO));
    }

    #[test]
    fn unequal_same_token_amounts_stay_unknown() {
        // Regression for block 25302239: an executor with same token set
        // but unequal in/out amounts must not be classified as Router.
        let executor = addr("0x1f2f");
        let token = addr("0x3333");
        let user = addr("0x4444");
        let pool = addr("0x5555");

        let txs = vec![mock_tx(0, vec![
            transfer(user, executor, token, 100),
            transfer(executor, pool, token, 95),
        ])];
        let logs: Vec<Vec<crate::rpc::DxgLog>> = vec![vec![]];

        let c = classifier().classify(&txs, &logs);
        assert!(!c.pool_or_router.contains(&executor));
        assert!(c.unknown.contains(&executor));
    }
}
