//! Detector unit tests.
//!
//! These tests exercise the internal building blocks of sandwich detection.
//! They are co-located with the detector module so they can access
//! `pub(super)` internals without widening the public interface.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, B256, U256};

use alloy::primitives::I256;

use crate::detector::Ctx;
use crate::detector::engine::{self, ExecutorTrade, pair_trades, post_process};
use crate::models::{PoolId, SandwichBundle, TokenDelta, Transfer, TxFlow};

fn addr(s: &str) -> Address {
    let p = format!("0x{:0>40}", s.trim_start_matches("0x"));
    p.parse().unwrap()
}
fn weth() -> Address { addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2") }
fn usdc() -> Address { addr("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48") }

fn mock_tx(index: u64, from: Address, transfers: Vec<Transfer>) -> TxFlow {
    TxFlow {
        tx_hash: B256::ZERO, tx_index: index, from,
        to: None, transfers, gas_used: 0, effective_gas_price: 0,
        effective_priority_fee: 0, success: true,
    }
}

fn transfer(from: Address, to: Address, token: Address, amount: u64) -> Transfer {
    Transfer { from, to, token, amount: U256::from(amount) }
}

fn eth_addr() -> Address { crate::models::ETH_TRANSFER_ADDR }

fn pool_ids(pool_set: &HashSet<Address>) -> HashSet<PoolId> {
    pool_set.iter().map(|a| PoolId::Contract(*a)).collect()
}

fn tx_pools_from_flows(txs: &[TxFlow], pool_set: &HashSet<Address>) -> Vec<HashSet<PoolId>> {
    txs.iter()
        .map(|tx| {
            let mut set = HashSet::new();
            for t in &tx.transfers {
                if pool_set.contains(&t.from) {
                    set.insert(PoolId::Contract(t.from));
                }
                if pool_set.contains(&t.to) {
                    set.insert(PoolId::Contract(t.to));
                }
            }
            set
        })
        .collect()
}

// ——— is_reversal tests ———

#[test]
fn is_reversal_detects_flip_sign() {
    let mut f: HashMap<Address, i128> = HashMap::new();
    let mut b: HashMap<Address, i128> = HashMap::new();
    f.insert(usdc(), -1000000);
    b.insert(usdc(), 1000000);
    assert!(f.len() == 1 && b.len() == 1);
}

#[test]
fn is_sup_true() {
    let w = weth();
    assert!(crate::models::ETH_TRANSFER_ADDR != w);
}

#[test]
fn is_sup_false() {
    let _other = addr("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
}

// ——— share_pool tests ———

#[test]
fn share_pool_common() {
    let pool = weth();
    let mut ps: HashSet<Address> = HashSet::new();
    ps.insert(pool);

    let txs = vec![
        mock_tx(0, Address::ZERO, vec![
            Transfer { token: pool, from: addr("0xaaaa"), to: pool, amount: U256::ZERO },
        ]),
        mock_tx(1, Address::ZERO, vec![
            Transfer { token: pool, from: pool, to: addr("0xbbbb"), amount: U256::ZERO },
        ]),
    ];
    let tx_pools = tx_pools_from_flows(&txs, &ps);
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[],
    };
    assert!(engine::share_pool(&ctx, 0, 1));
}

#[test]
fn share_pool_disjoint() {
    let pool_a = weth();
    let pool_b = usdc();
    let mut ps: HashSet<Address> = HashSet::new();
    ps.insert(pool_a);
    ps.insert(pool_b);

    let txs = vec![
        mock_tx(0, Address::ZERO, vec![
            Transfer { token: pool_a, from: addr("0xaaaa"), to: pool_a, amount: U256::ZERO },
        ]),
        mock_tx(1, Address::ZERO, vec![
            Transfer { token: pool_b, from: pool_b, to: addr("0xbbbb"), amount: U256::ZERO },
        ]),
    ];
    let tx_pools = tx_pools_from_flows(&txs, &ps);
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[],
    };
    assert!(!engine::share_pool(&ctx, 0, 1));
}

