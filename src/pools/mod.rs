//! Liquidity registry and routing for MEV strategies.
//!
//! This module is intentionally separate from sandwich detection. It maintains
//! a global top-1,000-by-TVL pool list, keeps per-block liquidity snapshots,
//! and exposes multi-hop routing.

pub mod bootstrap;
pub mod graph;
pub mod identity;
pub mod job;
pub mod lending;
pub mod liquidity;
pub mod pricing;
pub mod quoting;
pub mod registry;
pub mod routing;
pub mod types;
