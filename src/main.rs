mod backtest;
mod config;
mod engine;
mod executor;
mod flash_api;
#[allow(dead_code)]
mod fishing;
mod funding_capture;
#[allow(dead_code)]
mod hl_info;
#[allow(dead_code)]
mod liquidation;
#[allow(dead_code)]
mod liquidity_memory;
#[allow(dead_code)]
mod hl_paper;
#[allow(dead_code)]
mod imperial;
#[allow(dead_code)]
mod market_data;
mod monitor;
mod paper;
#[allow(dead_code)]
mod pnl_tracker;
#[allow(dead_code)]
mod pyramiding;
#[allow(dead_code)]
mod replay;
#[allow(dead_code)]
mod regime;
#[allow(dead_code)]
mod risk;
#[allow(dead_code)]
mod route_cost;
mod signal;
mod strategy;

use clap::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[derive(Parser, Debug)]
#[command(name = "zekt", about = "Flash Trade momentum scalper bot")]
struct Args {
    /// Path to config file
    #[arg(short, long, default_value = "config/perps.toml")]
    config: PathBuf,

    /// Solana keypair path (overrides config, not needed for --paper)
    #[arg(short, long)]
    keypair: Option<String>,

    /// Market to trade (overrides config). For multi-market paper trading, use --markets instead.
    #[arg(short, long)]
    market: Option<String>,

    /// Strategy to use (overrides config [strategy] active field). For multi-strategy, use --strategies.
    #[arg(short, long)]
    strategy: Option<String>,

    /// Comma-separated list of strategies for multi-strategy paper trading
    /// (e.g., --strategies momentum-scalper,lp-consumption)
    #[arg(long)]
    strategies: Option<String>,

    /// Comma-separated list of markets for multi-market paper trading
    /// (e.g., --markets SOL,BTC,ETH)
    #[arg(long)]
    markets: Option<String>,

    /// Dry run mode -- single preview, no signing, then exit
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Paper trading -- full open/monitor/close loop against live prices, simulated PnL, no real transactions
    #[arg(long, default_value_t = false)]
    paper: bool,

    /// HL paper trading -- simulate perpetual futures on Hyperliquid using live HL data, HL fee model
    #[arg(long, default_value_t = false)]
    hl_paper: bool,

    /// Starting balance for paper trading (USD), defaults to 1000
    #[arg(long, default_value_t = 1000.0)]
    paper_balance: f64,

    /// Output directory for paper trading results (default: data/paper-results)
    #[arg(long, default_value = "data/paper-results")]
    paper_output: String,

    /// Backtest mode -- replay Hyperliquid historical candles through strategies
    #[arg(long, default_value_t = false)]
    backtest: bool,

    /// Backtest start time (ISO 8601, e.g., "2025-05-01T00:00:00Z" or "2025-05-01")
    #[arg(long)]
    backtest_start: Option<String>,

    /// Backtest end time (ISO 8601, defaults to now)
    #[arg(long)]
    backtest_end: Option<String>,

    /// Backtest candle interval (e.g., "1m", "5m", "15m", "1h", "4h")
    #[arg(long, default_value = "5m")]
    backtest_interval: String,

    /// Backtest fee rate as decimal per side (default: 0.001 = 0.1%, matching Flash Trade base taker fee)
    #[arg(long, default_value_t = 0.001)]
    backtest_fee_rate: f64,

    /// Backtest cost mode: "flash-only" (default) or "imperial-route-oracle" (uses RouteCostOracle for cross-venue routing)
    #[arg(long, default_value = "flash-only")]
    cost_mode: String,

    /// Sizing mode: "fixed-notional" (default), "fixed-fractional", "volatility-adjusted", "drawdown-throttled", "route-cost-adjusted"
    #[arg(long, default_value = "fixed-notional")]
    sizing_mode: String,

    /// Leverage override for backtest mode (must be > 0)
    #[arg(long)]
    leverage: Option<f64>,

    /// Output directory for backtest results (creates directory if needed, default: data/backtest-results)
    #[arg(long)]
    output_path: Option<String>,

    /// JSON string of parameter overrides applied on top of strategy defaults
    /// Example: --param-override '{"clip_size_usd": 200, "take_profit_pct": 1.5}'
    #[arg(long)]
    param_override: Option<String>,

    /// Borrow rate override (hourly rate on notional, default: 0.0001)
    #[arg(long)]
    borrow_rate: Option<f64>,

    /// Walk-forward mode: "single" (existing 70/30 split) or "expanding" (N expanding windows)
    #[arg(long, default_value = "single")]
    walk_forward_mode: String,

    /// Number of expanding walk-forward windows (default: 5, only used with --walk-forward-mode expanding)
    #[arg(long, default_value_t = 5)]
    walk_forward_windows: usize,

    /// Liquidation replay mode -- replay captured liquidation zone snapshots through strategies
    /// with fishing + pyramiding + 12-criterion promotion gate evaluation.
    #[arg(long, default_value_t = false)]
    liquidation_replay: bool,

    /// Directory containing captured liquidation zone snapshots (default: data/liquidation-zones/)
    #[arg(long, default_value = "data/liquidation-zones/")]
    snapshot_dir: PathBuf,

