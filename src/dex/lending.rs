//! Known lending platform addresses on Ethereum mainnet.
//!
//! A lending platform is a contract that holds user-supplied collateral and
//! issues loans (borrows) against it. In sandwich detection we need to identify
//! these to walk the funding chain: when an executor receives a borrowed token
//! from a lending platform, the actual capital source is whoever funded the
//! collateral deposit, not the lending platform itself.
//!
//! Coverage target: the platforms that handle the vast majority of mainnet
//! lending volume. Expand as new patterns surface.

use alloy::primitives::{address, Address};

/// All known lending platform addresses on Ethereum mainnet.
pub static LENDING_ADDRESSES: &[Address] = &[
    // Aave V2 — LendingPool
    address!("7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9"),
    // Aave V3 — Pool
    address!("87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"),
    // Aave V3 — USDC reserve proxy (flashloan provider seen in block 25301029)
    address!("98c23e9d8f34fefb1b7bd6a91b7ff122f4e16f5c"),
    // Compound V2 — Comptroller
    address!("3d9819210A31b4961b30EF54bE2aeD79B9c9Cd3B"),
    // Maker DAO — Vat (core CDP engine)
    address!("35D1b3F3D7966A1DFe207aa4514C12a259A0492B"),
    // Morpho Blue
    address!("BBBBBbbBBb9cC5e90e3b3Af64bdAF62C37EEFFCb"),
];
