//! mev-ant — Historical sandwich MEV scanner for Ethereum mainnet.

mod api;
mod classifier;
mod config;
mod db;
mod detector;
mod dex;
mod models;
mod rpc;

use alloy::primitives::Address;
use clap::Parser;
use futures::stream::{self, StreamExt};
use sqlx::Row;
use tracing::info;

use config::{Cli, Command};
use detector::sandwich::detect_sandwiches;

/// Token metadata: symbol, decimals.
const TOKEN_META: &[(&str, alloy::primitives::Address, u8)] = &[
    (
        "WETH",
        alloy::primitives::Address::new(hex_literal::hex!(
            "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
        )),
        18,
    ),
    (
        "USDC",
        alloy::primitives::Address::new(hex_literal::hex!(
            "A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        )),
        6,
    ),
    (
        "USDT",
        alloy::primitives::Address::new(hex_literal::hex!(
            "dAC17F958D2ee523a2206206994597C13D831ec7"
        )),
        6,
    ),
    (
        "DAI",
        alloy::primitives::Address::new(hex_literal::hex!(
            "6B175474E89094C44Da98b954EedeAC495271d0F"
        )),
        18,
    ),
    (
        "WBTC",
        alloy::primitives::Address::new(hex_literal::hex!(
            "2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"
        )),
        8,
    ),
    ("ETH", crate::models::ETH_TRANSFER_ADDR, 18),
];

fn format_amount(amount: &alloy::primitives::I256, token: alloy::primitives::Address) -> String {
    let (sign, abs) = amount.into_sign_and_abs();
    let dec = TOKEN_META
        .iter()
        .find(|m| m.1 == token)
        .map(|m| m.2)
        .unwrap_or(18);
    let sym = TOKEN_META
        .iter()
        .find(|m| m.1 == token)
        .map(|m| m.0)
        .unwrap_or("???");
    let prefix = if sign.is_negative() { "-" } else { "" };
    let raw: u128 = abs.to::<u128>();
    let div = 10u128.pow(dec as u32);
    let int_part = raw / div;
    let frac = raw % div;
    format!(
        "{}{}.{:0>width$} {}",
        prefix,
        int_part,
        frac,
        sym,
        width = dec as usize
    )
}

fn format_wei(wei: u128) -> String {
    let dec = 18u32;
    let div = 10u128.pow(dec);
    let int_part = wei / div;
    let frac = wei % div;
    format!("{}.{:0>18} ETH", int_part, frac)
}

/// Default infrastructure blacklist — contracts that should never be candidates.
pub const DEFAULT_BLACKLIST: &[alloy::primitives::Address] = &[
    alloy::primitives::Address::new(hex_literal::hex!(
        "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "E592427A0AEce92De3Edee1F18E0157C05861564"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "000000000004444c5dc75Cb358380D2e08dE62B0"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "BA12222222228d8Ba445958a75a0704d566BF2C8"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "1111111254EEB25477B68fb85Ed929f73A960582"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "111111125421cA6dc452d289314280a0f8842A65"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "C0FFEE0000000000000000000000000000000000"
    )),
];

/// Default supported tokens for profit calculation.
pub const DEFAULT_TOKENS: &[alloy::primitives::Address] = &[
    crate::models::ETH_TRANSFER_ADDR,
    alloy::primitives::Address::new(hex_literal::hex!(
        "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "dAC17F958D2ee523a2206206994597C13D831ec7"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "6B175474E89094C44Da98b954EedeAC495271d0F"
    )),
    alloy::primitives::Address::new(hex_literal::hex!(
        "2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"
    )),
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mev_ant=debug".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Scan(cfg) => run_scan(cfg).await?,
        Command::Peek(cfg) => run_peek(cfg).await?,
        Command::Export(cfg) => run_export(cfg).await?,
        Command::Serve(cfg) => run_serve(&cfg.config).await?,
        Command::Replay(cfg) => run_replay(cfg).await?,
    }

    Ok(())
}

/// Concurrent block-scanning loop.
async fn scan_blocks(
    provider: &rpc::RpcClient,
    block_range: &[u64],
    concurrency: usize,
) -> Vec<Result<(u64, Vec<models::SandwichBundle>), anyhow::Error>> {
    let results: Vec<_> = stream::iter(block_range.iter().copied())
        .map(|block_number| async move {
            let data = rpc::fetch_block(provider, block_number).await?;
            let bundles = detect_sandwiches(
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

async fn run_scan(cfg: config::ScanConfig) -> anyhow::Result<()> {
    let pool = db::init_pool(&cfg.db_url).await?;
    db::migrate(&pool).await?;
    let provider = rpc::RpcClient::new(&cfg.rpc_url)?;

    let from = if cfg.resume {
        db::last_scanned_block(&pool)
            .await?
            .map(|b| b + 1)
            .unwrap_or(cfg.from)
    } else {
        cfg.from
    };

    info!(
        "scanning blocks {}..{} (concurrency={})",
        from, cfg.to, cfg.concurrency
    );
    let block_range: Vec<u64> = (from..=cfg.to).collect();
    let mut total_sandwiches: u64 = 0;

    for chunk in block_range.chunks(cfg.concurrency * 4) {
        let results = scan_blocks(&provider, chunk, cfg.concurrency).await;
        for result in results {
            match result {
                Ok((block_num, bundles)) => {
                    let count = bundles.len();
                    if !bundles.is_empty() {
                        db::insert_sandwiches(&pool, &bundles).await?;
                    }
                    db::mark_block_scanned(&pool, block_num, count).await?;
                    total_sandwiches += count as u64;
                }
                Err(e) => tracing::error!("block fetch failed: {:?}", e),
            }
        }
        info!("chunk complete, total sandwiches: {}", total_sandwiches);
    }
    info!("scan complete. total sandwiches: {}", total_sandwiches);
    Ok(())
}

async fn run_replay(cfg: config::ReplayConfig) -> anyhow::Result<()> {
    let pool = db::init_pool(&cfg.db_url).await?;
    db::migrate(&pool).await?;

    // Fire-and-forget: set the flag and return. The scanner picks it up
    // on its next iteration and performs the replay (delete data, reset
    // cursor, clear flag, auto-resume if paused) without us holding any
    // lock. The replay is idempotent — a crash mid-replay is recovered
    // on next iteration from the still-set flag.
    info!("queuing replay from block {}...", cfg.from_block);
    db::set_pending_replay_from(&pool, cfg.from_block).await?;

    info!(
        "replay queued: scanner will re-scan from block {} on its next iteration",
        cfg.from_block
    );
    Ok(())
}

async fn run_peek(cfg: config::PeekConfig) -> anyhow::Result<()> {
    let provider = rpc::RpcClient::new(&cfg.rpc_url)?;
    eprintln!(
        "peeking blocks {}..{} (concurrency={})",
        cfg.from, cfg.to, cfg.concurrency
    );

    let block_range: Vec<u64> = (cfg.from..=cfg.to).collect();
    let mut total = 0u64;
    let mut hits = 0u64;

    for chunk in block_range.chunks(cfg.concurrency * 4) {
        let results = scan_blocks(&provider, chunk, cfg.concurrency).await;
        for result in results {
            match result {
                Ok((_, bundles)) => {
                    if !bundles.is_empty() {
                        hits += 1;
                        total += bundles.len() as u64;
                        for bundle in &bundles {
                            match cfg.format.as_str() {
                                "json" => println!("{}", serde_json::to_string(bundle)?),
                                "summary" => {
                                    let profit_str: Vec<String> = bundle
                                        .profit
                                        .iter()
                                        .map(|p| format_amount(&p.amount, p.token))
                                        .collect();
                                    let weth = alloy::primitives::Address::new(hex_literal::hex!(
                                        "C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
                                    ));
                                    let weth_profit: i128 = bundle
                                        .profit
                                        .iter()
                                        .filter(|p| p.token == weth)
                                        .map(|p| {
                                            p.amount.into_sign_and_abs().1.to::<u128>() as i128
                                        })
                                        .sum();
                                    let net = weth_profit - bundle.expense_wei as i128;
                                    let net_str = if net < 0 {
                                        format!("-{}", format_wei((-net) as u128))
                                    } else {
                                        format_wei(net as u128)
                                    };

                                    // Cost breakdown
                                    let direct_eth =
                                        bundle.expense_wei.saturating_sub(bundle.gas_cost_wei);
                                    let back_init = if bundle.back_initiator != Address::ZERO
                                        && bundle.back_initiator != bundle.initiator
                                    {
                                        format!(" back_initiator={:?}", bundle.back_initiator)
                                    } else {
                                        String::new()
                                    };
                                    println!(
                                        "block={} attacker={:?} funder={:?} executor={:?} initiator={:?}{} \
                                         front_tx={} back_tx={} victim_count={} profit=[{}] \
                                         cost={} (gas={} direct={}) coinbase={} net={}",
                                        bundle.block_number, bundle.attacker, bundle.funder,
                                        bundle.executor, bundle.initiator, back_init,
                                        bundle.front_tx_index, bundle.back_tx_index,
                                        bundle.victim_tx_indices.len(),
                                        profit_str.join(", "),
                                        format_wei(bundle.expense_wei),
                                        format_wei(bundle.gas_cost_wei),
                                        format_wei(direct_eth),
                                        format_wei(bundle.coinbase_bribe),
                                        net_str,
                                    );
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("block fetch failed: {:?}", e),
            }
        }
        eprintln!("chunk done, total: {} sandwiches in {} blocks", total, hits);
    }
    eprintln!(
        "peek complete. {} sandwiches in {} blocks out of {} scanned",
        total,
        hits,
        cfg.to - cfg.from + 1
    );
    Ok(())
}

async fn run_export(cfg: config::ExportConfig) -> anyhow::Result<()> {
    let pool = db::init_pool(&cfg.db_url).await?;

    let rows = sqlx::query(
        r#"
        SELECT block_number, attacker, front_tx_index, back_tx_index,
               attacked_pool, victim_count, profit_json, gas_cost_wei
        FROM sandwiches
        ORDER BY block_number DESC
        LIMIT $1
        "#,
    )
    .bind(cfg.limit as i64)
    .fetch_all(&pool)
    .await?;

    for row in rows {
        let block: i64 = row.get(0);
        let attacker: Vec<u8> = row.get(1);
        let front_tx: i64 = row.get(2);
        let back_tx: i64 = row.get(3);

        if cfg.format == "json" {
            let json = serde_json::json!({
                "block": block,
                "attacker": hex::encode(attacker),
                "front_tx": front_tx,
                "back_tx": back_tx,
            });
            println!("{}", serde_json::to_string(&json)?);
        } else {
            println!(
                "{},{:?},{},{}",
                block,
                hex::encode(attacker),
                front_tx,
                back_tx
            );
        }
    }

    Ok(())
}

async fn run_serve(config_path: &str) -> anyhow::Result<()> {
    let cfg = config::ServeConfig::load(config_path)?;
    let pool = db::init_pool(&cfg.db_url).await?;
    db::migrate(&pool).await?;
    let provider = rpc::RpcClient::new(&cfg.rpc_url)?;

    db::init_scan_state(&pool, cfg.from_block).await?;

    // Spawn HTTP management API
    let api_pool = pool.clone();
    let api_provider = provider.clone();
    let api_port = cfg.api_port;
    let dashboard_dir = cfg.dashboard_dir.clone();
    tokio::spawn(async move {
        let app = api::build_router(api_pool, Some(api_provider), dashboard_dir);
        let addr = format!("0.0.0.0:{}", api_port);
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        tracing::info!("API listening on http://{}", addr);
        axum::serve(listener, app).await.unwrap();
    });

    info!(
        "serve: starting from block {} (delay={})",
        cfg.from_block, cfg.delay_blocks
    );

    loop {
        // No transaction, no advisory lock. Each query auto-commits and
        // is atomic on its own. The pending-replay flag and the
        // post-fetch re-check give us eventually-consistent behavior
        // across admin actions (pause / replay), without the cost of
        // serializing through the DB.

        // Read state.
        let state = match db::read_scan_state(&pool).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("read_scan_state: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        // Pending replay: scanner performs its own replay. If the admin
        // endpoint set pending_replay_from, do the replay: delete data
        // from that block, reset the cursor, clear the flag, and
        // auto-resume if the scanner was paused. The replay operations
        // are idempotent — if the scanner crashes mid-replay, the next
        // iteration retries from the still-set flag.
        if state.pending_replay_from != 0 {
            let from = state.pending_replay_from;
            info!("performing pending replay from block {}", from);
            if let Err(e) = db::delete_sandwiches_from(&pool, from).await {
                tracing::error!("delete_sandwiches_from: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            if let Err(e) = db::delete_blocks_scanned_from(&pool, from).await {
                tracing::error!("delete_blocks_scanned_from: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            if let Err(e) = db::reset_scan_state_to(&pool, from).await {
                tracing::error!("reset_scan_state_to: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            if let Err(e) = db::clear_pending_replay_from(&pool).await {
                tracing::error!("clear_pending_replay_from: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            if !state.enabled {
                info!("auto-resuming scanner due to pending replay");
                if let Err(e) = db::set_scan_enabled(&pool, true).await {
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
        let chain_head = match provider.block_number().await {
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
                if let Err(e) = db::update_chain_head(&pool, chain_head).await {
                    tracing::error!("update_chain_head: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                LAST_HEAD.store(chain_head, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let target = chain_head.saturating_sub(cfg.delay_blocks as u64);
        if state.next_block > target {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }
        let block_to_scan = state.next_block;

        // Fetch (no DB).
        let data = match rpc::fetch_block(&provider, block_to_scan).await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("fetch_block {}: {:?}", block_to_scan, e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        let bundles = detector::sandwich::detect_sandwiches(
            block_to_scan,
            &data.flows,
            &data.raw_logs,
            data.coinbase,
            DEFAULT_BLACKLIST,
            DEFAULT_TOKENS,
        );

        // Re-check state and write. If admin's pending replay fired
        // during the fetch, the flag will be set now — let the next
        // iteration handle it, skip the writes for the stale block.
        // (The detected bundles are discarded; the next scan re-detects.)
        let state = match db::read_scan_state(&pool).await {
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
            if let Err(e) = db::insert_sandwiches(&pool, &bundles).await {
                tracing::error!("insert_sandwiches: {:?}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        }
        if let Err(e) = db::mark_block_scanned(&pool, block_to_scan, count).await {
            tracing::error!("mark_block_scanned: {:?}", e);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }
        if let Err(e) = db::advance_scan_state(&pool, block_to_scan + 1).await {
            tracing::error!("advance_scan_state: {:?}", e);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        info!("block {} scanned: {} sandwiches", block_to_scan, count);
    }
}

#[cfg(test)]
mod integration_tests;