    /// Starting balance for liquidation replay in USD (default: 1000)
    #[arg(long, default_value_t = 1000.0)]
    starting_balance: f64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config_path = if args.config.exists() {
        args.config
    } else {
        PathBuf::from("config/perps.toml")
    };

    let mut config = config::Config::load(&config_path)?;

    if let Some(market) = args.market {
        config.flash.market = market;
    }
    if let Some(keypair) = args.keypair {
        config.flash.keypair_path = keypair;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.agent.log_level)),
        )
        .with_target(false)
        .with_thread_ids(false)
        .init();

    tracing::info!("=== Zekt Momentum Scalper v0.3 (Flash Trade) ===");
    tracing::info!("Config: {}", config_path.display());
    tracing::info!("Market: {}", config.flash.market);

    // Resolve strategy name from CLI flag or config, validate early
    let strategy_name = config.strategy.resolve_active(args.strategy.as_deref());
    tracing::info!("Strategy: {}", strategy_name);

    // Validate strategy name first
    if !crate::strategy::available_strategies().contains(&strategy_name.as_str()) {
        let available = crate::strategy::available_strategies().join(", ");
        tracing::error!(
            "Unknown strategy '{}'. Available strategies: {}",
            strategy_name,
            available
        );
        std::process::exit(1);
    }

    // Validate parameters by attempting to create the strategy (dry run)
    let sub_table = config.strategy.get_sub_table(&strategy_name);
    // For strategies that have a sub-table (e.g., lp-consumption), get_params may fail
    // because they have different parameter schemas. Use flat legacy params as fallback.
    let fallback_params = config.strategy.get_params(&strategy_name)
        .unwrap_or_else(|_| {
            // Create minimal fallback params for validation
            crate::strategy::StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 60,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 2.5,
                stop_loss_pct: 1.0,
                trailing_stop_pct: 0.8,
                trailing_activation_pct: 1.5,
                cooldown_after_loss_secs: 300,
                use_native_tp_sl: true,
            }
        });
    if let Err(e) = crate::strategy::create_strategy_from_config(
        &strategy_name,
        sub_table,
        fallback_params,
    ) {
        tracing::error!("Invalid strategy parameters: {}", e);
        std::process::exit(1);
    }

    // Mutually-exclusive mode flags
    let mode_flags = [
        ("--dry-run", args.dry_run),
        ("--paper", args.paper),
        ("--hl-paper", args.hl_paper),
        ("--backtest", args.backtest),
        ("--liquidation-replay", args.liquidation_replay),
    ];
    let active_modes: Vec<&str> = mode_flags
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect();
    if active_modes.len() > 1 {
        anyhow::bail!("Cannot use {} together", active_modes.join(" and "));
    }

    if args.dry_run {
        tracing::warn!("DRY RUN -- single preview, then exit");
        return run_dry(config).await;
    }
    if args.paper {
        // Determine if this is multi-strategy multi-market mode
        let strategies_list = args.strategies.as_deref()
            .map(|s| s.split(',').map(|s| s.trim()).collect::<Vec<_>>());
        let markets_list = args.markets.as_deref()
            .map(|s| s.split(',').map(|s| s.trim().to_uppercase()).collect::<Vec<_>>());

        if strategies_list.is_some() || markets_list.is_some() {
            tracing::warn!("PAPER TRADING (MULTI) -- multi-strategy multi-market mode");
            let resolved_strategies = strategies_list.unwrap_or_else(|| vec![strategy_name.as_str()]);
            let resolved_markets = markets_list.unwrap_or_else(|| vec![config.flash.market.clone()]);
            return run_multi_paper(
                config,
                args.paper_balance,
                resolved_strategies,
                resolved_markets,
                &args.paper_output,
            ).await;
        } else {
            tracing::warn!("PAPER TRADING -- full loop, simulated PnL, NO real transactions");
            return run_paper(config, args.paper_balance, args.strategy.as_deref()).await;
        }
    }
    if args.hl_paper {
        tracing::warn!("HL PAPER TRADING -- Hyperliquid fee model, live HL prices");
        let strategies_list: Vec<String> = args.strategies.as_deref()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|| vec![strategy_name.clone()]);
        let markets_list: Vec<String> = args.markets.as_deref()
            .map(|s| s.split(',').map(|s| s.trim().to_uppercase()).collect())
            .unwrap_or_else(|| vec![config.flash.market.clone()]);
        return run_hl_paper(
            config,
            strategies_list,
            markets_list,
            args.paper_balance,
            &args.paper_output,
        ).await;
    }
    if args.backtest {
        // Validate leverage if provided
        if let Some(lev) = args.leverage
            && lev <= 0.0
        {
                anyhow::bail!(
                    "Invalid --leverage {}: must be > 0",
                    lev
                );
            }

        // Validate param-override JSON if provided
        if let Some(ref json_str) = args.param_override
            && serde_json::from_str::<serde_json::Value>(json_str).is_err()
        {
                anyhow::bail!(
                    "Invalid --param-override JSON: '{}'. Must be a valid JSON object, e.g. '{{\"clip_size_usd\": 200}}'",
                    json_str
                );
            }

        // Validate borrow-rate if provided
        if let Some(rate) = args.borrow_rate
            && rate < 0.0
        {
                anyhow::bail!(
                    "Invalid --borrow-rate {}: must be >= 0",
                    rate
                );
            }

        return run_backtest(
            config,
            args.strategies.as_deref(),
            args.markets.as_deref(),
            args.backtest_start.as_deref(),
            args.backtest_end.as_deref(),
            &args.backtest_interval,
            args.paper_balance,
            args.backtest_fee_rate,
            &args.cost_mode,
            &args.sizing_mode,
            args.leverage,
            args.output_path.as_deref(),
            args.param_override.as_deref(),
            args.borrow_rate,
            &args.walk_forward_mode,
            args.walk_forward_windows,
        ).await;
    }
    if args.liquidation_replay {
        tracing::warn!("LIQUIDATION REPLAY -- replaying captured snapshots through strategy");
        return run_liquidation_replay(
            config,
            args.strategy.as_deref(),
            &args.snapshot_dir,
            args.starting_balance,
        ).await;
    }

    tracing::warn!("LIVE TRADING -- real transactions with real funds");
    run_live(config, args.strategy.as_deref()).await
}

