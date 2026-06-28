//! Funder resolution — trace risk capital back to its source.
//!
//! This module isolates the most bug-prone part of sandwich detection:
//! deciding who provided the capital that the frontrun executor spends.
//! See `docs/adr/0001-classifier-and-trace-funder.md` for the five cases.

use std::collections::{HashMap, HashSet};
use alloy::primitives::{Address, U256};

use crate::detector::Ctx;
use crate::models::{TxFlow, ETH_TRANSFER_ADDR};

/// Resolver for the executor's capital source in a single front tx.
pub(crate) struct FunderResolver<'a> {
    ctx: &'a Ctx<'a>,
    ff: &'a TxFlow,
    executor: Address,
    pool_out_tokens: Vec<Address>,
}

impl<'a> FunderResolver<'a> {
    pub(crate) fn new(ctx: &'a Ctx<'a>, ff: &'a TxFlow, executor: Address) -> Self {
        let mut pool_out_tokens: Vec<Address> = ff.transfers.iter()
            .filter(|t| t.from == executor && ctx.pool_set.contains(&t.to))
            .map(|t| t.token)
            .collect();
        pool_out_tokens.sort();
        pool_out_tokens.dedup();
        Self { ctx, ff, executor, pool_out_tokens }
    }

    /// Resolve the executor's funder in priority order.
    pub(crate) fn resolve(&self) -> Option<Address> {
        // Cases 1-3: per pool-out token.
        for &pool_out in &self.pool_out_tokens {
            // Case 1: direct inbound of the pool-out token.
            if let Some(f) = self.resolve_direct_inbound(pool_out, self.ctx.pool_set) {
                if !self.is_round_trip(f, pool_out) { return Some(f); }
            }

            // Case 2: wrap — pool-out is WETH, executor received ETH.
            if pool_out == WETH && self.eth_sufficient_for_wrap() {
                if let Some(f) = self.resolve_eth_inbound() {
                    if !self.is_round_trip(f, ETH_TRANSFER_ADDR) { return Some(f); }
                }
            }

            // Case 3: borrow from a lending platform.
            if let Some(f) = self.resolve_borrow_funder(pool_out) {
                if !self.is_round_trip(f, pool_out) { return Some(f); }
            }
        }

        // Fallback: executor has no pool-outbound; trace pool→executor inbound upstream.
        if self.pool_out_tokens.is_empty() {
            if let Some((f, funded_token)) = self.resolve_pool_intermediary() {
                if !self.is_round_trip(f, funded_token) { return Some(f); }
            }
        }

        // Case 4: real-capital inbound on a non-pool-out token.
        if let Some(f) = self.resolve_real_capital_funder() {
            if !self.is_round_trip_inbound(f) { return Some(f); }
        }

        // Case 5: tx.target as funder (misclassified-as-Pool user contracts).
        if let Some(target) = self.ff.to {
            if !self.ctx.lending_set.contains(&target)
                && target != Address::ZERO
                && target != self.executor
                && self.ff.transfers.iter().any(|t| t.from == target && t.to == self.executor)
            {
                return Some(target);
            }
        }

        None
    }

    // ------------------------------------------------------------------
    // Case 1: direct inbound
    // ------------------------------------------------------------------

    fn resolve_direct_inbound(
        &self,
        token: Address,
        outbound_target_set: &HashSet<Address>,
    ) -> Option<Address> {
        let out_total: u128 = self.ff.transfers.iter()
            .filter(|t| t.from == self.executor
                && outbound_target_set.contains(&t.to)
                && t.token == token)
            .map(|t| amount_u128(t.amount))
            .sum();
        if out_total == 0 { return None; }

        let inbound_total: u128 = self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.token == token
                && t.from != Address::ZERO
                && self.ctx.unknown.contains(&t.from))
            .map(|t| amount_u128(t.amount))
            .sum();
        if inbound_total < out_total { return None; }

