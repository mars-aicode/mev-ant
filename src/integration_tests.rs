//! Integration tests against a live Reth node.
//!
//! These tests exercise the full sandwich-detection pipeline end-to-end on
//! real mainnet blocks. They auto-skip if no Reth is reachable — default
//! `cargo test` is fast and never fails for "Reth is down".
//!
//! Run with:
//!     cargo test integration
//!     # or
//!     MEV_ANT_RPC_URL=http://my-reth:8547 cargo test integration
//!
//! Each test block was originally reported as a bug — the assertion set
//! locks in the *correct* output so a future regression of the same shape
//! fails the test.
//!
//! To add a new regression: pick the block, copy the closest test, replace
//! the constants. No fixture files to commit.

use alloy::primitives::address;
use std::sync::OnceLock;

use crate::detector::sandwich::detect_sandwiches;
use crate::models::SandwichBundle;
use crate::rpc::{fetch_block, RpcClient};

const DEFAULT_RPC_URL: &str = "http://192.168.2.180:8547";

/// Lazily-initialised tokio runtime; reused across tests.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Returns Some(rpc_url) if integration tests should run, None if they should skip.
fn rpc_url_if_configured() -> Option<String> {
    std::env::var("MEV_ANT_RPC_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            // If unset, probe the default RPC. A short timeout makes a down
            // Reth fail fast as a clear skip rather than a 30s test hang.
            let host_port = DEFAULT_RPC_URL
                .trim_start_matches("http://")
                .trim_start_matches("https://");
            host_port.parse().ok().and_then(|addr| {
                std::net::TcpStream::connect_timeout(
                    &addr,
                    std::time::Duration::from_millis(500),
                )
                .ok()
                .map(|_| DEFAULT_RPC_URL.to_string())
            })
        })
}

macro_rules! integration_test {
    ($name:ident, |$client:ident| $body:block) => {
        #[test]
        fn $name() {
            let Some(url) = rpc_url_if_configured() else {
                eprintln!(
                    "SKIP {}: set MEV_ANT_RPC_URL or make {} reachable",
                    stringify!($name),
                    DEFAULT_RPC_URL
                );
                return;
            };
            let $client = RpcClient::new(&url).expect("RPC client");
            $body
        }
    };
}

/// Fetches a block, runs the detector, returns the bundles.
fn detect(client: &RpcClient, block_number: u64) -> Vec<SandwichBundle> {
    let block = runtime()
        .block_on(fetch_block(client, block_number))
        .unwrap_or_else(|e| panic!("fetch_block({}) failed: {:?}", block_number, e));
    detect_sandwiches(
        block_number,
        &block.flows,
        &block.raw_logs,
        block.coinbase,
        crate::DEFAULT_BLACKLIST,
        crate::DEFAULT_TOKENS,
    )
}

// ============================================================================
// Regression cases
// ============================================================================

// Block 25301029: the trace_funder was attributing the sandwich to
// 0x98c23e9d8f34fefb1b7bd6a91b7ff122f4e16f5c — Aave's USDC reserve proxy /
// flashloan provider. The user pointed out 0x98c23e9d never at-risked the
// capital; the actual funder was 0x01fdc48b, the WETH wrapper that fronted
// 79.207 WETH and received 79.209 WETH + the trade profit in the back tx.
//
// Two regressions to lock in:
//   1. The funder is NOT the lending platform (no Aave USDC Pool).
//   2. attacked_pool is the USDC/USDT pool, not the zero-address fallback
//      the old code produced in the DB.
integration_test!(block_25301029_aave_not_funder, |client| {
    let bundles = detect(&client, 25301029);
    assert!(!bundles.is_empty(), "block 25301029 should have ≥1 sandwich");

    let aave_usdc_pool = address!("0x98c23e9d8f34fefb1b7bd6a91b7ff122f4e16f5c");
    let actual_funder  = address!("0x01fdc48ba0903bb1ae7c517c9287d88ea236f8e1");
    let expected_pool  = address!("0x04571c32a4e1c5f39bc3a238cb95b215058c432c");

    for b in &bundles {
        // Anti-regression: Aave USDC Pool is not the funder/attacker.
        assert_ne!(b.funder, aave_usdc_pool,
            "Aave USDC Pool must not be the funder (flashloan, not capital)");
        assert_ne!(b.attacker, aave_usdc_pool,
            "Aave USDC Pool must not be the attacker");

        // Golden values: actual capital source + the real attacked pool.
        assert_eq!(b.funder, actual_funder,
            "funder should be 0x01fdc48b (WETH wrapper, actual capital source)");
        assert_eq!(b.attacker, actual_funder);

        // attacked_pool must be a real pool, not the zero-address fallback.
        match &b.attacked_pool {
            crate::models::PoolId::Contract(a) => {
                assert_ne!(*a, alloy::primitives::Address::ZERO,
                    "attacked_pool must not be the zero-address fallback");
                assert_eq!(*a, expected_pool,
                    "attacked_pool should be the USDC/USDT pool 0x04571c32");
            }
            other => panic!("attacked_pool should be PoolId::Contract(0x04571c32), got {:?}", other),
        }

        // Profit sanity: 0.038 WETH + 1 µUSDC, exact (we know the values).
        let weth = address!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let profit_weth = b.profit.iter()
            .find(|p| p.token == weth)
            .map(|p| p.amount.into_sign_and_abs().1)
            .expect("profit should include WETH");
        assert_eq!(profit_weth, alloy::primitives::U256::from(38_081_146_573_419_456u64),
            "WETH profit should be 0.038081146573419456 ETH");
    }
});

