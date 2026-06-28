//! Token-pool graph built from Liquid Pool snapshots.

use std::collections::HashMap;

use alloy::primitives::Address;

use crate::pools::types::{Hop, Pool, PoolSnapshot};

/// A directed edge in the token graph: pay `token_in` to receive `token_out`.
#[derive(Debug, Clone)]
pub struct Edge {
    pub pool: Pool,
    pub state: PoolSnapshot,
    pub token_in: Address,
    pub token_out: Address,
    pub fee: u32,
}

/// Graph of token → outgoing edges.
pub struct TokenGraph {
    edges: HashMap<Address, Vec<Edge>>,
}

impl TokenGraph {
    pub fn new(pools: Vec<(Pool, PoolSnapshot)>) -> Self {
        let mut edges: HashMap<Address, Vec<Edge>> = HashMap::new();
        for (pool, state) in pools {
            let fee = pool.fee.unwrap_or(0);
            // token0 -> token1
            edges.entry(pool.token0).or_default().push(Edge {
                pool: pool.clone(),
                state: state.clone(),
                token_in: pool.token0,
                token_out: pool.token1,
                fee,
            });
            // token1 -> token0
            edges.entry(pool.token1).or_default().push(Edge {
                pool: pool.clone(),
                state,
                token_in: pool.token1,
                token_out: pool.token0,
                fee,
            });
        }
        Self { edges }
    }

    pub fn edges_from(&self, token: Address) -> &[Edge] {
        self.edges.get(&token).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

impl Edge {
    pub fn to_hop(&self) -> Hop {
        Hop {
            pool_address: self.pool.address,
            pool_id: self.pool.pool_id,
            kind: self.pool.kind,
            token_in: self.token_in,
            token_out: self.token_out,
            fee: self.fee,
        }
    }
}