async fn run_live(config: config::Config, strategy_name: Option<&str>) -> anyhow::Result<()> {
    let executor = executor::Executor::new(&config.flash.rpc_url, &config.flash.keypair_path)?;
    let mut engine = engine::ScalperEngine::new(config, executor, strategy_name)?;

    let running = engine.shutdown_handle();
    let _ = ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal, finishing current tick...");
        running.store(false, Ordering::Relaxed);
    });

    engine.run().await
}

async fn run_paper(config: config::Config, starting_balance: f64, strategy_name: Option<&str>) -> anyhow::Result<()> {
    let mut engine = paper::PaperEngine::new(config, starting_balance, strategy_name)?;

    let running = engine.shutdown_handle();
    let _ = ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal, finishing current tick...");
        running.store(false, Ordering::Relaxed);
    });

    engine.run().await
}

async fn run_multi_paper(
    config: config::Config,
    starting_balance: f64,
    strategy_names: Vec<&str>,
    markets: Vec<String>,
    output_dir: &str,
) -> anyhow::Result<()> {
    let mut engine = paper::MultiPaperEngine::new(
        config,
        starting_balance,
        strategy_names,
        markets,
        output_dir,
    )?;

    let running = engine.shutdown_handle();
    let _ = ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal, finishing current tick...");
        running.store(false, Ordering::Relaxed);
    });

    engine.run().await
}

async fn run_hl_paper(
    config: config::Config,
    strategy_names: Vec<String>,
    markets: Vec<String>,
    starting_balance: f64,
    output_dir: &str,
) -> anyhow::Result<()> {
    use hl_paper::{HlPaperConfig, HlPaperEngine};
    use market_data::HlDataProvider;

    let provider = HlDataProvider::new();

    // Read min_hold_before_sl from funding-capture sub-table (default: 120 minutes = 7200s).
    let min_hold_before_sl_secs = config
        .strategy
        .get_sub_table("funding-capture")
        .and_then(|t| t.as_table())
        .and_then(|t| t.get("min_hold_before_sl_mins"))
        .and_then(|v| v.as_integer())
        .map(|mins| (mins * 60) as u64)
        .unwrap_or(7200);

    let hl_config = HlPaperConfig {
        poll_interval_secs: config.agent.poll_interval_secs,
        max_total_notional_usd: config.risk.max_total_notional_usd,
        max_24h_volatility_pct: 5.0, // skip entry when 24h volatility exceeds 5%
        min_hold_before_sl_secs,
    };

    let engine = HlPaperEngine::new(
        provider,
        hl_config,
        strategy_names,
        &|name: &str| -> anyhow::Result<Box<dyn crate::strategy::Strategy>> {
            let sub_table = config.strategy.get_sub_table(name);
            let params = config.strategy.get_params(name).unwrap_or_else(|_| {
                crate::strategy::StrategyParams {
                    direction_bias: "neutral".to_string(),
                    momentum_threshold_pct: 0.15,
                    lookback_count: 60,
                    scale_in_clips: 1,
                    clip_size_usd: 100.0,
                    max_hold_secs: 1800,
                    take_profit_pct: 2.5,
                    stop_loss_pct: 1.0,
                    trailing_stop_pct: 0.8,
                    trailing_activation_pct: 1.5,
                    cooldown_after_loss_secs: 300,
                    use_native_tp_sl: true,
                }
            });
            crate::strategy::create_strategy_from_config(name, sub_table, params)
        },
        markets,
        starting_balance,
        output_dir,
    )?;

    let running = engine.shutdown_handle();

    // Handle SIGINT via ctrlc.
    let running_int = running.clone();
    let _ = ctrlc::set_handler(move || {
        tracing::info!("Received SIGINT, finishing current tick...");
        running_int.store(false, Ordering::Relaxed);
    });

    // Handle SIGTERM so `timeout` and process managers trigger a clean shutdown.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        let running_term = running.clone();
        tokio::spawn(async move {
            sigterm.recv().await;
            tracing::info!("Received SIGTERM, finishing current tick...");
            running_term.store(false, Ordering::Relaxed);
        });
    }

    engine.run().await
}