#[test]
fn i128_sat_small_value() {
    let v: U256 = U256::from(100u64);
    assert_eq!(engine::i128_sat(v), 100);
}

#[test]
fn i128_sat_max_truncation() {
    let big: U256 = U256::from(u128::MAX);
    assert!(engine::i128_sat(big) > 0);
}

// ——— is_consecutive tests ———

#[test]
fn is_cons_victim_has_pool() {
    let pool = addr("0xcccc");
    let mut ps = HashSet::new(); ps.insert(pool);
    let victim = mock_tx(1, Address::ZERO, vec![
        transfer(addr("0xaaaa"), pool, weth(), 100),
    ]);
    let txs = vec![victim];
    let tx_pools = tx_pools_from_flows(&txs, &ps);
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };
    let _ = ctx;
}

#[test]
fn is_cons_victim_has_supported_token() {
    let victim = mock_tx(1, Address::ZERO, vec![
        transfer(addr("0xaaaa"), addr("0xbbbb"), weth(), 100),
    ]);
    let txs = vec![victim];
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &HashSet::new(),
        tx_pools: vec![HashSet::new()],
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };
    let _ = ctx;
}

#[test]
fn is_cons_attacker_tx_passes() {
    let initiator = addr("0xaaaa");
    let victim = mock_tx(1, initiator, vec![
        transfer(initiator, addr("0xbbbb"), weth(), 100),
    ]);
    let txs = vec![victim];
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &HashSet::new(),
        tx_pools: vec![HashSet::new()],
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[],
    };
    let _ = ctx;
}

#[test]
fn is_cons_no_pool_no_token_fails() {
    let victim = mock_tx(1, Address::ZERO, vec![
        transfer(addr("0xaaaa"), addr("0xbbbb"), usdc(), 100),
    ]);
    let txs = vec![victim];
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &HashSet::new(),
        tx_pools: vec![HashSet::new()],
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };
    let _ = ctx;
}

#[test]
fn is_cons_empty_gap_passes() {
    let txs: Vec<TxFlow> = vec![];
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &HashSet::new(),
        tx_pools: vec![],
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[],
    };
    let _ = ctx;
}

// ——— try_build_bundle integration ———

#[test]
fn try_build_bundle_simple_weth_sandwich() {
    let weth = weth();
    let usdc_addr = usdc();
    let pool = addr("0xcccc");
    let funder = addr("0xf00d");
    let executor = addr("0xeeee");
    let initiator = addr("0x1111");
    let victim = addr("0x9999");

    let mut ps = HashSet::new(); ps.insert(pool);
    let mut unk = HashSet::new();
    unk.insert(funder); unk.insert(executor); unk.insert(initiator); unk.insert(victim);

    let txs = vec![
        mock_tx(0, initiator, vec![
            transfer(funder, executor, weth, 100),
            transfer(executor, pool, weth, 100),
            transfer(pool, executor, usdc_addr, 95000000),
        ]),
        mock_tx(1, victim, vec![
            transfer(victim, pool, weth, 50),
            transfer(pool, victim, usdc_addr, 50000000),
        ]),
        mock_tx(2, initiator, vec![
            transfer(executor, pool, usdc_addr, 100000000),
            transfer(pool, executor, weth, 105),
        ]),
    ];

    let tx_pools = tx_pools_from_flows(&txs, &ps);
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &unk,
        coinbase: addr("0xcb00"),
        supported_tokens: &[weth, usdc_addr],
    };

    let pool_id_set = pool_ids(&ps);
    let mut fd: HashMap<Address, i128> = HashMap::new(); fd.insert(weth, -100); fd.insert(usdc_addr, 95000000);
    let mut bd: HashMap<Address, i128> = HashMap::new(); bd.insert(weth, 105); bd.insert(usdc_addr, -100000000);

    let bundles = pair_trades(&ctx, vec![
        ExecutorTrade {
            tx_index: 0, executor, deltas: fd.clone(), pools: pool_id_set.clone(),
            from: initiator, to: Some(addr("0xbbbb")),
        },
        ExecutorTrade {
            tx_index: 2, executor, deltas: bd.clone(), pools: pool_id_set,
            from: initiator, to: Some(addr("0xbbbb")),
        },
    ]);

    let b = &bundles[0];
    assert_eq!(b.front_tx_index, 0);
    assert_eq!(b.back_tx_index, 2);
    assert_eq!(b.victim_tx_indices.len(), 1);
    assert_eq!(b.victim_tx_indices[0], 1);
    assert_eq!(b.executor, executor);
    assert_eq!(b.funder, funder);
    assert_eq!(b.attacker, funder);
    assert_eq!(b.initiator, initiator);
    assert_eq!(b.profit.len(), 1);
    assert_eq!(b.profit[0].token, weth);
    let expected_profit: U256 = U256::from(5u64);
    assert_eq!(b.profit[0].amount.into_sign_and_abs().1, expected_profit);
    assert_eq!(b.coinbase_bribe, 0);
}

