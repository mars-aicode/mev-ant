//! Configuration: RPC endpoints, DB connection, scan parameters.

use clap::Parser;
use serde::Deserialize;

/// CLI for mev-ant — historical sandwich MEV scanner.
#[derive(Parser, Debug, Clone)]
#[command(name = "mev-ant", about = "Historical sandwich MEV scanner")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Parser, Debug, Clone)]
pub enum Command {
    /// Scan a block range for sandwich bundles and store in DB.
    Scan(ScanConfig),
    /// Scan and print sandwich bundles to stdout (no DB writes).
    Peek(PeekConfig),
    /// Export detected sandwiches from DB.
    Export(ExportConfig),
    /// Start continuous scanning service.
    Serve(ServeCliConfig),
    /// Replay: delete data after a block, re-scan, regenerate attackers.
    Replay(ReplayConfig),
    /// Seed UniV2 liquid pools from TheGraph and refresh state from chain.
    SeedPools(SeedPoolsConfig),
}

/// Parameters for the `serve` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct ServeCliConfig {
    /// Path to TOML config file.
    #[arg(long, env = "MEV_ANT_CONFIG", default_value = "serve.toml")]
    pub config: String,
}

/// Config loaded from serve.toml or equivalent.
#[derive(Debug, Clone, Deserialize)]
pub struct ServeConfig {
    pub rpc_url: String,
    pub db_url: String,
    #[serde(default = "default_from_block")]
    pub from_block: u64,
    #[serde(default = "default_delay")]
    pub delay_blocks: u32,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub dashboard_dir: Option<String>,
    /// Run the background liquidity job (default: true).
    #[serde(default = "default_liquidity_enabled")]
    pub liquidity_enabled: bool,
    /// How often (in seconds) the liquidity job polls for new blocks.
    #[serde(default = "default_liquidity_poll_interval_secs")]
    pub liquidity_poll_interval_secs: u64,
    /// UTC hour (0-23) to perform the daily full refresh.
    #[serde(default = "default_liquidity_full_refresh_hour")]
    pub liquidity_full_refresh_hour: u32,
    /// Number of pools to track as Liquid Pools.
    #[serde(default = "default_liquidity_top_n")]
    pub liquidity_top_n: usize,
    /// Track lending markets (Aave V3) per block (default: true).
    #[serde(default = "default_lending_enabled")]
    pub lending_enabled: bool,
}

fn default_from_block() -> u64 { 0 }
fn default_delay() -> u32 { 6 }
fn default_api_port() -> u16 { 6080 }
fn default_liquidity_enabled() -> bool { true }
fn default_liquidity_poll_interval_secs() -> u64 { 12 }
fn default_liquidity_full_refresh_hour() -> u32 { 0 }
fn default_liquidity_top_n() -> usize { 1000 }
fn default_lending_enabled() -> bool { true }

impl ServeConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}

/// Parameters for the `scan` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct ScanConfig {
    /// Start block number (inclusive).
    #[arg(long)]
    pub from: u64,

    /// End block number (inclusive).
    #[arg(long)]
    pub to: u64,

    /// Number of blocks to fetch concurrently.
    #[arg(long, default_value = "8")]
    pub concurrency: usize,

    /// Resume from last scanned block in DB.
    #[arg(long)]
    pub resume: bool,

    /// Ethereum EL HTTP RPC endpoint.
    #[arg(
        long,
        env = "MEV_ANT_RPC_URL",
        default_value = "http://192.168.2.180:8547"
    )]
    pub rpc_url: String,

    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "MEV_ANT_DB_URL",
        default_value = "postgres://postgres:postgres@192.168.2.185:5432/mev_ant"
    )]
    pub db_url: String,

    /// Batch size for DB inserts.
    #[arg(long, default_value = "1000")]
    pub db_batch_size: usize,
}

/// Parameters for the `peek` subcommand (no-DB dry-run).
#[derive(Parser, Debug, Clone)]
pub struct PeekConfig {
    /// Start block number (inclusive).
    #[arg(long)]
    pub from: u64,

    /// End block number (inclusive).
    #[arg(long)]
    pub to: u64,

    /// Number of blocks to fetch concurrently.
    #[arg(long, default_value = "8")]
    pub concurrency: usize,

    /// Ethereum EL HTTP RPC endpoint.
    #[arg(
        long,
        env = "MEV_ANT_RPC_URL",
        default_value = "http://192.168.2.180:8547"
    )]
    pub rpc_url: String,

    /// Output format: json | summary
    #[arg(long, default_value = "json", value_parser = ["json", "summary"])]
    pub format: String,
}

/// Parameters for the `replay` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct ReplayConfig {
    /// Delete data from this block onward, then resume scanner to re-scan.
    #[arg(long)]
    pub from_block: u64,

    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "MEV_ANT_DB_URL",
        default_value = "postgres://postgres:postgres@192.168.2.185:5432/mev_ant"
    )]
    pub db_url: String,
}

/// Parameters for the `seed-pools` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct SeedPoolsConfig {
    /// Number of pools to seed.
    #[arg(long, default_value = "1000")]
    pub top_n: usize,

    /// Ethereum EL HTTP RPC endpoint.
    #[arg(
        long,
        env = "MEV_ANT_RPC_URL",
        default_value = "http://192.168.2.180:8547"
    )]
    pub rpc_url: String,

    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "MEV_ANT_DB_URL",
        default_value = "postgres://postgres:postgres@192.168.2.185:5432/mev_ant"
    )]
    pub db_url: String,

    /// Optional path to a static bootstrap JSON file. When set, its pools
    /// are inserted into the `pools` table before TheGraph seeding runs,
    /// so the registry can be primed without depending on TheGraph.
    #[arg(long)]
    pub bootstrap: Option<std::path::PathBuf>,
}

/// Parameters for the `export` subcommand.
#[derive(Parser, Debug, Clone)]
pub struct ExportConfig {
    /// Minimum profit in wei (ETH terms).
    #[arg(long, default_value = "0")]
    pub min_profit_wei: u128,

    /// Limit number of results.
    #[arg(long, default_value = "100")]
    pub limit: u64,

    /// Output format.
    #[arg(long, default_value = "json", value_parser = ["json", "csv"])]
    pub format: String,

    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "MEV_ANT_DB_URL",
        default_value = "postgres://postgres:postgres@192.168.2.185:5432/mev_ant"
    )]
    pub db_url: String,
}
