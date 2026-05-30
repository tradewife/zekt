use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub agent: AgentConfig,
    pub flash: FlashConfig,
    pub strategy: StrategySection,
    pub risk: RiskConfig,
    #[serde(default)]
    pub imperial: ImperialConfig,
    #[serde(default, rename = "route-oracle")]
    pub route_oracle: RouteCostConfig,
    #[serde(default, rename = "alpha-scanner")]
    pub alpha_scanner: AlphaScannerConfig,
    #[serde(default, rename = "copy-trader")]
    pub copy_trader: CopyTraderConfig,
    #[serde(default, rename = "whale-watcher")]
    pub whale_watcher: WhaleWatcherConfig,
    #[serde(default)]
    pub hypurrscan: HypurrscanConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub poll_interval_secs: u64,
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlashConfig {
    pub api_url: String,
    pub rpc_url: String,
    pub keypair_path: String,
    pub market: String,
    pub input_token: String,
    pub pool: String,
    pub leverage: f64,
    pub slippage_pct: String,
}

/// Top-level strategy configuration section.
///
/// Supports two config formats:
///
/// **New format** (preferred):
/// ```toml
/// [strategy]
/// active = "momentum-scalper"
///
/// [strategy.momentum-scalper]
/// direction_bias = "neutral"
/// momentum_threshold_pct = 0.15
/// # ... all fields
/// ```
///
/// **Old format** (backward compatible):
/// ```toml
/// [strategy]
/// direction_bias = "neutral"
/// momentum_threshold_pct = 0.15
/// # ... all fields directly
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StrategySection {
    /// The name of the active strategy (e.g., "momentum-scalper").
    /// If absent, defaults to "momentum-scalper" for backward compatibility.
    #[serde(default)]
    pub active: Option<String>,

    /// Strategy-specific parameter sub-tables.
    /// Key is the strategy name (e.g., "momentum-scalper"), value is a TOML table.
    #[serde(default)]
    pub strategies: HashMap<String, toml::Value>,

    // --- Legacy flat fields (for backward compatibility) ---
    #[serde(default = "default_direction_bias")]
    pub direction_bias: String,
    #[serde(default = "default_momentum_threshold_pct")]
    pub momentum_threshold_pct: f64,
    #[serde(default = "default_lookback_count")]
    pub lookback_count: usize,
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
    #[serde(default = "default_clip_size_usd")]
    pub clip_size_usd: f64,
    #[serde(default = "default_max_hold_secs")]
    pub max_hold_secs: u64,
    #[serde(default = "default_take_profit_pct")]
    pub take_profit_pct: f64,
    #[serde(default = "default_stop_loss_pct")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    #[serde(default = "default_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,
}

fn default_direction_bias() -> String { "neutral".to_string() }
fn default_momentum_threshold_pct() -> f64 { 0.15 }
fn default_lookback_count() -> usize { 60 }
fn default_scale_in_clips() -> u32 { 1 }
fn default_clip_size_usd() -> f64 { 100.0 }
fn default_max_hold_secs() -> u64 { 1800 }
fn default_take_profit_pct() -> f64 { 2.5 }
fn default_stop_loss_pct() -> f64 { 1.0 }
fn default_trailing_stop_pct() -> f64 { 0.8 }
fn default_trailing_activation_pct() -> f64 { 1.5 }
fn default_cooldown_after_loss_secs() -> u64 { 300 }
fn default_use_native_tp_sl() -> bool { true }

// ---------------------------------------------------------------------------
// Alpha Scanner config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlphaScannerConfig {
    #[serde(default = "default_dextrabot_url")]
    pub dextrabot_url: String,
    #[serde(default = "default_min_sharpe_7d")]
    pub min_sharpe_7d: f64,
    #[serde(default = "default_min_sharpe_30d")]
    pub min_sharpe_30d: f64,
    #[serde(default = "default_min_pnl_30d")]
    pub min_pnl_30d: f64,
    #[serde(default = "default_watchlist_size")]
    pub watchlist_size: usize,
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u64,
    #[serde(default = "default_scanner_output_path")]
    pub output_path: String,
}

fn default_dextrabot_url() -> String { "https://dextradata.nftinit.io".to_string() }
fn default_min_sharpe_7d() -> f64 { 1.5 }
fn default_min_sharpe_30d() -> f64 { 2.0 }
fn default_min_pnl_30d() -> f64 { 5000.0 }
fn default_watchlist_size() -> usize { 20 }
fn default_refresh_interval_secs() -> u64 { 21600 }
fn default_scanner_output_path() -> String { "data/watchlist.json".to_string() }

impl Default for AlphaScannerConfig {
    fn default() -> Self {
        Self {
            dextrabot_url: default_dextrabot_url(),
            min_sharpe_7d: default_min_sharpe_7d(),
            min_sharpe_30d: default_min_sharpe_30d(),
            min_pnl_30d: default_min_pnl_30d(),
            watchlist_size: default_watchlist_size(),
            refresh_interval_secs: default_refresh_interval_secs(),
            output_path: default_scanner_output_path(),
        }
    }
}

// ---------------------------------------------------------------------------
// Copy Trader config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CopyTraderConfig {
    #[serde(default = "default_ct_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_ct_max_position_pct")]
    pub max_position_pct: f64,
    #[serde(default = "default_ct_max_positions")]
    pub max_positions: usize,
    #[serde(default = "default_ct_stop_loss_pct")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_ct_lag_secs")]
    pub lag_secs: u64,
    #[serde(default = "default_ct_sizing_multiplier")]
    pub sizing_multiplier: f64,
    #[serde(default = "default_ct_output_path")]
    pub output_path: String,
}