        self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.token == token
                && t.from != Address::ZERO
                && self.ctx.unknown.contains(&t.from))
            .max_by_key(|t| amount_u128(t.amount))
            .map(|t| t.from)
    }

    // ------------------------------------------------------------------
    // Case 2: wrap (ETH → WETH)
    // ------------------------------------------------------------------

    fn eth_sufficient_for_wrap(&self) -> bool {
        let total_eth_in: u128 = self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.token == ETH_TRANSFER_ADDR
                && t.from != Address::ZERO
                && !self.ctx.pool_set.contains(&t.from)
                && !self.ctx.lending_set.contains(&t.from))
            .map(|t| amount_u128(t.amount))
            .sum();
        let weth_out: u128 = self.ff.transfers.iter()
            .filter(|t| t.from == self.executor
                && self.ctx.pool_set.contains(&t.to)
                && t.token == WETH)
            .map(|t| amount_u128(t.amount))
            .sum();
        if weth_out == 0 { return true; }
        total_eth_in >= weth_out
    }

    fn resolve_eth_inbound(&self) -> Option<Address> {
        self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.token == ETH_TRANSFER_ADDR
                && t.from != Address::ZERO
                && self.ctx.unknown.contains(&t.from))
            .max_by_key(|t| amount_u128(t.amount))
            .map(|t| t.from)
    }

    // ------------------------------------------------------------------
    // Case 3: borrow from a lending platform
    // ------------------------------------------------------------------

    fn resolve_borrow_funder(&self, pool_out: Address) -> Option<Address> {
        for t_out in self.ff.transfers.iter()
            .filter(|t| t.from == self.executor
                && self.ctx.lending_set.contains(&t.to)
                && t.token != pool_out)
        {
            let has_borrow = self.ff.transfers.iter().any(|t_borrow|
                t_borrow.to == self.executor
                && t_borrow.from == t_out.to
                && t_borrow.token == pool_out);
            if !has_borrow { continue; }

            if let Some(f) = self.resolve_direct_inbound(t_out.token, self.ctx.lending_set) {
                return Some(f);
            }
            if t_out.token == WETH {
                if let Some(f) = self.resolve_eth_inbound() {
                    return Some(f);
                }
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Case 4: real-capital inbound
    // ------------------------------------------------------------------

    fn resolve_real_capital_funder(&self) -> Option<Address> {
        let mut exec_out: HashMap<Address, u128> = HashMap::new();
        for t in self.ff.transfers.iter()
            .filter(|t| t.from == self.executor
                && (self.ctx.pool_set.contains(&t.to) || self.ctx.lending_set.contains(&t.to))
                && self.ctx.supported_tokens.contains(&t.token))
        {
            *exec_out.entry(t.token).or_default() += amount_u128(t.amount);
        }
        for t in &self.pool_out_tokens {
            exec_out.entry(*t).or_insert(0);
        }

        let mut best: Option<(Address, Address, u128)> = None;
        for t in self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.from != Address::ZERO
                && !self.ctx.pool_set.contains(&t.from)
                && !self.ctx.lending_set.contains(&t.from)
                && !self.ctx.pool_set.contains(&t.to)
                && !self.ctx.supported_tokens.contains(&t.from)
                && exec_out.contains_key(&t.token))
        {
            let inbound = amount_u128(t.amount);
            if inbound == 0 { continue; }
            let spent = exec_out.get(&t.token).copied().unwrap_or(0);
            if inbound < spent { continue; }
            match best {
                None => best = Some((t.from, t.token, inbound)),
                Some((_, _, prev)) if inbound > prev => best = Some((t.from, t.token, inbound)),
                Some(prev) => { best = Some(prev); }
            }
        }
        best.map(|(from, _, _)| from)
    }

    // ------------------------------------------------------------------
    // Fallback: pool intermediary
    // ------------------------------------------------------------------

    fn resolve_pool_intermediary(&self) -> Option<(Address, Address)> {
        if let Some((from, token, inbound_total)) = self.ff.transfers.iter()
            .filter(|t| t.to == self.executor
                && t.from != Address::ZERO
                && self.ctx.unknown.contains(&t.from))
            .fold(None::<(Address, Address, u128)>, |acc, t| {
                let amt = amount_u128(t.amount);
                match acc {
                    None => Some((t.from, t.token, amt)),
                    Some((_, _, prev_amt)) if amt > prev_amt => Some((t.from, t.token, amt)),
                    Some(prev) => Some(prev),
                }
            })
        {
            let out_total: u128 = self.ff.transfers.iter()
                .filter(|t| t.from == self.executor && t.token == token)
                .map(|t| amount_u128(t.amount))
                .sum();
            if inbound_total >= out_total && out_total > 0 {
                return Some((from, token));
            }
        }

        let from_pool = self.ff.transfers.iter()
            .filter(|t| t.to == self.executor && self.ctx.pool_set.contains(&t.from))
            .max_by_key(|t| amount_u128(t.amount));
        if let Some(t) = from_pool {
            let pool = t.from;
            let token = t.token;
            if let Some(t) = self.ff.transfers.iter()
                .filter(|t| t.to == pool
                    && t.from != Address::ZERO
                    && self.ctx.unknown.contains(&t.from))
                .max_by_key(|t| amount_u128(t.amount))
            {
                return Some((t.from, token));
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Round-trip guards
    // ------------------------------------------------------------------

    fn is_round_trip(&self, candidate: Address, token: Address) -> bool {
        self.ff.transfers.iter().any(|t|
            t.from == self.executor && t.to == candidate && t.token == token)
    }

    fn is_round_trip_inbound(&self, candidate: Address) -> bool {
        self.ff.transfers.iter().any(|t| t.from == self.executor && t.to == candidate)
    }
}

/// WETH mainnet address.
pub(crate) const WETH: Address = Address::new(hex_literal::hex!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));

/// Saturating U256 → u128.
pub(crate) fn amount_u128(v: U256) -> u128 {
    let b = v.to_be_bytes::<32>();
    u128::from_be_bytes(b[16..].try_into().unwrap_or([0; 16]))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use alloy::primitives::{Address, B256, U256};

    use crate::detector::Ctx;
    use crate::detector::funder::FunderResolver;
    use crate::models::{Transfer, TxFlow, ETH_TRANSFER_ADDR};

    fn addr(s: &str) -> Address {
        let p = format!("0x{:0>40}", s.trim_start_matches("0x"));
        p.parse().unwrap()
    }
    fn weth() -> Address { addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2") }
    fn usdt_addr() -> Address { addr("0xdac17f958d2ee523a2206206994597c13d831ec7") }
    fn eth_addr() -> Address { ETH_TRANSFER_ADDR }

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

    fn resolve(ctx: &Ctx, ff: &TxFlow, executor: Address) -> Option<Address> {
        FunderResolver::new(ctx, ff, executor).resolve()
    }

    #[test]
    fn direct_sender() {
        let funder_s = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_s, executor, weth(), 100),
            transfer(executor, pool, weth(), 95),
        ]);
        let mut unk = HashSet::new(); unk.insert(funder_s); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_s));
    }

    #[test]
    fn pool_intermediary() {
        let funder_s = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_s, pool, weth(), 100),
            transfer(pool, executor, weth(), 95),
        ]);
        let mut unk = HashSet::new(); unk.insert(funder_s); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_s));
    }

    #[test]
    fn flashloan_detected() {
        let flashloaner = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(flashloaner, executor, weth(), 100),
            transfer(executor, flashloaner, weth(), 100),
        ]);
        let mut unk = HashSet::new(); unk.insert(flashloaner); unk.insert(executor);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), None);
    }

    #[test]
    fn pre_balance_weth_trade() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let pool = addr("0xacdb");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(initiator); unk.insert(executor);
        let w = weth(); let u = usdt_addr(); let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 67_500_000_000),
            transfer(executor, pool, w, 1_132_767_764_829_940_199),
            transfer(pool, executor, u, 1_888_328_466),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[w] };
        assert_eq!(resolve(&ctx, &ff, executor), None);
    }

    #[test]
    fn borrow_aave_weth_collateral() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let aave = addr("0x7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut ls = HashSet::new(); ls.insert(aave);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_500_000_000_000_000_000),
            transfer(executor, aave, w, 1_500_000_000_000_000_000),
            transfer(aave, executor, u, 2_000_000_000),
            transfer(executor, pool, u, 2_000_000_000),
            transfer(pool, executor, w, 1_600_000_000_000_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &ls, unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
    }

    #[test]
    fn borrow_aave_eth_collateral() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let aave = addr("0x7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut ls = HashSet::new(); ls.insert(aave);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr(); let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, eth, 1_500_000_000_000_000_000),
            transfer(executor, aave, w, 1_500_000_000_000_000_000),
            transfer(aave, executor, u, 2_000_000_000),
            transfer(executor, pool, u, 2_000_000_000),
            transfer(pool, executor, w, 1_600_000_000_000_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &ls, unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
    }

    #[test]
    fn pre_balance_no_inbound() {
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(executor, pool, w, 1_000_000_000_000_000_000),
            transfer(pool, executor, u, 1_500_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), None);
    }

    #[test]
    fn direct_weth_inbound() {
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
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
    }

    #[test]
    fn multi_pool_out() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool_a = addr("0xcccc");
        let pool_b = addr("0xdddd");
        let mut ps = HashSet::new(); ps.insert(pool_a); ps.insert(pool_b);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_000_000_000_000_000_000),
            transfer(executor, pool_a, w, 1_000_000_000_000_000_000),
            transfer(pool_a, executor, u, 1_500_000_000),
            transfer(executor, pool_b, u, 1_500_000_000),
            transfer(pool_b, executor, w, 1_050_000_000_000_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
    }

    #[test]
    fn round_trip_different_token_kept() {
        let funder_eoa = addr("0xaaaa");
        let executor = addr("0xbbbb");
        let pool = addr("0xcccc");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(funder_eoa); unk.insert(executor);
        let w = weth(); let u = usdt_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(funder_eoa, executor, w, 1_500_000_000_000_000_000),
            transfer(executor, pool, w, 1_500_000_000_000_000_000),
            transfer(pool, executor, u, 1_500_000_000),
            transfer(executor, funder_eoa, u, 50_000_000),
        ]);
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &[] };
        assert_eq!(resolve(&ctx, &ff, executor), Some(funder_eoa));
    }

    #[test]
    fn unwrap_dust_falls_through() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let weth_contract = addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let pool = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
        let token_back = addr("0x32708538a107253b51a735a724330a23106ca4ca");
        let mut ps = HashSet::new(); ps.insert(pool);
        let mut unk = HashSet::new(); unk.insert(initiator); unk.insert(executor);
        let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 59),
            transfer(weth_contract, executor, eth, 1_000_000_000_000_000),
            transfer(executor, pool, eth, 1_000_000_000_000_000),
            transfer(pool, executor, token_back, 1_000_000),
        ]);
        let supported = [eth, weth_contract];
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &supported };
        assert_eq!(resolve(&ctx, &ff, executor), None);
    }

    #[test]
    fn router_passthrough_dust() {
        let initiator = addr("0xae2f");
        let executor = addr("0x1f2f");
        let weth_contract = addr("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let router = addr("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
        let pool = addr("0x000000000004444c5dc75cb358380d2e3de08a90");
        let token_back = addr("0x6de037ef9ad2725eb40118bb1702ebb27e4aeb24");
        let mut ps = HashSet::new();
        ps.insert(pool);
        ps.insert(router);
        let mut unk = HashSet::new();
        unk.insert(initiator);
        unk.insert(executor);
        let eth = eth_addr();
        let ff = mock_tx(0, Address::ZERO, vec![
            transfer(initiator, executor, eth, 99),
            transfer(weth_contract, executor, eth, 1_000_000_000_000_000),
            transfer(executor, router, eth, 1_000_000_000_000_000),
            transfer(router, pool, eth, 1_000_000_000_000_000),
            transfer(pool, executor, token_back, 1_000_000),
        ]);
        let supported = [eth, weth_contract];
        let ctx = Ctx { block_number: 0, tx_flows: &[], pool_set: &ps, tx_pools: vec![], lending_set: &HashSet::new(), unknown: &unk, coinbase: Address::ZERO, supported_tokens: &supported };
        assert_eq!(resolve(&ctx, &ff, executor), None);
    }
}