async fn run_dry(config: config::Config) -> anyhow::Result<()> {
    use flash_api::FlashClient;

    let flash = FlashClient::new(&config.flash.api_url);
    let market = &config.flash.market;

    tracing::info!("--- Dry Run: Fetching {} price ---", market);
    let price = flash.get_price(market).await?;
    tracing::info!("{} price: ${:.2}", market, price);

    tracing::info!("--- Dry Run: Previewing {} LONG ${:.0} @ {}x ---",
        market, config.strategy.clip_size_usd, config.flash.leverage);
    let preview = flash.preview_open_position(
        &config.flash.input_token,
        market,
        config.strategy.clip_size_usd,
        config.flash.leverage,
        "LONG",
    ).await?;

    if let Some(ref err) = preview.err {
        tracing::warn!("Preview error: {}", err);
    } else {
        tracing::info!("Preview result:");
        tracing::info!("  Entry price: {}", preview.new_entry_price.as_deref().unwrap_or("N/A"));
        tracing::info!("  Liquidation: {}", preview.new_liquidation_price.as_deref().unwrap_or("N/A"));
        tracing::info!("  Entry fee: ${}", preview.entry_fee.as_deref().unwrap_or("N/A"));
        tracing::info!("  Notional: ${}", preview.you_recieve_usd_ui.as_deref().unwrap_or("N/A"));
        tracing::info!("  Output: {} {}", preview.output_amount_ui.as_deref().unwrap_or("N/A"), market);
    }

    tracing::info!("--- Dry Run: Checking pool data ---");
    match flash.get_pool_data().await {
        Ok(pools) => {
            tracing::info!("Pool data: {} pools loaded", pools.len());
        }
        Err(e) => tracing::warn!("Pool data error: {:#}", e),
    }

    tracing::info!("--- Dry run complete ---");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_backtest(
    config: config::Config,
    strategies: Option<&str>,
    markets: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
    interval: &str,
    starting_balance: f64,
    fee_rate: f64,
    cost_mode: &str,
    sizing_mode_str: &str,
    leverage_override: Option<f64>,
    output_path: Option<&str>,
    param_override_json: Option<&str>,
    borrow_rate_override: Option<f64>,
    walk_forward_mode_str: &str,
    walk_forward_windows: usize,
) -> anyhow::Result<()> {
    use chrono::Utc;

    tracing::warn!("BACKTEST -- replaying Hyperliquid historical data through strategies");

    // Parse sizing mode
    let sizing_mode = backtest::SizingMode::from_cli_str(sizing_mode_str)?;

    // Parse strategies
    let strategy_names: Vec<String> = strategies
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_else(|| {
            vec![config.strategy.resolve_active(None)]
        });

    // Parse markets
    let market_names: Vec<String> = markets
        .map(|s| s.split(',').map(|s| s.trim().to_uppercase()).collect())
        .unwrap_or_else(|| vec![config.flash.market.clone()]);

    // Parse start time
    let start_time_ms = parse_backtest_time(start, "start")?;

    // Parse end time (default: now)
    let end_time_ms = if let Some(end_str) = end {
        parse_backtest_time(Some(end_str), "end")?
    } else {
        Utc::now().timestamp_millis()
    };

    if start_time_ms >= end_time_ms {
        anyhow::bail!(
            "Start time ({}) must be before end time ({})",
            chrono::DateTime::from_timestamp_millis(start_time_ms)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            chrono::DateTime::from_timestamp_millis(end_time_ms)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
        );
    }

    // Resolve leverage: CLI override > config
    let leverage = leverage_override.unwrap_or(config.flash.leverage);

    // Resolve borrow rate: CLI override > config [backtest] section > default
    let borrow_rate_hourly = borrow_rate_override
        .unwrap_or(config.backtest.borrow_rate_hourly);

    // Parse param-override JSON into a HashMap
    let param_overrides: HashMap<String, serde_json::Value> = param_override_json
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();

    if !param_overrides.is_empty() {
        tracing::info!("Param overrides: {} key(s)", param_overrides.len());
        for (k, v) in &param_overrides {
            tracing::info!("  {} = {}", k, v);
        }
    }

    // Resolve output path: CLI override > default
    let output_dir = output_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| "data/backtest-results".to_string());

    tracing::info!("Strategies: {}", strategy_names.join(", "));
    tracing::info!("Markets: {}", market_names.join(", "));
    tracing::info!(
        "Period: {} → {}",
        chrono::DateTime::from_timestamp_millis(start_time_ms)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        chrono::DateTime::from_timestamp_millis(end_time_ms)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
    );
    tracing::info!("Interval: {}", interval);
    tracing::info!("Fee rate: {:.2}%  |  Leverage: {}x  |  Balance: ${:.0}  |  Sizing: {}  |  Borrow rate: {}",
        fee_rate * 100.0, leverage, starting_balance, sizing_mode.name(), borrow_rate_hourly);
    tracing::info!("Output path: {}", output_dir);

    // Parse walk-forward mode
    let walk_forward_mode = backtest::WalkForwardMode::from_cli_str(walk_forward_mode_str)?;
    tracing::info!("Walk-forward mode: {} (windows={})", walk_forward_mode.name(), walk_forward_windows);

    let bt_config = backtest::BacktestConfig {
        strategies: strategy_names,
        markets: market_names,
        start_time_ms,
        end_time_ms,
        interval: interval.to_string(),
        starting_balance,
        fee_rate,
        borrow_rate_hourly,
        leverage,
        regime_filter: true, // Always enable regime filtering
        walk_forward_enabled: walk_forward_mode_str != "single" || config.backtest.walk_forward_enabled,
        walk_forward_train_ratio: 0.7,
        walk_forward_mode: match &walk_forward_mode {
            backtest::WalkForwardMode::Single => backtest::WalkForwardMode::Single,
            backtest::WalkForwardMode::Expanding { .. } => backtest::WalkForwardMode::Expanding {
                windows: walk_forward_windows,
                initial_train_ratio: 0.6,
            },
        },
        slippage_bps: 0.0, // Default: no slippage; set via config to enable
        cost_mode: cost_mode.to_string(),
        sizing_mode,
        output_dir: output_dir.clone(),
        param_overrides,
    };

    let engine = backtest::BacktestEngine::new(config, bt_config)?;
    let result = engine.run().await?;

    tracing::info!("=== Backtest complete ===");
    tracing::info!("Final balance: ${:.2} (net PnL: ${:.2})", result.final_balance, result.total_net_pnl);

    Ok(())
}

