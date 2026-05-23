//! Whale Watcher — Real-Time Whale Monitor
//!
//! Connects to Hyperliquid WebSocket, subscribes to userFills for watched wallets,
//! detects large position entries (>$10K notional), emits alerts to data/whale-alerts.json,
//! and tracks alert accuracy with 1-hour follow-up price checks.
//!
//! Usage:
//!   cargo run --bin whale-watcher -- --watchlist data/watchlist.json
//!   cargo run --bin whale-watcher -- --help

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "whale-watcher", about = "Monitor watched wallets for large position entries via WebSocket")]
struct Args {
    /// Path to watchlist JSON file (from alpha-scanner)
    #[arg(long, default_value = "data/watchlist.json")]
    watchlist: String,

    /// Minimum notional (USD) to trigger an alert
    #[arg(long, default_value = "10000")]
    min_notional: f64,

    /// Output path for whale alerts
    #[arg(long, default_value = "data/whale-alerts.json")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _args = Args::parse();

    // Stub: full implementation in m2-traders milestone
    tracing::info!("whale-watcher: stub binary — awaiting m2-traders implementation");

    Ok(())
}