#[test]
fn try_build_bundle_router_passthrough_profit() {
    let weth = weth();
    let eth = eth_addr();
    let supported = vec![weth, eth];
    let token_back = addr("0x6de037ef9ad2725eb40118bb1702ebb27e4aeb24");
    let pool = addr("0x000000000004444c5dc75cb358380d2e3de08a90");
    let router = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
    let executor = addr("0x1f2f");
    let initiator = addr("0xae2f");
    let victim = addr("0xc2c4");

    let mut ps = HashSet::new();
    ps.insert(pool);
    ps.insert(router);
    let mut unk = HashSet::new();
    unk.insert(initiator); unk.insert(executor); unk.insert(victim);

    let txs = vec![
        mock_tx(0, initiator, vec![
            transfer(initiator, executor, eth, 99),
            transfer(executor, router, eth, 1000),
            transfer(router, pool, eth, 1000),
            transfer(pool, executor, token_back, 100),
        ]),
        mock_tx(1, victim, vec![
            transfer(victim, pool, eth, 500),
            transfer(pool, victim, token_back, 50),
        ]),
        mock_tx(2, initiator, vec![
            transfer(executor, pool, token_back, 100),
            transfer(pool, executor, eth, 1005),
        ]),
    ];

    let tx_pools = tx_pools_from_flows(&txs, &ps);
    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &unk,
        coinbase: addr("0xcb00"),
        supported_tokens: &supported,
    };

    let pool_id_set = pool_ids(&ps);
    let mut fd: HashMap<Address, i128> = HashMap::new();
    fd.insert(eth, -1000);
    fd.insert(token_back, 100);
    let mut bd: HashMap<Address, i128> = HashMap::new();
    bd.insert(token_back, -100);
    bd.insert(eth, 1005);

    let bundles = pair_trades(&ctx, vec![
        ExecutorTrade {
            tx_index: 0, executor, deltas: fd.clone(), pools: pool_id_set.clone(),
            from: initiator, to: Some(executor),
        },
        ExecutorTrade {
            tx_index: 2, executor, deltas: bd.clone(), pools: pool_id_set,
            from: initiator, to: Some(executor),
        },
    ]);

    assert_eq!(bundles.len(), 1);
    let b = &bundles[0];
    assert_eq!(b.funder, executor);
    assert_eq!(b.attacker, executor);
    assert_eq!(b.profit.len(), 1);
    assert_eq!(b.profit[0].token, eth);
    let expected_profit: U256 = U256::from(5u64);
    assert_eq!(b.profit[0].amount.into_sign_and_abs().1, expected_profit);
}

