//! DEX swap event registry: topic0 hashes, decoder dispatch, pool/sender extraction.
//!
//! Two pool-resolution strategies:
//!   Type A — pool IS the emitting contract (UniV2, UniV3, Curve, DODO, Maverick)
//!   Type B — pool ID comes from event params (Balancer V2/V3, UniV4 PoolManager)

pub mod lending;
pub mod registry;
pub mod types;
