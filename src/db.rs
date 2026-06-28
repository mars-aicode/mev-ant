//! PostgreSQL storage layer.

use alloy::primitives::{Address, B256};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;

use crate::models::{PoolId, SandwichBundle};
use crate::pools::types::{Pool, PoolKind, PoolSnapshot};

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
        // Liquidity registry tables
        r#"
        CREATE TABLE IF NOT EXISTS pools (
            address BYTEA NOT NULL,
            pool_id BYTEA NOT NULL DEFAULT '',
            kind TEXT NOT NULL,
            factory BYTEA,
            token0 BYTEA NOT NULL,
            token0_decimals SMALLINT NOT NULL DEFAULT 18,
            token1 BYTEA NOT NULL,
            token1_decimals SMALLINT NOT NULL DEFAULT 18,
            fee INTEGER,
            block_created BIGINT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (address, pool_id)
        )
        "#,
        // Destructive: pool_state was renamed to pool_snapshots and its
        // block_number column was renamed to observed_at_block. The table
        // holds the latest snapshot per pool (not per-block history), so
        // dropping it is recoverable: the LiquidityJob's daily full refresh
        // re-reads every liquid pool at the current chain head and repopulates
        // pool_snapshots. Deployments that have never run a refresh will see
        // empty pool_snapshots until the next refresh tick.
        "DROP TABLE IF EXISTS pool_state CASCADE",
        r#"
        CREATE TABLE IF NOT EXISTS pool_snapshots (
            address BYTEA NOT NULL,
            pool_id BYTEA NOT NULL DEFAULT '',
            observed_at_block BIGINT NOT NULL,
            reserve0 NUMERIC,
            reserve1 NUMERIC,
            tvl_usd NUMERIC,
            state JSONB,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (address, pool_id)
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS liquid_pools (
            address BYTEA NOT NULL,
            pool_id BYTEA NOT NULL DEFAULT '',
            rank INTEGER NOT NULL,
            tvl_usd NUMERIC NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (address, pool_id)
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS liquidity_job_state (
            id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            next_block BIGINT NOT NULL,
            last_full_refresh_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            latest_update_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        "ALTER TABLE liquidity_job_state ADD COLUMN IF NOT EXISTS last_full_refresh_at TIMESTAMPTZ",
        r#"
        CREATE TABLE IF NOT EXISTS lending_markets (
            market_address BYTEA NOT NULL,
            protocol TEXT NOT NULL,
            underlying_asset BYTEA NOT NULL,
            available_liquidity NUMERIC,
            supply_rate_ray NUMERIC,
            variable_borrow_rate_ray NUMERIC,
            stable_borrow_rate_ray NUMERIC,
            block_number BIGINT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (market_address, protocol)
        )
        "#,
        "CREATE INDEX IF NOT EXISTS idx_lending_markets_protocol ON lending_markets(protocol)",
        "CREATE INDEX IF NOT EXISTS idx_pools_tokens ON pools(token0, token1)",
        "CREATE INDEX IF NOT EXISTS idx_pools_kind ON pools(kind)",
        "CREATE INDEX IF NOT EXISTS idx_pool_snapshots_tvl ON pool_snapshots(tvl_usd DESC)",
        "CREATE INDEX IF NOT EXISTS idx_liquid_pools_rank ON liquid_pools(rank)",
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
    // Read by scanner/api callers, not inside this module.
    #[allow(dead_code)]
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

// ---------------------------------------------------------------------------
// Liquidity registry
// ---------------------------------------------------------------------------

pub async fn insert_pools(pool: &PgPool, pools: &[Pool]) -> anyhow::Result<()> {
    for p in pools {
        sqlx::query(
            r#"
            INSERT INTO pools (address, pool_id, kind, factory, token0, token0_decimals, token1, token1_decimals, fee, block_created)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (address, pool_id) DO NOTHING
            "#
        )
        .bind(p.address.as_slice())
        .bind(p.pool_id.as_slice())
        .bind(p.kind.to_string())
        .bind(p.factory.map(|a| a.as_slice().to_vec()))
        .bind(p.token0.as_slice())
        .bind(p.token0_decimals as i16)
        .bind(p.token1.as_slice())
        .bind(p.token1_decimals as i16)
        .bind(p.fee.map(|f| f as i32))
        .bind(p.block_created.map(|b| b as i64))
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn upsert_pool_snapshot(pool: &PgPool, snapshot: &PoolSnapshot) -> anyhow::Result<()> {
    let reserve0 = snapshot.reserve0.map(|r| r.to_string());
    let reserve1 = snapshot.reserve1.map(|r| r.to_string());
    sqlx::query(
        r#"
        INSERT INTO pool_snapshots (address, pool_id, observed_at_block, reserve0, reserve1, tvl_usd, state)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (address, pool_id) DO UPDATE SET
            observed_at_block = EXCLUDED.observed_at_block,
            reserve0 = EXCLUDED.reserve0,
            reserve1 = EXCLUDED.reserve1,
            tvl_usd = EXCLUDED.tvl_usd,
            state = EXCLUDED.state,
            updated_at = NOW()
        "#
    )
    .bind(snapshot.address.as_slice())
    .bind(snapshot.pool_id.as_slice())
    .bind(snapshot.observed_at_block as i64)
    .bind(reserve0)
    .bind(reserve1)
    .bind(snapshot.tvl_usd)
    .bind(snapshot.state.clone())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_pool_snapshot(
    pool: &PgPool,
    address: Address,
    pool_id: B256,
) -> anyhow::Result<Option<PoolSnapshot>> {
    let row = sqlx::query(
        r#"
        SELECT
            address, pool_id, observed_at_block, reserve0, reserve1, tvl_usd, state
        FROM pool_snapshots
        WHERE address = $1 AND pool_id = $2
        "#
    )
    .bind(address.as_slice())
    .bind(pool_id.as_slice())
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| parse_simple_snapshot_row(&r)).transpose()?)
}

fn parse_simple_snapshot_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<PoolSnapshot> {
    use sqlx::Row;
    let reserve0: Option<String> = row.get("reserve0");
    let reserve1: Option<String> = row.get("reserve1");
    Ok(PoolSnapshot {
        address: Address::from_slice(row.get("address")),
        pool_id: B256::from_slice(row.get("pool_id")),
        observed_at_block: row.get::<i64, _>("observed_at_block") as u64,
        reserve0: reserve0.and_then(|s| s.parse().ok()),
        reserve1: reserve1.and_then(|s| s.parse().ok()),
        tvl_usd: row.get("tvl_usd"),
        state: row.get::<serde_json::Value, _>("state"),
    })
}

// ---------------------------------------------------------------------------
// Liquidity job cursor
// ---------------------------------------------------------------------------

pub struct LiquidityJobState {
    pub next_block: u64,
    pub last_full_refresh_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn init_liquidity_job_state(pool: &PgPool, next_block: u64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO liquidity_job_state (id, next_block) VALUES (1, $1) ON CONFLICT (id) DO NOTHING"
    )
    .bind(next_block as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn read_liquidity_job_state(pool: &PgPool) -> anyhow::Result<LiquidityJobState> {
    let row = sqlx::query("SELECT next_block, last_full_refresh_at FROM liquidity_job_state WHERE id = 1")
        .fetch_one(pool)
        .await?;
    Ok(LiquidityJobState {
        next_block: row.get::<i64, _>(0) as u64,
        last_full_refresh_at: row.get(1),
    })
}

pub async fn advance_liquidity_job_state(pool: &PgPool, next_block: u64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE liquidity_job_state SET next_block = $1, latest_update_at = NOW() WHERE id = 1"
    )
    .bind(next_block as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_liquidity_full_refresh(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE liquidity_job_state SET last_full_refresh_at = NOW(), latest_update_at = NOW() WHERE id = 1"
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_liquid_pools(pool: &PgPool, pools: &[(Pool, f64)]) -> anyhow::Result<()> {
    // Wrap the DELETE + INSERT sequence in a transaction so a mid-failure
    // (e.g. one pool row violates a constraint) leaves the previous ranking
    // in place rather than emptying the table.
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM liquid_pools").execute(&mut *tx).await?;
    for (rank, (p, tvl)) in pools.iter().enumerate() {
        sqlx::query(
            "INSERT INTO liquid_pools (address, pool_id, rank, tvl_usd) VALUES ($1, $2, $3, $4)"
        )
        .bind(p.address.as_slice())
        .bind(p.pool_id.as_slice())
        .bind((rank + 1) as i32)
        .bind(*tvl)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn get_liquid_pools(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<(Pool, PoolSnapshot)>> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.address, p.pool_id, p.kind, p.factory,
            p.token0, p.token0_decimals, p.token1, p.token1_decimals, p.fee,
            s.observed_at_block, s.reserve0, s.reserve1, s.tvl_usd, s.state
        FROM liquid_pools lp
        JOIN pools p ON p.address = lp.address AND p.pool_id = lp.pool_id
        LEFT JOIN pool_snapshots s ON s.address = lp.address AND s.pool_id = lp.pool_id
        ORDER BY lp.rank ASC
        LIMIT $1
        "#
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let pool = parse_pool_row(&row)?;
        let state = parse_snapshot_row(&row)?;
        out.push((pool, state));
    }
    Ok(out)
}

#[allow(dead_code)]
pub async fn get_pools_for_pair(
    pool: &PgPool,
    token_a: Address,
    token_b: Address,
) -> anyhow::Result<Vec<(Pool, PoolSnapshot)>> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.address, p.pool_id, p.kind, p.factory,
            p.token0, p.token0_decimals, p.token1, p.token1_decimals, p.fee,
            s.observed_at_block, s.reserve0, s.reserve1, s.tvl_usd, s.state
        FROM pools p
        LEFT JOIN pool_snapshots s ON s.address = p.address AND s.pool_id = p.pool_id
        WHERE (p.token0 = $1 AND p.token1 = $2)
           OR (p.token0 = $2 AND p.token1 = $1)
        ORDER BY COALESCE(s.tvl_usd, 0) DESC
        "#
    )
    .bind(token_a.as_slice())
    .bind(token_b.as_slice())
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let pool = parse_pool_row(&row)?;
        let state = parse_snapshot_row(&row)?;
        out.push((pool, state));
    }
    Ok(out)
}

pub async fn get_all_pools_with_snapshots(pool: &PgPool) -> anyhow::Result<Vec<(Pool, PoolSnapshot)>> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.address, p.pool_id, p.kind, p.factory,
            p.token0, p.token0_decimals, p.token1, p.token1_decimals, p.fee,
            s.observed_at_block, s.reserve0, s.reserve1, s.tvl_usd, s.state
        FROM pools p
        LEFT JOIN pool_snapshots s ON s.address = p.address AND s.pool_id = p.pool_id
        "#
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let pool = parse_pool_row(&row)?;
        let state = parse_snapshot_row(&row)?;
        out.push((pool, state));
    }
    Ok(out)
}

/// Look up any pool in the `pools` table by `(address, pool_id)`, with
/// its latest snapshot if one exists. Returns `None` when the pool
/// is not registered (i.e. the classifier never saw it on chain and
/// the bootstrap didn't seed it).
pub async fn get_tracked_pool(
    pool: &PgPool,
    address: Address,
    pool_id: B256,
) -> anyhow::Result<Option<(Pool, PoolSnapshot)>> {
    let row = sqlx::query(
        r#"
        SELECT
            p.address, p.pool_id, p.kind, p.factory,
            p.token0, p.token0_decimals, p.token1, p.token1_decimals, p.fee,
            s.observed_at_block, s.reserve0, s.reserve1, s.tvl_usd, s.state
        FROM pools p
        LEFT JOIN pool_snapshots s ON s.address = p.address AND s.pool_id = p.pool_id
        WHERE p.address = $1 AND p.pool_id = $2
        "#
    )
    .bind(address.as_slice())
    .bind(pool_id.as_slice())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let pool = parse_pool_row(&row)?;
            let state = parse_snapshot_row(&row)?;
            Ok(Some((pool, state)))
        }
        None => Ok(None),
    }
}

fn parse_pool_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Pool> {
    use sqlx::Row;
    let kind: String = row.get("kind");
    Ok(Pool {
        address: Address::from_slice(row.get("address")),
        pool_id: B256::from_slice(row.get("pool_id")),
        kind: match kind.as_str() {
            "uniswap_v2" => PoolKind::UniswapV2,
            "uniswap_v3" => PoolKind::UniswapV3,
            "uniswap_v4" => PoolKind::UniswapV4,
            "curve_vyper" => PoolKind::CurveVyper,
            "curve_router" => PoolKind::CurveRouter,
            "balancer_v2" => PoolKind::BalancerV2,
            "balancer_v3" => PoolKind::BalancerV3,
            "dodo" => PoolKind::Dodo,
            "maverick_v1" => PoolKind::MaverickV1,
            "maverick_v2" => PoolKind::MaverickV2,
            "solidly" => PoolKind::Solidly,
            "ekubo" => PoolKind::Ekubo,
            "liquidity_book" => PoolKind::LiquidityBook,
            "fluid" => PoolKind::Fluid,
            "frax_swap" => PoolKind::FraxSwap,
            "pancake_v3" => PoolKind::PancakeV3,
            "bancor" => PoolKind::Bancor,
            _ => PoolKind::Unknown,
        },
        factory: {
            let bytes: Option<Vec<u8>> = row.get("factory");
            bytes.map(|b| Address::from_slice(&b))
        },
        token0: Address::from_slice(row.get("token0")),
        token0_decimals: row.get::<i16, _>("token0_decimals") as u8,
        token1: Address::from_slice(row.get("token1")),
        token1_decimals: row.get::<i16, _>("token1_decimals") as u8,
        fee: row.get::<Option<i32>, _>("fee").map(|f| f as u32),
        block_created: None,
    })
}

fn parse_snapshot_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<PoolSnapshot> {
    use sqlx::Row;
    let reserve0: Option<String> = row.get("reserve0");
    let reserve1: Option<String> = row.get("reserve1");
    Ok(PoolSnapshot {
        address: Address::from_slice(row.get("address")),
        pool_id: B256::from_slice(row.get("pool_id")),
        observed_at_block: row.get::<i64, _>("observed_at_block") as u64,
        reserve0: reserve0.and_then(|s| s.parse().ok()),
        reserve1: reserve1.and_then(|s| s.parse().ok()),
        tvl_usd: row.get("tvl_usd"),
        state: row.get::<serde_json::Value, _>("state"),
    })
}

// ---------------------------------------------------------------------------
// Lending markets
// ---------------------------------------------------------------------------

use crate::pools::lending::LendingMarket;

pub async fn upsert_lending_markets(
    pool: &PgPool,
    markets: &[LendingMarket],
    block_number: u64,
) -> anyhow::Result<()> {
    for m in markets {
        sqlx::query(
            r#"
            INSERT INTO lending_markets
                (market_address, protocol, underlying_asset,
                 available_liquidity, supply_rate_ray,
                 variable_borrow_rate_ray, stable_borrow_rate_ray,
                 block_number)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (market_address, protocol) DO UPDATE SET
                underlying_asset = EXCLUDED.underlying_asset,
                available_liquidity = EXCLUDED.available_liquidity,
                supply_rate_ray = EXCLUDED.supply_rate_ray,
                variable_borrow_rate_ray = EXCLUDED.variable_borrow_rate_ray,
                stable_borrow_rate_ray = EXCLUDED.stable_borrow_rate_ray,
                block_number = EXCLUDED.block_number,
                updated_at = NOW()
            "#
        )
        .bind(m.market_address.as_slice())
        .bind(m.protocol.as_str())
        .bind(m.underlying_asset.as_slice())
        .bind(m.available_liquidity.map(|v| v.to_string()))
        .bind(m.supply_rate_ray.map(|v| v.to_string()))
        .bind(m.variable_borrow_rate_ray.map(|v| v.to_string()))
        .bind(m.stable_borrow_rate_ray.map(|v| v.to_string()))
        .bind(block_number as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn list_lending_markets(pool: &PgPool) -> Result<Vec<LendingMarket>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT market_address, protocol, underlying_asset,
               available_liquidity, supply_rate_ray,
               variable_borrow_rate_ray, stable_borrow_rate_ray,
               block_number
        FROM lending_markets
        ORDER BY protocol, market_address
        "#
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        let addr: Vec<u8> = row.get("market_address");
        let underlying: Vec<u8> = row.get("underlying_asset");
        let supply_rate: Option<String> = row.get("supply_rate_ray");
        let var_borrow_rate: Option<String> = row.get("variable_borrow_rate_ray");
        let stable_borrow_rate: Option<String> = row.get("stable_borrow_rate_ray");
        let available_liquidity: Option<String> = row.get("available_liquidity");
        let protocol_str: String = row.get("protocol");
        let protocol = match protocol_str.as_str() {
            "aave_v3" => crate::pools::lending::LendingProtocol::AaveV3,
            _ => crate::pools::lending::LendingProtocol::AaveV3,
        };
        out.push(LendingMarket {
            market_address: Address::from_slice(&addr),
            protocol,
            underlying_asset: Address::from_slice(&underlying),
            available_liquidity: available_liquidity.and_then(|s| s.parse().ok()),
            supply_rate_ray: supply_rate.and_then(|s| s.parse().ok()),
            variable_borrow_rate_ray: var_borrow_rate.and_then(|s| s.parse().ok()),
            stable_borrow_rate_ray: stable_borrow_rate.and_then(|s| s.parse().ok()),
        });
    }
    Ok(out)
}
