//! Static bootstrap file for first-run seeding.
//!
//! A bootstrap is a JSON file containing a curated list of well-known
//! pools. `mev-ant seed-pools` reads this file (when configured) before
//! hitting TheGraph, so the registry can be seeded without depending on
//! TheGraph availability. TheGraph is then used only for daily refresh.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::pools::types::Pool;

const SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapFile {
    pub version: u32,
    pub pools: Vec<Pool>,
}

/// Read and parse a bootstrap file from `path`.
///
/// Fails loudly if the file cannot be read, the JSON is malformed, the
/// `version` field is missing or unsupported, or any pool record fails
/// to deserialise.
pub fn load_bootstrap(path: &Path) -> Result<Vec<Pool>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read bootstrap file {}", path.display()))?;
    let parsed: BootstrapFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse bootstrap file {}", path.display()))?;
    if parsed.version != SUPPORTED_VERSION {
        anyhow::bail!(
            "bootstrap file {} has unsupported version {} (expected {})",
            path.display(),
            parsed.version,
            SUPPORTED_VERSION
        );
    }
    Ok(parsed.pools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, B256};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Returns a unique temp path for a test bootstrap file. Caller writes
    /// the file and the test cleans it up. Avoids depending on `tempfile`.
    fn unique_temp_path() -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir()
            .join(format!("mev_ant_bootstrap_{}_{}.json", pid, n))
    }

    fn sample_pool_json() -> &'static str {
        r#"{
            "version": 1,
            "pools": [
                {
                    "address": "0x0000000000000000000000000000000000000001",
                    "pool_id": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "kind": "uniswap_v2",
                    "factory": null,
                    "token0": "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                    "token0_decimals": 18,
                    "token1": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                    "token1_decimals": 6,
                    "fee": 30,
                    "block_created": null
                }
            ]
        }"#
    }

    #[test]
    fn load_bootstrap_reads_pools_from_file() {
        let path = unique_temp_path();
        std::fs::write(&path, sample_pool_json()).unwrap();
        let pools = load_bootstrap(&path).expect("load_bootstrap should succeed");
        std::fs::remove_file(&path).ok();

        assert_eq!(pools.len(), 1);
        let p = &pools[0];
        assert_eq!(
            p.address,
            address!("0000000000000000000000000000000000000001")
        );
        assert_eq!(p.pool_id, B256::ZERO);
        assert_eq!(p.token0, address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
        assert_eq!(p.token1, address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));
        assert_eq!(p.token0_decimals, 18);
        assert_eq!(p.token1_decimals, 6);
        assert_eq!(p.fee, Some(30));
    }

    #[test]
    fn load_bootstrap_errors_on_malformed_json() {
        let path = unique_temp_path();
        std::fs::write(&path, "{ this is not json").unwrap();
        let err = load_bootstrap(&path).expect_err("malformed JSON must error");
        std::fs::remove_file(&path).ok();
        let msg = format!("{:#}", err);
        assert!(msg.contains("parse bootstrap file"), "got: {}", msg);
    }

    #[test]
    fn load_bootstrap_errors_on_unsupported_version() {
        let path = unique_temp_path();
        std::fs::write(
            &path,
            r#"{
                "version": 99,
                "pools": []
            }"#,
        )
        .unwrap();
        let err = load_bootstrap(&path).expect_err("version 99 must error");
        std::fs::remove_file(&path).ok();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("unsupported version 99"),
            "expected 'unsupported version 99' in error, got: {}",
            msg
        );
    }

    #[test]
    fn load_bootstrap_accepts_empty_pools_list() {
        let path = unique_temp_path();
        std::fs::write(
            &path,
            r#"{
                "version": 1,
                "pools": []
            }"#,
        )
        .unwrap();
        let pools = load_bootstrap(&path).expect("empty pools list should succeed");
        std::fs::remove_file(&path).ok();
        assert!(pools.is_empty());
    }
}
