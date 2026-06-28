//! Repository layer — all sandwich/scan read queries.
//!
//! This module owns SQL SELECTs and the row-to-struct mapping for the
//! admin dashboard. Write operations and scan-state mutations live in
//! `crate::db`.

use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct SandwichRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub total_sandwiches: i64,
    pub distinct_attackers: i64,
    pub blocks_scanned: i64,
    pub chain_head: i64,
    pub current_block: i64,
}

#[derive(Debug, Clone)]
pub struct SandwichSummary {
    pub id: i64,
    pub block_number: i64,
    pub attacker: Vec<u8>,
    pub profit: String,
    pub victim_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct SandwichList {
    pub sandwiches: Vec<SandwichSummary>,
    pub total: i64,
}

#[derive(Debug, Clone)]
pub struct AttackerSummary {
    pub address: Vec<u8>,
    pub sandwich_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone)]
pub struct SandwichRecord {
    pub id: i64,
    pub block_number: i64,
    pub front_tx_index: i64,
    pub back_tx_index: i64,
    pub victim_count: i32,
    pub attacker: Vec<u8>,
    pub funder: Vec<u8>,
    pub executor: Vec<u8>,
    pub initiator: Vec<u8>,
    pub back_initiator: Vec<u8>,
    pub target: Vec<u8>,
    pub attacked_pool: String,
    pub profit_json: String,
    pub gas_cost_wei: i64,
    pub coinbase_bribe: i64,
    pub expense_wei: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub front_tx_hash: Option<Vec<u8>>,
    pub back_tx_hash: Option<Vec<u8>>,
    pub front_transfers: Option<String>,
    pub victim_transfers: Option<String>,
    pub back_transfers: Option<String>,
    pub victim_tx_hashes: Option<String>,
    pub coinbase: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ScanStateRecord {
    pub next_block: i64,
    pub enabled: bool,
    pub pending_replay_from: i64,
}

impl SandwichRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn stats(&self) -> Result<Stats, sqlx::Error> {
        let (total_sandwiches,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sandwiches")
            .fetch_one(&self.pool).await?;

        let (distinct_attackers,): (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT attacker) FROM sandwich_attackers"
        )
        .fetch_one(&self.pool).await?;

        let (blocks_scanned,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM blocks_scanned")
            .fetch_one(&self.pool).await?;

        let row = sqlx::query("SELECT next_block, chain_head FROM scan_state WHERE id = 1")
            .fetch_one(&self.pool).await?;
        let current_block: i64 = row.get(0);
        let chain_head: i64 = row.get(1);

        Ok(Stats { total_sandwiches, distinct_attackers, blocks_scanned, chain_head, current_block })
    }

    pub async fn list_sandwiches(
        &self,
        page: i64,
        page_size: i64,
        block_from: i64,
        block_to: i64,
        attacker: Option<&[u8]>,
    ) -> Result<SandwichList, sqlx::Error> {
        let offset = (page - 1) * page_size;

        let (total, rows) = if let Some(attacker_bytes) = attacker {
            let (total,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sandwiches \
                 WHERE attacker = $1 AND block_number >= $2 AND block_number <= $3"
            )
            .bind(attacker_bytes).bind(block_from).bind(block_to)
            .fetch_one(&self.pool).await?;

            let rows: Vec<(i64, i64, Vec<u8>, String, i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                "SELECT id, block_number, attacker, profit_json::text, victim_count, created_at \
                 FROM sandwiches \
                 WHERE attacker = $1 AND block_number >= $2 AND block_number <= $3 \
                 ORDER BY block_number DESC LIMIT $4 OFFSET $5"
            )
            .bind(attacker_bytes).bind(block_from).bind(block_to)
            .bind(page_size).bind(offset)
            .fetch_all(&self.pool).await?;

            (total, rows)
        } else {
            let (total,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sandwiches WHERE block_number >= $1 AND block_number <= $2"
            )
            .bind(block_from).bind(block_to)
            .fetch_one(&self.pool).await?;

            let rows: Vec<(i64, i64, Vec<u8>, String, i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                "SELECT id, block_number, attacker, profit_json::text, victim_count, created_at \
                 FROM sandwiches WHERE block_number >= $1 AND block_number <= $2 \
                 ORDER BY block_number DESC LIMIT $3 OFFSET $4"
            )
            .bind(block_from).bind(block_to).bind(page_size).bind(offset)
            .fetch_all(&self.pool).await?;

            (total, rows)
        };

        let sandwiches = rows.into_iter().map(|(id, block_number, attacker, profit, victim_count, created_at)| {
            SandwichSummary { id, block_number, attacker, profit, victim_count, created_at }
        }).collect();

        Ok(SandwichList { sandwiches, total })
    }

    pub async fn list_attackers(&self) -> Result<Vec<AttackerSummary>, sqlx::Error> {
        let rows: Vec<(Vec<u8>, i64, i64, i64)> = sqlx::query_as(
            "SELECT attacker, COUNT(*) as cnt, \
                    MIN(block_number) as first_seen, \
                    MAX(block_number) as last_seen \
             FROM sandwiches \
             GROUP BY attacker ORDER BY cnt DESC LIMIT 100"
        )
        .fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|(address, sandwich_count, first_seen, last_seen)| {
            AttackerSummary { address, sandwich_count, first_seen, last_seen }
        }).collect())
    }

    pub async fn get_sandwich(&self, id: i64) -> Result<SandwichRecord, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, block_number, front_tx_index, back_tx_index, victim_count, \
                    attacker, funder, executor, initiator, back_initiator, target, \
                    attacked_pool, profit_json::text, gas_cost_wei::bigint, coinbase_bribe::bigint, expense_wei::bigint, created_at, \
                    front_tx_hash, back_tx_hash, front_transfers::text, victim_transfers::text, back_transfers::text, \
                    victim_tx_hashes::text, \
                    coinbase \
             FROM sandwiches WHERE id = $1"
        )
        .bind(id)
        .fetch_one(&self.pool).await?;

        Ok(SandwichRecord {
            id: row.get(0),
            block_number: row.get(1),
            front_tx_index: row.get(2),
            back_tx_index: row.get(3),
            victim_count: row.get(4),
            attacker: row.get(5),
            funder: row.get(6),
            executor: row.get(7),
            initiator: row.get(8),
            back_initiator: row.get(9),
            target: row.get(10),
            attacked_pool: row.get(11),
            profit_json: row.get(12),
            gas_cost_wei: row.get(13),
            coinbase_bribe: row.get(14),
            expense_wei: row.get(15),
            created_at: row.get(16),
            front_tx_hash: row.get(17),
            back_tx_hash: row.get(18),
            front_transfers: row.get(19),
            victim_transfers: row.get(20),
            back_transfers: row.get(21),
            victim_tx_hashes: row.get(22),
            coinbase: row.get(23),
        })
    }

    pub async fn get_scan_state(&self) -> Result<ScanStateRecord, sqlx::Error> {
        let row = sqlx::query("SELECT next_block, enabled, pending_replay_from FROM scan_state WHERE id = 1")
            .fetch_one(&self.pool).await?;
        Ok(ScanStateRecord {
            next_block: row.get::<i64, _>(0),
            enabled: row.get(1),
            pending_replay_from: row.get::<i64, _>(2),
        })
    }
}