fn default_ct_poll_interval_secs() -> u64 { 30 }
fn default_ct_max_position_pct() -> f64 { 10.0 }
fn default_ct_max_positions() -> usize { 3 }
fn default_ct_stop_loss_pct() -> f64 { 5.0 }
fn default_ct_lag_secs() -> u64 { 30 }
fn default_ct_sizing_multiplier() -> f64 { 0.1 }
fn default_ct_output_path() -> String { "data/copy-trades.json".to_string() }

impl Default for CopyTraderConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_ct_poll_interval_secs(),
            max_position_pct: default_ct_max_position_pct(),
            max_positions: default_ct_max_positions(),
            stop_loss_pct: default_ct_stop_loss_pct(),
            lag_secs: default_ct_lag_secs(),
            sizing_multiplier: default_ct_sizing_multiplier(),
            output_path: default_ct_output_path(),
        }
    }
}

// ---------------------------------------------------------------------------
// Whale Watcher config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhaleWatcherConfig {
    #[serde(default = "default_min_notional_usd")]
    pub min_notional_usd: f64,
    #[serde(default = "default_accuracy_window_secs")]
    pub accuracy_window_secs: u64,
    #[serde(default = "default_whale_output_path")]
    pub output_path: String,
}

fn default_min_notional_usd() -> f64 { 10000.0 }
fn default_accuracy_window_secs() -> u64 { 3600 }
fn default_whale_output_path() -> String { "data/whale-alerts.json".to_string() }

impl Default for WhaleWatcherConfig {
    fn default() -> Self {
        Self {
            min_notional_usd: default_min_notional_usd(),
            accuracy_window_secs: default_accuracy_window_secs(),
            output_path: default_whale_output_path(),
        }
    }
}

// ---------------------------------------------------------------------------
// Hypurrscan config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HypurrscanConfig {
    #[serde(default = "default_jwt_env_var")]
    pub jwt_env_var: String,
    #[serde(default = "default_refresh_token_env_var")]
    pub refresh_token_env_var: String,
    #[serde(default = "default_hypurrscan_base_url")]
    pub base_url: String,
    #[serde(default = "default_hypurrscan_refresh_url")]
    pub refresh_url: String,
}

fn default_jwt_env_var() -> String { "HYPURRSCAN_JWT".to_string() }
fn default_refresh_token_env_var() -> String { "HYPURRSCAN_REFRESH_TOKEN".to_string() }
fn default_hypurrscan_base_url() -> String { "https://api.hypurrscan.io".to_string() }
fn default_hypurrscan_refresh_url() -> String { "https://hypurrscan.io/api/auth/refresh".to_string() }

impl Default for HypurrscanConfig {
    fn default() -> Self {
        Self {
            jwt_env_var: default_jwt_env_var(),
            refresh_token_env_var: default_refresh_token_env_var(),
            base_url: default_hypurrscan_base_url(),
            refresh_url: default_hypurrscan_refresh_url(),
        }
    }
}

// ---------------------------------------------------------------------------
// Imperial config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImperialConfig {
    /// Whether the Imperial integration is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Base URL for the Imperial API.
    #[serde(default = "default_imperial_base_url")]
    pub base_url: String,

    /// HTTP request timeout in seconds.
    #[serde(default = "default_imperial_timeout_secs")]
    pub timeout_secs: u64,

    /// Cache TTL for route responses in seconds.
    #[serde(default = "default_imperial_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
}

fn default_imperial_base_url() -> String {
    crate::imperial::IMPERIAL_BASE_URL.to_string()
}
fn default_imperial_timeout_secs() -> u64 {
    crate::imperial::IMPERIAL_DEFAULT_TIMEOUT_SECS
}
fn default_imperial_cache_ttl_secs() -> u64 {
    60
}

impl Default for ImperialConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_imperial_base_url(),
            timeout_secs: default_imperial_timeout_secs(),
            cache_ttl_secs: default_imperial_cache_ttl_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// Route Cost Oracle config
// ---------------------------------------------------------------------------