/// Run the liquidation replay pipeline.
///
/// Loads captured liquidation zone snapshots, converts them to replay data points,
/// builds the specified strategy via `create_strategy_from_config`, runs the replay
/// pipeline with fishing + pyramiding, evaluates the 12-criterion promotion gate,
/// and writes results to JSON and Markdown files.
async fn run_liquidation_replay(
    config: config::Config,
    strategy_name: Option<&str>,
    snapshot_dir: &Path,
    starting_balance: f64,
) -> anyhow::Result<()> {
    use chrono::Utc;
    use crate::fishing::FishingLadderConfig;
    use crate::pyramiding::PyramidConfig;
    use crate::replay::{PromotionGateConfig, ReplayPipeline};

    // Resolve strategy name(s) to replay
    let liquidation_strategies = [
        "liquidation-cascade-continuation",
        "sweep-reclaim",
        "liquidity-memory-fisher",
        "liquidation-zone-arbiter",
    ];

    // Map short aliases to canonical names
    let alias_map: &[(&str, &str)] = &[
        ("cascade-continuation", "liquidation-cascade-continuation"),
        ("liquidation-cascade-hunter", "liquidation-cascade-continuation"),
    ];

    let strategy_names: Vec<String> = if let Some(name) = strategy_name {
        if name == "all" {
            liquidation_strategies.iter().map(|s| s.to_string()).collect()
        } else {
            // Check if it's an alias
            let canonical = alias_map
                .iter()
                .find(|(alias, _)| *alias == name)
                .map(|(_, canonical)| *canonical)
                .unwrap_or(name);
            vec![canonical.to_string()]
        }
    } else {
        // Default: cascade-continuation
        vec!["liquidation-cascade-continuation".to_string()]
    };

    // Validate strategy names
    let available = crate::strategy::available_strategies();
    for name in &strategy_names {
        if !available.contains(&name.as_str()) {
            anyhow::bail!(
                "Unknown liquidation strategy '{}'. Valid: {}",
                name,
                liquidation_strategies.join(", ")
            );
        }
    }

    tracing::info!("Liquidation replay: {} strategy/strategies", strategy_names.len());
    tracing::info!("Snapshot directory: {}", snapshot_dir.display());
    tracing::info!("Starting balance: ${:.0}", starting_balance);

    // Load snapshots
    let snapshots = ReplayPipeline::load_snapshots(snapshot_dir)?;
    if snapshots.is_empty() {
        anyhow::bail!(
            "No liquidation zone snapshots found in {}",
            snapshot_dir.display()
        );
    }
    tracing::info!("Loaded {} snapshots", snapshots.len());

    // Convert to replay data points
    let data_points = ReplayPipeline::snapshots_to_replay_points(&snapshots);
    tracing::info!("Converted to {} replay data points", data_points.len());

    // Create output directory
    let output_dir = "data/liquidation-replay-results";
    std::fs::create_dir_all(output_dir)?;

    // Default configs
    let fishing_config = FishingLadderConfig::default();
    let pyramid_config = PyramidConfig::default();
    let gate_config = PromotionGateConfig {
        starting_balance,
        ..PromotionGateConfig::default()
    };

    // Run replay for each strategy
    let mut all_results = Vec::new();

    for strategy_name in &strategy_names {
        tracing::info!("--- Running replay for strategy: {} ---", strategy_name);

        // Create strategy from config
        let sub_table = config.strategy.get_sub_table(strategy_name);
        let params = config.strategy.get_params(strategy_name).unwrap_or_else(|_| {
            crate::strategy::StrategyParams {
                direction_bias: "neutral".to_string(),
                momentum_threshold_pct: 0.15,
                lookback_count: 60,
                scale_in_clips: 1,
                clip_size_usd: 100.0,
                max_hold_secs: 1800,
                take_profit_pct: 2.5,
                stop_loss_pct: 1.0,
                trailing_stop_pct: 0.8,
                trailing_activation_pct: 1.5,
                cooldown_after_loss_secs: 300,
                use_native_tp_sl: true,
            }
        });

        let mut strategy = crate::strategy::create_strategy_from_config(
            strategy_name,
            sub_table,
            params,
        )?;

        // Create the replay pipeline
        let cascade_params = crate::strategy::LiquidationCascadeParams::default();
        let pipeline = ReplayPipeline::new(cascade_params, gate_config.clone());

        // Run with generic strategy + fishing + pyramiding
        let result = pipeline.run_generic_with_fishing_and_pyramiding(
            strategy.as_mut(),
            &data_points,
            starting_balance,
            300, // stale_data_threshold_secs
            &fishing_config,
            &pyramid_config,
        );

        // Generate timestamp for filenames
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let safe_name = strategy_name.replace('-', "_");
        let json_path = format!("{}/{}_{}.json", output_dir, safe_name, timestamp);
        let md_path = format!("{}/{}_{}.md", output_dir, safe_name, timestamp);

        // Write JSON report
        ReplayPipeline::write_json_report(&result, Path::new(&json_path))?;
        tracing::info!("JSON report: {}", json_path);

        // Write Markdown report
        ReplayPipeline::write_markdown_report(&result, Path::new(&md_path))?;
        tracing::info!("Markdown report: {}", md_path);

        // Print summary to stdout
        println!("\n=== Liquidation Replay: {} ===", strategy_name);
        println!("Data points: {}", result.data_points_replayed);
        println!("Trades: {} ({}W / {}L)", result.trade_count, result.win_count, result.loss_count);
        println!("Win rate: {:.1}%", result.win_rate_pct);
        println!("Net PnL: ${:.2}", result.net_pnl);
        println!("Sharpe: {:.4}  |  Sortino: {:.4}  |  Calmar: {:.4}",
            result.sharpe_ratio, result.sortino_ratio, result.calmar_ratio);
        println!("Max drawdown: ${:.2} ({:.2}%)", result.max_drawdown_usd, result.max_drawdown_pct);
        println!("Fishing fill rate: {:.1}%", result.fishing_fill_rate * 100.0);
        println!("Promotion verdict: {:?}", result.promotion_verdict);

        let pass_count = result.promotion_criteria.iter().filter(|c| c.passed).count();
        println!("Gate: {}/12 criteria passed", pass_count);
        for c in &result.promotion_criteria {
            let status = if c.passed { "✅" } else { "❌" };
            println!("  {} {} — {} {} (threshold: {} {})",
                status, c.description, c.actual_value, c.unit, c.threshold_value, c.unit);
        }

        all_results.push((strategy_name.clone(), result));
    }

    // Print cross-strategy comparison if multiple strategies were run
    if all_results.len() > 1 {
        println!("\n=== Strategy Comparison ===");
        println!("{:<35} {:>8} {:>8} {:>8} {:>10} {:>8}",
            "Strategy", "Trades", "WinRate", "Sharpe", "NetPnL", "Gate");
        for (name, result) in &all_results {
            let pass_count = result.promotion_criteria.iter().filter(|c| c.passed).count();
            println!("{:<35} {:>8} {:>7.1}% {:>8.4} {:>10.2} {:>5}/12",
                name, result.trade_count, result.win_rate_pct,
                result.sharpe_ratio, result.net_pnl, pass_count);
        }
    }

    tracing::info!("Liquidation replay complete");
    Ok(())
}

