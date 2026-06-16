//! HTTP API for the management dashboard.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tower_http::services::{ServeDir, ServeFile};

use sqlx::Row;

use crate::db;
use crate::detector::sandwich::detect_sandwiches;
use crate::rpc::RpcClient;

#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
    pub provider: Option<RpcClient>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct StatsResponse {
    total_sandwiches: i64,
    distinct_attackers: i64,
    blocks_scanned: i64,
    chain_head: i64,
    current_block: i64,
}

#[derive(Serialize)]
pub struct SandwichRow {
    id: i64,
    block_number: i64,
    attacker: String,
    profit: String,
    victim_count: i32,
    scanned_at: String,
}

#[derive(Serialize)]
pub struct SandwichListResponse {
    sandwiches: Vec<SandwichRow>,
    total: i64,
}

#[derive(Serialize)]
pub struct AttackerRow {
    address: String,
    sandwich_count: i64,
    first_seen: i64,
    last_seen: i64,
}

#[derive(Serialize)]
pub struct StateResponse {
    next_block: i64,
    enabled: bool,
    pending_replay_from: i64,
}

#[derive(Deserialize)]
pub struct Pagination {
    page: Option<i64>,
    page_size: Option<i64>,
    attacker: Option<String>,
    block_from: Option<i64>,
    block_to: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn stats(
    State(state): State<ApiState>,
) -> Result<Json<StatsResponse>, StatusCode> {
    let row = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM sandwiches")
        .fetch_one(&state.pool).await.map_err(|e| { tracing::error!("stats query error: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let total_sandwiches = row.0;

    let row = sqlx::query_as::<_, (i64,)>("SELECT COUNT(DISTINCT attacker) FROM sandwich_attackers")
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let distinct_attackers = row.0;

    let row = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM blocks_scanned")
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let blocks_scanned = row.0;

    let state_row = sqlx::query("SELECT next_block, chain_head FROM scan_state WHERE id = 1")
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let current_block: i64 = state_row.get(0);
    let chain_head: i64 = state_row.get(1);

    Ok(Json(StatsResponse {
        total_sandwiches, distinct_attackers, blocks_scanned, chain_head, current_block,
    }))
}

async fn sandwiches(
    State(state): State<ApiState>,
    Query(p): Query<Pagination>,
) -> Result<Json<SandwichListResponse>, StatusCode> {
    let page = p.page.unwrap_or(1).max(1);
    let page_size = p.page_size.unwrap_or(20).min(100);
    let offset = (page - 1) * page_size;
    let block_from = p.block_from.unwrap_or(0);
    let block_to = p.block_to.unwrap_or(i64::MAX);

    let (total, sandwiches) = if let Some(ref attacker) = p.attacker {
        let attacker_bytes = hex::decode(attacker.strip_prefix("0x").unwrap_or(attacker))
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let (total,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sandwiches \
             WHERE attacker = $1 AND block_number >= $2 AND block_number <= $3"
        )
        .bind(&attacker_bytes).bind(block_from).bind(block_to)
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let rows: Vec<(i64, i64, Vec<u8>, String, i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT id, block_number, attacker, profit_json::text, victim_count, created_at \
             FROM sandwiches \
             WHERE attacker = $1 AND block_number >= $2 AND block_number <= $3 \
             ORDER BY block_number DESC LIMIT $4 OFFSET $5"
        )
        .bind(&attacker_bytes).bind(block_from).bind(block_to)
        .bind(page_size).bind(offset)
        .fetch_all(&state.pool).await.map_err(|e| {
            tracing::error!("sandwiches query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        (total, rows.into_iter().map(|(id, block, addr, profit, victims, ts)| SandwichRow {
            id,
            block_number: block,
            attacker: hex::encode(addr),
            profit,
            victim_count: victims,
            scanned_at: ts.format("%Y-%m-%d %H:%M:%S").to_string(),
        }).collect())
    } else {
        let (total,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sandwiches WHERE block_number >= $1 AND block_number <= $2"
        )
        .bind(block_from).bind(block_to)
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let rows: Vec<(i64, i64, Vec<u8>, String, i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT id, block_number, attacker, profit_json::text, victim_count, created_at \
             FROM sandwiches WHERE block_number >= $1 AND block_number <= $2 \
             ORDER BY block_number DESC LIMIT $3 OFFSET $4"
        )
        .bind(block_from).bind(block_to).bind(page_size).bind(offset)
        .fetch_all(&state.pool).await.map_err(|e| {
            tracing::error!("sandwiches query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        (total, rows.into_iter().map(|(id, block, addr, profit, victims, ts)| SandwichRow {
            id,
            block_number: block,
            attacker: hex::encode(addr),
            profit,
            victim_count: victims,
            scanned_at: ts.format("%Y-%m-%d %H:%M:%S").to_string(),
        }).collect())
    };

    Ok(Json(SandwichListResponse { sandwiches, total }))
}

async fn attackers(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AttackerRow>>, StatusCode> {
    let rows: Vec<(Vec<u8>, i64, i64, i64)> = sqlx::query_as(
        "SELECT attacker, COUNT(*) as cnt, \
                MIN(block_number) as first_seen, \
                MAX(block_number) as last_seen \
         FROM sandwiches \
         GROUP BY attacker ORDER BY cnt DESC LIMIT 100"
    )
    .fetch_all(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let attackers = rows.into_iter().map(|(addr, cnt, first, last)| AttackerRow {
        address: hex::encode(addr),
        sandwich_count: cnt,
        first_seen: first,
        last_seen: last,
    }).collect();

    Ok(Json(attackers))
}

#[derive(Deserialize)]
pub struct SandwichDetailQuery {
    id: i64,
}

#[derive(Serialize)]
pub struct SandwichDetail {
    id: i64,
    block_number: i64,
    front_tx_index: i64,
    back_tx_index: i64,
    victim_count: i32,
    attacker: String,
    funder: String,
    executor: String,
    initiator: String,
    back_initiator: String,
    target: String,
    attacked_pool: String,
    profit_json: String,
    gas_cost_wei: i64,
    coinbase_bribe: i64,
    expense_wei: i64,
    pure_profit_wei: i64,
    created_at: String,
    front_tx_hash: String,
    back_tx_hash: String,
    front_transfers: String,
    victim_transfers: String,
    back_transfers: String,
    victim_tx_hashes: String,
    coinbase: String,
}

fn hex_or_empty(bytes: Option<&[u8]>) -> String {
    bytes.map(|b| format!("0x{}", hex::encode(b))).unwrap_or_default()
}

const WETH: alloy::primitives::Address = alloy::primitives::Address::new(hex_literal::hex!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
const ETH: alloy::primitives::Address = crate::models::ETH_TRANSFER_ADDR;

/// Compute "pure profit" in wei: gross WETH/ETH profit minus total attacker
/// expense (gas + bribes). ETH-denominated because cost is always ETH and
/// WETH unwraps 1:1. Returns signed i64 (may be negative for loss-making
/// sandwiches).
fn pure_profit_wei(profit_json: &str, expense_wei: i64) -> i64 {
    #[derive(serde::Deserialize)]
    struct ProfitItem {
        token: String,
        amount: String,
    }
    let items: Vec<ProfitItem> = serde_json::from_str(profit_json).unwrap_or_default();
    let mut gross: i128 = 0;
    for item in items {
        let token = item.token.to_lowercase();
        if token != format!("0x{}", hex::encode(WETH)).to_lowercase()
            && token != format!("0x{}", hex::encode(ETH)).to_lowercase()
        {
            continue;
        }
        // amount is a signed decimal string (I256 serializes as decimal)
        let sign = if item.amount.starts_with('-') { -1 } else { 1 };
        let digits = item.amount.trim_start_matches('-');
        let value: i128 = digits.parse().unwrap_or(0);
        gross += sign * value;
    }
    gross.saturating_sub(expense_wei as i128) as i64
}

async fn sandwich_detail(
    State(state): State<ApiState>,
    Query(q): Query<SandwichDetailQuery>,
) -> Result<Json<SandwichDetail>, StatusCode> {
    let row = sqlx::query(
        "SELECT id, block_number, front_tx_index, back_tx_index, victim_count, \
                attacker, funder, executor, initiator, back_initiator, target, \
                attacked_pool, profit_json::text, gas_cost_wei::bigint, coinbase_bribe::bigint, expense_wei::bigint, created_at, \
                front_tx_hash, back_tx_hash, front_transfers::text, victim_transfers::text, back_transfers::text, \
                victim_tx_hashes::text, \
                coinbase \
         FROM sandwiches WHERE id = $1"
    )
    .bind(q.id)
    .fetch_one(&state.pool).await.map_err(|e| {
        if matches!(&e, sqlx::Error::RowNotFound) {
            return StatusCode::NOT_FOUND;
        }
        tracing::error!("detail query error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(SandwichDetail {
        id: row.get(0),
        block_number: row.get(1),
        front_tx_index: row.get(2),
        back_tx_index: row.get(3),
        victim_count: row.get(4),
        attacker: hex::encode(row.get::<Vec<u8>, _>(5)),
        funder: hex::encode(row.get::<Vec<u8>, _>(6)),
        executor: hex::encode(row.get::<Vec<u8>, _>(7)),
        initiator: hex::encode(row.get::<Vec<u8>, _>(8)),
        back_initiator: hex::encode(row.get::<Vec<u8>, _>(9)),
        target: hex::encode(row.get::<Vec<u8>, _>(10)),
        attacked_pool: row.get(11),
        profit_json: row.get(12),
        gas_cost_wei: row.get(13),
        coinbase_bribe: row.get(14),
        expense_wei: row.get(15),
        pure_profit_wei: pure_profit_wei(row.get::<String, _>(12).as_str(), row.get(15)),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>(16).format("%Y-%m-%d %H:%M:%S").to_string(),
        front_tx_hash: hex_or_empty(row.get::<Option<Vec<u8>>, _>(17).as_deref()),
        back_tx_hash: hex_or_empty(row.get::<Option<Vec<u8>>, _>(18).as_deref()),
        front_transfers: row.get::<Option<String>, _>(19).unwrap_or_default(),
        victim_transfers: row.get::<Option<String>, _>(20).unwrap_or_default(),
        back_transfers: row.get::<Option<String>, _>(21).unwrap_or_default(),
        victim_tx_hashes: row.get::<Option<String>, _>(22).unwrap_or_default(),
        coinbase: hex_or_empty(row.get::<Option<Vec<u8>>, _>(23).as_deref()),
    }))
}

async fn get_state(
    State(state): State<ApiState>,
) -> Result<Json<StateResponse>, StatusCode> {
    let row = sqlx::query("SELECT next_block, enabled, pending_replay_from FROM scan_state WHERE id = 1")
        .fetch_one(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(StateResponse {
        next_block: row.get::<i64, _>(0),
        enabled: row.get(1),
        pending_replay_from: row.get::<i64, _>(2),
    }))
}

async fn pause(State(state): State<ApiState>) -> Result<StatusCode, StatusCode> {
    db::set_scan_enabled(&state.pool, false)
        .await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

async fn resume(State(state): State<ApiState>) -> Result<StatusCode, StatusCode> {
    db::set_scan_enabled(&state.pool, true)
        .await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ReplayRequest {
    from_block: i64,
}

// ---------------------------------------------------------------------------
// Detect (test check)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct DetectRequest {
    block_number: i64,
}

#[derive(Serialize)]
pub struct DetectedBundle {
    id: i64,
    block_number: i64,
    front_tx_index: i64,
    back_tx_index: i64,
    victim_count: i32,
    attacker: String,
    funder: String,
    executor: String,
    initiator: String,
    back_initiator: String,
    target: String,
    attacked_pool: String,
    profit_json: String,
    gas_cost_wei: i64,
    coinbase_bribe: i64,
    expense_wei: i64,
    pure_profit_wei: i64,
    created_at: String,
    front_tx_hash: String,
    back_tx_hash: String,
    front_transfers: String,
    victim_transfers: String,
    back_transfers: String,
    victim_tx_hashes: String,
    coinbase: String,
}

#[derive(Serialize)]
pub struct DetectResponse {
    bundles: Vec<DetectedBundle>,
}

fn address_hex(a: alloy::primitives::Address) -> String {
    format!("0x{}", hex::encode(a))
}

fn b256_hex(b: alloy::primitives::B256) -> String {
    format!("0x{}", hex::encode(b))
}

fn pool_id_hex(p: crate::models::PoolId) -> String {
    match p {
        crate::models::PoolId::Contract(a) => address_hex(a),
        crate::models::PoolId::Param(id) => format!("0x{}", hex::encode(id)),
    }
}

async fn detect(
    State(state): State<ApiState>,
    Query(q): Query<DetectRequest>,
) -> Result<Json<DetectResponse>, StatusCode> {
    let provider = state.provider.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let block_number = q.block_number.max(0) as u64;

    let data = crate::rpc::fetch_block(provider, block_number)
        .await
        .map_err(|e| {
            tracing::error!("detect fetch_block error: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

    let bundles = detect_sandwiches(
        block_number,
        &data.flows,
        &data.raw_logs,
        data.coinbase,
        crate::DEFAULT_BLACKLIST,
        crate::DEFAULT_TOKENS,
    );

    let detected: Vec<DetectedBundle> = bundles.into_iter().enumerate().map(|(idx, b)| {
        let profit_json = serde_json::to_string(&b.profit).unwrap_or_else(|_| "[]".to_string());
        let front_transfers = serde_json::to_string(&b.frontrun_transfers).unwrap_or_else(|_| "[]".to_string());
        let victim_transfers = serde_json::to_string(&b.victim_transfers).unwrap_or_else(|_| "[]".to_string());
        let back_transfers = serde_json::to_string(&b.backrun_transfers).unwrap_or_else(|_| "[]".to_string());
        let victim_tx_hashes = serde_json::to_string(&b.victim_tx_hashes).unwrap_or_else(|_| "[]".to_string());
        let pure_profit = pure_profit_wei(&profit_json, b.expense_wei as i64);

        DetectedBundle {
            id: (idx + 1) as i64,
            block_number: b.block_number as i64,
            front_tx_index: b.front_tx_index as i64,
            back_tx_index: b.back_tx_index as i64,
            victim_count: b.victim_tx_indices.len() as i32,
            attacker: address_hex(b.attacker),
            funder: address_hex(b.funder),
            executor: address_hex(b.executor),
            initiator: address_hex(b.initiator),
            back_initiator: address_hex(b.back_initiator),
            target: address_hex(b.target),
            attacked_pool: pool_id_hex(b.attacked_pool),
            profit_json,
            gas_cost_wei: b.gas_cost_wei as i64,
            coinbase_bribe: b.coinbase_bribe as i64,
            expense_wei: b.expense_wei as i64,
            pure_profit_wei: pure_profit,
            created_at: "".to_string(),
            front_tx_hash: b256_hex(b.front_tx_hash),
            back_tx_hash: b256_hex(b.back_tx_hash),
            front_transfers,
            victim_transfers,
            back_transfers,
            victim_tx_hashes,
            coinbase: address_hex(b.coinbase),
        }
    }).collect();

    Ok(Json(DetectResponse { bundles: detected }))
}

async fn replay(
    State(state): State<ApiState>,
    Json(body): Json<ReplayRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let from = body.from_block.max(0) as u64;

    // Just set the flag. The scanner picks it up on its next iteration
    // and performs the replay atomically under its own advisory lock
    // (delete data, reset cursor, clear flag, auto-resume if paused).
    // No lock here — the flag is a one-way signal; concurrent writes
    // are idempotent. The user gets immediate "queued" feedback and
    // can poll /api/state to watch `pending_replay_from` clear to 0.
    db::set_pending_replay_from(&state.pool, from)
        .await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "status": "queued",
        "from_block": from,
    })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn build_router(pool: PgPool, provider: Option<RpcClient>, dashboard_dir: Option<String>) -> Router {
    let state = ApiState { pool, provider };

    let mut router = Router::new()
        .route("/api/stats", get(stats))
        .route("/api/sandwiches", get(sandwiches))
        .route("/api/sandwich", get(sandwich_detail))
        .route("/api/attackers", get(attackers))
        .route("/api/state", get(get_state))
        .route("/api/state/pause", post(pause))
        .route("/api/state/resume", post(resume))
        .route("/api/replay", post(replay))
        .route("/api/detect", get(detect))
        .layer(
            tower_http::cors::CorsLayer::permissive()
        )
        .with_state(state);

    // Static files first (for assets like JS, CSS), then SPA fallback
    if let Some(dir) = dashboard_dir {
        let index_path = std::path::PathBuf::from(&dir).join("index.html");
        let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index_path));
        router = router.fallback_service(serve_dir);
    }

    router
}
