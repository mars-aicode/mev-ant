//! mev-ant — Historical sandwich MEV scanner for Ethereum mainnet.

mod api;
mod classifier;
mod config;
mod db;
mod detector;
mod dex;
mod models;
mod pools;
mod repository;
mod rpc;
mod scanner;
mod services;
mod tokens;

pub use tokens::{DEFAULT_BLACKLIST, DEFAULT_TOKENS};

use alloy::primitives::Address;
use anyhow::Context;
use clap::Parser;
use sqlx::Row;
use tracing::info;

use config::{Cli, Command};
use scanner::Scanner;
use tokens::{format_amount, format_wei};

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
        Command::SeedPools(cfg) => run_seed_pools(cfg).await?,
    }

    Ok(())
}

async fn run_scan(cfg: config::ScanConfig) -> anyhow::Result<()> {
    let pool = db::init_pool(&cfg.db_url).await?;
    db::migrate(&pool).await?;
    let provider = rpc::RpcClient::new(&cfg.rpc_url)?;
    let scanner = Scanner::new(provider);

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
        let results = scanner.scan_blocks(chunk, cfg.concurrency).await;
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
    let scanner = Scanner::new(provider);
    eprintln!(
        "peeking blocks {}..{} (concurrency={})",
        cfg.from, cfg.to, cfg.concurrency
    );

    let block_range: Vec<u64> = (cfg.from..=cfg.to).collect();
    let mut total = 0u64;
    let mut hits = 0u64;

    for chunk in block_range.chunks(cfg.concurrency * 4) {
        let results = scanner.scan_blocks(chunk, cfg.concurrency).await;
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

    if cfg.liquidity_enabled {
        let job_provider = rpc::RpcClient::new(&cfg.rpc_url)?;
        let job_db = pool.clone();
        let job_top_n = cfg.liquidity_top_n;
        let job_poll = cfg.liquidity_poll_interval_secs;
        let job_hour = cfg.liquidity_full_refresh_hour;
        let lending_enabled = cfg.lending_enabled;
        tokio::spawn(async move {
            let job = crate::pools::job::LiquidityJob::with_options(
                job_provider, job_db, job_top_n, job_poll, job_hour, lending_enabled,
            );
            job.run(None).await;
        });
        info!(
            "liquidity job spawned (top_n={}, poll={}s, refresh_hour={} UTC, lending={})",
            job_top_n, job_poll, job_hour, lending_enabled
        );
    }

    let scanner = Scanner::new(provider);
    scanner.run_continuous(&pool, cfg.delay_blocks as u64).await
}

async fn run_seed_pools(cfg: config::SeedPoolsConfig) -> anyhow::Result<()> {
    let pool = db::init_pool(&cfg.db_url).await?;
    db::migrate(&pool).await?;
    let provider = rpc::RpcClient::new(&cfg.rpc_url)?;

    if let Some(path) = cfg.bootstrap.as_ref() {
        let bootstrap_pools = crate::pools::bootstrap::load_bootstrap(path)
            .with_context(|| format!("load bootstrap from {}", path.display()))?;
        db::insert_pools(&pool, &bootstrap_pools).await?;
        tracing::info!(
            "bootstrap inserted {} pools from {}",
            bootstrap_pools.len(),
            path.display()
        );
    }

    crate::pools::registry::refresh_liquid_pools(&provider, &pool, cfg.top_n)
        .await
        .context("refresh liquid pools")?;

    Ok(())
}

#[cfg(test)]
mod integration_tests;
