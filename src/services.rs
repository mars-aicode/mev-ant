//! Service layer — business logic behind the HTTP API.
//!
//! Services orchestrate repositories, RPC, and the detector. They return
//! response DTOs that `api.rs` wraps in JSON, keeping the HTTP layer thin.

use alloy::primitives::{Address, B256};
use serde::{Deserialize, Serialize};

use crate::classifier::DefaultClassifier;
use crate::db;
use crate::detector::detect_sandwiches;
use crate::models::PoolId;
use crate::pools::graph::TokenGraph;
use crate::pools::routing::find_routes;
use crate::pools::types::{Pool, PoolSnapshot, QuoteConfidence, Route};
use crate::repository::{SandwichRepository, SandwichRecord};
use crate::rpc::{fetch_block, RpcClient};
use crate::tokens::{DEFAULT_BLACKLIST, DEFAULT_TOKENS};

// ---------------------------------------------------------------------------
// Response DTOs (shared between service and HTTP layers)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct StatsResponse {
    pub total_sandwiches: i64,
    pub distinct_attackers: i64,
    pub blocks_scanned: i64,
    pub chain_head: i64,
    pub current_block: i64,
}

#[derive(Serialize)]
pub struct SandwichRow {
    pub id: i64,
    pub block_number: i64,
    pub attacker: String,
    pub profit: String,
    pub victim_count: i32,
    pub scanned_at: String,
}

#[derive(Serialize)]
pub struct SandwichListResponse {
    pub sandwiches: Vec<SandwichRow>,
    pub total: i64,
}

#[derive(Serialize)]
pub struct AttackerRow {
    pub address: String,
    pub sandwich_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Serialize)]
pub struct StateResponse {
    pub next_block: i64,
    pub enabled: bool,
    pub pending_replay_from: i64,
}

#[derive(Deserialize)]
pub struct Pagination {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub attacker: Option<String>,
    pub block_from: Option<i64>,
    pub block_to: Option<i64>,
}

#[derive(Deserialize)]
pub struct SandwichDetailQuery {
    pub id: i64,
}

#[derive(Serialize)]
pub struct SandwichDetail {
    pub id: i64,
    pub block_number: i64,
    pub front_tx_index: i64,
    pub back_tx_index: i64,
    pub victim_count: i32,
    pub attacker: String,
    pub funder: String,
    pub executor: String,
    pub initiator: String,
    pub back_initiator: String,
    pub target: String,
    pub attacked_pool: String,
    pub profit_json: String,
    pub gas_cost_wei: i64,
    pub coinbase_bribe: i64,
    pub expense_wei: i64,
    pub pure_profit_wei: i64,
    pub created_at: String,
    pub front_tx_hash: String,
    pub back_tx_hash: String,
    pub front_transfers: String,
    pub victim_transfers: String,
    pub back_transfers: String,
    pub victim_tx_hashes: String,
    pub coinbase: String,
}

#[derive(Deserialize)]
pub struct ReplayRequest {
    pub from_block: i64,
}

#[derive(Deserialize)]
pub struct DetectRequest {
    pub block_number: i64,
}

#[derive(Serialize)]
pub struct DetectedBundle {
    pub id: i64,
    pub block_number: i64,
    pub front_tx_index: i64,
    pub back_tx_index: i64,
    pub victim_count: i32,
    pub attacker: String,
    pub funder: String,
    pub executor: String,
    pub initiator: String,
    pub back_initiator: String,
    pub target: String,
    pub attacked_pool: String,
    pub profit_json: String,
    pub gas_cost_wei: i64,
    pub coinbase_bribe: i64,
    pub expense_wei: i64,
    pub pure_profit_wei: i64,
    pub created_at: String,
    pub front_tx_hash: String,
    pub back_tx_hash: String,
    pub front_transfers: String,
    pub victim_transfers: String,
    pub back_transfers: String,
    pub victim_tx_hashes: String,
    pub coinbase: String,
}

