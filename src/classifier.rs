//! Address classification — Pool, Router, Token identification from events + fund flow.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, B256};
use tracing::debug;

use crate::dex::registry::lookup_topic0;
use crate::models::TxFlow;

// ---------------------------------------------------------------------------
// Classification
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
    /// Pool + Router addresses (for tx_touches_pool).
    pub pool_or_router: HashSet<Address>,
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

/// Classify all addresses in a block using event signatures and fund flow.
pub fn classify(
    tx_flows: &[TxFlow],
    all_logs_per_tx: &[Vec<crate::rpc::DxgLog>],
    blacklist: &[Address],
    lending_addresses: &[Address],
) -> Classified {
    let mut kinds: HashMap<Address, AddressKind> = HashMap::new();
    let mut lending_set: HashSet<Address> = HashSet::new();

    // --- Tier 1: Recognized events ---
    for tx_logs in all_logs_per_tx {
        for log in tx_logs {
            let addr = log.address;
            if log.topics.is_empty() { continue; }
            let t0 = log.topics[0];

            // Pool/Router events (highest priority — a contract can be both pool and token)
            if lookup_topic0(t0).is_some() {
                kinds.insert(addr, AddressKind::Pool);
            } else if t0 == TRANSFER_TOPIC || t0 == APPROVAL_TOPIC {
                // Fallback: ERC20 Token (only if not already Pool)
                kinds.entry(addr).or_insert(AddressKind::Token);
            }
            // Other event → unknown for now, will be analyzed in tier 2
        }
    }

    // --- Tier 1b: Blacklist → Infra (only if not already classified as Pool)
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
    // Collect all addresses that appear in transfers but aren't classified yet
    // (both emitting and non-emitting). A pure passthrough router (1inch,
    // 0x-router, etc.) doesn't emit any events of its own in the block, but
    // it forwards tokens through itself: it receives token T from X and
    // sends token T to Y in the same tx. Treating it as a Router puts it
    // in `pool_or_router`, which makes the discovery include
    // `executor → router` transfers in the executor's trade deltas — without
    // this, executor → router → pool flows hide the executor's actual
    // capital outflow and profit is over-counted.
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
        tokens,
        lending_set,
        unknown,
    }
}

/// Classify unknown event emitters by fund flow pattern.
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
            // The zero address is the standard ERC20 mint/burn sentinel —
            // it appears as the `from` of mints and the `to` of burns in
            // every block. Treating it as a real address (Router, Pool) is
            // a misclassification: it has same-token in/out purely as an
            // artifact of how mints/burns are logged, not because the zero
            // address is a passthrough. Skip it.
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
                // Check if same tokens flow through
                let sent_tokens: HashSet<Address> = sent.iter().map(|(t, _)| *t).collect();
                let recv_tokens: HashSet<Address> = received.iter().map(|(t, _)| *t).collect();
                if sent_tokens.is_disjoint(&recv_tokens) {
                    // Different tokens in vs out: a swap pool.
                    kinds.insert(addr, AddressKind::Pool);
                } else if sent_tokens == recv_tokens {
                    // Single token in == single token out: pure passthrough
                    // (e.g. 1inch router forwarding ETH through itself).
                    // Strict equality — multi-token flows (e.g. an executor
                    // receiving ETH from WETH unwrap and sending ETH to a
                    // router, while also receiving/sending SNX) are left
                    // Unknown so they can be candidates for executor
                    // discovery. Any-overlap was too loose: an executor
                    // with one shared token gets misclassified as a
                    // router and the executor's trade capital is hidden.
                    kinds.insert(addr, AddressKind::Router);
                }
            }
        }
    }
}

fn amount_as_u128(v: alloy::primitives::U256) -> u128 {
    let bytes = v.to_be_bytes::<32>();
    u128::from_be_bytes(bytes[16..].try_into().unwrap_or([0; 16]))
}
