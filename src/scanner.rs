//! Block scanner — fetch, detect, and persist sandwich bundles.
//!
//! This module owns the core scanning loop that `run_serve` uses, as well
//! as the batch helper shared by `run_scan` and `run_peek`. Moving it out
//! of `main.rs` makes the scanner testable and keeps the binary entry
//! point focused on CLI dispatch.

use futures::stream::{self, StreamExt};
use sqlx::PgPool;
use tracing::info;

use crate::classifier::DefaultClassifier;
use crate::db;
use crate::detector::detect_sandwiches;
use crate::models::SandwichBundle;
use crate::rpc::{fetch_block, RpcClient};
use crate::tokens::{DEFAULT_BLACKLIST, DEFAULT_TOKENS};

#[derive(Clone)]
pub struct Scanner {
    provider: RpcClient,
}

impl Scanner {
    pub fn new(provider: RpcClient) -> Self {
        Self { provider }
    }

    /// Concurrent batch scan over a range of blocks. Returns each block's
    /// detected bundles without persisting anything.
    pub async fn scan_blocks(
        &self,
        block_range: &[u64],
        concurrency: usize,
    ) -> Vec<Result<(u64, Vec<SandwichBundle>), anyhow::Error>> {
        let classifier = DefaultClassifier::new(DEFAULT_BLACKLIST, crate::dex::lending::LENDING_ADDRESSES);
        let results: Vec<_> = stream::iter(block_range.iter().copied())
            .map(|block_number| async move {
                let data = fetch_block(&self.provider, block_number).await?;
                let bundles = detect_sandwiches(
                    &classifier,
                    block_number,
                    &data.flows,
                    &data.raw_logs,
                    data.coinbase,
                    DEFAULT_BLACKLIST,
                    DEFAULT_TOKENS,
                );
                Ok((block_number, bundles))
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;
        results
    }

    /// Continuous scanner loop used by `serve`.
    ///
    /// Reads scan state, handles pending replays, fetches new blocks up to
    /// `chain_head - delay_blocks`, detects sandwiches, and persists
    /// results. Loops forever until the process exits.
    pub async fn run_continuous(
        &self,
        pool: &PgPool,
        delay_blocks: u64,
    ) -> anyhow::Result<()> {
        let classifier = DefaultClassifier::new(DEFAULT_BLACKLIST, crate::dex::lending::LENDING_ADDRESSES);
        loop {
            let state = match db::read_scan_state(pool).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("read_scan_state: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            // Pending replay: delete data from that block, reset the
            // cursor, clear the flag, and auto-resume if paused.
            if state.pending_replay_from != 0 {
                let from = state.pending_replay_from;
                info!("performing pending replay from block {}", from);
                if let Err(e) = db::delete_sandwiches_from(pool, from).await {
                    tracing::error!("delete_sandwiches_from: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                if let Err(e) = db::delete_blocks_scanned_from(pool, from).await {
                    tracing::error!("delete_blocks_scanned_from: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                if let Err(e) = db::reset_scan_state_to(pool, from).await {
                    tracing::error!("reset_scan_state_to: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                if let Err(e) = db::clear_pending_replay_from(pool).await {
                    tracing::error!("clear_pending_replay_from: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                if !state.enabled {
                    info!("auto-resuming scanner due to pending replay");
                    if let Err(e) = db::set_scan_enabled(pool, true).await {
                        tracing::error!("set_scan_enabled: {:?}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                }
                continue;
            }

            if !state.enabled {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let chain_head = match self.provider.block_number().await {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!("block_number: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            {
                static LAST_HEAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                if chain_head != LAST_HEAD.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Err(e) = db::update_chain_head(pool, chain_head).await {
                        tracing::error!("update_chain_head: {:?}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    LAST_HEAD.store(chain_head, std::sync::atomic::Ordering::Relaxed);
                }
            }

            let target = chain_head.saturating_sub(delay_blocks);
            if state.next_block > target {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            let block_to_scan = state.next_block;

            let data = match fetch_block(&self.provider, block_to_scan).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("fetch_block {}: {:?}", block_to_scan, e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            let bundles = detect_sandwiches(
                &classifier,
                block_to_scan,
                &data.flows,
                &data.raw_logs,
                data.coinbase,
                DEFAULT_BLACKLIST,
                DEFAULT_TOKENS,
            );

            // Re-check state after fetch to avoid writing stale blocks.
            let state = match db::read_scan_state(pool).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("read_scan_state (post-fetch): {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            if state.pending_replay_from != 0 || !state.enabled || state.next_block != block_to_scan {
                continue;
            }

            let count = bundles.len();
            if !bundles.is_empty() {
                if let Err(e) = db::insert_sandwiches(pool, &bundles).await {
                    tracing::error!("insert_sandwiches: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            }
            if let Err(e) = db::mark_block_scanned(pool, block_to_scan, count).await {
                tracing::error!("mark_block_scanned: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            if let Err(e) = db::advance_scan_state(pool, block_to_scan + 1).await {
                tracing::error!("advance_scan_state: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }

            info!("block {} scanned: {} sandwiches", block_to_scan, count);
        }
    }
}