/// Configuration for the Route Cost Oracle.
///
/// Controls how `RouteCostOracle` compares trade costs across Solana perps venues
/// using the Imperial API. The oracle is **disabled by default** — when `enabled = false`,
/// no Imperial API calls are made and the existing Flash Trade cost model is used.
///
/// ```toml
/// [route-oracle]
/// enabled = false
/// improvement_threshold_bps = 5.0
/// edge_budget_pct = 100.0
/// staleness_threshold_secs = 60
/// cache_ttl_secs = 60
/// cache_bucket_usd = 100.0
/// excluded_venues = []
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteCostConfig {
    /// Whether the route cost oracle is enabled.
    /// When false, no Imperial API calls are made and flash-only costs are used everywhere.
    #[serde(default)]
    pub enabled: bool,

    /// Minimum improvement in basis points for the oracle to mark a route as `route_improved`.
    /// If Imperial route cost is cheaper than Flash cost by >= this many bps, route_improved = true.
    #[serde(default = "default_improvement_threshold_bps")]
    pub improvement_threshold_bps: f64,

    /// Maximum route cost as a percentage of the trade's expected edge (profit).
    /// If route cost exceeds this percentage, the trade is vetoed.
    /// 100% means never veto (allow all trades). 50% means cost must be < 50% of expected edge.
    #[serde(default = "default_edge_budget_pct")]
    pub edge_budget_pct: f64,

    /// How old (in seconds) route data can be before it's considered stale.
    /// Stale data triggers fallback to Flash-only costs with a degradation log.
    #[serde(default = "default_staleness_threshold_secs")]
    pub staleness_threshold_secs: u64,

    /// Cache TTL in seconds. Identical (market, side, size_bucket) queries within
    /// this window return cached results without new API calls.
    #[serde(default = "default_route_cache_ttl_secs")]
    pub cache_ttl_secs: u64,

    /// Size bucket width in USD for cache grouping. Queries with sizes in the same
    /// bucket share cache entries (e.g., $100 means $1050 and $1099 share a cache entry).
    #[serde(default = "default_cache_bucket_usd")]
    pub cache_bucket_usd: f64,

    /// Venue names to exclude from routing decisions (e.g., ["jupiter"]).
    #[serde(default)]
    pub excluded_venues: Vec<String>,
}

fn default_improvement_threshold_bps() -> f64 { 5.0 }
fn default_edge_budget_pct() -> f64 { 100.0 }
fn default_staleness_threshold_secs() -> u64 { 60 }
fn default_route_cache_ttl_secs() -> u64 { 60 }
fn default_cache_bucket_usd() -> f64 { 100.0 }

impl Default for RouteCostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            improvement_threshold_bps: default_improvement_threshold_bps(),
            edge_budget_pct: default_edge_budget_pct(),
            staleness_threshold_secs: default_staleness_threshold_secs(),
            cache_ttl_secs: default_route_cache_ttl_secs(),
            cache_bucket_usd: default_cache_bucket_usd(),
            excluded_venues: Vec::new(),
        }
    }
}

impl StrategySection {
    /// Resolve the active strategy name.
    /// Priority: explicit `active` field → default "momentum-scalper".
    pub fn resolve_active(&self, cli_override: Option<&str>) -> String {
        cli_override
            .map(|s| s.to_string())
            .or_else(|| self.active.clone())
            .unwrap_or_else(|| "momentum-scalper".to_string())
    }

    /// Get the parameters for a specific strategy.
    ///
    /// If a sub-table exists for the strategy name, parse it.
    /// Otherwise, fall back to the flat legacy fields (backward compatibility).
    pub fn get_params(&self, strategy_name: &str) -> anyhow::Result<crate::strategy::StrategyParams> {
        if let Some(sub_table) = self.strategies.get(strategy_name) {
            // New format: parse the sub-table
            let params: StrategyFlatParams = sub_table.clone().try_into().map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse [strategy.{}] sub-table: {}. \
                     Config format updated. Please restructure [strategy] to include \
                     active = \"{}\" and [strategy.{}] sub-table with all strategy fields.",
                    strategy_name, e, strategy_name, strategy_name
                )
            })?;
            Ok(params.into_strategy_params())
        } else if strategy_name == "momentum-scalper" {
            // Old format: use flat fields directly
            Ok(crate::strategy::StrategyParams {
                direction_bias: self.direction_bias.clone(),
                momentum_threshold_pct: self.momentum_threshold_pct,
                lookback_count: self.lookback_count,
                scale_in_clips: self.scale_in_clips,
                clip_size_usd: self.clip_size_usd,
                max_hold_secs: self.max_hold_secs,
                take_profit_pct: self.take_profit_pct,
                stop_loss_pct: self.stop_loss_pct,
                trailing_stop_pct: self.trailing_stop_pct,
                trailing_activation_pct: self.trailing_activation_pct,
                cooldown_after_loss_secs: self.cooldown_after_loss_secs,
                use_native_tp_sl: self.use_native_tp_sl,
            })
        } else {
            anyhow::bail!(
                "No configuration found for strategy '{}'. Available sub-tables: [{}]. \
                 Add a [strategy.{}] section to your config file.",
                strategy_name,
                self.strategies.keys().cloned().collect::<Vec<_>>().join(", "),
                strategy_name
            )
        }
    }

    /// Get the TOML sub-table for a strategy, if one exists.
    /// Returns None if the strategy uses the flat legacy config format.
    pub fn get_sub_table(&self, strategy_name: &str) -> Option<&toml::Value> {
        self.strategies.get(strategy_name)
    }
}

/// Intermediate struct for parsing strategy sub-tables from TOML.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct StrategyFlatParams {
    #[serde(default = "default_direction_bias")]
    pub direction_bias: String,
    #[serde(default = "default_momentum_threshold_pct")]
    pub momentum_threshold_pct: f64,
    #[serde(default = "default_lookback_count")]
    pub lookback_count: usize,
    #[serde(default = "default_scale_in_clips")]
    pub scale_in_clips: u32,
    #[serde(default = "default_clip_size_usd")]
    pub clip_size_usd: f64,
    #[serde(default = "default_max_hold_secs")]
    pub max_hold_secs: u64,
    #[serde(default = "default_take_profit_pct")]
    pub take_profit_pct: f64,
    #[serde(default = "default_stop_loss_pct")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_trailing_stop_pct")]
    pub trailing_stop_pct: f64,
    #[serde(default = "default_trailing_activation_pct")]
    pub trailing_activation_pct: f64,
    #[serde(default = "default_cooldown_after_loss_secs")]
    pub cooldown_after_loss_secs: u64,
    #[serde(default = "default_use_native_tp_sl")]
    pub use_native_tp_sl: bool,
}

