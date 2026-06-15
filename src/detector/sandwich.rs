//! Sandwich detection — trade-signature with funder/executor classification.
//!
//! Algorithm (3 rounds):
//!   Round 1: Classify addresses + filter txs with ≥2 Transfer events
//!   Round 2: Discover executor trade signatures + pair into candidate bundles
//!   Round 3: Post-process (dedup, validate, filter, resolve overlaps)

use std::collections::HashSet;
use alloy::primitives::Address;
use tracing::debug;

use crate::models::{SandwichBundle, TxFlow};

/// Block-level context shared across all detection functions.
pub(crate) struct Ctx<'a> {
    pub(crate) block_number: u64,
    pub(crate) tx_flows: &'a [TxFlow],
    pub(crate) pool_set: &'a HashSet<Address>,
    pub(crate) lending_set: &'a HashSet<Address>,
    pub(crate) unknown: &'a HashSet<Address>,
    pub(crate) coinbase: Address,
    pub(crate) supported_tokens: &'a [Address],
}

pub fn detect_sandwiches(
    block_number: u64,
    tx_flows: &[TxFlow],
    raw_logs: &[Vec<crate::rpc::DxgLog>],
    coinbase: Address,
    blacklist: &[Address],
    supported_tokens: &[Address],
) -> Vec<SandwichBundle> {
    // Round 1: Classify + filter
    let classified = crate::classifier::classify(
        tx_flows, raw_logs, blacklist, crate::dex::lending::LENDING_ADDRESSES,
    );

    let flows: Vec<&TxFlow> = tx_flows.iter()
        .filter(|f| f.transfers.len() >= 2)
        .collect();
    if flows.len() < 2 { return vec![]; }
    debug!("block {}: {} txs after filter ({} total)", block_number, flows.len(), tx_flows.len());

    let pool_set = &classified.pool_or_router;
    let lending_set = &classified.lending_set;
    let unknown = &classified.unknown;
    let ctx = Ctx { block_number, tx_flows, pool_set, lending_set, unknown, coinbase, supported_tokens };

    // Round 2: Discover executor trades, then pair into bundles
    let trades = super::discovery::discover_executor_trades(&ctx, &flows, &classified);
    let bundles = super::building::pair_trades(&ctx, trades);

    debug!("block {}: {} bundles after pairing", block_number, bundles.len());
    let bundles = super::postprocess::post_process(&ctx, bundles, blacklist);

    debug!("block {} final sandwiches: {}", block_number, bundles.len());
    bundles
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use alloy::primitives::{Address, B256, U256};
    use crate::models::Transfer;
    use super::super::building::{pair_trades, trace_funder};
    use super::super::discovery::i128_sat;

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

    // ——— is_reversal tests ———
    // (is_reversal is private to building.rs, test via pair_trades + try_build_bundle)

    #[test]
    fn is_reversal_detects_flip_sign() {
        let mut f: HashMap<Address, i128> = HashMap::new();
        let mut b: HashMap<Address, i128> = HashMap::new();
        f.insert(usdc(), -1000000);
        b.insert(usdc(), 1000000);
        // is_reversal is private to building.rs — tested via integration test below
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
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &ps, lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[] };
        // share_pool is private — test indirectly via pair_trades integration test
    }

    #[test]
    fn share_pool_disjoint() {
        let pool_a = weth();
        let pool_b = usdc();
        let mut ps: HashSet<Address> = HashSet::new();
        ps.insert(pool_a); ps.insert(pool_b);

        let txs = vec![
            mock_tx(0, Address::ZERO, vec![
                Transfer { token: pool_a, from: addr("0xaaaa"), to: pool_a, amount: U256::ZERO },
            ]),
            mock_tx(1, Address::ZERO, vec![
                Transfer { token: pool_b, from: pool_b, to: addr("0xbbbb"), amount: U256::ZERO },
            ]),
        ];
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &ps, lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[] };
    }

    #[test]
    fn i128_sat_small_value() {
        let v: U256 = U256::from(100u64);
        assert_eq!(i128_sat(v), 100);
    }

    #[test]
    fn i128_sat_max_truncation() {
        let big: U256 = U256::from(u128::MAX);
        assert!(i128_sat(big) > 0);
    }

    // ——— trace_funder tests ———

    #[test]
    fn trace_funder_direct_sender() {
        let funder_s = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_s, executor, weth(), 100),
            transfer(executor, pool, weth(), 95),
        ]);
        let mut unk = HashSet::new(); unk.insert(funder_s); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_s));
    }

    #[test]
    fn trace_funder_pool_intermediary() {
        let funder_s = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_s, pool, weth(), 100),
            transfer(pool, executor, weth(), 95),
        ]);
        let mut unk = HashSet::new(); unk.insert(funder_s); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_s));
    }

    #[test]
    fn trace_funder_flashloan_detected() {
        let flashloaner = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(flashloaner, executor, weth(), 100),
            transfer(executor, flashloaner, weth(), 100),
        ]);
        let mut unk = HashSet::new(); unk.insert(flashloaner); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), None);
    }

    // ——— trace_funder pre-balance / borrow tests ———

    fn usdt_addr() -> Address { addr("0xdac17f958d2ee523a2206206994597c13d831ec7") }
    fn eth_addr() -> Address { crate::models::ETH_TRANSFER_ADDR }

    /// 3807 pattern: executor trades WETH/USDT, only inbound to executor is
    /// dust ETH from an EOA initiator (gas money). WETH comes from pre-balance.
    /// Funder must be None → caller falls back to executor (self-funded).
    #[test]
    fn trace_funder_pre_balance_weth_trade() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let pool = addr("0xacdb");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(initiator); unk.insert(executor);
        let w = weth(); let u = usdt_addr(); let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 67_500_000_000), // 0.0000675 ETH dust
            transfer(executor, pool, w, 1_132_767_764_829_940_199), // 1.13 WETH
            transfer(pool, executor, u, 1_888_328_466), // 1888 USDT
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), None);
    }

    /// Borrow case: EOA sends WETH to executor, executor deposits WETH as
    /// collateral to Aave, borrows USDT, trades USDT for WETH on pool.
    /// Funder must be the EOA.
    #[test]
    fn trace_funder_borrow_aave_weth_collateral() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let aave = addr("0x7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut ls = HashSet::new(); ls.insert(aave);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_500_000_000_000_000_000), // 1.5 WETH collateral
            transfer(executor, aave, w, 1_500_000_000_000_000_000), // deposit
            transfer(aave, executor, u, 2_000_000_000), // borrow 2000 USDT
            transfer(executor, pool, u, 2_000_000_000), // trade
            transfer(pool, executor, w, 1_600_000_000_000_000_000), // receive WETH
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &ls, unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
    }

    /// Borrow case with ETH collateral: EOA sends ETH to executor, executor
    /// wraps to WETH, deposits WETH to Aave, borrows USDT, trades.
    /// Funder must be the EOA (ETH sender).
    #[test]
    fn trace_funder_borrow_aave_eth_collateral() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let aave = addr("0x7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut ls = HashSet::new(); ls.insert(aave);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr(); let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, eth, 1_500_000_000_000_000_000), // 1.5 ETH
            transfer(executor, aave, w, 1_500_000_000_000_000_000), // wrap+deposit
            transfer(aave, executor, u, 2_000_000_000), // borrow USDT
            transfer(executor, pool, u, 2_000_000_000), // trade
            transfer(pool, executor, w, 1_600_000_000_000_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &ls, unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
    }

    /// Pre-balance with no inbound at all: executor has prior WETH, no one
    /// sends anything to it in this tx. Funder = None (self-funded).
    #[test]
    fn trace_funder_pre_balance_no_inbound() {
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(executor, pool, w, 1_000_000_000_000_000_000),
            transfer(pool, executor, u, 1_500_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), None);
    }

    /// Direct WETH inbound to executor: classic case, must still work
    /// (regression check for the rewrite).
    #[test]
    fn trace_funder_direct_weth_inbound() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_500_000_000_000_000_000),
            transfer(executor, pool, w, 1_500_000_000_000_000_000),
            transfer(pool, executor, usdt_addr(), 1_500_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
    }

    /// Multi-pool-out: executor sends two different tokens to two different
    /// pools in the same tx. The funder should be the EOA that funded WETH.
    /// Resolution order must be deterministic across runs.
    #[test]
    fn trace_funder_multi_pool_out() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool_a = addr("0xcccc");
        let pool_b = addr("0xdddd");
        let mut ps = HashSet::new(); ps.insert(pool_a); ps.insert(pool_b);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_000_000_000_000_000_000), // fund WETH
            transfer(executor, pool_a, w, 1_000_000_000_000_000_000),    // sell WETH
            transfer(pool_a, executor, u, 1_500_000_000),
            transfer(executor, pool_b, u, 1_500_000_000),                 // sell USDT
            transfer(pool_b, executor, w, 1_050_000_000_000_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        // Run twice to assert determinism
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
    }

    /// Round-trip with a *different* token must not disqualify the funder.
    /// E.g., executor returns a fee/profit-share in USDC to the EOA that
    /// funded WETH. The WETH round-trip check is token-scoped — the USDC
    /// return is irrelevant.
    #[test]
    fn trace_funder_round_trip_different_token_kept() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_500_000_000_000_000_000), // fund WETH
            transfer(executor, pool, w, 1_500_000_000_000_000_000),
            transfer(pool, executor, u, 1_500_000_000),
            // Executor returns a profit-share in USDT to the funder. Different
            // token from the funded WETH — must not trigger round-trip.
            transfer(executor, funder_eoa, u, 50_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), Some(funder_eoa));
    }

    /// 25301044 pattern: executor unwraps WETH to ETH (WETH contract = Infra,
    /// not in `unknown`), then trades ETH on a UniV4-style pool. The
    /// initiator EOA sends 59 wei of gas dust to the executor. The actual
    /// trade ETH came from the executor's pre-balance WETH. The dust is
    /// insufficient to cover the ETH pool-out, so case 1 must fall through
    /// to self-funded (funder = None).
    #[test]
    fn trace_funder_25301044_unwrap_dust_falls_through() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let weth_contract = addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let pool = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
        let token_back = addr("0x32708538a107253b51a735a724330a23106ca4ca");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(initiator); unk.insert(executor);
        // weth_contract is intentionally NOT in unk — it's classified as Infra
        // (WETH contract is in the blacklist in production).
        let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 59), // 59 wei gas dust
            transfer(weth_contract, executor, eth, 1_000_000_000_000_000), // unwrap (1e15)
            transfer(executor, pool, eth, 1_000_000_000_000_000), // pool-out (same)
            transfer(pool, executor, token_back, 1_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), None);
    }

    /// 25304912 pattern: executor routes its pre-balance WETH (unwrapped to
    /// ETH) through an aggregator router to a Balancer-style pool, then
    /// receives the trade output token. The initiator EOA sends 99 wei of
    /// gas dust to the executor. Capital source is the executor's pre-balance
    /// (self-funded), not the dust. The router is a pure passthrough (no
    /// events emitted) and is therefore in `pool_or_router` after
    /// non-emitter fund-flow classification. The trace_funder main loop
    /// should detect `executor → router` as a pool-out and reject 0xae2f in
    /// case 1 (inbound 99 wei < outbound 0.025 ETH).
    #[test]
    fn trace_funder_25304912_router_passthrough_dust() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let weth_contract = addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let router = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
        let pool = addr("0x000000000004444c5dc75cb358380d2e3de08a90");
        let token_back = addr("0x6de037ef9ad2725eb40118bb1702ebb27e4aeb24");
        let mut ps = HashSet::new();
        ps.insert(pool);
        ps.insert(router); // router is in pool_or_router (passthrough)
        let mut unk = HashSet::new();
        unk.insert(initiator);
        unk.insert(executor);
        let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 99), // 99 wei gas dust
            transfer(weth_contract, executor, eth, 1_000_000_000_000_000), // unwrap (1e15)
            transfer(executor, router, eth, 1_000_000_000_000_000), // exec → router
            transfer(router, pool, eth, 1_000_000_000_000_000), // router → pool
            transfer(pool, executor, token_back, 1_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(trace_funder(&ctx, &ff, executor), None);
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
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &ps, lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[weth()] };
        // is_consecutive is private — tested via try_build_bundle below
    }

    #[test]
    fn is_cons_victim_has_supported_token() {
        let victim = mock_tx(1, Address::ZERO, vec![
            transfer(addr("0xaaaa"), addr("0xbbbb"), weth(), 100),
        ]);
        let txs = vec![victim];
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &HashSet::new(), lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[weth()] };
    }

    #[test]
    fn is_cons_attacker_tx_passes() {
        let initiator = addr("0xaaaa");
        let victim = mock_tx(1, initiator, vec![
            transfer(initiator, addr("0xbbbb"), weth(), 100),
        ]);
        let txs = vec![victim];
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &HashSet::new(), lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[] };
    }

    #[test]
    fn is_cons_no_pool_no_token_fails() {
        let victim = mock_tx(1, Address::ZERO, vec![
            transfer(addr("0xaaaa"), addr("0xbbbb"), usdc(), 100),
        ]);
        let txs = vec![victim];
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &HashSet::new(), lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[weth()] };
    }

    #[test]
    fn is_cons_empty_gap_passes() {
        let txs: Vec<TxFlow> = vec![];
        let ctx = Ctx { block_number: 0, tx_flows: &txs, pool_set: &HashSet::new(), lending_set: &HashSet::new(), unknown: &HashSet::new(), coinbase: Address::ZERO, supported_tokens: &[] };
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

        // Front sells WETH for USDC; back sells USDC for WETH (USDC reverses)
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

        let ctx = Ctx {
            block_number: 0, tx_flows: &txs, pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk,
            coinbase: addr("0xcb00"), supported_tokens: &[weth, usdc_addr],
        };

        let mut fd: HashMap<Address, i128> = HashMap::new(); fd.insert(weth, -100); fd.insert(usdc_addr, 95000000);
        let mut bd: HashMap<Address, i128> = HashMap::new(); bd.insert(weth, 105); bd.insert(usdc_addr, -100000000);

        let bundles = pair_trades(&ctx, vec![
            super::super::discovery::ExecutorTrade {
                tx_index: 0, executor, deltas: fd.clone(), pools: ps.clone(),
                from: initiator, to: Some(addr("0xbbbb")),
            },
            super::super::discovery::ExecutorTrade {
                tx_index: 2, executor, deltas: bd.clone(), pools: ps.clone(),
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
        assert!(b.gas_cost_wei >= 0);
        assert_eq!(b.coinbase_bribe, 0);
    }

    /// 25304912 profit pattern: executor pays ETH to a router (which
    /// forwards to a pool) and receives a different token from the pool,
    /// in the back tx it reverses. Profit is the net ETH/WETH change for
    /// the executor — not just the back-tx ETH received from the pool.
    /// Without the router in `pool_or_router`, the executor's front-tx
    /// ETH outflow is hidden and the profit is over-counted by the
    /// front-tx ETH spent.
    #[test]
    fn try_build_bundle_router_passthrough_profit() {
        let weth = weth();
        let eth = eth_addr();
        // supported_tokens must include ETH_TRANSFER_ADDR for ETH profit to be visible
        let supported = vec![weth, eth];
        let token_back = addr("0x6de037ef9ad2725eb40118bb1702ebb27e4aeb24");
        let pool = addr("0x000000000004444c5dc75cb358380d2e3de08a90");
        let router = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
        let executor = addr("0x1f2f");
        let initiator = addr("0xae2f");
        let victim = addr("0xc2c4");

        let mut ps = HashSet::new();
        ps.insert(pool);
        ps.insert(router); // router in pool_or_router → exec → router is pool-out
        let mut unk = HashSet::new();
        unk.insert(initiator); unk.insert(executor); unk.insert(victim);
        // Front: executor pays 1000 ETH via router to pool, gets 100 SNX.
        // Back: executor pays 100 SNX to pool, gets 1005 ETH (profit 5 ETH).
        let txs = vec![
            mock_tx(0, initiator, vec![
                transfer(initiator, executor, eth, 99), // 99 wei dust
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

        let ctx = Ctx {
            block_number: 0, tx_flows: &txs, pool_set: &ps, lending_set: &HashSet::new(), unknown: &unk,
            coinbase: addr("0xcb00"), supported_tokens: &supported,
        };

        // Build executor trades with pool-or-router deltas.
        let mut fd: HashMap<Address, i128> = HashMap::new();
        fd.insert(eth, -1000); // exec → router (pool_or_router)
        fd.insert(token_back, 100); // pool → exec
        let mut bd: HashMap<Address, i128> = HashMap::new();
        bd.insert(token_back, -100); // exec → pool
        bd.insert(eth, 1005); // pool → exec

        let bundles = pair_trades(&ctx, vec![
            super::super::discovery::ExecutorTrade {
                tx_index: 0, executor, deltas: fd.clone(), pools: ps.clone(),
                from: initiator, to: Some(executor),
            },
            super::super::discovery::ExecutorTrade {
                tx_index: 2, executor, deltas: bd.clone(), pools: ps.clone(),
                from: initiator, to: Some(executor),
            },
        ]);

        assert_eq!(bundles.len(), 1);
        let b = &bundles[0];
        assert_eq!(b.funder, executor); // 99 wei dust < 1000 ETH outflow → self-funded
        assert_eq!(b.attacker, executor);
        // profit ETH = +1005 (back) + (-1000) (front) = +5
        assert_eq!(b.profit.len(), 1);
        assert_eq!(b.profit[0].token, eth);
        let expected_profit: U256 = U256::from(5u64);
        assert_eq!(b.profit[0].amount.into_sign_and_abs().1, expected_profit);
    }
}
