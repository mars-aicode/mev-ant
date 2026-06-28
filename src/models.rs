//! Core data models for sandwich MEV analysis.

use alloy::primitives::{Address, B256, I256, U256};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Transfer event — unified ERC20 + ETH
// ---------------------------------------------------------------------------

/// A token transfer event. ETH transfers use `token == ETH_TRANSFER_ADDR`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub token: Address,
    pub from: Address,
    pub to: Address,
    pub amount: U256,
}

/// Canonical address representing native ETH in Transfer logs.
pub const ETH_TRANSFER_ADDR: Address = Address::new(hex_literal::hex!(
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
));

// ---------------------------------------------------------------------------
// Per-transaction flow
// ---------------------------------------------------------------------------

/// All transfers within a single transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxFlow {
    pub tx_hash: B256,
    pub tx_index: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub transfers: Vec<Transfer>,
    pub gas_used: u128,
    pub effective_gas_price: u128,
    pub effective_priority_fee: u128,
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Pool identity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoolId {
    Contract(Address),
    Param(B256),
}

// ---------------------------------------------------------------------------
// Profit tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenDelta {
    pub token: Address,
    pub amount: I256,
}

// ---------------------------------------------------------------------------
// Sandwich bundle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandwichBundle {
    pub block_number: u64,
    pub front_tx_index: u64,
    pub back_tx_index: u64,
    pub victim_tx_indices: Vec<u64>,
    pub victim_tx_hashes: Vec<B256>,
    pub attacked_pool: PoolId,
    pub auxiliary_pools: Vec<PoolId>,
    pub attacker: Address,
    pub frontrun_transfers: Vec<Transfer>,
    pub victim_transfers: Vec<Transfer>,
    pub backrun_transfers: Vec<Transfer>,
    pub funder: Address,
    pub executor: Address,
    pub initiator: Address,
    pub back_initiator: Address,
    pub target: Address,
    pub coinbase: Address,
    pub front_tx_hash: B256,
    pub back_tx_hash: B256,
    pub profit: Vec<TokenDelta>,
    pub gas_cost_wei: u128,
    pub coinbase_bribe: u128,
    pub expense_wei: u128,
}
