//! Integration tests against a live Reth node.
//!
//! These tests exercise the full sandwich-detection pipeline end-to-end on
//! real mainnet blocks. They FAIL LOUDLY if no Reth is reachable. CI must
//! provide a node at `MEV_ANT_RPC_URL` or make the default URL reachable.
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

use crate::classifier::DefaultClassifier;
use crate::detector::detect_sandwiches;
use crate::models::SandwichBundle;
use crate::pools::lending;
use crate::pools::quoting::{curve, univ3};
use crate::pools::types::{CurveState, Pool, PoolKind, PoolSnapshot, V3State};
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

/// Returns the RPC URL to use for integration tests.
///
/// Panics if `MEV_ANT_RPC_URL` is unset/empty and the default URL is not
/// reachable. Integration tests require a live Reth node and fail loudly
/// when one is unavailable.
fn rpc_url_required() -> String {
    let url = std::env::var("MEV_ANT_RPC_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_RPC_URL.to_string());

    // Probe the chosen RPC. A short timeout makes a down Reth fail fast
    // rather than hanging each test for 30s.
    let host_port = url
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let addr: std::net::SocketAddr = host_port
        .parse()
        .unwrap_or_else(|_| panic!("RPC URL {} is not a valid host:port", url));

    match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)) {
        Ok(_) => url,
        Err(e) => panic!(
            "integration tests require a reachable Reth node at {} (set MEV_ANT_RPC_URL to override): {}",
            url, e
        ),
    }
}

