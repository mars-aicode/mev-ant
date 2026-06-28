//! HTTP API for the management dashboard.
//!
//! This file is intentionally thin: it defines routes, extracts request
//! data, delegates to services, and serializes responses. All business
//! logic (SQL, detector orchestration, formatting) lives in `crate::services`
//! and `crate::repository`.

use alloy::primitives::Address;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tower_http::services::{ServeDir, ServeFile};

use crate::repository::SandwichRepository;
use crate::rpc::RpcClient;
use crate::services::{
    DetectRequest, DetectService, LendingService, LiquidPoolService, ReplayRequest,
    SandwichDetailQuery, SandwichListError, SandwichService, StateResponse,
};

#[derive(Clone)]
pub struct ApiState {
    pub sandwich_service: SandwichService,
    pub detect_service: Option<DetectService>,
    pub liquid_pool_service: LiquidPoolService,
    pub lending_service: LendingService,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn stats(
    State(state): State<ApiState>,
) -> Result<Json<crate::services::StatsResponse>, StatusCode> {
    state.sandwich_service.stats()
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("stats service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn sandwiches(
    State(state): State<ApiState>,
    Query(p): Query<crate::services::Pagination>,
) -> Result<Json<crate::services::SandwichListResponse>, StatusCode> {
    let page = p.page.unwrap_or(1).max(1);
    let page_size = p.page_size.unwrap_or(20).min(100);
    let block_from = p.block_from.unwrap_or(0);
    let block_to = p.block_to.unwrap_or(i64::MAX);

    state.sandwich_service.list_sandwiches(page, page_size, block_from, block_to, p.attacker.as_deref())
        .await
        .map(Json)
        .map_err(|e| match e {
            SandwichListError::BadRequest => StatusCode::BAD_REQUEST,
            SandwichListError::Sql(e) => {
                tracing::error!("sandwiches service error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })
}

async fn attackers(
    State(state): State<ApiState>,
) -> Result<Json<Vec<crate::services::AttackerRow>>, StatusCode> {
    state.sandwich_service.list_attackers()
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("attackers service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn sandwich_detail(
    State(state): State<ApiState>,
    Query(q): Query<SandwichDetailQuery>,
) -> Result<Json<crate::services::SandwichDetail>, StatusCode> {
    state.sandwich_service.get_sandwich(q.id)
        .await
        .map(Json)
        .map_err(|e| {
            if matches!(&e, sqlx::Error::RowNotFound) {
                return StatusCode::NOT_FOUND;
            }
            tracing::error!("sandwich detail service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_state(
    State(state): State<ApiState>,
) -> Result<Json<StateResponse>, StatusCode> {
    state.sandwich_service.get_state()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn pause(State(state): State<ApiState>) -> Result<StatusCode, StatusCode> {
    state.sandwich_service.pause()
        .await
        .map(|_| StatusCode::OK)
        .map_err(|e| {
            tracing::error!("pause service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn resume(State(state): State<ApiState>) -> Result<StatusCode, StatusCode> {
    state.sandwich_service.resume()
        .await
        .map(|_| StatusCode::OK)
        .map_err(|e| {
            tracing::error!("resume service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn replay(
    State(state): State<ApiState>,
    Json(body): Json<ReplayRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let from = body.from_block.max(0) as u64;

    state.sandwich_service.queue_replay(from)
        .await
        .map_err(|e| {
            tracing::error!("replay service error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "status": "queued",
        "from_block": from,
    })))
}

async fn detect(
    State(state): State<ApiState>,
    Query(q): Query<DetectRequest>,
) -> Result<Json<crate::services::DetectResponse>, StatusCode> {
    let service = state.detect_service.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let block_number = q.block_number.max(0) as u64;

    service.detect(block_number)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("detect service error: {:?}", e);
            StatusCode::BAD_GATEWAY
        })
}

async fn liquid_pools(
    State(state): State<ApiState>,
    Query(q): Query<LiquidPoolQuery>,
) -> Result<Json<crate::services::LiquidPoolListResponse>, StatusCode> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let token = match q.token.as_deref() {
        Some(s) if !s.is_empty() => Some(s.parse::<Address>().map_err(|_| StatusCode::BAD_REQUEST)?),
        _ => None,
    };
    state.liquid_pool_service.list_liquid_pools(limit, token)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("liquid pools service error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn liquid_pool(
    State(state): State<ApiState>,
    Path(pool): Path<String>,
) -> Result<Json<crate::services::LiquidPoolResponse>, StatusCode> {
    let (address, pool_id) = crate::services::parse_pool_key(&pool)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let row = state.liquid_pool_service.get_liquid_pool(address, pool_id)
        .await
        .map_err(|e| {
            tracing::error!("liquid pool service error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    match row {
        Some(row) => Ok(Json(crate::services::LiquidPoolResponse { pool: row })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn tracked_pool(
    State(state): State<ApiState>,
    Path(pool): Path<String>,
) -> Result<Json<crate::services::TrackedPoolResponse>, StatusCode> {
    let (address, pool_id) = crate::services::parse_pool_key(&pool)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let row = state.liquid_pool_service.get_tracked_pool(address, pool_id)
        .await
        .map_err(|e| {
            tracing::error!("tracked pool service error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    match row {
        Some(row) => Ok(Json(crate::services::TrackedPoolResponse { pool: row })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn routes(
    State(state): State<ApiState>,
    Query(q): Query<RouteQuery>,
) -> Result<Json<crate::services::RouteListResponse>, StatusCode> {
    let from: Address = q.from.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let to: Address = q.to.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let amount = q.amount.as_deref()
        .and_then(|s| s.parse::<alloy::primitives::U256>().ok());
    let max_hops = q.max_hops.unwrap_or(3).clamp(1, 4) as usize;

    state.liquid_pool_service.find_routes(from, to, amount, max_hops)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("routing service error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn lending_markets(
    State(state): State<ApiState>,
) -> Result<Json<crate::services::LendingMarketListResponse>, StatusCode> {
    state.lending_service.list_markets()
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("lending service error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

#[derive(Deserialize)]
struct LiquidPoolQuery {
    limit: Option<i64>,
    /// Filter to pools whose `token0` or `token1` matches this address.
    token: Option<String>,
}

#[derive(Deserialize)]
struct RouteQuery {
    from: String,
    to: String,
    amount: Option<String>,
    max_hops: Option<i64>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn build_router(pool: sqlx::PgPool, provider: Option<RpcClient>, dashboard_dir: Option<String>) -> Router {
    let repo = SandwichRepository::new(pool.clone());
    let sandwich_service = SandwichService::new(repo);
    let detect_service = provider.map(DetectService::new);
    let liquid_pool_service = LiquidPoolService::new(pool.clone());
    let lending_service = LendingService::new(pool);
    let state = ApiState { sandwich_service, detect_service, liquid_pool_service, lending_service };

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
        .route("/api/liquid-pools", get(liquid_pools))
        .route("/api/liquid-pools/:pool", get(liquid_pool))
        .route("/api/pools/:pool", get(tracked_pool))
        .route("/api/routes", get(routes))
        .route("/api/lending-markets", get(lending_markets))
        .layer(
            tower_http::cors::CorsLayer::permissive()
        )
        .with_state(state);

    if let Some(dir) = dashboard_dir {
        let index_path = std::path::PathBuf::from(&dir).join("index.html");
        let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index_path));
        router = router.fallback_service(serve_dir);
    }

    router
}