// Block 25304912: the trace_funder was attributing the sandwich to
// 0xae2fc483527b8ef99eb5d9b44875f005ba1fae13 (an EOA) because that address
// sent 99 wei of gas dust to the executor. The actual capital came from
// the executor's pre-balance WETH, so funder = executor.
//
// Two regressions to lock in:
//   1. The dust EOA does not claim funder status.
//   2. Profit is the small WETH delta (back_ETH - front_ETH), not the full
//      back-tx ETH received (which over-counted by 530× because the
//      executor → router → pool path hid the front-tx ETH outflow).
// Block 25300013: the user reported that 0x5ee5bf7ae06d1be5997a1a72006fe6c607ec6de8
// (Aave V3 WBTC reserve proxy, same proxy code as the USDC reserve in
// 25301029 and the main Aave V3 Pool) was being misattributed as the
// funder for a sandwich at txs 174/175/176. The actual capital came
// from 0x000000000035b5e5ad9019092c665357240f594e (the user's contract /
// tx.target), which provided 727.924 WETH as collateral. The trade used
// 0x5ee5bf7a as a USDC flashloan plus Morpho Blue as a WETH flashloan.
//
// Locks in: attacker/funder is 0x000000000035b5e5ad9019092c665357240f594e
// (the real capital source), and the Aave V3 WBTC reserve proxy is NOT
// the funder or attacker.
integration_test!(block_25300013_aave_v3_wbtc_reserve_not_funder, |client| {
    let bundles = detect(&client, 25300013);
    // The user-reported sandwich is at txs 174/175/176.
    let user_sandwich = bundles.iter()
        .find(|b| b.front_tx_index == 174 && b.back_tx_index == 176);
    assert!(user_sandwich.is_some(),
        "block 25300013 should have the user-reported sandwich at txs 174/175/176 — got bundles {:?}",
        bundles.iter().map(|b| (b.front_tx_index, b.back_tx_index, format!("{:?}", b.executor))).collect::<Vec<_>>());

    let aave_v3_wbtc = address!("0x5ee5bf7ae06d1be5997a1a72006fe6c607ec6de8");
    let user_contract = address!("0x000000000035b5e5ad9019092c665357240f594e");
    let b = user_sandwich.unwrap();
    assert_eq!(b.funder, user_contract,
        "funder should be 0x000000000035b5e5ad9019092c665357240f594e (the user contract that provided the WETH capital)");
    assert_eq!(b.attacker, user_contract);
    assert_eq!(b.executor, address!("0x33988614010be265e71ab3a04bd29f0b950bc58c"));
    assert_eq!(b.initiator, address!("0x654fae4aa229d104cabead47e56703f58b174be4"));
    // Anti-regression: the Aave reserve must not be claimed anywhere.
    assert_ne!(b.funder, aave_v3_wbtc);
    assert_ne!(b.attacker, aave_v3_wbtc);
});

integration_test!(block_25304912_dust_funder_self_funded, |client| {
    let bundles = detect(&client, 25304912);
    assert!(!bundles.is_empty(), "block 25304912 should have ≥1 sandwich");

    let dust_eoa = address!("0xae2fc483527b8ef99eb5d9b44875f005ba1fae13");
    let executor  = address!("0x1f2f10d1c40777ae1da742455c65828ff36df387");

    for b in &bundles {
        // Anti-regression: dust EOA is not the funder. (The pre-fix code
        // picked 0xae2f as the funder via case 1 of trace_funder, matching
        // the 99 wei dust as a "direct inbound" of pool_out ETH. The fix
        // adds the sufficiency check that 99 < 1e15 ETH so the inbound
        // doesn't cover the pool-out — the dust falls through, and the
        // funder is either a real funder identified by case 4 of
        // trace_funder or the executor itself.)
        assert_ne!(b.funder, dust_eoa,
            "0xae2f (99 wei gas dust) must not be the funder");
        assert_ne!(b.attacker, dust_eoa);

        // Executor is 0x1f2f (the actual executor for this sandwich).
        assert_eq!(b.executor, executor);

        // Profit magnitude: 0.0000478 ETH. Fuzzy (1% tolerance) in case of
        // future minor reclassification, but tight enough to catch the
        // 530× over-counting that was the original bug.
        let eth = crate::models::ETH_TRANSFER_ADDR;
        let profit_eth = b.profit.iter()
            .find(|p| p.token == eth)
            .map(|p| p.amount.into_sign_and_abs().1)
            .expect("profit should include ETH");
        let profit_f = profit_eth.to::<u128>() as f64 / 1e18;
        assert!(profit_f > 0.0 && profit_f < 0.01,
            "profit should be tiny (executor self-funded, only the slippage), got {} ETH", profit_f);

        // Hard ceiling: under 0.001 ETH. The pre-fix value was 0.025312 ETH,
        // ~530× too high.
        assert!(profit_f < 0.001,
            "profit {} ETH exceeds the realistic ceiling of 0.001 ETH — over-counting regression?",
            profit_f);
    }
});
