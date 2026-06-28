//! Multi-hop route discovery.

use std::cmp::Ordering;
use std::collections::HashSet;

use alloy::primitives::{Address, U256};

use crate::pools::graph::{Edge, TokenGraph};
use crate::pools::quoting::quote_exact_output;
use crate::pools::types::{QuoteConfidence, Route, RouteSortMode};

/// Intermediate-token whitelist for V1.
const INTERMEDIATE_WHITELIST: &[Address] = &[
    alloy::primitives::address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), // WETH
    alloy::primitives::address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"), // WBTC
    alloy::primitives::address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
    alloy::primitives::address!("dAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
    alloy::primitives::address!("6B175474E89094C44Da98b954EedeAC495271d0F"), // DAI
    alloy::primitives::address!("853d955aCEf822Db058eb8505911ED77F175b99e"), // FRAX
    alloy::primitives::address!("f939E0A03FB07F59A73314E73794Be0E57ac1b4E"), // crvUSD
    alloy::primitives::address!("4c9EDD5852cd905f086C759E8383e09bff1E68B3"), // USDe
    alloy::primitives::address!("40D16FC0246aD3160Ccc09B8D0D3A2cD28aE6C2f"), // GHO
];

fn is_intermediate_allowed(token: Address) -> bool {
    INTERMEDIATE_WHITELIST.iter().any(|a| *a == token)
}

/// Find all simple routes from `from` to `to` with at most `max_hops` hops.
/// Routes are sorted by the default (Output) mode.
pub fn find_routes(
    graph: &TokenGraph,
    from: Address,
    to: Address,
    max_hops: usize,
    amount_in: Option<U256>,
) -> Vec<Route> {
    let mut routes = Vec::new();
    let mut path: Vec<Edge> = Vec::new();
    let mut visited: HashSet<Address> = HashSet::new();
    visited.insert(from);

    dfs(graph, from, to, max_hops, &mut path, &mut visited, &mut routes, amount_in);
    sort_routes(&mut routes, RouteSortMode::default());
    routes
}