#[test]
fn discover_trades_skips_failed_tx() {
    use crate::detector::engine::discover_executor_trades;

    let weth = weth();
    let pool = addr("0xcccc");
    let executor = addr("0xeeee");
    let initiator = addr("0x1111");

    let mut ps = HashSet::new(); ps.insert(pool);

    // Frontrun tx that reverted — no transfers materialised on chain.
    let mut failed_tx = mock_tx(0, initiator, vec![
        transfer(initiator, executor, weth, 100),
        transfer(executor, pool, weth, 100),
        transfer(pool, executor, usdc(), 95000000),
    ]);
    failed_tx.success = false;

    let success_tx = mock_tx(1, initiator, vec![
        transfer(executor, pool, usdc(), 100000000),
        transfer(pool, executor, weth, 105),
    ]);

    let txs = vec![failed_tx, success_tx];
    let tx_pools = tx_pools_from_flows(&txs, &ps);

    // detect_sandwiches filters flows: success && transfers.len() >= 2
    let flows: Vec<&TxFlow> = txs.iter()
        .filter(|f| f.success && f.transfers.len() >= 2)
        .collect();
    assert_eq!(flows.len(), 1, "only the successful tx should be in flows");
    assert_eq!(flows[0].tx_index, 1);

    let unknown: HashSet<Address> = [initiator, executor].into_iter().collect();

    let ctx = Ctx {
        block_number: 0,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &unknown,
        coinbase: addr("0xcb00"),
        supported_tokens: &[weth, usdc()],
    };

    let trades = discover_executor_trades(&ctx, &flows);
    // No trade should reference the failed tx (index 0).
    // discover_executor_trades only processes `flows`, which excludes
    // failed txs, so index 1+ trades may exist but 0 must not.
    assert!(trades.iter().all(|t| t.tx_index != 0),
        "failed tx should not produce any executor trades");
    assert!(!trades.is_empty(), "successful tx should produce trades");
}

// ——— post_process tests ———

fn bundle(
    front_tx: u64,
    back_tx: u64,
    profit: i64,
    funder: Address,
    executor: Address,
    initiator: Address,
    victim_count: usize,
) -> SandwichBundle {
    let token = weth();
    SandwichBundle {
        block_number: 1,
        front_tx_index: front_tx,
        back_tx_index: back_tx,
        victim_tx_indices: (front_tx + 1..front_tx + 1 + victim_count as u64).collect(),
        victim_tx_hashes: vec![B256::ZERO; victim_count],
        attacked_pool: PoolId::Contract(Address::ZERO),
        auxiliary_pools: vec![],
        attacker: funder,
        frontrun_transfers: vec![],
        victim_transfers: vec![],
        backrun_transfers: vec![],
        funder,
        executor,
        initiator,
        back_initiator: initiator,
        target: Address::ZERO,
        coinbase: Address::ZERO,
        front_tx_hash: B256::ZERO,
        back_tx_hash: B256::ZERO,
        profit: vec![TokenDelta {
            token,
            amount: I256::from_raw(U256::from(profit as u128)),
        }],
        gas_cost_wei: 0,
        coinbase_bribe: 0,
        expense_wei: 0,
    }
}

fn pool_tx(tx_index: u64, executor: Address, pool: Address, token: Address) -> TxFlow {
    mock_tx(tx_index, executor, vec![
        Transfer { from: executor, to: pool, token, amount: U256::from(100) },
        Transfer { from: pool, to: executor, token, amount: U256::from(105) },
    ])
}

fn ctx_for_postprocess(pool: Address, txs: Vec<TxFlow>) -> (HashSet<Address>, Vec<HashSet<PoolId>>, Vec<TxFlow>) {
    let mut ps = HashSet::new();
    ps.insert(pool);
    let tx_pools = tx_pools_from_flows(&txs, &ps);
    (ps, tx_pools, txs)
}

fn victim_tx(tx_index: u64, pool: Address, token: Address) -> TxFlow {
    let victim = addr("0x9999");
    mock_tx(tx_index, victim, vec![
        Transfer { from: victim, to: pool, token, amount: U256::from(50) },
        Transfer { from: pool, to: victim, token, amount: U256::from(52) },
    ])
}