impl StrategyFlatParams {
    fn into_strategy_params(self) -> crate::strategy::StrategyParams {
        crate::strategy::StrategyParams {
            direction_bias: self.direction_bias,
            momentum_threshold_pct: self.momentum_threshold_pct,
            lookback_count: self.lookback_count,
            scale_in_clips: self.scale_in_clips,
            clip_size_usd: self.clip_size_usd,
            max_hold_secs: self.max_hold_secs,
            take_profit_pct: self.take_profit_pct,
            stop_loss_pct: self.stop_loss_pct,
            trailing_stop_pct: self.trailing_stop_pct,
            trailing_activation_pct: self.trailing_activation_pct,
            cooldown_after_loss_secs: self.cooldown_after_loss_secs,
            use_native_tp_sl: self.use_native_tp_sl,
        }
    }
}

/// Legacy type alias for backward compatibility with code that expects `StrategyConfig`.
/// This is the old flat struct; new code should use `StrategySection` + `StrategyParams`.
#[allow(dead_code)]
pub type StrategyConfig = StrategySection;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RiskConfig {
    pub max_position_notional_usd: f64,
    pub max_daily_loss_usd: f64,
    pub max_drawdown_pct: f64,
    /// Maximum total notional exposure across ALL open positions (cross-cell limit).
    /// New positions are rejected if total open notional + new position notional > this value.
    #[serde(default = "default_max_total_notional_usd")]
    pub max_total_notional_usd: f64,
    /// Maximum weekly loss in USD. Trading halts when cumulative weekly losses exceed this.
    #[serde(default = "default_max_weekly_loss_usd")]
    pub max_weekly_loss_usd: f64,
    /// Maximum correlated exposure as a percentage (0-100) of total account balance.
    /// Rejects new positions in correlated markets when combined exposure exceeds this.
    #[serde(default = "default_max_correlated_exposure_pct")]
    pub max_correlated_exposure_pct: f64,
    /// Number of consecutive losses before circuit breaker triggers. Set to 0 to disable.
    #[serde(default = "default_consecutive_loss_circuit_breaker")]
    pub consecutive_loss_circuit_breaker: u32,
    /// Enable ATR-based volatility position sizing. Reduces clip size in high-vol regimes.
    #[serde(default)]
    pub volatility_sizing_enabled: bool,
    /// ATR percentile threshold (0-100) above which clip sizes are reduced.
    /// At 100th percentile, clip is reduced to volatility_sizing_min_fraction.
    #[serde(default = "default_volatility_sizing_atr_threshold_pct")]
    pub volatility_sizing_atr_threshold_pct: f64,
    /// Minimum fraction of clip_size to use even in extreme volatility (0.0-1.0).
    #[serde(default = "default_volatility_sizing_min_fraction")]
    pub volatility_sizing_min_fraction: f64,
    /// Number of consecutive API failures before trading halts. Set to 0 to disable.
    #[serde(default = "default_api_degradation_threshold")]
    pub api_degradation_threshold: u32,
    /// Correlated market groups. Each group is a list of symbols that count toward
    /// correlated exposure. If empty, all markets are treated as independent.
    #[serde(default)]
    pub correlated_groups: Vec<CorrelatedGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CorrelatedGroup {
    /// Human-readable name for the group (e.g., "L1 majors", "Solana ecosystem").
    pub name: String,
    /// Symbols in this group (e.g., ["SOL", "JTO", "JUP"]).
    pub symbols: Vec<String>,
}

fn default_max_total_notional_usd() -> f64 { 100_000.0 }
fn default_max_weekly_loss_usd() -> f64 { 100_000.0 }
fn default_max_correlated_exposure_pct() -> f64 { 100.0 }
fn default_consecutive_loss_circuit_breaker() -> u32 { 0 }
fn default_volatility_sizing_atr_threshold_pct() -> f64 { 75.0 }
fn default_volatility_sizing_min_fraction() -> f64 { 0.25 }
fn default_api_degradation_threshold() -> u32 { 0 }

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        // Post-process: extract strategy sub-tables from the raw TOML.
        // TOML sub-tables like [strategy.lp-consumption] are not automatically
        // picked up by serde's HashMap<String, Value> when the struct also has
        // named fields. We need to manually extract them.
        let raw: toml::Value = toml::from_str(&content)?;
        if let Some(strategy_table) = raw.get("strategy").and_then(|v| v.as_table()) {
            let known_flat_fields = [
                "active", "direction_bias", "momentum_threshold_pct", "lookback_count",
                "scale_in_clips", "clip_size_usd", "max_hold_secs", "take_profit_pct",
                "stop_loss_pct", "trailing_stop_pct", "trailing_activation_pct",
                "cooldown_after_loss_secs", "use_native_tp_sl",
            ];
            for (key, value) in strategy_table {
                if !known_flat_fields.contains(&key.as_str()) && value.is_table() {
                    config.strategy.strategies.insert(
                        key.clone(),
                        value.clone(),
                    );
                }
            }
        }

        Ok(config)
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.agent.poll_interval_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: create a temp TOML file with the given content.
    fn write_temp_toml(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("create temp file");
        write!(f, "{}", content).expect("write temp file");
        f
    }

    // ── Baseline: existing sections parse unchanged ────────────────────────

    #[test]
    fn test_existing_config_sections_parse() {
        let minimal = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"
direction_bias = "neutral"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0
"#;
        let f = write_temp_toml(minimal);
        let config = Config::load(f.path()).expect("minimal config should parse");

        // Existing values
        assert_eq!(config.agent.poll_interval_secs, 300);
        assert_eq!(config.agent.log_level, "info");
        assert_eq!(config.flash.market, "SOL");
        assert_eq!(config.flash.leverage, 3.0);
        assert_eq!(config.risk.max_position_notional_usd, 5000.0);
        assert_eq!(config.risk.max_daily_loss_usd, 500.0);
        assert_eq!(config.risk.max_drawdown_pct, 15.0);

        // New sections get defaults
        assert_eq!(config.alpha_scanner.watchlist_size, 20);
        assert_eq!(config.copy_trader.poll_interval_secs, 30);
        assert_eq!(config.whale_watcher.min_notional_usd, 10000.0);
        assert_eq!(config.hypurrscan.base_url, "https://api.hypurrscan.io");
    }

    #[test]
    fn test_existing_config_values_unchanged_with_new_sections() {
        let full = r#"
[agent]
poll_interval_secs = 60
log_level = "debug"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "BTC"
input_token = "USDC"
pool = "Crypto.1"
leverage = 5.0
slippage_pct = "1.0"

[strategy]
active = "trend-follower"
direction_bias = "long"
momentum_threshold_pct = 0.25

[risk]
max_position_notional_usd = 10000.0
max_daily_loss_usd = 1000.0
max_drawdown_pct = 20.0
max_total_notional_usd = 50000.0

[alpha-scanner]
dextrabot_url = "https://custom.api.example.com"
min_sharpe_7d = 2.0
min_sharpe_30d = 3.0
min_pnl_30d = 10000
watchlist_size = 50
refresh_interval_secs = 43200
output_path = "custom/watchlist.json"

[copy-trader]
poll_interval_secs = 60
max_position_pct = 15.0
max_positions = 5
stop_loss_pct = 3.0
lag_secs = 15
sizing_multiplier = 0.2
output_path = "custom/copy-trades.json"

[whale-watcher]
min_notional_usd = 50000
accuracy_window_secs = 7200
output_path = "custom/whale-alerts.json"

[hypurrscan]
jwt_env_var = "CUSTOM_JWT"
refresh_token_env_var = "CUSTOM_REFRESH"
base_url = "https://custom.hypurrscan.io"
refresh_url = "https://custom.hypurrscan.io/refresh"

[strategy.funding-capture]
min_annualized_rate_pct = 30.0
exit_annualized_rate_pct = 10.0
max_position_hours = 48
leverage = 2.0
clip_size_usd = 500.0
"#;
        let f = write_temp_toml(full);
        let config = Config::load(f.path()).expect("full config should parse");

        // Existing sections unchanged
        assert_eq!(config.agent.poll_interval_secs, 60);
        assert_eq!(config.agent.log_level, "debug");
        assert_eq!(config.flash.market, "BTC");
        assert_eq!(config.flash.leverage, 5.0);
        assert_eq!(config.strategy.active.as_deref(), Some("trend-follower"));
        assert_eq!(config.strategy.momentum_threshold_pct, 0.25);
        assert_eq!(config.risk.max_position_notional_usd, 10000.0);
        assert_eq!(config.risk.max_total_notional_usd, 50000.0);

        // Alpha scanner
        assert_eq!(config.alpha_scanner.dextrabot_url, "https://custom.api.example.com");
        assert_eq!(config.alpha_scanner.min_sharpe_7d, 2.0);
        assert_eq!(config.alpha_scanner.min_sharpe_30d, 3.0);
        assert_eq!(config.alpha_scanner.min_pnl_30d, 10000.0);
        assert_eq!(config.alpha_scanner.watchlist_size, 50);
        assert_eq!(config.alpha_scanner.refresh_interval_secs, 43200);
        assert_eq!(config.alpha_scanner.output_path, "custom/watchlist.json");

        // Copy trader
        assert_eq!(config.copy_trader.poll_interval_secs, 60);
        assert_eq!(config.copy_trader.max_position_pct, 15.0);
        assert_eq!(config.copy_trader.max_positions, 5);
        assert_eq!(config.copy_trader.stop_loss_pct, 3.0);
        assert_eq!(config.copy_trader.lag_secs, 15);
        assert_eq!(config.copy_trader.sizing_multiplier, 0.2);
        assert_eq!(config.copy_trader.output_path, "custom/copy-trades.json");

        // Whale watcher
        assert_eq!(config.whale_watcher.min_notional_usd, 50000.0);
        assert_eq!(config.whale_watcher.accuracy_window_secs, 7200);
        assert_eq!(config.whale_watcher.output_path, "custom/whale-alerts.json");

        // Hypurrscan
        assert_eq!(config.hypurrscan.jwt_env_var, "CUSTOM_JWT");
        assert_eq!(config.hypurrscan.refresh_token_env_var, "CUSTOM_REFRESH");
        assert_eq!(config.hypurrscan.base_url, "https://custom.hypurrscan.io");
        assert_eq!(config.hypurrscan.refresh_url, "https://custom.hypurrscan.io/refresh");

        // Funding capture strategy sub-table
        let fc_sub = config.strategy.get_sub_table("funding-capture")
            .expect("funding-capture sub-table should exist");
        assert!(fc_sub.is_table());
        let fc_table = fc_sub.as_table().unwrap();
        assert_eq!(fc_table.get("min_annualized_rate_pct").unwrap().as_float(), Some(30.0));
        assert_eq!(fc_table.get("exit_annualized_rate_pct").unwrap().as_float(), Some(10.0));
        assert_eq!(fc_table.get("max_position_hours").unwrap().as_integer(), Some(48));
        assert_eq!(fc_table.get("leverage").unwrap().as_float(), Some(2.0));
        assert_eq!(fc_table.get("clip_size_usd").unwrap().as_float(), Some(500.0));
    }

    // ── Default values ─────────────────────────────────────────────────────

    #[test]
    fn test_alpha_scanner_defaults() {
        let default_config = AlphaScannerConfig::default();
        assert_eq!(default_config.dextrabot_url, "https://dextradata.nftinit.io");
        assert_eq!(default_config.min_sharpe_7d, 1.5);
        assert_eq!(default_config.min_sharpe_30d, 2.0);
        assert_eq!(default_config.min_pnl_30d, 5000.0);
        assert_eq!(default_config.watchlist_size, 20);
        assert_eq!(default_config.refresh_interval_secs, 21600);
        assert_eq!(default_config.output_path, "data/watchlist.json");
    }

    #[test]
    fn test_copy_trader_defaults() {
        let default_config = CopyTraderConfig::default();
        assert_eq!(default_config.poll_interval_secs, 30);
        assert_eq!(default_config.max_position_pct, 10.0);
        assert_eq!(default_config.max_positions, 3);
        assert_eq!(default_config.stop_loss_pct, 5.0);
        assert_eq!(default_config.lag_secs, 30);
        assert_eq!(default_config.sizing_multiplier, 0.1);
        assert_eq!(default_config.output_path, "data/copy-trades.json");
    }

    #[test]
    fn test_whale_watcher_defaults() {
        let default_config = WhaleWatcherConfig::default();
        assert_eq!(default_config.min_notional_usd, 10000.0);
        assert_eq!(default_config.accuracy_window_secs, 3600);
        assert_eq!(default_config.output_path, "data/whale-alerts.json");
    }

    #[test]
    fn test_hypurrscan_defaults() {
        let default_config = HypurrscanConfig::default();
        assert_eq!(default_config.jwt_env_var, "HYPURRSCAN_JWT");
        assert_eq!(default_config.refresh_token_env_var, "HYPURRSCAN_REFRESH_TOKEN");
        assert_eq!(default_config.base_url, "https://api.hypurrscan.io");
        assert_eq!(default_config.refresh_url, "https://hypurrscan.io/api/auth/refresh");
    }

    // ── New sections absent → defaults applied ────────────────────────────

    #[test]
    fn test_missing_new_sections_get_defaults() {
        let minimal = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0
"#;
        let f = write_temp_toml(minimal);
        let config = Config::load(f.path()).expect("config without new sections should parse");

        // All new sections get defaults via #[serde(default)]
        assert_eq!(config.alpha_scanner.dextrabot_url, "https://dextradata.nftinit.io");
        assert_eq!(config.alpha_scanner.watchlist_size, 20);

        assert_eq!(config.copy_trader.poll_interval_secs, 30);
        assert_eq!(config.copy_trader.max_positions, 3);

        assert_eq!(config.whale_watcher.min_notional_usd, 10000.0);

        assert_eq!(config.hypurrscan.jwt_env_var, "HYPURRSCAN_JWT");
    }

    // ── Partial overrides ──────────────────────────────────────────────────

    #[test]
    fn test_partial_new_section_overrides() {
        let partial = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0

[alpha-scanner]
watchlist_size = 10

[copy-trader]
max_positions = 5
"#;
        let f = write_temp_toml(partial);
        let config = Config::load(f.path()).expect("partial config should parse");

        // Alpha scanner: overridden + defaults
        assert_eq!(config.alpha_scanner.watchlist_size, 10);
        assert_eq!(config.alpha_scanner.min_sharpe_7d, 1.5); // default
        assert_eq!(config.alpha_scanner.dextrabot_url, "https://dextradata.nftinit.io"); // default

        // Copy trader: overridden + defaults
        assert_eq!(config.copy_trader.max_positions, 5);
        assert_eq!(config.copy_trader.poll_interval_secs, 30); // default
        assert_eq!(config.copy_trader.stop_loss_pct, 5.0); // default

        // Whale watcher: all defaults
        assert_eq!(config.whale_watcher.min_notional_usd, 10000.0);

        // Hypurrscan: all defaults
        assert_eq!(config.hypurrscan.base_url, "https://api.hypurrscan.io");
    }

    // ── Real perps.toml loads ──────────────────────────────────────────────

    #[test]
    fn test_real_perps_toml_loads() {
        let config_path = Path::new("config/perps.toml");
        if !config_path.exists() {
            eprintln!("Skipping: config/perps.toml not found");
            return;
        }
        let config = Config::load(config_path).expect("real perps.toml should load");

        // Existing sections
        assert_eq!(config.agent.poll_interval_secs, 300);
        assert_eq!(config.flash.market, "SOL");
        assert_eq!(config.risk.max_position_notional_usd, 5000.0);

        // New sections with expected values from perps.toml
        assert_eq!(config.alpha_scanner.dextrabot_url, "https://dextradata.nftinit.io");
        assert_eq!(config.alpha_scanner.min_sharpe_7d, 1.5);
        assert_eq!(config.alpha_scanner.min_sharpe_30d, 2.0);
        assert_eq!(config.alpha_scanner.min_pnl_30d, 5000.0);
        assert_eq!(config.alpha_scanner.watchlist_size, 20);
        assert_eq!(config.alpha_scanner.refresh_interval_secs, 21600);
        assert_eq!(config.alpha_scanner.output_path, "data/watchlist.json");

        assert_eq!(config.copy_trader.poll_interval_secs, 30);
        assert_eq!(config.copy_trader.max_position_pct, 10.0);
        assert_eq!(config.copy_trader.max_positions, 3);
        assert_eq!(config.copy_trader.stop_loss_pct, 5.0);
        assert_eq!(config.copy_trader.lag_secs, 30);
        assert_eq!(config.copy_trader.sizing_multiplier, 0.1);
        assert_eq!(config.copy_trader.output_path, "data/copy-trades.json");

        assert_eq!(config.whale_watcher.min_notional_usd, 10000.0);
        assert_eq!(config.whale_watcher.accuracy_window_secs, 3600);
        assert_eq!(config.whale_watcher.output_path, "data/whale-alerts.json");

        assert_eq!(config.hypurrscan.jwt_env_var, "HYPURRSCAN_JWT");
        assert_eq!(config.hypurrscan.refresh_token_env_var, "HYPURRSCAN_REFRESH_TOKEN");
        assert_eq!(config.hypurrscan.base_url, "https://api.hypurrscan.io");
        assert_eq!(config.hypurrscan.refresh_url, "https://hypurrscan.io/api/auth/refresh");

        // Funding capture strategy sub-table
        let fc_sub = config.strategy.get_sub_table("funding-capture")
            .expect("funding-capture sub-table should exist");
        let fc_table = fc_sub.as_table().unwrap();
        assert_eq!(fc_table.get("min_annualized_rate_pct").unwrap().as_float(), Some(15.0));
        assert_eq!(fc_table.get("exit_annualized_rate_pct").unwrap().as_float(), Some(3.0));
        assert_eq!(fc_table.get("max_position_hours").unwrap().as_integer(), Some(72));
        assert_eq!(fc_table.get("leverage").unwrap().as_float(), Some(1.0));
        assert_eq!(fc_table.get("clip_size_usd").unwrap().as_float(), Some(100.0));
        assert_eq!(fc_table.get("stop_loss_pct").unwrap().as_float(), Some(3.0));
        assert_eq!(fc_table.get("min_hold_before_sl_mins").unwrap().as_integer(), Some(120));

        // Existing strategy sub-tables still work
        assert!(config.strategy.get_sub_table("lp-consumption").is_some());
        assert!(config.strategy.get_sub_table("mean-reversion").is_some());
        assert!(config.strategy.get_sub_table("trend-follower").is_some());
    }

    // ── Existing strategy sub-tables preserved ─────────────────────────────

    #[test]
    fn test_existing_strategy_subtables_preserved() {
        let config_path = Path::new("config/perps.toml");
        if !config_path.exists() {
            eprintln!("Skipping: config/perps.toml not found");
            return;
        }
        let config = Config::load(config_path).expect("real perps.toml should load");

        // Verify existing strategy sub-tables are all present
        let sub_tables = &config.strategy.strategies;
        assert!(sub_tables.contains_key("lp-consumption"), "lp-consumption sub-table missing");
        assert!(sub_tables.contains_key("mean-reversion"), "mean-reversion sub-table missing");
        assert!(sub_tables.contains_key("trend-follower"), "trend-follower sub-table missing");
        assert!(sub_tables.contains_key("funding-capture"), "funding-capture sub-table missing");

        // Verify existing sub-tables have expected fields
        let lp = sub_tables.get("lp-consumption").unwrap().as_table().unwrap();
        assert!(lp.contains_key("consumption_velocity_threshold"));
        assert!(lp.contains_key("clip_size_usd"));

        let mr = sub_tables.get("mean-reversion").unwrap().as_table().unwrap();
        assert!(mr.contains_key("mean_lookback"));
        assert!(mr.contains_key("deviation_threshold_pct"));
    }

    // ── Imperial config tests ──────────────────────────────────────────────

    #[test]
    fn test_imperial_config_defaults_when_absent() {
        let minimal = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0
"#;
        let f = write_temp_toml(minimal);
        let config = Config::load(f.path()).expect("config without [imperial] should parse");

        // Imperial section gets defaults
        assert!(!config.imperial.enabled, "imperial should be disabled by default");
        assert_eq!(config.imperial.base_url, "https://api.imperial.space");
        assert_eq!(config.imperial.timeout_secs, 30);
        assert_eq!(config.imperial.cache_ttl_secs, 60);
    }

    #[test]
    fn test_imperial_config_custom_values() {
        let custom = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0

[imperial]
enabled = true
base_url = "http://test.example.com"
timeout_secs = 15
cache_ttl_secs = 120
"#;
        let f = write_temp_toml(custom);
        let config = Config::load(f.path()).expect("config with [imperial] should parse");

        assert!(config.imperial.enabled);
        assert_eq!(config.imperial.base_url, "http://test.example.com");
        assert_eq!(config.imperial.timeout_secs, 15);
        assert_eq!(config.imperial.cache_ttl_secs, 120);
    }

    #[test]
    fn test_imperial_config_partial_override() {
        let partial = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0

[imperial]
timeout_secs = 10
"#;
        let f = write_temp_toml(partial);
        let config = Config::load(f.path()).expect("partial imperial config should parse");

        // Overridden
        assert_eq!(config.imperial.timeout_secs, 10);
        // Defaults for the rest
        assert!(!config.imperial.enabled);
        assert_eq!(config.imperial.base_url, "https://api.imperial.space");
        assert_eq!(config.imperial.cache_ttl_secs, 60);
    }

    #[test]
    fn test_imperial_config_default_impl() {
        let default_config = ImperialConfig::default();
        assert!(!default_config.enabled);
        assert_eq!(default_config.base_url, "https://api.imperial.space");
        assert_eq!(default_config.timeout_secs, 30);
        assert_eq!(default_config.cache_ttl_secs, 60);
    }

    #[test]
    fn test_real_perps_toml_imperial_section() {
        let config_path = Path::new("config/perps.toml");
        if !config_path.exists() {
            eprintln!("Skipping: config/perps.toml not found");
            return;
        }
        let config = Config::load(config_path).expect("real perps.toml should load");

        // Imperial section from perps.toml
        assert!(!config.imperial.enabled, "imperial should be disabled by default in perps.toml");
        assert_eq!(config.imperial.base_url, "https://api.imperial.space");
        assert_eq!(config.imperial.timeout_secs, 30);
        assert_eq!(config.imperial.cache_ttl_secs, 60);
    }

    // ── Route Cost Oracle config tests ─────────────────────────────────────

    #[test]
    fn test_route_oracle_config_defaults_when_absent() {
        let minimal = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0
"#;
        let f = write_temp_toml(minimal);
        let config = Config::load(f.path()).expect("config without [route-oracle] should parse");

        assert!(!config.route_oracle.enabled, "route oracle should be disabled by default");
        assert!((config.route_oracle.improvement_threshold_bps - 5.0).abs() < 0.001);
        assert!((config.route_oracle.edge_budget_pct - 100.0).abs() < 0.001);
        assert_eq!(config.route_oracle.staleness_threshold_secs, 60);
        assert_eq!(config.route_oracle.cache_ttl_secs, 60);
        assert!((config.route_oracle.cache_bucket_usd - 100.0).abs() < 0.001);
        assert!(config.route_oracle.excluded_venues.is_empty());
    }

    #[test]
    fn test_route_oracle_config_custom_values() {
        let custom = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0

[route-oracle]
enabled = true
improvement_threshold_bps = 10.0
edge_budget_pct = 50.0
staleness_threshold_secs = 120
cache_ttl_secs = 30
cache_bucket_usd = 200.0
excluded_venues = ["jupiter", "gmtrade"]
"#;
        let f = write_temp_toml(custom);
        let config = Config::load(f.path()).expect("config with [route-oracle] should parse");

        assert!(config.route_oracle.enabled);
        assert!((config.route_oracle.improvement_threshold_bps - 10.0).abs() < 0.001);
        assert!((config.route_oracle.edge_budget_pct - 50.0).abs() < 0.001);
        assert_eq!(config.route_oracle.staleness_threshold_secs, 120);
        assert_eq!(config.route_oracle.cache_ttl_secs, 30);
        assert!((config.route_oracle.cache_bucket_usd - 200.0).abs() < 0.001);
        assert_eq!(config.route_oracle.excluded_venues, vec!["jupiter", "gmtrade"]);
    }

    #[test]
    fn test_route_oracle_config_default_impl() {
        let default_config = RouteCostConfig::default();
        assert!(!default_config.enabled);
        assert!((default_config.improvement_threshold_bps - 5.0).abs() < 0.001);
        assert!((default_config.edge_budget_pct - 100.0).abs() < 0.001);
        assert_eq!(default_config.staleness_threshold_secs, 60);
        assert_eq!(default_config.cache_ttl_secs, 60);
        assert!((default_config.cache_bucket_usd - 100.0).abs() < 0.001);
        assert!(default_config.excluded_venues.is_empty());
    }

    #[test]
    fn test_route_oracle_config_partial_override() {
        let partial = r#"
[agent]
poll_interval_secs = 300
log_level = "info"

[flash]
api_url = "https://flashapi.trade"
rpc_url = "https://api.mainnet-beta.solana.com"
keypair_path = "~/.config/solana/id.json"
market = "SOL"
input_token = "USDC"
pool = "Crypto.1"
leverage = 3.0
slippage_pct = "0.5"

[strategy]
active = "momentum-scalper"

[risk]
max_position_notional_usd = 5000.0
max_daily_loss_usd = 500.0
max_drawdown_pct = 15.0

[route-oracle]
improvement_threshold_bps = 2.0
"#;
        let f = write_temp_toml(partial);
        let config = Config::load(f.path()).expect("partial route-oracle config should parse");

        // Overridden
        assert!((config.route_oracle.improvement_threshold_bps - 2.0).abs() < 0.001);
        // Defaults for the rest
        assert!(!config.route_oracle.enabled);
        assert!((config.route_oracle.edge_budget_pct - 100.0).abs() < 0.001);
        assert_eq!(config.route_oracle.staleness_threshold_secs, 60);
    }
}