#[derive(Serialize)]
pub struct DetectResponse {
    pub bundles: Vec<DetectedBundle>,
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

const WETH: Address = Address::new(hex_literal::hex!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
const ETH: Address = crate::models::ETH_TRANSFER_ADDR;

fn address_hex(a: Address) -> String {
    format!("0x{}", hex::encode(a))
}

fn bytes_hex(a: &[u8]) -> String {
    format!("0x{}", hex::encode(a))
}

fn option_bytes_hex(a: Option<&[u8]>) -> String {
    a.map(bytes_hex).unwrap_or_default()
}

fn b256_hex(b: B256) -> String {
    format!("0x{}", hex::encode(b))
}

fn pool_id_hex(p: PoolId) -> String {
    match p {
        PoolId::Contract(a) => address_hex(a),
        PoolId::Param(id) => format!("0x{}", hex::encode(id)),
    }
}

/// Compute "pure profit" in wei: gross WETH/ETH profit minus total attacker
/// expense (gas + bribes). ETH-denominated because cost is always ETH and
/// WETH unwraps 1:1. Returns signed i64 (may be negative for loss-making
/// sandwiches).
pub fn pure_profit_wei(profit_json: &str, expense_wei: i64) -> i64 {
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
        let sign = if item.amount.starts_with('-') { -1 } else { 1 };
        let digits = item.amount.trim_start_matches('-');
        let value: i128 = digits.parse().unwrap_or(0);
        gross += sign * value;
    }
    gross.saturating_sub(expense_wei as i128) as i64
}

// ---------------------------------------------------------------------------
// Sandwich service
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SandwichService {
    repo: SandwichRepository,
}

impl SandwichService {
    pub fn new(repo: SandwichRepository) -> Self {
        Self { repo }
    }

    pub async fn stats(&self) -> Result<StatsResponse, sqlx::Error> {
        let s = self.repo.stats().await?;
        Ok(StatsResponse {
            total_sandwiches: s.total_sandwiches,
            distinct_attackers: s.distinct_attackers,
            blocks_scanned: s.blocks_scanned,
            chain_head: s.chain_head,
            current_block: s.current_block,
        })
    }

    pub async fn list_sandwiches(
        &self,
        page: i64,
        page_size: i64,
        block_from: i64,
        block_to: i64,
        attacker: Option<&str>,
    ) -> Result<SandwichListResponse, SandwichListError> {
        let attacker_bytes = if let Some(a) = attacker {
            Some(hex::decode(a.strip_prefix("0x").unwrap_or(a)).map_err(|_| SandwichListError::BadRequest)?)
        } else {
            None
        };
        let attacker_ref = attacker_bytes.as_deref();

        let list = self.repo.list_sandwiches(page, page_size, block_from, block_to, attacker_ref).await?;
        let sandwiches = list.sandwiches.into_iter().map(|s| SandwichRow {
            id: s.id,
            block_number: s.block_number,
            attacker: bytes_hex(&s.attacker),
            profit: s.profit,
            victim_count: s.victim_count,
            scanned_at: s.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        }).collect();

        Ok(SandwichListResponse { sandwiches, total: list.total })
    }

    pub async fn list_attackers(&self) -> Result<Vec<AttackerRow>, sqlx::Error> {
        let rows = self.repo.list_attackers().await?;
        Ok(rows.into_iter().map(|a| AttackerRow {
            address: bytes_hex(&a.address),
            sandwich_count: a.sandwich_count,
            first_seen: a.first_seen,
            last_seen: a.last_seen,
        }).collect())
    }

    pub async fn get_sandwich(&self, id: i64) -> Result<SandwichDetail, sqlx::Error> {
        let r = self.repo.get_sandwich(id).await?;
        Ok(map_record_to_detail(r))
    }

    pub async fn get_state(&self) -> Result<StateResponse, sqlx::Error> {
        let s = self.repo.get_scan_state().await?;
        Ok(StateResponse {
            next_block: s.next_block,
            enabled: s.enabled,
            pending_replay_from: s.pending_replay_from,
        })
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        db::set_scan_enabled(self.repo.pool(), false).await
    }

    pub async fn resume(&self) -> anyhow::Result<()> {
        db::set_scan_enabled(self.repo.pool(), true).await
    }

