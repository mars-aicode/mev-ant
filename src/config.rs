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
}

fn default_from_block() -> u64 { 0 }
fn default_delay() -> u32 { 6 }
fn default_api_port() -> u16 { 6080 }

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