/// Sort routes by a caller-chosen primary key, falling back to the B-variant
/// order (output > fee > TVL > confidence > hops) for secondary comparison.
pub fn sort_routes(routes: &mut Vec<Route>, mode: RouteSortMode) {
    routes.sort_by(|a, b| {
        primary_cmp(a, b, mode)
            .then_with(|| b.total_output.cmp(&a.total_output))
            .then_with(|| a.total_fee_bps.cmp(&b.total_fee_bps))
            .then_with(|| {
                b.min_pool_tvl_usd
                    .partial_cmp(&a.min_pool_tvl_usd)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| a.quote_confidence.cmp(&b.quote_confidence))
            .then_with(|| a.hop_count.cmp(&b.hop_count))
    });
}

fn primary_cmp(a: &Route, b: &Route, mode: RouteSortMode) -> Ordering {
    match mode {
        RouteSortMode::Output => b.total_output.cmp(&a.total_output),
        RouteSortMode::Fee => a.total_fee_bps.cmp(&b.total_fee_bps),
        RouteSortMode::Tvl => b.min_pool_tvl_usd
            .partial_cmp(&a.min_pool_tvl_usd)
            .unwrap_or(Ordering::Equal),
        RouteSortMode::Confidence => a.quote_confidence.cmp(&b.quote_confidence),
        RouteSortMode::Hops => a.hop_count.cmp(&b.hop_count),
    }
}

fn dfs(
    graph: &TokenGraph,
    current: Address,
    target: Address,
    max_hops: usize,
    path: &mut Vec<Edge>,
    visited: &mut HashSet<Address>,
    routes: &mut Vec<Route>,
    amount_in: Option<U256>,
) {
    if current == target && !path.is_empty() {
        if let Some(route) = build_route(path, amount_in) {
            routes.push(route);
        }
        return;
    }

    if path.len() >= max_hops {
        return;
    }

    for edge in graph.edges_from(current) {
        let next = edge.token_out;

        // Avoid cycles.
        if visited.contains(&next) {
            continue;
        }

        // Intermediate tokens must be in the whitelist (except the target itself).
        if next != target && !path.is_empty() && !is_intermediate_allowed(next) {
            continue;
        }

        visited.insert(next);
        path.push(edge.clone());

        dfs(graph, next, target, max_hops, path, visited, routes, amount_in);

        path.pop();
        visited.remove(&next);
    }
}

fn build_route(path: &[Edge], amount_in: Option<U256>) -> Option<Route> {
    let hops: Vec<_> = path.iter().map(|e| e.to_hop()).collect();
    let hop_count = hops.len();
    let total_fee_bps = hops.iter().map(|h| h.fee as u64).sum();
    let min_pool_tvl_usd = path
        .iter()
        .map(|e| e.state.tvl_usd.unwrap_or(0.0))
        .fold(f64::INFINITY, f64::min);

    let (total_output, quote_confidence) = if let Some(amount_in) = amount_in {
        let mut amount = amount_in;
        let mut all_exact = true;
        let mut has_output = true;
        for edge in path {
            if let Some(out) = quote_exact_output(&edge.pool, &edge.state, edge.token_in, amount) {
                amount = out;
                if !is_exact_kind(edge.pool.kind) {
                    all_exact = false;
                }
            } else {
                all_exact = false;
                has_output = false;
                break;
            }
        }
        (
            if has_output { Some(amount) } else { None },
            if all_exact {
                QuoteConfidence::Exact
            } else {
                QuoteConfidence::Estimated
            },
        )
    } else {
        (None, QuoteConfidence::Estimated)
    };

    Some(Route {
        hops,
        hop_count,
        total_fee_bps,
        total_output,
        min_pool_tvl_usd,
        quote_confidence,
    })
}

fn is_exact_kind(kind: crate::pools::types::PoolKind) -> bool {
    use crate::pools::types::PoolKind;
    matches!(
        kind,
        PoolKind::UniswapV2
            | PoolKind::UniswapV3
            | PoolKind::CurveVyper
            | PoolKind::CurveRouter
            // UniV2 forks: same k=x*y math, same quoter applies.
            | PoolKind::FraxSwap
            // UniV3 forks: same concentrated-liquidity math, same quoter applies.
            | PoolKind::PancakeV3
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, B256, U256};

    use crate::pools::types::{Pool, PoolKind, PoolSnapshot};

    fn mk_pool(a: Address, b: Address, tvl: f64) -> (Pool, PoolSnapshot) {
        let pool = Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: a,
            token0_decimals: 18,
            token1: b,
            token1_decimals: 18,
            fee: Some(30),
            block_created: None,
        };
        let state = PoolSnapshot {
            address: pool.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1000)),
            reserve1: Some(U256::from(1000)),
            tvl_usd: Some(tvl),
            state: serde_json::json!({}),
        };
        (pool, state)
    }

    #[test]
    fn direct_route_found() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let graph = TokenGraph::new(vec![mk_pool(weth, usdc, 1_000_000.0)]);
        let routes = find_routes(&graph, weth, usdc, 3, Some(U256::from(1)));
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].hop_count, 1);
    }

    #[test]
    fn two_hop_route_via_whitelist() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let graph = TokenGraph::new(vec![
            mk_pool(weth, usdc, 1_000_000.0),
            mk_pool(usdc, dai, 500_000.0),
        ]);
        let routes = find_routes(&graph, weth, dai, 3, Some(U256::from(1)));
        assert!(routes.iter().any(|r| r.hop_count == 2));
    }

    #[test]
    fn cycle_prevented() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let graph = TokenGraph::new(vec![mk_pool(weth, usdc, 1_000_000.0)]);
        let routes = find_routes(&graph, weth, weth, 3, Some(U256::from(1)));
        // A cycle WETH->USDC->WETH is prevented because visited tokens are not revisited.
        assert!(routes.is_empty());
    }

    fn mk_pool_with_kind(a: Address, b: Address, tvl: f64, kind: PoolKind) -> (Pool, PoolSnapshot) {
        let pool = Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind,
            factory: None,
            token0: a,
            token0_decimals: 18,
            token1: b,
            token1_decimals: 18,
            fee: Some(30),
            block_created: None,
        };
        let state = PoolSnapshot {
            address: pool.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1000)),
            reserve1: Some(U256::from(1000)),
            tvl_usd: Some(tvl),
            state: serde_json::json!({}),
        };
        (pool, state)
    }

    #[test]
    fn fraxswap_pool_is_exact_quoted() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let graph = TokenGraph::new(vec![mk_pool_with_kind(weth, usdc, 1_000_000.0, PoolKind::FraxSwap)]);
        let routes = find_routes(&graph, weth, usdc, 3, Some(U256::from(1_000_000_000_000_000_000u128)));
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].quote_confidence, QuoteConfidence::Exact);
        assert!(routes[0].total_output.is_some());
    }

    #[test]
    fn fluid_pool_is_estimated() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        // Fluid pools have TVL None; they still appear in the graph.
        let pool = Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: Some(0),
            block_created: None,
        };
        let state = PoolSnapshot {
            address: pool.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: None,
            reserve1: None,
            tvl_usd: None,
            state: serde_json::json!({}),
        };
        // Without an amount_in, the pathfinder should still build a route
        // (no quote attempt) and mark it as estimated.
        let graph = TokenGraph::new(vec![(pool, state)]);
        let routes = find_routes(&graph, weth, usdc, 3, None);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].quote_confidence, QuoteConfidence::Estimated);
        assert!(routes[0].total_output.is_none());
    }

    #[test]
    fn fluid_pool_survives_with_amount_in() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let pool = Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: Some(0),
            block_created: None,
        };
        let state = PoolSnapshot {
            address: pool.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: None,
            reserve1: None,
            tvl_usd: None,
            state: serde_json::json!({}),
        };
        // With amount_in, a Fluid pool that can't quote should NOT be
        // dropped — it should survive as Estimated with no output.
        let graph = TokenGraph::new(vec![(pool, state)]);
        let routes = find_routes(&graph, weth, usdc, 3, Some(U256::from(1_000_000_000u128)));
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].quote_confidence, QuoteConfidence::Estimated);
        assert!(routes[0].total_output.is_none());
    }

    #[test]
    fn dead_end_no_route() {
        // WETH has no outgoing edge to a fictional X token, so no route
        // should be found.
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let x = address!("0000000000000000000000000000000000000099");
        let graph = TokenGraph::new(vec![]);
        let routes = find_routes(&graph, weth, x, 3, Some(U256::from(1)));
        assert!(routes.is_empty());
    }

    #[test]
    fn max_hops_respected() {
        // WETH -> USDC -> DAI requires 2 hops. max_hops=1 must return nothing.
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let graph = TokenGraph::new(vec![
            mk_pool(weth, usdc, 1_000_000.0),
            mk_pool(usdc, dai, 500_000.0),
        ]);
        let routes = find_routes(&graph, weth, dai, 1, Some(U256::from(1)));
        assert!(routes.is_empty(), "max_hops=1 should not reach dai");
    }

    #[test]
    fn intermediate_token_not_whitelisted_blocks_route() {
        // The whitelist blocks non-whitelisted tokens from being a *pass-through*
        // (i.e. the destination of a non-first, non-terminal hop). The first
        // hop is unrestricted; the final hop is always allowed.
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let x = address!("0000000000000000000000000000000000000099"); // not whitelisted
        let y = address!("00000000000000000000000000000000000000aa"); // not whitelisted

        // WETH -> USDC -> X: USDC is the intermediate, IS whitelisted → allowed.
        // Final hop X is the target → always allowed.
        let graph = TokenGraph::new(vec![
            mk_pool(weth, usdc, 1_000_000.0),
            mk_pool_with_kind(usdc, x, 1_000_000.0, PoolKind::UniswapV2),
        ]);
        let via = find_routes(&graph, weth, x, 3, Some(U256::from(1)));
        assert!(via.iter().any(|r| r.hop_count == 2),
            "WETH->USDC->X must be found (USDC is whitelisted, X is target)");

        // WETH -> USDC -> X -> Y: X is a pass-through and is NOT whitelisted,
        // so this 3-hop path is rejected even though Y is the target.
        let graph3 = TokenGraph::new(vec![
            mk_pool(weth, usdc, 1_000_000.0),
            mk_pool_with_kind(usdc, x, 1_000_000.0, PoolKind::UniswapV2),
            mk_pool_with_kind(x, y, 1_000_000.0, PoolKind::UniswapV2),
        ]);
        let blocked = find_routes(&graph3, weth, y, 3, Some(U256::from(1)));
        assert!(blocked.is_empty(),
            "non-whitelist pass-through X must block the 3-hop WETH->USDC->X->Y path");
    }

    #[test]
    fn route_to_self_without_pool_returns_empty() {
        // Request a route from WETH to WETH with no pool available.
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let graph = TokenGraph::new(vec![]);
        let routes = find_routes(&graph, weth, weth, 3, Some(U256::from(1)));
        assert!(routes.is_empty());
    }

    /// Two routes to the same destination. The direct 1-hop route has
    /// a larger reserve and thus a larger output for the same `amount_in`.
    /// The 2-hop route goes through a small intermediate pool. Sort order
    /// must put the higher-output (direct) route first, even though the
    /// 2-hop pool's TVL is comparable.
    #[test]
    fn higher_output_route_ranks_first() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        // Direct WETH/USDC with 1B / 1B reserves (large).
        let direct_pool = mk_pool_with_kind(weth, usdc, 1_000_000.0, PoolKind::UniswapV2);
        // 2-hop via DAI: WETH/DAI small, DAI/USDC small.
        let small_eth_dai = mk_pool_with_kind(weth, dai, 500_000.0, PoolKind::UniswapV2);
        let small_dai_usdc = mk_pool_with_kind(dai, usdc, 500_000.0, PoolKind::UniswapV2);

        let graph = TokenGraph::new(vec![
            direct_pool,
            small_eth_dai,
            small_dai_usdc,
        ]);
        let amount_in = U256::from(1_000_000_000_000_000_000u128); // 1 WETH
        let routes = find_routes(&graph, weth, usdc, 3, Some(amount_in));
        assert!(routes.len() >= 2, "expected at least 2 routes, got {}", routes.len());
        let first = &routes[0];
        let second = &routes[1];
        assert_eq!(first.hop_count, 1, "direct route should be first");
        assert_eq!(second.hop_count, 2, "2-hop route should be second");
        // Output ordering: direct > 2-hop.
        let a = first.total_output.expect("direct route has output");
        let b = second.total_output.expect("2-hop route has output");
        assert!(a > b, "expected direct output {} > 2-hop output {}", a, b);
    }

    /// Two routes with the same output but different total fees. The
    /// cheaper (lower total_fee_bps) route should rank first.
    /// Constructed by giving the 2-hop pool a high per-hop fee.
    #[test]
    fn lower_fee_route_ranks_first_when_output_equals() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        // Identical pool parameters, but the 2-hop pool's fee is set higher
        // (1000 bps) to make the 2-hop total_fee_bps larger.
        let direct = Pool {
            address: address!("0000000000000000000000000000000000000001"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 18,
            fee: Some(30),
            block_created: None,
        };
        let direct_state = PoolSnapshot {
            address: direct.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            reserve1: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            tvl_usd: Some(1_000_000.0),
            state: serde_json::json!({}),
        };
        let hop1 = Pool {
            address: address!("0000000000000000000000000000000000000002"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: dai,
            token1_decimals: 18,
            fee: Some(1000),
            block_created: None,
        };
        let hop1_state = PoolSnapshot {
            address: hop1.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            reserve1: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            tvl_usd: Some(1_000_000.0),
            state: serde_json::json!({}),
        };
        let hop2 = Pool {
            address: address!("0000000000000000000000000000000000000003"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: dai,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 18,
            fee: Some(1000),
            block_created: None,
        };
        let hop2_state = PoolSnapshot {
            address: hop2.address,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            reserve1: Some(U256::from(1_000_000_000_000_000_000_000u128)),
            tvl_usd: Some(1_000_000.0),
            state: serde_json::json!({}),
        };

        let graph = TokenGraph::new(vec![
            (direct, direct_state),
            (hop1, hop1_state),
            (hop2, hop2_state),
        ]);
        let amount_in = U256::from(1u128);
        let routes = find_routes(&graph, weth, usdc, 3, Some(amount_in));
        assert!(routes.len() >= 2);
        let first = &routes[0];
        let second = &routes[1];
        // Different fees, possibly different outputs (reserves identical, but
        // 2-hop pays the fee twice). The direct route should rank first
        // because it has strictly lower total_fee_bps.
        assert!(
            first.total_fee_bps < second.total_fee_bps,
            "expected first.total_fee_bps ({}) < second.total_fee_bps ({})",
            first.total_fee_bps,
            second.total_fee_bps
        );
    }

    #[test]
    fn sort_by_fee_lower_first() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let low_fee = Pool {
            address: address!("000000000000000000000000000000000000000a"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: Some(30),
            block_created: None,
        };
        let high_fee = Pool {
            address: address!("000000000000000000000000000000000000000b"),
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: Some(100),
            block_created: None,
        };
        let state = PoolSnapshot {
            address: Address::ZERO,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1000)),
            reserve1: Some(U256::from(1000)),
            tvl_usd: Some(1_000_000.0),
            state: serde_json::json!({}),
        };
        let mut low_state = state.clone(); low_state.address = low_fee.address;
        let mut high_state = state.clone(); high_state.address = high_fee.address;
        let graph = TokenGraph::new(vec![(low_fee, low_state), (high_fee, high_state)]);
        let mut routes = find_routes(&graph, weth, usdc, 3, None);
        // Default (Output) sort — output is equal (no amount_in), tie-break by fee.
        // Both have None output so output comparison is Equal; fee sorts ascending.
        assert_eq!(routes.len(), 2);
        assert!(routes[0].total_fee_bps <= routes[1].total_fee_bps);

        // Fee-priority sort — lower fee ranks first regardless of other keys.
        sort_routes(&mut routes, RouteSortMode::Fee);
        assert_eq!(routes[0].total_fee_bps, 30);
        assert_eq!(routes[1].total_fee_bps, 100);
    }

    #[test]
    fn sort_by_tvl_higher_first() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let low_tvl = mk_pool(weth, usdc, 100_000.0);
        let high_tvl = mk_pool(weth, usdc, 1_000_000.0);
        let graph = TokenGraph::new(vec![low_tvl, high_tvl]);
        let mut routes = find_routes(&graph, weth, usdc, 3, None);
        assert_eq!(routes.len(), 2);
        sort_routes(&mut routes, RouteSortMode::Tvl);
        assert!(routes[0].min_pool_tvl_usd >= routes[1].min_pool_tvl_usd,
            "Tvl sort: higher TVL should rank first");
    }

    #[test]
    fn sort_by_confidence_exact_first() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        // Exact pool (UniV2)
        let exact_pool = mk_pool(weth, usdc, 1_000_000.0);
        // Estimated pool (Fluid, no quoter) — same token pair, different address
        let est = (Pool {
            address: address!("00000000000000000000000000000000000000fe"),
            pool_id: B256::ZERO,
            kind: PoolKind::Fluid,
            factory: None,
            token0: weth,
            token0_decimals: 18,
            token1: usdc,
            token1_decimals: 6,
            fee: Some(0),
            block_created: None,
        }, PoolSnapshot {
            address: address!("00000000000000000000000000000000000000fe"),
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: None,
            reserve1: None,
            tvl_usd: Some(1_000_000.0),
            state: serde_json::json!({}),
        });
        let graph = TokenGraph::new(vec![exact_pool, est]);
        // Supply amount_in so the exact pool computes output (Exact) while
        // the Fluid pool cannot quote (Estimated).
        let mut routes = find_routes(&graph, weth, usdc, 3, Some(U256::from(1_000_000_000u128)));
        assert_eq!(routes.len(), 2);
        sort_routes(&mut routes, RouteSortMode::Confidence);
        assert_eq!(routes[0].quote_confidence, QuoteConfidence::Exact);
        assert_eq!(routes[1].quote_confidence, QuoteConfidence::Estimated);
    }

    #[test]
    fn sort_by_hops_fewer_first() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let graph = TokenGraph::new(vec![
            mk_pool(weth, usdc, 1_000_000.0),
            mk_pool(weth, dai, 500_000.0),
            mk_pool_with_kind(dai, usdc, 500_000.0, PoolKind::UniswapV2),
        ]);
        let mut routes = find_routes(&graph, weth, usdc, 3, None);
        assert!(routes.len() >= 2, "expected at least 2 routes");
        sort_routes(&mut routes, RouteSortMode::Hops);
        assert!(routes[0].hop_count <= routes[1].hop_count,
            "Hops sort: fewer hops should rank first");
    }
}