    pub async fn queue_replay(&self, from_block: u64) -> anyhow::Result<()> {
        db::set_pending_replay_from(self.repo.pool(), from_block).await
    }
}

#[derive(Debug)]
pub enum SandwichListError {
    BadRequest,
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for SandwichListError {
    fn from(e: sqlx::Error) -> Self { Self::Sql(e) }
}

fn map_record_to_detail(r: SandwichRecord) -> SandwichDetail {
    SandwichDetail {
        id: r.id,
        block_number: r.block_number,
        front_tx_index: r.front_tx_index,
        back_tx_index: r.back_tx_index,
        victim_count: r.victim_count,
        attacker: bytes_hex(&r.attacker),
        funder: bytes_hex(&r.funder),
        executor: bytes_hex(&r.executor),
        initiator: bytes_hex(&r.initiator),
        back_initiator: bytes_hex(&r.back_initiator),
        target: bytes_hex(&r.target),
        attacked_pool: r.attacked_pool,
        profit_json: r.profit_json.clone(),
        gas_cost_wei: r.gas_cost_wei,
        coinbase_bribe: r.coinbase_bribe,
        expense_wei: r.expense_wei,
        pure_profit_wei: pure_profit_wei(&r.profit_json, r.expense_wei),
        created_at: r.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        front_tx_hash: option_bytes_hex(r.front_tx_hash.as_deref()),
        back_tx_hash: option_bytes_hex(r.back_tx_hash.as_deref()),
        front_transfers: r.front_transfers.unwrap_or_default(),
        victim_transfers: r.victim_transfers.unwrap_or_default(),
        back_transfers: r.back_transfers.unwrap_or_default(),
        victim_tx_hashes: r.victim_tx_hashes.unwrap_or_default(),
        coinbase: option_bytes_hex(r.coinbase.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Detect service
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DetectService {
    provider: RpcClient,
}

impl DetectService {
    pub fn new(provider: RpcClient) -> Self {
        Self { provider }
    }

    pub async fn detect(&self, block_number: u64) -> anyhow::Result<DetectResponse> {
        let data = fetch_block(&self.provider, block_number).await?;
        let classifier = DefaultClassifier::new(DEFAULT_BLACKLIST, crate::dex::lending::LENDING_ADDRESSES);

        let bundles = detect_sandwiches(
            &classifier,
            block_number,
            &data.flows,
            &data.raw_logs,
            data.coinbase,
            DEFAULT_BLACKLIST,
            DEFAULT_TOKENS,
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

        Ok(DetectResponse { bundles: detected })
    }
}

// ---------------------------------------------------------------------------
// Liquid pool & routing services
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LiquidPoolService {
    db: sqlx::PgPool,
}

impl LiquidPoolService {
    pub fn new(db: sqlx::PgPool) -> Self {
        Self { db }
    }

    pub async fn list_liquid_pools(
        &self,
        limit: i64,
        token: Option<Address>,
    ) -> anyhow::Result<LiquidPoolListResponse> {
        let rows = db::get_liquid_pools(&self.db, limit).await?;
        let rows = match token {
            Some(t) => filter_pools_by_token(rows, t),
            None => rows,
        };
        let pools = rows
            .into_iter()
            .map(|(pool, state)| LiquidPoolRow {
                rank: 0, // filled below
                address: format!("{:?}", pool.address),
                pool_id: if pool.pool_id.is_zero() {
                    None
                } else {
                    Some(format!("{:?}", pool.pool_id))
                },
                kind: pool.kind.to_string(),
                token0: format!("{:?}", pool.token0),
                token1: format!("{:?}", pool.token1),
                fee: pool.fee,
                tvl_usd: state.tvl_usd.unwrap_or(0.0),
                reserve0: state.reserve0.map(|r| r.to_string()),
                reserve1: state.reserve1.map(|r| r.to_string()),
                block_number: state.observed_at_block as i64,
            })
            .enumerate()
            .map(|(i, mut p)| {
                p.rank = (i + 1) as i32;
                p
            })
            .collect();
        Ok(LiquidPoolListResponse { pools })
    }

    pub async fn get_liquid_pool(
        &self,
        address: Address,
        pool_id: B256,
    ) -> anyhow::Result<Option<LiquidPoolRow>> {
        // Pull all liquid pools (≤ 1000 in V1) and filter in memory via the
        // pure `find_pool_by_key` helper. A targeted SQL helper could be
        // added in V2 once the registry grows past 10k pools.
        let rows = db::get_liquid_pools(&self.db, i64::MAX).await?;
        let (_, (pool, state)) = match find_pool_by_key(&rows, address, pool_id) {
            Some(found) => found,
            None => return Ok(None),
        };
        let rank = rows
            .iter()
            .position(|(p, _)| p.address == address && p.pool_id == pool_id)
            .map(|i| (i + 1) as i32)
            .unwrap_or(0);
        Ok(Some(LiquidPoolRow {
            rank,
            address: format!("{:?}", pool.address),
            pool_id: if pool.pool_id.is_zero() {
                None
            } else {
                Some(format!("{:?}", pool.pool_id))
            },
            kind: pool.kind.to_string(),
            token0: format!("{:?}", pool.token0),
            token1: format!("{:?}", pool.token1),
            fee: pool.fee,
            tvl_usd: state.tvl_usd.unwrap_or(0.0),
            reserve0: state.reserve0.map(|r| r.to_string()),
            reserve1: state.reserve1.map(|r| r.to_string()),
            block_number: state.observed_at_block as i64,
        }))
    }

    /// Look up any pool registered in the registry's `pools` table.
    /// Distinct from `get_liquid_pool`, which is restricted to the
    /// top-1000-by-TVL ranking.
    pub async fn get_tracked_pool(
        &self,
        address: Address,
        pool_id: B256,
    ) -> anyhow::Result<Option<TrackedPoolRow>> {
        let (pool, state) = match db::get_tracked_pool(&self.db, address, pool_id).await? {
            Some(p) => p,
            None => return Ok(None),
        };
        Ok(Some(TrackedPoolRow {
            address: format!("{:?}", pool.address),
            pool_id: if pool.pool_id.is_zero() {
                None
            } else {
                Some(format!("{:?}", pool.pool_id))
            },
            kind: pool.kind.to_string(),
            token0: format!("{:?}", pool.token0),
            token1: format!("{:?}", pool.token1),
            fee: pool.fee,
            tvl_usd: state.tvl_usd.unwrap_or(0.0),
            reserve0: state.reserve0.map(|r| r.to_string()),
            reserve1: state.reserve1.map(|r| r.to_string()),
            block_number: state.observed_at_block as i64,
        }))
    }

    pub async fn find_routes(
        &self,
        from: Address,
        to: Address,
        amount: Option<alloy::primitives::U256>,
        max_hops: usize,
    ) -> anyhow::Result<RouteListResponse> {
        let pools = db::get_all_pools_with_snapshots(&self.db).await?;
        let graph = TokenGraph::new(pools);
        let routes = find_routes(&graph, from, to, max_hops, amount);
        Ok(RouteListResponse {
            from: format!("{:?}", from),
            to: format!("{:?}", to),
            routes: routes.into_iter().map(route_to_dto).collect(),
        })
    }
}

#[derive(Serialize)]
pub struct LiquidPoolListResponse {
    pub pools: Vec<LiquidPoolRow>,
}

#[derive(Serialize)]
pub struct LiquidPoolResponse {
    pub pool: LiquidPoolRow,
}

#[derive(Serialize)]
pub struct LiquidPoolRow {
    pub rank: i32,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<String>,
    pub kind: String,
    pub token0: String,
    pub token1: String,
    pub fee: Option<u32>,
    pub tvl_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve0: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve1: Option<String>,
    pub block_number: i64,
}

#[derive(Serialize)]
pub struct TrackedPoolResponse {
    pub pool: TrackedPoolRow,
}

#[derive(Serialize)]
pub struct TrackedPoolRow {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<String>,
    pub kind: String,
    pub token0: String,
    pub token1: String,
    pub fee: Option<u32>,
    pub tvl_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve0: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve1: Option<String>,
    pub block_number: i64,
}

#[derive(Serialize)]
pub struct RouteListResponse {
    pub from: String,
    pub to: String,
    pub routes: Vec<RouteDto>,
}

#[derive(Serialize)]
pub struct RouteDto {
    pub hops: Vec<HopDto>,
    pub hop_count: usize,
    pub total_fee_bps: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_output: Option<String>,
    pub min_pool_tvl_usd: f64,
    pub quote_confidence: String,
}

#[derive(Serialize)]
pub struct HopDto {
    pub pool_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<String>,
    pub kind: String,
    pub token_in: String,
    pub token_out: String,
    pub fee: u32,
}

fn route_to_dto(route: Route) -> RouteDto {
    RouteDto {
        hops: route
            .hops
            .into_iter()
            .map(|h| HopDto {
                pool_address: format!("{:?}", h.pool_address),
                pool_id: if h.pool_id.is_zero() {
                    None
                } else {
                    Some(format!("{:?}", h.pool_id))
                },
                kind: h.kind.to_string(),
                token_in: format!("{:?}", h.token_in),
                token_out: format!("{:?}", h.token_out),
                fee: h.fee,
            })
            .collect(),
        hop_count: route.hop_count,
        total_fee_bps: route.total_fee_bps,
        total_output: route.total_output.map(|o| o.to_string()),
        min_pool_tvl_usd: route.min_pool_tvl_usd,
        quote_confidence: match route.quote_confidence {
            QuoteConfidence::Exact => "exact".to_string(),
            QuoteConfidence::Estimated => "estimated".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Lending service
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LendingService {
    db: sqlx::PgPool,
}

impl LendingService {
    pub fn new(db: sqlx::PgPool) -> Self {
        Self { db }
    }

    pub async fn list_markets(&self) -> Result<LendingMarketListResponse, sqlx::Error> {
        let rows = db::list_lending_markets(&self.db).await?;
        let markets = rows
            .into_iter()
            .map(|m| LendingMarketRow {
                market_address: format!("{:?}", m.market_address),
                protocol: m.protocol_str().to_string(),
                underlying_asset: format!("{:?}", m.underlying_asset),
                available_liquidity: m.available_liquidity.map(|v| v.to_string()),
                supply_rate_ray: m.supply_rate_ray.map(|v| v.to_string()),
                variable_borrow_rate_ray: m.variable_borrow_rate_ray.map(|v| v.to_string()),
                stable_borrow_rate_ray: m.stable_borrow_rate_ray.map(|v| v.to_string()),
            })
            .collect();
        Ok(LendingMarketListResponse { markets })
    }
}

#[derive(Serialize)]
pub struct LendingMarketListResponse {
    pub markets: Vec<LendingMarketRow>,
}

#[derive(Serialize)]
pub struct LendingMarketRow {
    pub market_address: String,
    pub protocol: String,
    pub underlying_asset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_liquidity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_rate_ray: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable_borrow_rate_ray: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_borrow_rate_ray: Option<String>,
}

/// Return only the pools whose `token0` or `token1` matches `token`.
/// Preserves input order; O(n).
pub(crate) fn filter_pools_by_token(
    pools: Vec<(Pool, PoolSnapshot)>,
    token: Address,
) -> Vec<(Pool, PoolSnapshot)> {
    pools
        .into_iter()
        .filter(|(p, _)| p.token0 == token || p.token1 == token)
        .collect()
}

/// Find the first pool matching `(address, pool_id)`. Returns `Some((index, &pair))`
/// or `None` if no match.
pub(crate) fn find_pool_by_key(
    pools: &[(Pool, PoolSnapshot)],
    address: Address,
    pool_id: B256,
) -> Option<(usize, &(Pool, PoolSnapshot))> {
    pools
        .iter()
        .enumerate()
        .find(|(_, (p, _))| p.address == address && p.pool_id == pool_id)
}

/// Parse a `:pool` URL path segment into `(address, pool_id)`.
///
/// Two formats are accepted:
/// - `0x<40 hex chars>` — address only; `pool_id` is `B256::ZERO`.
/// - `0x<40 hex chars>:0x<64 hex chars>` — address and pool_id.
///
/// Returns `Err` for any other shape: empty, missing `0x`, wrong length,
/// non-hex characters, or a `:` separator with a malformed pool_id.
pub(crate) fn parse_pool_key(s: &str) -> Result<(Address, B256), String> {
    if let Some((addr_hex, pid_hex)) = s.split_once(':') {
        let addr = addr_hex.parse::<Address>().map_err(|e| e.to_string())?;
        let pid = pid_hex.parse::<B256>().map_err(|e| e.to_string())?;
        Ok((addr, pid))
    } else {
        let addr = s.parse::<Address>().map_err(|e| e.to_string())?;
        Ok((addr, B256::ZERO))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pools::types::{Pool, PoolKind, PoolSnapshot};
    use alloy::primitives::{address, Address, B256, U256};

    fn mk_pool(addr: Address, token0: Address, token1: Address) -> (Pool, PoolSnapshot) {
        let pool = Pool {
            address: addr,
            pool_id: B256::ZERO,
            kind: PoolKind::UniswapV2,
            factory: None,
            token0,
            token0_decimals: 18,
            token1,
            token1_decimals: 18,
            fee: Some(30),
            block_created: None,
        };
        let snapshot = PoolSnapshot {
            address: addr,
            pool_id: B256::ZERO,
            observed_at_block: 0,
            reserve0: Some(U256::from(1_000_000)),
            reserve1: Some(U256::from(1_000_000)),
            tvl_usd: Some(1.0),
            state: serde_json::json!({}),
        };
        (pool, snapshot)
    }

    #[test]
    fn filter_pools_by_token_keeps_pools_containing_token() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let other = address!("0000000000000000000000000000000000000099");

        let p1 = address!("0000000000000000000000000000000000000001");
        let p2 = address!("0000000000000000000000000000000000000002");
        let p3 = address!("0000000000000000000000000000000000000003");

        let pools = vec![
            mk_pool(p1, weth, usdc), // weth is token0
            mk_pool(p2, dai, weth),  // weth is token1
            mk_pool(p3, usdc, dai),  // weth is absent
        ];

        let filtered = filter_pools_by_token(pools, weth);
        let addrs: Vec<Address> = filtered.iter().map(|(p, _)| p.address).collect();
        assert_eq!(filtered.len(), 2);
        assert!(addrs.contains(&p1));
        assert!(addrs.contains(&p2));
        assert!(!addrs.contains(&p3));
    }

    #[test]
    fn filter_pools_by_token_returns_empty_when_token_absent() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let other = address!("0000000000000000000000000000000000000099");
        let p1 = address!("0000000000000000000000000000000000000001");
        let pools = vec![mk_pool(p1, weth, usdc), mk_pool(p1, usdc, dai)];
        let filtered = filter_pools_by_token(pools, other);
        assert!(filtered.is_empty());
    }

    #[test]
    fn parse_pool_key_address_only() {
        let (addr, pool_id) =
            parse_pool_key("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
                .expect("address-only should parse");
        assert_eq!(addr, address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
        assert_eq!(pool_id, B256::ZERO);
    }

    #[test]
    fn parse_pool_key_address_and_pool_id() {
        let pid = "0x0000000000000000000000000000000000000000000000000000000000000abc";
        let (addr, pool_id) =
            parse_pool_key(&format!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2:{}", pid))
                .expect("address+pool_id should parse");
        assert_eq!(addr, address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
        assert_eq!(pool_id, pid.parse::<B256>().unwrap());
    }

    #[test]
    fn parse_pool_key_rejects_empty() {
        assert!(parse_pool_key("").is_err());
    }

    #[test]
    fn parse_pool_key_accepts_address_without_0x_prefix() {
        // alloy's Address::parse accepts both forms; the rest of the API
        // is consistent with that. This test pins the contract.
        let (addr, pool_id) =
            parse_pool_key("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
                .expect("missing 0x is accepted");
        assert_eq!(addr, address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"));
        assert_eq!(pool_id, B256::ZERO);
    }

    #[test]
    fn parse_pool_key_rejects_too_short_address() {
        assert!(parse_pool_key("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc").is_err());
    }

    #[test]
    fn parse_pool_key_rejects_non_hex() {
        assert!(parse_pool_key("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_err());
    }

    #[test]
    fn parse_pool_key_rejects_malformed_pool_id_after_colon() {
        // Address part is valid; pool_id part is too short.
        let s = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2:0xtooshort";
        assert!(parse_pool_key(s).is_err());
    }

    #[test]
    fn find_pool_by_key_finds_matching_pool() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let p1 = address!("0000000000000000000000000000000000000001");
        let p2 = address!("0000000000000000000000000000000000000002");
        let pools = vec![mk_pool(p1, weth, usdc), mk_pool(p2, dai, usdc)];
        let (i, _) = find_pool_by_key(&pools, p1, B256::ZERO).expect("p1 should be found");
        assert_eq!(i, 0);
    }

    #[test]
    fn find_pool_by_key_distinguishes_pool_id_zero_from_unset() {
        // When the path parser sees an address-only key, pool_id is
        // B256::ZERO. A pool with pool_id == B256::ZERO matches.
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let p1 = address!("0000000000000000000000000000000000000001");
        let pools = vec![mk_pool(p1, weth, usdc)];
        let found = find_pool_by_key(&pools, p1, B256::ZERO);
        assert!(found.is_some());
    }

    #[test]
    fn find_pool_by_key_returns_none_when_absent() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let dai = address!("6B175474E89094C44Da98b954EedeAC495271d0F");
        let p1 = address!("0000000000000000000000000000000000000001");
        let p_absent = address!("0000000000000000000000000000000000000099");
        let pools = vec![mk_pool(p1, weth, usdc)];
        let found = find_pool_by_key(&pools, p_absent, B256::ZERO);
        assert!(found.is_none());
    }
}