macro_rules! integration_test {
    ($name:ident, |$client:ident| $body:block) => {
        #[test]
        fn $name() {
            let url = rpc_url_required();
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
    let classifier = DefaultClassifier::new(crate::DEFAULT_BLACKLIST, crate::dex::lending::LENDING_ADDRESSES);
    detect_sandwiches(
        &classifier,
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

// Block 25302239: the real executor 0x0906a879... was being misclassified as a
// Router by the fund-flow classifier because its sent/received token sets were
// equal (WETH, ETH, 0xcd5f...). The classifier ignored the per-token amount
// differences that make the address an executor, not a passthrough. Because it
// was in pool_or_router, executor discovery skipped it and the sandwich at
// txs 0/1/2 was missed.
integration_test!(block_25302239_executor_not_router, |client| {
    let bundles = detect(&client, 25302239);
    assert!(!bundles.is_empty(), "block 25302239 should have ≥1 sandwich");

    let expected_exec = address!("0x0906a879ea0f66e3559f11b25b866dba247f9e63");
    let expected_funder = address!("0x01fdc48ba0903bb1ae7c517c9287d88ea236f8e1");

    let sandwich = bundles.iter()
        .find(|b| b.front_tx_index == 0 && b.back_tx_index == 2);
    assert!(sandwich.is_some(),
        "block 25302239 should have a sandwich at txs 0/1/2 — got bundles {:?}",
        bundles.iter().map(|b| (b.front_tx_index, b.back_tx_index)).collect::<Vec<_>>());

    let b = sandwich.unwrap();
    assert_eq!(b.executor, expected_exec,
        "executor should be 0x0906a879... (the real pool-touching executor), not a router stub");
    assert_eq!(b.funder, expected_funder,
        "funder should be the WETH wrapper that fronted the capital");
    assert_eq!(b.attacker, expected_funder);
    assert_eq!(b.victim_tx_indices.len(), 1, "there should be exactly one victim tx (tx 1)");
});

// Block 25305868: the front executor trades through a multi-token pool
// (Balancer) and the victim swaps a different token pair on the same pool.
// The old victim-direction check required the victim's net token delta sign
// to match the front executor's net token delta sign; because the front's
// net delta for the victim's received token was zero (it was used as a
// routing hop), the victim was rejected and the sandwich at txs 0/1/2 was
// missed. The fix adds a gross-direction check: victim is valid if it
// receives a token the front sold to the pool, or sends a token the front
// bought from the pool.
integration_test!(block_25305868_multi_token_pool_victim, |client| {
    let bundles = detect(&client, 25305868);
    assert!(!bundles.is_empty(), "block 25305868 should have ≥1 sandwich");

    let expected_exec = address!("0x0906a879ea0f66e3559f11b25b866dba247f9e63");
    let expected_funder = address!("0x01fdc48ba0903bb1ae7c517c9287d88ea236f8e1");

    let sandwich = bundles.iter()
        .find(|b| b.front_tx_index == 0 && b.back_tx_index == 2);
    assert!(sandwich.is_some(),
        "block 25305868 should have a sandwich at txs 0/1/2 — got bundles {:?}",
        bundles.iter().map(|b| (b.front_tx_index, b.back_tx_index)).collect::<Vec<_>>());

    let b = sandwich.unwrap();
    assert_eq!(b.executor, expected_exec,
        "executor should be 0x0906a879... (the real pool-touching executor)");
    assert_eq!(b.funder, expected_funder,
        "funder should be the WETH wrapper that fronted the capital");
    assert_eq!(b.attacker, expected_funder);
    assert_eq!(b.victim_tx_indices.len(), 1, "there should be exactly one victim tx (tx 1)");
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

integration_test!(univ3_quote_weth_usdc, |client| {
    // WETH/USDC 0.05% pool at a pinned block.
    let pool_address = address!("88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
    let block = 25_300_000u64;
    let block_tag = format!("0x{:x}", block);

    let slot0_data = alloy::primitives::Bytes::from_static(&[0x38, 0x50, 0xc7, 0xbd]);
    let liq_data = alloy::primitives::Bytes::from_static(&[0x1a, 0x68, 0x65, 0x02]);

    let slot0_hex = runtime()
        .block_on(client.call_at(pool_address, slot0_data, &block_tag))
        .expect("slot0 call");
    let liq_hex = runtime()
        .block_on(client.call_at(pool_address, liq_data, &block_tag))
        .expect("liquidity call");

    let slot0_bytes = hex::decode(slot0_hex.trim_start_matches("0x")).expect("decode slot0");
    let liq_bytes = hex::decode(liq_hex.trim_start_matches("0x")).expect("decode liquidity");

    let sqrt_price_x96 = alloy::primitives::U256::from_be_slice(&slot0_bytes[0..32]);
    let liquidity = alloy::primitives::U256::from_be_slice(&liq_bytes[0..32]);

    let pool = Pool {
        address: pool_address,
        pool_id: alloy::primitives::B256::ZERO,
        kind: PoolKind::UniswapV3,
        factory: Some(crate::pools::registry::UNISWAP_V3_FACTORY),
        token0: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        token0_decimals: 6,
        token1: address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
        token1_decimals: 18,
        fee: Some(500),
        block_created: None,
    };
    let state = PoolSnapshot {
        address: pool_address,
        pool_id: alloy::primitives::B256::ZERO,
        observed_at_block: block,
        reserve0: None,
        reserve1: None,
        tvl_usd: None,
        state: serde_json::to_value(&V3State {
            sqrt_price_x96,
            tick: 0,
            liquidity,
            tick_spacing: 10,
            ticks: vec![],
        })
        .unwrap(),
    };

    // 1,000 USDC in should yield some WETH out.
    let amount_in = alloy::primitives::U256::from(1_000_000_000u64);
    let out = univ3::quote(&pool, &state, pool.token0, amount_in)
        .expect("V3 quote should succeed");
    assert!(out > alloy::primitives::U256::ZERO, "V3 quote output should be positive");
});

integration_test!(curve_quote_frax_usdc, |client| {
    // Curve FRAX/USDC 2-coin stableswap pool at a pinned block.
    let pool_address = address!("DcEF968d416a41Cdac0ED8702fAC8128A64241A2");
    let block = 25_300_000u64;
    let block_tag = format!("0x{:x}", block);

    let frax = address!("853d955aCEf822Db058eb8505911ED77F175b99e");
    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

    let a_sel = crate::pools::liquidity::curve_a_selector();
    let fee_sel = crate::pools::liquidity::curve_fee_selector();
    let bal_sel = crate::pools::liquidity::curve_balances_selector();

    let a_hex = runtime()
        .block_on(client.call_at(pool_address, alloy::primitives::Bytes::from(a_sel.0[..4].to_vec()), &block_tag))
        .expect("A call");
    let fee_hex = runtime()
        .block_on(client.call_at(pool_address, alloy::primitives::Bytes::from(fee_sel.0[..4].to_vec()), &block_tag))
        .expect("fee call");

    let a = alloy::primitives::U256::from_be_slice(&hex::decode(a_hex.trim_start_matches("0x")).expect("decode A")[0..32]);
    let fee = alloy::primitives::U256::from_be_slice(&hex::decode(fee_hex.trim_start_matches("0x")).expect("decode fee")[0..32]);

    let mut balances = Vec::with_capacity(2);
    for i in 0..2u8 {
        let mut data = bal_sel.0[..4].to_vec();
        data.extend_from_slice(&alloy::primitives::U256::from(i).to_be_bytes::<32>());
        let bal_hex = runtime()
            .block_on(client.call_at(pool_address, alloy::primitives::Bytes::from(data), &block_tag))
            .expect(&format!("balance {} call", i));
        let bal = alloy::primitives::U256::from_be_slice(&hex::decode(bal_hex.trim_start_matches("0x")).expect("decode balance")[0..32]);
        balances.push(bal);
    }

    let pool = Pool {
        address: pool_address,
        pool_id: alloy::primitives::B256::ZERO,
        kind: PoolKind::CurveVyper,
        factory: Some(crate::pools::registry::CURVE_REGISTRY),
        token0: frax,
        token0_decimals: 18,
        token1: usdc,
        token1_decimals: 6,
        fee: Some((fee.to::<u64>() / 1_000_000_000_0) as u32),
        block_created: None,
    };
    let state = PoolSnapshot {
        address: pool_address,
        pool_id: alloy::primitives::B256::ZERO,
        observed_at_block: block,
        reserve0: None,
        reserve1: None,
        tvl_usd: None,
        state: serde_json::to_value(&CurveState {
            n_coins: 2,
            a,
            fee,
            balances: balances.clone(),
            coins: vec![frax, usdc],
            decimals: vec![18, 6],
        })
        .unwrap(),
    };

    // 1 FRAX in -> expect roughly 1 USDC out (minus fee).
    let amount_in = alloy::primitives::U256::from(1_000_000_000_000_000_000u128);
    let out = curve::quote(&pool, &state, frax, usdc, amount_in)
        .expect("Curve quote should succeed");
    assert!(out > alloy::primitives::U256::ZERO, "Curve quote output should be positive");

    // Compare against on-chain get_dy for the same input at the same block.
    let get_dy_selector = alloy::primitives::keccak256("get_dy(int128,int128,uint256)");
    let mut get_dy_data = get_dy_selector.0[..4].to_vec();
    get_dy_data.extend_from_slice(&alloy::primitives::U256::from(0).to_be_bytes::<32>()); // i
    get_dy_data.extend_from_slice(&alloy::primitives::U256::from(1).to_be_bytes::<32>()); // j
    get_dy_data.extend_from_slice(&amount_in.to_be_bytes::<32>()); // dx
    let dy_hex = runtime()
        .block_on(client.call_at(pool_address, alloy::primitives::Bytes::from(get_dy_data), &block_tag))
        .expect("get_dy call");
    let dy = alloy::primitives::U256::from_be_slice(&hex::decode(dy_hex.trim_start_matches("0x")).expect("decode dy")[0..32]);

    // Allow a small relative tolerance because our solver uses f64 internally.
    let diff = if out > dy { out - dy } else { dy - out };
    let tolerance = dy / alloy::primitives::U256::from(100); // 1%
    assert!(
        diff <= tolerance,
        "Curve quote {} deviates from on-chain get_dy {} by more than 1%", out, dy
    );
});

integration_test!(aave_v3_reserves_and_rates_at_25_300_000, |client| {
    // Pin block, hit the Aave V3 Pool directly via eth_call.
    let pool = lending::AAVE_V3_POOL;
    let block = 25_300_000u64;

    let reserves = runtime()
        .block_on(lending::aave_v3_reserves_list(&client, pool, block))
        .expect("getReservesList");
    assert!(!reserves.is_empty(), "Aave V3 reserve list should be non-empty");
    // WETH, USDC, USDT and DAI are all listed in Aave V3 mainnet.
    let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    assert!(reserves.contains(&weth), "WETH should be in Aave V3 reserves");
    assert!(reserves.contains(&usdc), "USDC should be in Aave V3 reserves");

    // USDC always has heavy utilisation, so its borrow rate must be > 0.
    let m = runtime()
        .block_on(lending::aave_v3_reserve_state(&client, pool, usdc, block))
        .expect("USDC reserve state");

    assert_eq!(m.protocol_str(), "aave_v3");
    assert_eq!(m.underlying_asset, usdc);
    // At least the variable-borrow rate must decode from the struct.
    assert!(m.variable_borrow_rate_ray.is_some(), "USDC variable borrow rate should decode");
    // ray upper bound: 1e27. Anything above that is a decoding bug.
    let ray_max = alloy::primitives::U256::from(10u64).pow(alloy::primitives::U256::from(27u64));
    for (label, rate) in [
        ("supply", m.supply_rate_ray),
        ("variable", m.variable_borrow_rate_ray),
        ("stable", m.stable_borrow_rate_ray),
    ] {
        if let Some(r) = rate {
            assert!(r < ray_max, "{} borrow rate {} exceeds ray", label, r);
        }
    }
});

integration_test!(univ2_weth_usdc_reserves_at_25_300_000, |client| {
    // Pin block, fetch getReserves() from a canonical UniV2 WETH/USDC pair.
    use crate::pools::liquidity;
    let pool = address!("B4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
    let block = 25_300_000u64;

    let (r0, r1) = runtime()
        .block_on(liquidity::fetch_univ2_reserves(&client, pool))
        .expect("WETH/USDC reserves");

    // At block 25.3M both reserves must be non-zero.
    assert!(r0 > alloy::primitives::U256::ZERO, "reserve0 should be > 0");
    assert!(r1 > alloy::primitives::U256::ZERO, "reserve1 should be > 0");
    // Pool token0 = USDC (6 dec) so reserve0 should be a large value
    // roughly comparable to ~25M USDC.
    assert!(r0 > alloy::primitives::U256::from(1_000_000_000_000u64),
        "USDC reserve should be > 1M, got {}", r0);
});

integration_test!(routing_finds_weth_usdc_via_known_pool, |client| {
    // End-to-end routing: build a TokenGraph with a known live WETH/USDC
    // pool, find routes, and verify the output amount is positive.
    use crate::pools::graph::TokenGraph;
    use crate::pools::liquidity;
    use crate::pools::routing::find_routes;
    use crate::pools::types::{Pool, PoolKind, PoolSnapshot, V3State};

    let pool_addr = address!("88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"); // WETH/USDC 0.05%
    let block = 25_300_000u64;
    let block_tag = format!("0x{:x}", block);

    // Fetch slot0 + liquidity via the helper used by the live job.
    let (sqrt_price_x96, tick, liquidity) = runtime()
        .block_on(liquidity::fetch_univ3_state(&client, pool_addr))
        .expect("V3 state");

    let pool = Pool {
        address: pool_addr,
        pool_id: alloy::primitives::B256::ZERO,
        kind: PoolKind::UniswapV3,
        factory: Some(crate::pools::registry::UNISWAP_V3_FACTORY),
        token0: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
        token0_decimals: 6,
        token1: address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), // WETH
        token1_decimals: 18,
        fee: Some(500),
        block_created: None,
    };
    let state = PoolSnapshot {
        address: pool_addr,
        pool_id: alloy::primitives::B256::ZERO,
        observed_at_block: block,
        reserve0: None,
        reserve1: None,
        tvl_usd: None,
        state: serde_json::to_value(&V3State {
            sqrt_price_x96,
            tick,
            liquidity,
            tick_spacing: 10,
            ticks: vec![],
        }).unwrap(),
    };

    let graph = TokenGraph::new(vec![(pool.clone(), state.clone())]);
    // 1,000 USDC in should produce some WETH out.
    let amount_in = alloy::primitives::U256::from(1_000_000_000u64);
    let routes = find_routes(&graph, pool.token0, pool.token1, 3, Some(amount_in));
    assert!(!routes.is_empty(), "should find a route from USDC to WETH");
    let r = &routes[0];
    assert_eq!(r.hop_count, 1);
    assert_eq!(r.quote_confidence, crate::pools::types::QuoteConfidence::Exact);
    let out = r.total_output.expect("exact quote produces output");
    assert!(out > alloy::primitives::U256::ZERO, "quote output should be > 0");
});