/// Parse a backtest time string (ISO 8601 full or date-only) to milliseconds.
fn parse_backtest_time(input: Option<&str>, label: &str) -> anyhow::Result<i64> {
    let s = input.ok_or_else(|| anyhow::anyhow!("--backtest-{} is required in backtest mode", label))?;

    // Try full ISO 8601 datetime first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }

    // Try date-only format (e.g., "2025-05-01")
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date.and_hms_opt(0, 0, 0).unwrap()
            .and_local_timezone(chrono::Utc)
            .single()
            .ok_or_else(|| anyhow::anyhow!("Invalid date: {}", s))?;
        return Ok(dt.timestamp_millis());
    }

    anyhow::bail!(
        "Invalid --backtest-{} time: '{}'. Use ISO 8601 (2025-05-01T00:00:00Z) or date-only (2025-05-01)",
        label, s
    )
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::path::PathBuf;

    use super::Args;

    #[test]
    fn test_hl_paper_flag_parsed() {
        let args = Args::try_parse_from(["zekt", "--hl-paper"]);
        assert!(args.is_ok(), "Failed to parse --hl-paper: {:?}", args.err());
        assert!(args.unwrap().hl_paper);
    }

    #[test]
    fn test_hl_paper_default_balance() {
        let args = Args::try_parse_from(["zekt", "--hl-paper"]).unwrap();
        assert!((args.paper_balance - 1000.0).abs() < f64::EPSILON,
            "Default paper balance should be 1000, got {}", args.paper_balance);
    }

    #[test]
    fn test_hl_paper_custom_balance() {
        let args = Args::try_parse_from(["zekt", "--hl-paper", "--paper-balance", "5000"]).unwrap();
        assert!((args.paper_balance - 5000.0).abs() < f64::EPSILON,
            "Custom balance should be 5000, got {}", args.paper_balance);
    }

    #[test]
    fn test_hl_paper_multi_strategy() {
        let args = Args::try_parse_from([
            "zekt", "--hl-paper",
            "--strategies", "momentum-scalper,mean-reversion",
        ]).unwrap();
        assert!(args.hl_paper);
        let strategies: Vec<&str> = args.strategies.as_deref().unwrap().split(',').collect();
        assert_eq!(strategies, vec!["momentum-scalper", "mean-reversion"]);
    }

    #[test]
    fn test_hl_paper_multi_market() {
        let args = Args::try_parse_from([
            "zekt", "--hl-paper",
            "--markets", "BTC,SOL,ETH",
        ]).unwrap();
        assert!(args.hl_paper);
        let markets: Vec<&str> = args.markets.as_deref().unwrap().split(',').collect();
        assert_eq!(markets, vec!["BTC", "SOL", "ETH"]);
    }

    #[test]
    fn test_hl_paper_missing_markets_uses_default() {
        let args = Args::try_parse_from(["zekt", "--hl-paper"]).unwrap();
        assert!(args.hl_paper);
        // No --markets provided; should be None (runtime falls back to config default)
        assert!(args.markets.is_none());
    }

    #[test]
    fn test_hl_paper_missing_strategies_uses_default() {
        let args = Args::try_parse_from(["zekt", "--hl-paper"]).unwrap();
        assert!(args.hl_paper);
        // No --strategies provided; should be None (runtime falls back to config active)
        assert!(args.strategies.is_none());
    }

    #[test]
    fn test_hl_paper_default_output_dir() {
        let args = Args::try_parse_from(["zekt", "--hl-paper"]).unwrap();
        assert_eq!(args.paper_output, "data/paper-results");
    }

    #[test]
    fn test_hl_paper_conflicts_with_paper() {
        // Both flags set — the runtime check in main() rejects this,
        // but clap will happily parse both; the mode-exclusivity check
        // happens inside main().
        let args = Args::try_parse_from(["zekt", "--hl-paper", "--paper"]).unwrap();
        assert!(args.hl_paper);
        assert!(args.paper);
    }

    #[test]
    fn test_hl_paper_conflicts_with_backtest() {
        let args = Args::try_parse_from(["zekt", "--hl-paper", "--backtest"]).unwrap();
        assert!(args.hl_paper);
        assert!(args.backtest);
    }

    // ── New CLI flag tests (VAL-M1-018 through VAL-M1-024) ────────────────

    #[test]
    fn test_leverage_flag_parsed() {
        let args = Args::try_parse_from(["zekt", "--backtest", "--leverage", "3.0"]).unwrap();
        assert!(args.backtest);
        assert!((args.leverage.unwrap() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_leverage_flag_not_provided() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert!(args.leverage.is_none());
    }

    #[test]
    fn test_output_path_flag_parsed() {
        let args = Args::try_parse_from(["zekt", "--backtest", "--output-path", "data/custom"]).unwrap();
        assert_eq!(args.output_path.as_deref(), Some("data/custom"));
    }

    #[test]
    fn test_output_path_flag_not_provided() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert!(args.output_path.is_none());
    }

    #[test]
    fn test_param_override_valid_json() {
        let args = Args::try_parse_from([
            "zekt", "--backtest",
            "--param-override", "{\"clip_size_usd\": 200}",
        ]).unwrap();
        let json = args.param_override.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["clip_size_usd"], 200);
    }

    #[test]
    fn test_param_override_multiple_keys() {
        let args = Args::try_parse_from([
            "zekt", "--backtest",
            "--param-override", "{\"clip_size_usd\": 200, \"take_profit_pct\": 1.5}",
        ]).unwrap();
        let json = args.param_override.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["clip_size_usd"], 200);
        assert_eq!(parsed["take_profit_pct"], 1.5);
    }

    #[test]
    fn test_param_override_not_provided() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert!(args.param_override.is_none());
    }

    #[test]
    fn test_sizing_mode_flag_valid() {
        for mode in &["fixed-notional", "fixed-fractional", "volatility-adjusted",
                       "drawdown-throttled", "route-cost-adjusted"] {
            let args = Args::try_parse_from(["zekt", "--backtest", "--sizing-mode", mode]).unwrap();
            assert_eq!(args.sizing_mode, *mode);
        }
    }

    #[test]
    fn test_borrow_rate_flag_parsed() {
        let args = Args::try_parse_from(["zekt", "--backtest", "--borrow-rate", "0.0005"]).unwrap();
        assert!((args.borrow_rate.unwrap() - 0.0005).abs() < f64::EPSILON);
    }

    #[test]
    fn test_borrow_rate_flag_not_provided() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert!(args.borrow_rate.is_none());
    }

    #[test]
    fn test_all_new_flags_together() {
        let args = Args::try_parse_from([
            "zekt", "--backtest",
            "--leverage", "3.0",
            "--output-path", "data/custom-run",
            "--param-override", "{\"clip_size_usd\": 200}",
            "--sizing-mode", "volatility-adjusted",
            "--borrow-rate", "0.0005",
        ]).unwrap();
        assert!(args.backtest);
        assert!((args.leverage.unwrap() - 3.0).abs() < f64::EPSILON);
        assert_eq!(args.output_path.as_deref(), Some("data/custom-run"));
        assert!(args.param_override.is_some());
        assert_eq!(args.sizing_mode, "volatility-adjusted");
        assert!((args.borrow_rate.unwrap() - 0.0005).abs() < f64::EPSILON);
    }

    // ── Walk-forward CLI flag tests (VAL-M1-029) ───────────────────────────

    #[test]
    fn test_walk_forward_mode_flag_default() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert_eq!(args.walk_forward_mode, "single");
    }

    #[test]
    fn test_walk_forward_mode_flag_expanding() {
        let args = Args::try_parse_from([
            "zekt", "--backtest", "--walk-forward-mode", "expanding",
        ]).unwrap();
        assert_eq!(args.walk_forward_mode, "expanding");
    }

    #[test]
    fn test_walk_forward_mode_flag_single() {
        let args = Args::try_parse_from([
            "zekt", "--backtest", "--walk-forward-mode", "single",
        ]).unwrap();
        assert_eq!(args.walk_forward_mode, "single");
    }

    #[test]
    fn test_walk_forward_windows_flag_default() {
        let args = Args::try_parse_from(["zekt", "--backtest"]).unwrap();
        assert_eq!(args.walk_forward_windows, 5);
    }

    #[test]
    fn test_walk_forward_windows_flag_custom() {
        let args = Args::try_parse_from([
            "zekt", "--backtest", "--walk-forward-windows", "10",
        ]).unwrap();
        assert_eq!(args.walk_forward_windows, 10);
    }

    #[test]
    fn test_walk_forward_flags_together() {
        let args = Args::try_parse_from([
            "zekt", "--backtest",
            "--walk-forward-mode", "expanding",
            "--walk-forward-windows", "7",
        ]).unwrap();
        assert_eq!(args.walk_forward_mode, "expanding");
        assert_eq!(args.walk_forward_windows, 7);
    }

    // ── Liquidation replay CLI flag tests ────────────────────────────────

    #[test]
    fn test_liquidation_replay_flag_parsed() {
        let args = Args::try_parse_from(["zekt", "--liquidation-replay"]);
        assert!(args.is_ok(), "Failed to parse --liquidation-replay: {:?}", args.err());
        assert!(args.unwrap().liquidation_replay);
    }

    #[test]
    fn test_liquidation_replay_default_snapshot_dir() {
        let args = Args::try_parse_from(["zekt", "--liquidation-replay"]).unwrap();
        assert_eq!(args.snapshot_dir, PathBuf::from("data/liquidation-zones/"));
    }

    #[test]
    fn test_liquidation_replay_custom_snapshot_dir() {
        let args = Args::try_parse_from([
            "zekt", "--liquidation-replay",
            "--snapshot-dir", "data/custom-snapshots/",
        ]).unwrap();
        assert!(args.liquidation_replay);
        assert_eq!(args.snapshot_dir, PathBuf::from("data/custom-snapshots/"));
    }

    #[test]
    fn test_liquidation_replay_default_starting_balance() {
        let args = Args::try_parse_from(["zekt", "--liquidation-replay"]).unwrap();
        assert!((args.starting_balance - 1000.0).abs() < f64::EPSILON,
            "Default starting balance should be 1000, got {}", args.starting_balance);
    }

    #[test]
    fn test_liquidation_replay_custom_starting_balance() {
        let args = Args::try_parse_from([
            "zekt", "--liquidation-replay",
            "--starting-balance", "5000",
        ]).unwrap();
        assert!((args.starting_balance - 5000.0).abs() < f64::EPSILON,
            "Custom starting balance should be 5000, got {}", args.starting_balance);
    }

    #[test]
    fn test_liquidation_replay_with_strategy() {
        let args = Args::try_parse_from([
            "zekt", "--liquidation-replay",
            "--strategy", "sweep-reclaim",
        ]).unwrap();
        assert!(args.liquidation_replay);
        assert_eq!(args.strategy.as_deref(), Some("sweep-reclaim"));
    }

    #[test]
    fn test_liquidation_replay_with_strategy_all() {
        let args = Args::try_parse_from([
            "zekt", "--liquidation-replay",
            "--strategy", "all",
        ]).unwrap();
        assert!(args.liquidation_replay);
        assert_eq!(args.strategy.as_deref(), Some("all"));
    }

    #[test]
    fn test_liquidation_replay_conflicts_with_backtest() {
        let args = Args::try_parse_from(["zekt", "--liquidation-replay", "--backtest"]).unwrap();
        assert!(args.liquidation_replay);
        assert!(args.backtest);
        // The runtime mode-exclusivity check in main() handles this
    }

    #[test]
    fn test_liquidation_replay_conflicts_with_paper() {
        let args = Args::try_parse_from(["zekt", "--liquidation-replay", "--paper"]).unwrap();
        assert!(args.liquidation_replay);
        assert!(args.paper);
    }

    #[test]
    fn test_liquidation_replay_all_flags_together() {
        let args = Args::try_parse_from([
            "zekt", "--liquidation-replay",
            "--strategy", "cascade-continuation",
            "--snapshot-dir", "data/my-snapshots",
            "--starting-balance", "2500",
        ]).unwrap();
        assert!(args.liquidation_replay);
        assert_eq!(args.strategy.as_deref(), Some("cascade-continuation"));
        assert_eq!(args.snapshot_dir, PathBuf::from("data/my-snapshots"));
        assert!((args.starting_balance - 2500.0).abs() < f64::EPSILON);
    }
}