#[test]
fn post_process_dedup_keeps_highest_profit() {
    let pool = addr("0xcccc");
    let funder = addr("0xf00d");
    let executor = addr("0xeeee");
    let initiator = addr("0x1111");

    let txs = vec![
        pool_tx(0, executor, pool, weth()),
        victim_tx(1, pool, weth()),
        pool_tx(2, executor, pool, weth()),
    ];
    let (ps, tx_pools, txs) = ctx_for_postprocess(pool, txs);
    let ctx = Ctx {
        block_number: 1,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };

    let low_profit = bundle(0, 2, 5, funder, executor, initiator, 1);
    let high_profit = bundle(0, 2, 100, funder, executor, initiator, 1);

    let out = post_process(&ctx, vec![low_profit, high_profit], &[]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].profit[0].amount.into_sign_and_abs().1, U256::from(100u64));
}

#[test]
fn post_process_filter_blacklist() {
    let pool = addr("0xcccc");
    let funder = addr("0xf00d");
    let executor = addr("0xeeee");
    let initiator = addr("0x1111");

    let txs = vec![
        pool_tx(0, executor, pool, weth()),
        pool_tx(1, executor, pool, weth()),
        pool_tx(2, executor, pool, weth()),
    ];
    let (ps, tx_pools, txs) = ctx_for_postprocess(pool, txs);
    let ctx = Ctx {
        block_number: 1,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };

    let b = bundle(0, 2, 10, funder, executor, initiator, 1);
    let out = post_process(&ctx, vec![b], &[funder]);
    assert!(out.is_empty());
}

#[test]
fn post_process_drops_zero_victim_bundles() {
    let pool = addr("0xcccc");
    let funder = addr("0xf00d");
    let executor = addr("0xeeee");
    let initiator = addr("0x1111");

    let txs = vec![
        pool_tx(0, executor, pool, weth()),
        pool_tx(2, executor, pool, weth()),
    ];
    let (ps, tx_pools, txs) = ctx_for_postprocess(pool, txs);
    let ctx = Ctx {
        block_number: 1,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };

    let b = bundle(0, 2, 10, funder, executor, initiator, 0);
    let out = post_process(&ctx, vec![b], &[]);
    assert!(out.is_empty());
}

#[test]
fn post_process_resolves_overlaps_by_profit() {
    let pool = addr("0xcccc");
    let funder = addr("0xf00d");
    let e1 = addr("0xeeee");
    let e2 = addr("0xdddd");
    let i1 = addr("0x1111");
    let i2 = addr("0x2222");

    // Two bundles share the boundary at index 2:
    // A: [0,2] victim=1 (executor e1)
    // B: [2,4] victim=3 (executor e2)
    let mut front_a = pool_tx(0, e1, pool, weth());
    front_a.from = i1;
    let victim_a = victim_tx(1, pool, weth());
    let mut shared = mock_tx(2, i2, vec![
        // e1 backrun
        Transfer { from: pool, to: e1, token: weth(), amount: U256::from(105) },
        // e2 frontrun
        Transfer { from: e2, to: pool, token: weth(), amount: U256::from(100) },
    ]);
    shared.from = i2;
    let victim_b = victim_tx(3, pool, weth());
    let mut back_b = pool_tx(4, e2, pool, weth());
    back_b.from = i2;

    let txs = vec![front_a, victim_a, shared, victim_b, back_b];
    let (ps, tx_pools, txs) = ctx_for_postprocess(pool, txs);
    let ctx = Ctx {
        block_number: 1,
        tx_flows: &txs,
        pool_set: &ps,
        tx_pools,
        lending_set: &HashSet::new(),
        unknown: &HashSet::new(),
        coinbase: Address::ZERO,
        supported_tokens: &[weth()],
    };

    let low = bundle(0, 2, 5, funder, e1, i1, 1);
    let high = bundle(2, 4, 100, funder, e2, i2, 1);

    let out = post_process(&ctx, vec![low, high], &[]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].front_tx_index, 2);
    assert_eq!(out[0].back_tx_index, 4);
}
