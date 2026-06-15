//! PostgreSQL storage layer.

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;

use crate::models::{PoolId, SandwichBundle};

pub async fn init_pool(db_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(db_url)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    let statements = [
        r#"
        CREATE TABLE IF NOT EXISTS sandwiches (
            id                  BIGSERIAL PRIMARY KEY,
            block_number        BIGINT NOT NULL,
            front_tx_index      BIGINT NOT NULL,
            back_tx_index       BIGINT NOT NULL,
            victim_count        INTEGER NOT NULL,
            attacker            BYTEA NOT NULL,
            funder              BYTEA NOT NULL,
            executor            BYTEA NOT NULL,
            initiator           BYTEA NOT NULL,
            back_initiator      BYTEA NOT NULL,
            target              BYTEA NOT NULL,
            attacked_pool       TEXT NOT NULL,
            profit_json         JSONB NOT NULL DEFAULT '[]'::jsonb,
            gas_cost_wei        NUMERIC(78,0) NOT NULL DEFAULT 0,
            coinbase_bribe      NUMERIC(78,0) NOT NULL DEFAULT 0,
            expense_wei         NUMERIC(78,0) NOT NULL DEFAULT 0,
            created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            latest_update_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS blocks_scanned (
            block_number    BIGINT PRIMARY KEY,
            sandwich_count  INTEGER NOT NULL DEFAULT 0,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            latest_update_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS sandwich_attackers (
            id                  BIGSERIAL PRIMARY KEY,
            attacker            BYTEA NOT NULL,
            funder              BYTEA NOT NULL,
            executor            BYTEA NOT NULL,
            initiator           BYTEA NOT NULL,
            back_initiator      BYTEA NOT NULL,
            created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            latest_update_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS scan_state (
            id              INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            next_block      BIGINT NOT NULL,
            enabled         BOOLEAN NOT NULL DEFAULT true,
            pending_replay_from BIGINT NOT NULL DEFAULT 0,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            latest_update_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        "CREATE INDEX IF NOT EXISTS idx_sandwiches_block ON sandwiches(block_number)",
        "CREATE INDEX IF NOT EXISTS idx_sandwiches_attacker ON sandwiches(attacker)",
        "CREATE INDEX IF NOT EXISTS idx_sandwiches_profit ON sandwiches(gas_cost_wei)",
        "ALTER TABLE sandwich_attackers ADD COLUMN IF NOT EXISTS first_block BIGINT NOT NULL DEFAULT 0",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_sattackers_roles ON sandwich_attackers(attacker, funder, executor, initiator, back_initiator)",
        "CREATE INDEX IF NOT EXISTS idx_sattackers_first_block ON sandwich_attackers(first_block)",
        // Schema additions — idempotent via IF NOT EXISTS
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS front_tx_hash BYTEA",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS back_tx_hash BYTEA",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS front_transfers JSONB NOT NULL DEFAULT '[]'::jsonb",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS victim_transfers JSONB NOT NULL DEFAULT '[]'::jsonb",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS back_transfers JSONB NOT NULL DEFAULT '[]'::jsonb",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS victim_tx_hashes JSONB NOT NULL DEFAULT '[]'::jsonb",
        "ALTER TABLE sandwiches ADD COLUMN IF NOT EXISTS coinbase BYTEA",
        "ALTER TABLE scan_state ADD COLUMN IF NOT EXISTS chain_head BIGINT NOT NULL DEFAULT 0",
        "ALTER TABLE scan_state ADD COLUMN IF NOT EXISTS pending_replay_from BIGINT NOT NULL DEFAULT 0",
        "CREATE INDEX IF NOT EXISTS idx_sandwiches_attacker_block ON sandwiches(attacker, block_number DESC)",
    ];

    for stmt in &statements {
        sqlx::query(stmt).execute(pool).await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Sandwich writes
// ---------------------------------------------------------------------------

pub async fn insert_sandwiches(
    pool: &PgPool,
    bundles: &[SandwichBundle],
) -> anyhow::Result<()> {
    for bundle in bundles {
        let attacked_pool = match &bundle.attacked_pool {
            PoolId::Contract(addr) => format!("{:?}", addr),
            PoolId::Param(id) => format!("{:?}", id),
        };

        let profit_json = serde_json::to_value(&bundle.profit)?;

        sqlx::query(
            r#"
            INSERT INTO sandwiches
                (block_number, front_tx_index, back_tx_index, victim_count,
                 attacker, funder, executor, initiator, back_initiator, target,
                 attacked_pool, profit_json, gas_cost_wei, coinbase_bribe, expense_wei,
                 front_tx_hash, back_tx_hash, front_transfers, victim_transfers, back_transfers,
                 victim_tx_hashes, coinbase)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                    $16, $17, $18, $19, $20, $21, $22)
            "#,
        )
        .bind(bundle.block_number as i64)
        .bind(bundle.front_tx_index as i64)
        .bind(bundle.back_tx_index as i64)
        .bind(bundle.victim_tx_indices.len() as i32)
        .bind(bundle.attacker.to_vec())
        .bind(bundle.funder.to_vec())
        .bind(bundle.executor.to_vec())
        .bind(bundle.initiator.to_vec())
        .bind(bundle.back_initiator.to_vec())
        .bind(bundle.target.to_vec())
        .bind(&attacked_pool)
        .bind(&profit_json)
        .bind(bundle.gas_cost_wei as i64)
        .bind(bundle.coinbase_bribe as i64)
        .bind(bundle.expense_wei as i64)
        .bind(bundle.front_tx_hash.to_vec())
        .bind(bundle.back_tx_hash.to_vec())
        .bind(serde_json::to_value(&bundle.frontrun_transfers)?)
        .bind(serde_json::to_value(&bundle.victim_transfers)?)
        .bind(serde_json::to_value(&bundle.backrun_transfers)?)
        .bind(serde_json::to_value(&bundle.victim_tx_hashes.iter().map(|h| format!("0x{}", hex::encode(h.as_slice()))).collect::<Vec<_>>())?)
        .bind(bundle.coinbase.to_vec())
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT INTO sandwich_attackers (attacker, funder, executor, initiator, back_initiator, first_block)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (attacker, funder, executor, initiator, back_initiator) DO NOTHING",
        )
        .bind(bundle.attacker.to_vec())
        .bind(bundle.funder.to_vec())
        .bind(bundle.executor.to_vec())
        .bind(bundle.initiator.to_vec())
        .bind(bundle.back_initiator.to_vec())
        .bind(bundle.block_number as i64)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn mark_block_scanned(
    pool: &PgPool,
    block_number: u64,
    sandwich_count: usize,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO blocks_scanned (block_number, sandwich_count)
        VALUES ($1, $2)
        ON CONFLICT (block_number) DO UPDATE SET sandwich_count = $2, latest_update_at = NOW()
        "#,
    )
    .bind(block_number as i64)
    .bind(sandwich_count as i32)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn last_scanned_block(pool: &PgPool) -> anyhow::Result<Option<u64>> {
    let row = sqlx::query("SELECT MAX(block_number) FROM blocks_scanned")
        .fetch_one(pool)
        .await?;
    let max: Option<i64> = row.get(0);
    Ok(max.map(|b| b as u64))
}

// ---------------------------------------------------------------------------
// Scan state — singleton row for continuous scanning service
// ---------------------------------------------------------------------------

pub struct ScanState {
    pub next_block: u64,
    pub enabled: bool,
    pub chain_head: u64,
    pub pending_replay_from: u64,
}

pub async fn init_scan_state(pool: &PgPool, from_block: u64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO scan_state (id, next_block) VALUES (1, $1) ON CONFLICT (id) DO NOTHING"
    )
    .bind(from_block as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn read_scan_state(pool: &PgPool) -> anyhow::Result<ScanState> {
    let row = sqlx::query("SELECT next_block, enabled, chain_head, pending_replay_from FROM scan_state WHERE id = 1")
        .fetch_one(pool)
        .await?;
    Ok(ScanState {
        next_block: row.get::<i64, _>(0) as u64,
        enabled: row.get(1),
        chain_head: row.get::<i64, _>(2) as u64,
        pending_replay_from: row.get::<i64, _>(3) as u64,
    })
}

pub async fn set_scan_enabled(pool: &PgPool, enabled: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE scan_state SET enabled = $1, latest_update_at = NOW() WHERE id = 1")
        .bind(enabled)
        .execute(pool).await?;
    Ok(())
}

pub async fn update_chain_head(pool: &PgPool, head: u64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE scan_state SET chain_head = $1, latest_update_at = NOW() WHERE id = 1"
    )
    .bind(head as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_sandwiches_from(pool: &PgPool, from_block: u64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sandwiches WHERE block_number >= $1")
        .bind(from_block as i64)
        .execute(pool).await?;
    sqlx::query("DELETE FROM sandwich_attackers WHERE first_block >= $1 OR first_block = 0")
        .bind(from_block as i64)
        .execute(pool).await?;
    Ok(())
}

pub async fn delete_blocks_scanned_from(pool: &PgPool, from_block: u64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM blocks_scanned WHERE block_number >= $1")
        .bind(from_block as i64)
        .execute(pool).await?;
    Ok(())
}

pub async fn reset_scan_state_to(pool: &PgPool, block: u64) -> anyhow::Result<()> {
    sqlx::query("UPDATE scan_state SET next_block = $1, latest_update_at = NOW() WHERE id = 1")
        .bind(block as i64)
    .execute(pool).await?;
    Ok(())
}

pub async fn advance_scan_state(pool: &PgPool, next_block: u64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE scan_state SET next_block = $1, latest_update_at = NOW() WHERE id = 1"
    )
    .bind(next_block as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Set the `pending_replay_from` flag. Used by the admin endpoint
/// (no lock needed — the flag is a one-way signal; the scanner picks
/// it up on its next iteration).
pub async fn set_pending_replay_from(pool: &PgPool, from_block: u64) -> anyhow::Result<()> {
    sqlx::query("UPDATE scan_state SET pending_replay_from = $1, latest_update_at = NOW() WHERE id = 1")
        .bind(from_block as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clear the `pending_replay_from` flag. Called by the scanner after
/// successfully performing a replay.
pub async fn clear_pending_replay_from(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("UPDATE scan_state SET pending_replay_from = 0, latest_update_at = NOW() WHERE id = 1")
        .execute(pool)
        .await?;
    Ok(())
}
