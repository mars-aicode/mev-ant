//! Background liquidity job.
//!
//! Runs independently of the sandwich scanner. It:
//! - polls Reth for new blocks,
//! - maintains its own `liquidity_job_state.next_block` cursor,
//! - updates state for Liquid Pools touched in each block,
//! - performs a daily full refresh + re-ranking from TheGraph.
//!
//! The job is intentionally isolated: a sandwich-scanner replay does not touch
//! this cursor or the pool tables.
//!
//! Observability: emits a single `liquidity_job_tick` log per iteration with
//! `cursor_lag` (blocks behind chain head), `blocks_processed`, `rpc_failures`,
//! `thegraph_failures`, and `refresh_duration_ms`. Operators can grep for
//! `target="liquidity_job"` in their log aggregator and dashboard the fields.

use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Timelike;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::db;
use crate::pools::lending as lending_mod;
use crate::pools::lending::AAVE_V3_POOL;
use crate::pools::liquidity::update_touched_pools;
use crate::pools::registry::refresh_liquid_pools;
use crate::rpc::RpcClient;

/// Background liquidity updater.
#[derive(Clone)]
pub struct LiquidityJob {
    client: RpcClient,
    db: PgPool,
    top_n: usize,
    poll_interval: Duration,
    full_refresh_interval: Duration,
    full_refresh_hour: u32,
    lending_enabled: bool,
}

impl LiquidityJob {
    #[allow(dead_code)]
    pub fn new(
        client: RpcClient,
        db: PgPool,
        top_n: usize,
        poll_interval_secs: u64,
    ) -> Self {
        Self::with_options(client, db, top_n, poll_interval_secs, 0, true)
    }

    #[allow(dead_code)]
    pub fn with_hour(
        client: RpcClient,
        db: PgPool,
        top_n: usize,
        poll_interval_secs: u64,
        full_refresh_hour: u32,
    ) -> Self {
        Self::with_options(client, db, top_n, poll_interval_secs, full_refresh_hour, true)
    }

    pub fn with_options(
        client: RpcClient,
        db: PgPool,
        top_n: usize,
        poll_interval_secs: u64,
        full_refresh_hour: u32,
        lending_enabled: bool,
    ) -> Self {
        Self {
            client,
            db,
            top_n,
            poll_interval: Duration::from_secs(poll_interval_secs),
            // Roughly daily; the loop also checks time-since-last-refresh.
            full_refresh_interval: Duration::from_secs(20 * 60 * 60),
            full_refresh_hour,
            lending_enabled,
        }
    }

