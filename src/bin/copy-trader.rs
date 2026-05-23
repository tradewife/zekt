//! Copy Trader — Position Mirroring Engine
//!
//! Loads watchlist from alpha-scanner output, polls watched wallets' positions
//! every 30s via HL clearinghouseState, detects new/closed/modified positions,
//! mirrors them in paper trading mode with configurable sizing and risk management.
//!
//! Usage:
//!   cargo run --bin copy-trader -- --paper --watchlist data/watchlist.json
//!   cargo run --bin copy-trader -- --help

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "copy-trader", about = "Mirror profitable wallets' positions in real-time")]
struct Args {
    /// Run in paper trading mode (simulated)
    #[arg(long)]
    paper: bool,

    /// Run in live trading mode (requires keypair, human approval)
    #[arg(long)]
    live: bool,

    /// Path to watchlist JSON file (from alpha-scanner)
    #[arg(long, default_value = "data/watchlist.json")]
    watchlist: String,

    /// Maximum position size as percentage of account balance
    #[arg(long, default_value = "10.0")]
    max_position_pct: f64,

    /// Maximum number of concurrent mirrored positions
    #[arg(long, default_value = "3")]
    max_positions: usize,

    /// Stop-loss percentage for mirrored positions
    #[arg(long, default_value = "5.0")]
    stop_loss_pct: f64,

    /// Delay before mirroring a detected position (seconds)
    #[arg(long, default_value = "30")]
    lag_secs: u64,

    /// Position sizing multiplier
    #[arg(long, default_value = "0.1")]
    sizing_multiplier: f64,

    /// Output path for trade log
    #[arg(long, default_value = "data/copy-trades.json")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _args = Args::parse();

    // Stub: full implementation in m2-traders milestone
    tracing::info!("copy-trader: stub binary — awaiting m2-traders implementation");

    Ok(())
}