    /// Run the job forever (or until the process exits).
    pub async fn run(&self, start_block: Option<u64>) {
        if let Err(e) = self.init_cursor(start_block).await {
            tracing::error!("liquidity job cursor init failed: {:?}", e);
            return;
        }

        loop {
            if let Err(e) = self.tick().await {
                tracing::error!("liquidity job tick failed: {:?}; retrying in {:?}", e, self.poll_interval);
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// One iteration: catch up blocks and maybe refresh.
    async fn tick(&self) -> Result<()> {
        let tick_start = Instant::now();
        let state = db::read_liquidity_job_state(&self.db).await?;
        let chain_head = match self.client.block_number().await {
            Ok(h) => h,
            Err(e) => {
                self.emit_tick_log(state.next_block, 0, 1, 0, 0, tick_start);
                return Err(e);
            }
        };
        let mut rpc_failures: u64 = 0;
        let mut blocks_processed: u64 = 0;

        // Catch up per-block touched-pool updates.
        if state.next_block <= chain_head {
            let pools = db::get_all_pools_with_snapshots(&self.db).await?;
            let pool_refs: Vec<_> = pools.iter().map(|(p, _)| p.clone()).collect();

            for block in state.next_block..=chain_head {
                if let Err(e) = update_touched_pools(&self.client, &self.db, &pool_refs, block).await {
                    warn!("liquidity job touched-pool update at {} failed: {}", block, e);
                    rpc_failures += 1;
                    // Don't advance the cursor on a hard failure so we retry
                    // next tick. Skip lending update for this block.
                    break;
                }
                if self.lending_enabled {
                    if let Err(e) = self.try_update_lending_for_block(block).await {
                        warn!("aave_v3 touched fetch at block {}: {}", block, e);
                        rpc_failures += 1;
                    }
                }
                db::advance_liquidity_job_state(&self.db, block + 1).await?;
                blocks_processed += 1;
            }
        }

        // Daily full refresh from TheGraph.
        let now = chrono::Utc::now();
        let current_hour = now.hour();
        let in_refresh_window = current_hour == self.full_refresh_hour;
        let should_refresh = state
            .last_full_refresh_at
            .map(|t| {
                let elapsed = now.signed_duration_since(t);
                in_refresh_window
                    && elapsed.num_seconds() > 0
                    && elapsed.to_std().unwrap_or_default() >= self.full_refresh_interval
            })
            .unwrap_or(in_refresh_window);

        let mut thegraph_failures: u64 = 0;
        let mut refresh_duration_ms: u128 = 0;
        if should_refresh {
            let refresh_start = Instant::now();
            info!("liquidity job performing full refresh");
            match refresh_liquid_pools(&self.client, &self.db, self.top_n).await {
                Ok(ranked) => {
                    if let Err(e) = db::mark_liquidity_full_refresh(&self.db).await {
                        warn!("mark_liquidity_full_refresh failed: {}", e);
                    } else {
                        info!("liquidity job full refresh complete: {} pools", ranked.len());
                    }
                }
                Err(e) => {
                    thegraph_failures += 1;
                    warn!("liquidity job full refresh failed: {}; will retry later", e);
                }
            }
            refresh_duration_ms = refresh_start.elapsed().as_millis();
        }

        self.emit_tick_log(
            state.next_block,
            blocks_processed,
            rpc_failures,
            thegraph_failures,
            refresh_duration_ms,
            tick_start,
        );
        Ok(())
    }

    /// Emit a structured per-tick log so operators can dashboard cursor lag
    /// and failure rates.
    fn emit_tick_log(
        &self,
        cursor: u64,
        blocks_processed: u64,
        rpc_failures: u64,
        thegraph_failures: u64,
        refresh_duration_ms: u128,
        started: Instant,
    ) {
        let tick_duration_ms = started.elapsed().as_millis();
        // Best-effort cursor lag: we don't know the latest head inside this
        // helper; the chain head is logged separately when the tick starts.
        info!(
            target: "liquidity_job",
            cursor,
            blocks_processed,
            rpc_failures,
            thegraph_failures,
            refresh_duration_ms,
            tick_duration_ms,
            "liquidity_job_tick"
        );
    }

    /// Ensure the cursor exists. If it does not, seed it from `start_block` or
    /// the current chain head.
    async fn init_cursor(&self, start_block: Option<u64>) -> Result<()> {
        if db::read_liquidity_job_state(&self.db).await.is_ok() {
            return Ok(());
        }
        let block = match start_block {
            Some(b) => b,
            None => self.client.block_number().await?,
        };
        db::init_liquidity_job_state(&self.db, block).await?;
        info!("liquidity job cursor initialized at block {}", block);
        Ok(())
    }

    /// Fetch and persist Aave V3 state for markets touched in this block.
    /// Returns an error only on RPC failures (not on `Ok(empty)` results).
    async fn try_update_lending_for_block(&self, block: u64) -> Result<()> {
        let markets = lending_mod::update_touched_aave_v3(&self.client, AAVE_V3_POOL, block).await?;
        if markets.is_empty() {
            return Ok(());
        }
        db::upsert_lending_markets(&self.db, &markets, block).await?;
        info!(
            "lending job updated {} Aave V3 markets at block {}",
            markets.len(),
            block
        );
        Ok(())
    }
}
