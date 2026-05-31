#!/usr/bin/env python3
"""
Pyramiding Analysis Script

Runs all 5 pyramid variants (none, reclaim, retest, profit-funded, ATR trail)
through a simulated replay pipeline using captured liquidation zone data.
Produces data/pyramiding-analysis.md with comparison tables and recommendation.

Usage:
    python3 scripts/pyramiding-analysis.py
    python3 scripts/pyramiding-analysis.py --output data/pyramiding-analysis.md
    python3 scripts/pyramiding-analysis.py --synthetic-only
    python3 scripts/pyramiding-analysis.py --capture-dir data/liquidation-zones
"""

import json
import math
import os
import random
import sys
import tempfile
import logging
from dataclasses import dataclass, field
from typing import List, Optional, Tuple, Dict
from pathlib import Path

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

# ─── Constants ────────────────────────────────────────────────────────

BASE_DIR = Path(__file__).resolve().parent.parent
DEFAULT_CAPTURE_DIR = BASE_DIR / "data" / "liquidation-zones"
DEFAULT_OUTPUT = BASE_DIR / "data" / "pyramiding-analysis.md"

FEE_RATE = 0.001  # 0.1% per side
ROUTE_COST_BPS = 3.0
STARTING_BALANCE = 1000.0
MAX_TRANCHES = 4
TARGET_SIZE_USD = 1000.0
TRANCHES_FRACTIONS = [0.25, 0.25, 0.25, 0.25]
ATR_MULTIPLIER = 2.0
STALE_THRESHOLD_SECS = 300.0

# ─── Data Types ───────────────────────────────────────────────────────


@dataclass
class PyramidTranche:
    """A single tranche in a pyramided position."""
    entry_price: float
    size_usd: float
    trigger_reason: str
    timestamp_ms: int
    tranche_index: int


@dataclass
class PyramidPosition:
    """A pyramided position tracking multiple tranches."""
    symbol: str
    is_long: bool
    tranches: List[PyramidTranche] = field(default_factory=list)
    combined_stop_price: float = 0.0
    target_size_usd: float = TARGET_SIZE_USD
    tranche_fractions: List[float] = field(default_factory=lambda: list(TRANCHES_FRACTIONS))
    max_tranches: int = MAX_TRANCHES
    atr_multiplier: float = ATR_MULTIPLIER
    max_risk_per_idea_usd: float = 2000.0
    max_correlated_exposure_usd: float = 50000.0

    def total_size_usd(self) -> float:
        return sum(t.size_usd for t in self.tranches)

    def avg_entry_price(self) -> float:
        if not self.tranches:
            return 0.0
        total_size = self.total_size_usd()
        if total_size == 0:
            return 0.0
        return sum(t.entry_price * t.size_usd for t in self.tranches) / total_size

    def unrealized_pnl(self, current_price: float) -> float:
        total_size = self.total_size_usd()
        avg = self.avg_entry_price()
        if avg == 0 or total_size == 0:
            return 0.0
        if self.is_long:
            return (current_price - avg) / avg * total_size
        else:
            return (avg - current_price) / avg * total_size

    def is_below_avg_entry(self, current_price: float) -> bool:
        avg = self.avg_entry_price()
        if avg == 0:
            return False
        if self.is_long:
            return current_price < avg
        else:
            return current_price > avg

    def tranche_count(self) -> int:
        return len(self.tranches)

    def tranche_size_for_index(self, index: int) -> float:
        if index < len(self.tranche_fractions):
            return self.target_size_usd * self.tranche_fractions[index]
        return 0.0

    def compute_combined_stop(self, current_atr: float) -> float:
        if not self.tranches:
            return 0.0
        avg = self.avg_entry_price()
        if self.is_long:
            if current_atr > 0:
                stop = avg - current_atr * self.atr_multiplier
            else:
                stop = avg * 0.95
            lowest = min(t.entry_price for t in self.tranches)
            return min(stop, lowest * 0.99)
        else:
            if current_atr > 0:
                stop = avg + current_atr * self.atr_multiplier
            else:
                stop = avg * 1.05
            highest = max(t.entry_price for t in self.tranches)
            return max(stop, highest * 1.01)

    def is_stop_hit(self, current_price: float) -> bool:
        if self.combined_stop_price == 0 or not self.tranches:
            return False
        if self.is_long:
            return current_price <= self.combined_stop_price
        else:
            return current_price >= self.combined_stop_price


@dataclass
class AddTrancheContext:
    """Context for evaluating whether a tranche can be added."""
    current_price: float
    timestamp_ms: int
    data_timestamp_ms: int
    reclaim_detected: bool = False
    higher_low_detected: bool = False
    retest_successful: bool = False
    current_atr: float = 0.0
    correlated_exposure_usd: float = 0.0


@dataclass
class Trade:
    """A completed trade."""
    symbol: str
    side: str
    entry_price: float
    exit_price: float
    size_usd: float
    gross_pnl: float
    entry_fee: float
    exit_fee: float
    route_cost_usd: float
    net_pnl: float
    hold_secs: float
    exit_reason: str
    entry_timestamp_ms: int
    exit_timestamp_ms: int
    tranche_count: int
    total_size_usd: float
    is_stopped_out: bool


@dataclass
class VariantResult:
    """Results for a single pyramid variant."""
    variant: str
    trades: List[Trade] = field(default_factory=list)
    final_balance: float = STARTING_BALANCE
    total_trades: int = 0
    win_count: int = 0
    loss_count: int = 0
    gross_pnl: float = 0.0
    total_fees: float = 0.0
    net_pnl: float = 0.0
    win_rate_pct: float = 0.0
    sharpe_ratio: float = 0.0
    sortino_ratio: float = 0.0
    calmar_ratio: float = 0.0
    max_drawdown_usd: float = 0.0
    max_drawdown_pct: float = 0.0
    net_expectancy: float = 0.0
    avg_hold_secs: float = 0.0
    avg_tranche_count: float = 0.0
    max_tranche_count: int = 0
    min_tranche_count: int = 0
    single_trade_dependency_flagged: bool = False
    stopped_out_count: int = 0
    tranche_distribution: Dict[int, int] = field(default_factory=dict)


# ─── Metrics Computation ──────────────────────────────────────────────


def compute_sharpe(returns: List[float]) -> float:
    """Compute annualized Sharpe ratio from trade returns."""
    if not returns:
        return 0.0
    n = len(returns)
    mean = sum(returns) / n
    variance = sum((r - mean) ** 2 for r in returns) / n
    std_dev = math.sqrt(variance)
    if std_dev < 1e-10:
        return 0.0
    trades_per_year = 252.0 * 5.0
    return (mean / std_dev) * math.sqrt(trades_per_year)


def compute_sortino(returns: List[float]) -> float:
    """Compute annualized Sortino ratio from trade returns."""
    if not returns:
        return 0.0
    n = len(returns)
    mean = sum(returns) / n
    downside = [min(r, 0.0) for r in returns]
    downside_var = sum(d ** 2 for d in downside) / n
    downside_dev = math.sqrt(downside_var)
    if downside_dev < 1e-10:
        return 0.0
    trades_per_year = 252.0 * 5.0
    return (mean / downside_dev) * math.sqrt(trades_per_year)


def compute_calmar(net_pnl: float, starting_balance: float, max_drawdown_usd: float, data_points: int) -> float:
    """Compute Calmar ratio: annualized_return / max_drawdown."""
    if starting_balance <= 0 or max_drawdown_usd <= 0 or data_points == 0:
        return 0.0
    total_return_pct = net_pnl / starting_balance
    annualization_factor = 1260.0 / data_points
    annualized_return = total_return_pct * annualization_factor
    max_dd_pct = max_drawdown_usd / starting_balance
    if max_dd_pct < 1e-10:
        return 0.0
    return annualized_return / max_dd_pct


def compute_net_expectancy(trades: List[Trade]) -> float:
    """Compute net expectancy: (win_rate * avg_win) - (loss_rate * avg_loss) - avg_route_cost."""
    if not trades:
        return 0.0
    wins = [t for t in trades if t.net_pnl > 0]
    losses = [t for t in trades if t.net_pnl <= 0]
    win_rate = len(wins) / len(trades)
    loss_rate = len(losses) / len(trades)
    avg_win = sum(t.net_pnl for t in wins) / len(wins) if wins else 0.0
    avg_loss = sum(t.net_pnl for t in losses) / len(losses) if losses else 0.0
    avg_route = sum(t.route_cost_usd for t in trades) / len(trades)
    return (win_rate * avg_win) - (loss_rate * abs(avg_loss)) - avg_route


# ─── Pyramid Variant Logic ────────────────────────────────────────────


def try_add_tranche(
    variant: str,
    position: PyramidPosition,
    ctx: AddTrancheContext,
) -> Tuple[Optional[PyramidTranche], str]:
    """
    Attempt to add a new tranche.
    Returns (Some(tranche), reason) on success or (None, reason) on rejection.
    Mirrors the Rust pyramiding.rs logic.
    """
    max_t = min(len(position.tranche_fractions), position.max_tranches)
    if position.tranche_count() >= max_t:
        return None, f"max {max_t} tranches reached"

    # Stale data check
    stale_threshold_ms = STALE_THRESHOLD_SECS * 1000
    if ctx.timestamp_ms - ctx.data_timestamp_ms > stale_threshold_ms:
        return None, "data is stale"

    # No adding to losers (except first tranche)
    if position.tranches:
        pnl = position.unrealized_pnl(ctx.current_price)
        if pnl <= 0:
            return None, f"no adding to losers: pnl={pnl:.2f}"

    # No adds below average entry
    if position.tranches and position.is_below_avg_entry(ctx.current_price):
        return None, f"price below avg entry"

    tranche_index = position.tranche_count()
    tranche_size = position.tranche_size_for_index(tranche_index)

    # Max risk check
    new_total = position.total_size_usd() + tranche_size
    if new_total > position.max_risk_per_idea_usd:
        return None, "exceeds max risk per idea"

    # Correlated exposure check
    total_after = ctx.correlated_exposure_usd + position.total_size_usd() + tranche_size
    if total_after > position.max_correlated_exposure_usd:
        return None, "exceeds correlated exposure"

    # Variant-specific logic
    trigger_reason = ""
    if variant == "none":
        if position.tranche_count() > 0:
            return None, "none variant: no pyramiding"
        trigger_reason = "probe"

    elif variant == "reclaim":
        if position.tranche_count() == 0:
            trigger_reason = "probe"
        else:
            if not ctx.reclaim_detected:
                return None, "reclaim: no reclaim detected"
            if not ctx.higher_low_detected:
                return None, "reclaim: no higher low"
            reasons = {1: "confirm", 2: "retest", 3: "final"}
            trigger_reason = reasons.get(tranche_index, f"tranche_{tranche_index}")

    elif variant == "retest":
        if position.tranche_count() == 0:
            trigger_reason = "probe"
        else:
            if not ctx.retest_successful:
                return None, "retest: no successful retest"
            reasons = {1: "confirm", 2: "retest", 3: "final"}
            trigger_reason = reasons.get(tranche_index, f"tranche_{tranche_index}")

    elif variant == "profit_funded":
        if position.tranche_count() == 0:
            trigger_reason = "probe"
        else:
            unrealized = position.unrealized_pnl(ctx.current_price)
            if unrealized <= 0:
                return None, f"profit_funded: no profit ({unrealized:.2f})"
            if tranche_size > unrealized:
                tranche_size = unrealized
            reasons = {1: "confirm_profit", 2: "retest_profit", 3: "final_profit"}
            trigger_reason = reasons.get(tranche_index, f"tranche_profit_{tranche_index}")

    elif variant == "atr_trail":
        if ctx.current_atr <= 0:
            return None, "atr_trail: ATR must be > 0"
        if position.tranche_count() == 0:
            trigger_reason = "probe_atr"
        else:
            # Compute ATR trail stop
            trail_stop = ctx.current_price - ctx.current_atr * ATR_MULTIPLIER
            if position.is_long and ctx.current_price <= trail_stop:
                return None, f"atr_trail: price below trail stop"
            if not position.is_long and ctx.current_price >= trail_stop:
                return None, f"atr_trail: price above trail stop"
            reasons = {1: "confirm_atr", 2: "retest_atr", 3: "final_atr"}
            trigger_reason = reasons.get(tranche_index, f"tranche_atr_{tranche_index}")
    else:
        return None, f"unknown variant: {variant}"

    tranche = PyramidTranche(
        entry_price=ctx.current_price,
        size_usd=tranche_size,
        trigger_reason=trigger_reason,
        timestamp_ms=ctx.timestamp_ms,
        tranche_index=tranche_index,
    )
    position.tranches.append(tranche)
    position.combined_stop_price = position.compute_combined_stop(ctx.current_atr)
    return tranche, trigger_reason


# ─── Synthetic Data Generation ────────────────────────────────────────


def generate_synthetic_price_series(
    base_price: float,
    n_points: int,
    volatility_pct: float = 1.5,
    drift_pct: float = 0.02,
    seed: int = 42,
) -> List[float]:
    """Generate a realistic price series using geometric Brownian motion."""
    rng = random.Random(seed)
    prices = [base_price]
    dt = 1.0 / 252.0
    for _ in range(n_points - 1):
        shock = rng.gauss(0, 1)
        ret = (drift_pct - 0.5 * volatility_pct ** 2) * dt + volatility_pct * math.sqrt(dt) * shock
        prices.append(prices[-1] * math.exp(ret))
    return prices


def generate_zone_scenarios(
    base_price: float,
    n_scenarios: int = 200,
    seed: int = 42,
) -> List[Dict]:
    """
    Generate synthetic trading scenarios with liquidation zone events.
    Each scenario has a price path around a liquidation zone event.
    Returns list of dicts with keys: prices, zones, is_long, atr_values
    """
    rng = random.Random(seed)
    scenarios = []

    for i in range(n_scenarios):
        # Alternate between long and short scenarios
        is_long = i % 2 == 0

        # Generate zone price near current price
        zone_distance_pct = rng.uniform(0.5, 5.0)  # 0.5% to 5% from price
        if is_long:
            zone_price = base_price * (1 - zone_distance_pct / 100.0)
        else:
            zone_price = base_price * (1 + zone_distance_pct / 100.0)

        # Generate price path: approach zone, sweep, then reverse or continue
        n_points = rng.randint(20, 80)

        # ATR values (roughly 1-3% of price)
        base_atr = base_price * rng.uniform(0.005, 0.02)

        prices = []
        atr_values = []

        # Phase 1: Approach zone (first 30% of points)
        approach_count = max(5, n_points // 3)
        for j in range(approach_count):
            frac = j / approach_count
            if is_long:
                p = base_price - (base_price - zone_price) * frac
                p += rng.gauss(0, base_atr * 0.3)
            else:
                p = base_price + (zone_price - base_price) * frac
                p += rng.gauss(0, base_atr * 0.3)
            prices.append(max(p, 1.0))
            atr_values.append(base_atr * rng.uniform(0.8, 1.2))

        # Phase 2: At/beyond zone (10% of points)
        sweep_count = max(3, n_points // 10)
        for j in range(sweep_count):
            if is_long:
                overshoot = zone_price * rng.uniform(-0.01, 0.005)
                p = zone_price + overshoot
            else:
                overshoot = zone_price * rng.uniform(-0.005, 0.01)
                p = zone_price + overshoot
            prices.append(max(p, 1.0))
            atr_values.append(base_atr * rng.uniform(0.9, 1.3))

        # Phase 3: After zone (remaining points) — determine outcome
        remaining = n_points - approach_count - sweep_count

        # 55% of scenarios are winners (zone respected → reversal)
        is_winner = rng.random() < 0.55

        for j in range(remaining):
            frac = j / max(remaining, 1)
            if is_winner:
                # Price reverses away from zone
                if is_long:
                    move_pct = rng.uniform(0.5, 3.0) * frac
                    p = zone_price * (1 + move_pct / 100.0)
                else:
                    move_pct = rng.uniform(0.5, 3.0) * frac
                    p = zone_price * (1 - move_pct / 100.0)
            else:
                # Price continues through zone
                if is_long:
                    move_pct = rng.uniform(0.3, 2.0) * frac
                    p = zone_price * (1 - move_pct / 100.0)
                else:
                    move_pct = rng.uniform(0.3, 2.0) * frac
                    p = zone_price * (1 + move_pct / 100.0)
            p += rng.gauss(0, base_atr * 0.2)
            prices.append(max(p, 1.0))
            atr_values.append(base_atr * rng.uniform(0.8, 1.2))

        scenarios.append({
            "prices": prices,
            "atr_values": atr_values,
            "is_long": is_long,
            "zone_price": zone_price,
            "is_winner": is_winner,
        })

    return scenarios


# ─── Load Captured Snapshots ──────────────────────────────────────────


def load_captured_snapshots(capture_dir: Path) -> List[Dict]:
    """Load all liquidation zone snapshots from the capture directory."""
    snapshots = []
    if not capture_dir.exists():
        logger.warning(f"Capture directory does not exist: {capture_dir}")
        return snapshots

    for path in sorted(capture_dir.glob("*.json")):
        try:
            with open(path) as f:
                snap = json.load(f)
            snapshots.append(snap)
        except Exception as e:
            logger.warning(f"Failed to parse {path}: {e}")

    snapshots.sort(key=lambda s: s.get("timestamp_ms", 0))
    logger.info(f"Loaded {len(snapshots)} snapshots from {capture_dir}")
    return snapshots


def get_base_prices_from_snapshots(snapshots: List[Dict]) -> Dict[str, float]:
    """Extract latest mark prices per symbol from snapshots."""
    prices = {}
    for snap in snapshots:
        symbol = snap.get("symbol", "")
        price = snap.get("mark_price", 0)
        if symbol and price > 0:
            prices[symbol] = price
    return prices


# ─── Run Single Variant ───────────────────────────────────────────────


def run_variant(
    variant: str,
    scenarios: List[Dict],
    symbol: str = "BTC",
) -> VariantResult:
    """
    Run all scenarios through a single pyramid variant.
    Returns aggregated metrics for the variant.
    """
    result = VariantResult(variant=variant)
    balance = STARTING_BALANCE
    peak_balance = STARTING_BALANCE
    max_drawdown_usd = 0.0
    trade_returns = []
    tranche_dist = {}

    for scenario in scenarios:
        prices = scenario["prices"]
        atr_values = scenario["atr_values"]
        is_long = scenario["is_long"]

        # Create position
        pos = PyramidPosition(
            symbol=symbol,
            is_long=is_long,
            target_size_usd=TARGET_SIZE_USD,
            tranche_fractions=list(TRANCHES_FRACTIONS),
            max_tranches=MAX_TRANCHES,
        )

        base_ts = 1_780_000_000_000
        interval_ms = 5000  # 5s between points

        # Phase tracking for variant conditions
        entered = False
        entry_price = 0.0
        entry_ts = 0
        entry_size = 0.0

        for idx, (price, atr) in enumerate(zip(prices, atr_values)):
            ts = base_ts + idx * interval_ms

            if not entered:
                # Try to enter at the zone sweep point (around 30% through the series)
                approach_count = len(prices) // 3
                if idx >= approach_count and idx <= approach_count + 3:
                    # Enter position
                    ctx = AddTrancheContext(
                        current_price=price,
                        timestamp_ms=ts,
                        data_timestamp_ms=ts,
                        current_atr=atr,
                    )
                    tranche, reason = try_add_tranche(variant, pos, ctx)
                    if tranche is not None:
                        entered = True
                        entry_price = pos.avg_entry_price()
                        entry_ts = ts
                        entry_size = pos.total_size_usd()
                continue

            # After entry, try to add tranches
            # Determine variant-specific conditions based on price behavior
            reclaim_detected = False
            higher_low_detected = False
            retest_successful = False

            if variant == "reclaim":
                # Reclaim: price pulls back then makes higher low
                if idx > len(prices) // 3 + 5:
                    pullback_idx = idx - 3
                    if pullback_idx >= 0:
                        recent_low = min(prices[max(0, idx - 5):idx + 1]) if is_long else max(prices[max(0, idx - 5):idx + 1])
                        prev_low = min(prices[max(0, idx - 10):idx - 4]) if is_long else max(prices[max(0, idx - 10):idx - 4])
                        if is_long and recent_low > prev_low:
                            higher_low_detected = True
                            reclaim_detected = True
                        elif not is_long and recent_low < prev_low:
                            higher_low_detected = True
                            reclaim_detected = True

            elif variant == "retest":
                # Retest: price returns to entry zone and holds
                if idx > len(prices) // 3 + 5:
                    # Price near entry level
                    price_near_entry = abs(price - entry_price) / entry_price < 0.005
                    if price_near_entry:
                        retest_successful = True

            elif variant == "profit_funded":
                # Profit-funded: automatically triggered when position is profitable
                pass  # Handled by pnl check in try_add_tranche

            elif variant == "atr_trail":
                # ATR trail: automatically triggered by ATR continuation
                pass  # Handled by ATR check in try_add_tranche

            ctx = AddTrancheContext(
                current_price=price,
                timestamp_ms=ts,
                data_timestamp_ms=ts,
                reclaim_detected=reclaim_detected,
                higher_low_detected=higher_low_detected,
                retest_successful=retest_successful,
                current_atr=atr,
            )
            try_add_tranche(variant, pos, ctx)

            # Check stop hit
            if pos.is_stop_hit(price):
                # Close position at stop
                break

            # Check take profit (1.5% gain) or time stop (1800s)
            hold_secs = (ts - entry_ts) / 1000.0
            pnl_pct = 0
            if is_long:
                pnl_pct = (price - entry_price) / entry_price * 100
            else:
                pnl_pct = (entry_price - price) / entry_price * 100

            if pnl_pct >= 1.5 or hold_secs >= 1800:
                break

        # Force close at last price
        if entered and pos.tranches:
            exit_price = prices[-1]
            exit_ts = base_ts + (len(prices) - 1) * interval_ms
            hold_secs = (exit_ts - entry_ts) / 1000.0

            total_size = pos.total_size_usd()
            avg_entry = pos.avg_entry_price()

            if is_long:
                gross_pnl = (exit_price - avg_entry) / avg_entry * total_size
            else:
                gross_pnl = (avg_entry - exit_price) / avg_entry * total_size

            entry_fee = total_size * FEE_RATE
            exit_fee = total_size * FEE_RATE
            route_cost = total_size * ROUTE_COST_BPS / 10000.0
            net_pnl = gross_pnl - entry_fee - exit_fee - route_cost

            is_stopped = pos.is_stop_hit(exit_price)

            exit_reason = "StopLoss" if is_stopped else "TakeProfit" if net_pnl > 0 else "TimeStop"
            if hold_secs >= 1800:
                exit_reason = "TimeStop"

            trade = Trade(
                symbol=symbol,
                side="long" if is_long else "short",
                entry_price=avg_entry,
                exit_price=exit_price,
                size_usd=total_size,
                gross_pnl=gross_pnl,
                entry_fee=entry_fee,
                exit_fee=exit_fee,
                route_cost_usd=route_cost,
                net_pnl=net_pnl,
                hold_secs=hold_secs,
                exit_reason=exit_reason,
                entry_timestamp_ms=entry_ts,
                exit_timestamp_ms=exit_ts,
                tranche_count=pos.tranche_count(),
                total_size_usd=total_size,
                is_stopped_out=is_stopped,
            )

            result.trades.append(trade)
            balance += net_pnl
            if balance > peak_balance:
                peak_balance = balance
            dd = peak_balance - balance
            if dd > max_drawdown_usd:
                max_drawdown_usd = dd
            trade_returns.append(net_pnl)

            tc = pos.tranche_count()
            tranche_dist[tc] = tranche_dist.get(tc, 0) + 1

    # Compute aggregate metrics
    result.final_balance = balance
    result.total_trades = len(result.trades)
    result.win_count = sum(1 for t in result.trades if t.net_pnl > 0)
    result.loss_count = sum(1 for t in result.trades if t.net_pnl <= 0)
    result.gross_pnl = sum(t.gross_pnl for t in result.trades)
    result.total_fees = sum(t.entry_fee + t.exit_fee + t.route_cost_usd for t in result.trades)
    result.net_pnl = result.gross_pnl - result.total_fees
    result.win_rate_pct = result.win_count / result.total_trades * 100 if result.total_trades > 0 else 0
    result.sharpe_ratio = compute_sharpe(trade_returns)
    result.sortino_ratio = compute_sortino(trade_returns)
    result.calmar_ratio = compute_calmar(result.net_pnl, STARTING_BALANCE, max_drawdown_usd, sum(len(s["prices"]) for s in scenarios))
    result.max_drawdown_usd = max_drawdown_usd
    result.max_drawdown_pct = max_drawdown_usd / STARTING_BALANCE * 100
    result.net_expectancy = compute_net_expectancy(result.trades)
    result.avg_hold_secs = sum(t.hold_secs for t in result.trades) / result.total_trades if result.total_trades > 0 else 0
    result.tranche_distribution = tranche_dist

    if result.trades:
        tranche_counts = [t.tranche_count for t in result.trades]
        result.avg_tranche_count = sum(tranche_counts) / len(tranche_counts)
        result.max_tranche_count = max(tranche_counts)
        result.min_tranche_count = min(tranche_counts)

    result.stopped_out_count = sum(1 for t in result.trades if t.is_stopped_out)

    # Single-trade dependency
    if result.trades and result.net_pnl > 0:
        max_single = max(t.net_pnl for t in result.trades if t.net_pnl > 0) if any(t.net_pnl > 0 for t in result.trades) else 0
        if max_single / result.net_pnl > 0.25:
            result.single_trade_dependency_flagged = True

    return result


# ─── Report Generation ─────────────────────────────────────────────────


def generate_report(results: List[VariantResult], n_scenarios: int, symbols_used: List[str]) -> str:
    """Generate the pyramiding-analysis.md report."""
    lines = []

    lines.append("# Pyramiding Analysis Report")
    lines.append("")
    lines.append("## Overview")
    lines.append("")
    lines.append("This report compares 5 pyramiding variants through a simulated replay pipeline:")
    lines.append("")
    lines.append("1. **None** — Single tranche only (baseline)")
    lines.append("2. **Reclaim** — Add tranches after price reclaims a level + higher low")
    lines.append("3. **Retest** — Add tranches after successful retest of support/resistance")
    lines.append("4. **Profit-funded** — Tranche size limited to unrealized profit")
    lines.append("5. **ATR Trail** — Add when ATR trail confirms continuation")
    lines.append("")
    lines.append(f"- **Scenarios:** {n_scenarios}")
    lines.append(f"- **Symbols:** {', '.join(symbols_used)}")
    lines.append(f"- **Starting Balance:** ${STARTING_BALANCE:.2f}")
    lines.append(f"- **Target Position Size:** ${TARGET_SIZE_USD:.2f}")
    lines.append(f"- **Default Sizing:** {' / '.join(f'{f*100:.0f}%' for f in TRANCHES_FRACTIONS)}")
    lines.append(f"- **Max Tranches:** {MAX_TRANCHES}")
    lines.append(f"- **Fee Rate:** {FEE_RATE*100:.1f}% per side")
    lines.append(f"- **Route Cost:** {ROUTE_COST_BPS:.1f} bps")
    lines.append("")

    # ─── Variant Comparison Table ────────────────────────────────

    lines.append("## Variant Comparison Table")
    lines.append("")
    lines.append("| Metric | None (baseline) | Reclaim | Retest | Profit-Funded | ATR Trail |")
    lines.append("|--------|----------------|---------|--------|---------------|-----------|")

    # Build metric rows
    metrics = [
        ("Total Trades", lambda r: f"{r.total_trades}"),
        ("Win Count", lambda r: f"{r.win_count}"),
        ("Loss Count", lambda r: f"{r.loss_count}"),
        ("Win Rate", lambda r: f"{r.win_rate_pct:.1f}%"),
        ("Gross PnL", lambda r: f"${r.gross_pnl:.2f}"),
        ("Total Fees", lambda r: f"${r.total_fees:.2f}"),
        ("Net PnL", lambda r: f"${r.net_pnl:.2f}"),
        ("Sharpe Ratio", lambda r: f"{r.sharpe_ratio:.4f}"),
        ("Sortino Ratio", lambda r: f"{r.sortino_ratio:.4f}"),
        ("Calmar Ratio", lambda r: f"{r.calmar_ratio:.4f}"),
        ("Max Drawdown", lambda r: f"${r.max_drawdown_usd:.2f} ({r.max_drawdown_pct:.1f}%)"),
        ("Net Expectancy", lambda r: f"${r.net_expectancy:.4f}"),
        ("Avg Hold (s)", lambda r: f"{r.avg_hold_secs:.0f}"),
        ("Stopped Out", lambda r: f"{r.stopped_out_count}"),
        ("Single-Trade Dep.", lambda r: "⚠️ FLAGGED" if r.single_trade_dependency_flagged else "✅ OK"),
    ]

    for name, getter in metrics:
        cells = [getter(r) for r in results]
        lines.append(f"| {name} | {' | '.join(cells)} |")

    lines.append("")

    # ─── Per-Variant Detailed Metrics ────────────────────────────

    lines.append("## Per-Variant Metrics")
    lines.append("")

    for r in results:
        lines.append(f"### {r.variant.capitalize()}")
        lines.append("")
        lines.append(f"- **Trades:** {r.total_trades} ({r.win_count}W / {r.loss_count}L)")
        lines.append(f"- **Win Rate:** {r.win_rate_pct:.1f}%")
        lines.append(f"- **Sharpe Ratio:** {r.sharpe_ratio:.4f}")
        lines.append(f"- **Sortino Ratio:** {r.sortino_ratio:.4f}")
        lines.append(f"- **Calmar Ratio:** {r.calmar_ratio:.4f}")
        lines.append(f"- **Max Drawdown:** ${r.max_drawdown_usd:.2f} ({r.max_drawdown_pct:.1f}%)")
        lines.append(f"- **Net Expectancy:** ${r.net_expectancy:.4f}")
        lines.append(f"- **Net PnL:** ${r.net_pnl:.2f}")
        lines.append(f"- **Gross PnL:** ${r.gross_pnl:.2f}")
        lines.append(f"- **Total Fees:** ${r.total_fees:.2f}")
        lines.append(f"- **Fee/Gross:** {(r.total_fees / abs(r.gross_pnl) * 100) if abs(r.gross_pnl) > 0.01 else 0:.1f}%")
        lines.append(f"- **Avg Tranche Count:** {r.avg_tranche_count:.2f}")
        lines.append(f"- **Tranche Range:** {r.min_tranche_count} - {r.max_tranche_count}")
        lines.append(f"- **Avg Hold Time:** {r.avg_hold_secs:.0f}s")
        lines.append(f"- **Stopped Out:** {r.stopped_out_count} trades")
        lines.append(f"- **Single-Trade Dependency:** {'⚠️ FLAGGED' if r.single_trade_dependency_flagged else '✅ OK'}")
        lines.append("")

    # ─── Tranche Distribution ────────────────────────────────────

    lines.append("## Tranche Distribution Analysis")
    lines.append("")
    lines.append("Distribution of final tranche counts across all trades per variant:")
    lines.append("")

    all_tranche_counts = set()
    for r in results:
        all_tranche_counts.update(r.tranche_distribution.keys())
    all_tranche_counts = sorted(all_tranche_counts)

    header = "| Tranches | " + " | ".join(r.variant.capitalize() for r in results) + " |"
    separator = "|----------|" + "|".join(["--------" for _ in results]) + "|"
    lines.append(header)
    lines.append(separator)

    for tc in all_tranche_counts:
        cells = []
        for r in results:
            count = r.tranche_distribution.get(tc, 0)
            pct = count / r.total_trades * 100 if r.total_trades > 0 else 0
            cells.append(f"{count} ({pct:.0f}%)")
        lines.append(f"| {tc} | " + " | ".join(cells) + " |")

    lines.append("")

    # Tranche size breakdown
    lines.append("### Tranche Size Allocation")
    lines.append("")
    lines.append("Default allocation per tranche:")
    lines.append("")
    for i, frac in enumerate(TRANCHES_FRACTIONS):
        lines.append(f"- **Tranche {i}** ({['Probe', 'Confirm', 'Retest', 'Final'][i]}): {frac*100:.0f}% = ${TARGET_SIZE_USD * frac:.2f}")
    lines.append("")
    lines.append("Note: Profit-funded variant caps tranche size to unrealized PnL, so actual sizes vary.")
    lines.append("")

    # ─── Expectancy Impact Analysis ──────────────────────────────

    lines.append("## Does Pyramiding Improve Expectancy?")
    lines.append("")

    baseline = results[0]  # "none" variant
    pyramid_variants = results[1:]  # All pyramid variants

    # Compare each pyramid variant against baseline
    improvements = []
    for r in pyramid_variants:
        expectancy_delta = r.net_expectancy - baseline.net_expectancy
        pnl_delta = r.net_pnl - baseline.net_pnl
        sharpe_delta = r.sharpe_ratio - baseline.sharpe_ratio
        sortino_delta = r.sortino_ratio - baseline.sortino_ratio
        drawdown_delta = r.max_drawdown_usd - baseline.max_drawdown_usd

        improves_expectancy = expectancy_delta > 0
        improves_risk_adj = sharpe_delta > 0

        improvements.append({
            "variant": r.variant,
            "expectancy_delta": expectancy_delta,
            "pnl_delta": pnl_delta,
            "sharpe_delta": sharpe_delta,
            "sortino_delta": sortino_delta,
            "drawdown_delta": drawdown_delta,
            "improves_expectancy": improves_expectancy,
            "improves_risk_adj": improves_risk_adj,
        })

    lines.append("### Expectancy vs Baseline (None)")
    lines.append("")
    lines.append("| Variant | Δ Expectancy | Δ Net PnL | Δ Sharpe | Δ Sortino | Δ Drawdown | Improves? |")
    lines.append("|---------|-------------|-----------|----------|-----------|------------|-----------|")

    for imp in improvements:
        verdict = "✅ Yes" if imp["improves_expectancy"] else "❌ No"
        lines.append(
            f"| {imp['variant'].capitalize()} | "
            f"${imp['expectancy_delta']:+.4f} | "
            f"${imp['pnl_delta']:+.2f} | "
            f"{imp['sharpe_delta']:+.4f} | "
            f"{imp['sortino_delta']:+.4f} | "
            f"${imp['drawdown_delta']:+.2f} | "
            f"{verdict} |"
        )

    lines.append("")

    # ─── Variance Analysis ───────────────────────────────────────

    lines.append("## Variance Impact Analysis")
    lines.append("")

    for r in results:
        if r.trades:
            returns = [t.net_pnl for t in r.trades]
            mean_ret = sum(returns) / len(returns)
            variance = sum((ret - mean_ret) ** 2 for ret in returns) / len(returns)
            std_dev = math.sqrt(variance)
            lines.append(f"- **{r.variant.capitalize()}:** mean=${mean_ret:.4f}, std=${std_dev:.4f}, "
                         f"CV={std_dev / abs(mean_ret) if abs(mean_ret) > 1e-10 else float('inf'):.2f}")
        else:
            lines.append(f"- **{r.variant.capitalize()}:** no trades")

    lines.append("")

    # ─── Recommendation ──────────────────────────────────────────

    lines.append("## Recommendation")
    lines.append("")

    # Determine best variant
    best_expectancy = max(improvements, key=lambda x: x["expectancy_delta"])
    best_sharpe = max(improvements, key=lambda x: x["sharpe_delta"])
    best_sortino = max(improvements, key=lambda x: x["sortino_delta"])
    any_improves = any(imp["improves_expectancy"] for imp in improvements)
    all_harm = all(not imp["improves_expectancy"] for imp in improvements)

    if all_harm:
        lines.append("**Verdict: Pyramiding magnifies variance without improving expectancy.**")
        lines.append("")
        lines.append("In this analysis, none of the pyramiding variants improved net expectancy over")
        lines.append("the single-tranche baseline. This is consistent with the fee-dominated environment")
        lines.append("where additional tranches increase fee drag without proportional gain capture.")
        lines.append("Pyramiding increases position size during favorable moves, but the additional")
        lines.append("fee cost and execution risk offset the marginal benefit.")
    elif any_improves:
        lines.append("**Verdict: Pyramiding shows mixed results — some variants improve expectancy.**")
        lines.append("")
        lines.append(f"- Best variant by expectancy: **{best_expectancy['variant'].capitalize()}** "
                     f"(Δ ${best_expectancy['expectancy_delta']:+.4f})")
        lines.append(f"- Best variant by Sharpe: **{best_sharpe['variant'].capitalize()}** "
                     f"(Δ {best_sharpe['sharpe_delta']:+.4f})")
        lines.append(f"- Best variant by Sortino: **{best_sortino['variant'].capitalize()}** "
                     f"(Δ {best_sortino['sortino_delta']:+.4f})")
        lines.append("")

        # Check if best variant is safe enough for paper trading
        best_r = next(r for r in results if r.variant == best_expectancy["variant"])
        if best_r.max_drawdown_pct > 10:
            lines.append(f"⚠️ **Warning:** {best_expectancy['variant'].capitalize()} has "
                         f"max drawdown of {best_r.max_drawdown_pct:.1f}% (threshold: 10%). "
                         f"Cannot recommend for paper trading despite positive expectancy delta.")
        else:
            lines.append(f"**Recommendation:** Use **{best_expectancy['variant'].capitalize()}** "
                         f"pyramiding variant for paper trading validation. "
                         f"Monitor closely for variance amplification in live conditions.")
    else:
        lines.append("**Verdict: Pyramiding results are neutral.**")
        lines.append("")
        lines.append("No variant showed meaningful improvement or degradation. "
                     "This may indicate insufficient signal diversity in the test scenarios. "
                     "Continue capture and re-evaluate with more data.")

    lines.append("")
    lines.append("### Key Findings")
    lines.append("")

    # General findings about pyramiding mechanics
    for r in results:
        if r.trades:
            avg_tc = r.avg_tranche_count
            lines.append(f"- **{r.variant.capitalize()}:** Average {avg_tc:.1f} tranches per trade, "
                         f"{r.stopped_out_count} stopped out, "
                         f"${r.total_fees:.2f} total fees")
        else:
            lines.append(f"- **{r.variant.capitalize()}:** No trades generated")

    lines.append("")
    lines.append("### Caveats")
    lines.append("")
    lines.append("1. Scenarios are synthetic — real liquidation zone dynamics may differ")
    lines.append("2. Fee rates are fixed at 0.1% per side — actual rates vary by venue")
    lines.append("3. Slippage is not modeled — multi-tranche exits may face worse execution")
    lines.append("4. The replay pipeline uses the Rust `pyramiding.rs` logic for production validation")
    lines.append("5. This report serves as a pre-validation analysis before the full Rust replay pipeline")
    lines.append("")

    # ─── Metadata ────────────────────────────────────────────────

    lines.append("## Metadata")
    lines.append("")
    lines.append(f"- **Generated:** {__import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat()}")
    lines.append(f"- **Script:** scripts/pyramiding-analysis.py")
    lines.append(f"- **Pyramiding Module:** src/pyramiding.rs")
    lines.append(f"- **Replay Module:** src/replay.rs")
    lines.append(f"- **Scenarios:** {n_scenarios}")
    lines.append(f"- **Variants:** none, reclaim, retest, profit_funded, atr_trail")
    lines.append("")

    return "\n".join(lines)


# ─── Atomic Write ──────────────────────────────────────────────────────


def atomic_write(path: Path, content: str) -> None:
    """Write content to file atomically (write .tmp then rename)."""
    tmp_path = path.with_suffix(".tmp")
    with open(tmp_path, "w") as f:
        f.write(content)
    os.rename(tmp_path, path)
    logger.info(f"Report written to {path}")


# ─── Main ──────────────────────────────────────────────────────────────


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Pyramiding Analysis")
    parser.add_argument("--output", type=str, default=str(DEFAULT_OUTPUT), help="Output report path")
    parser.add_argument("--capture-dir", type=str, default=str(DEFAULT_CAPTURE_DIR), help="Capture data directory")
    parser.add_argument("--synthetic-only", action="store_true", help="Use only synthetic data")
    parser.add_argument("--n-scenarios", type=int, default=200, help="Number of synthetic scenarios")
    parser.add_argument("--seed", type=int, default=42, help="Random seed")
    args = parser.parse_args()

    output_path = Path(args.output)
    capture_dir = Path(args.capture_dir)

    # Load captured data for price context
    symbols_used = ["BTC", "ETH", "SOL"]
    base_prices = {
        "BTC": 74000.0,
        "ETH": 2500.0,
        "SOL": 170.0,
    }

    if not args.synthetic_only:
        snapshots = load_captured_snapshots(capture_dir)
        if snapshots:
            captured_prices = get_base_prices_from_snapshots(snapshots)
            base_prices.update(captured_prices)
            logger.info(f"Using captured prices: {base_prices}")

    # Generate scenarios for each symbol
    all_scenarios = []
    for symbol, base_price in base_prices.items():
        scenarios = generate_zone_scenarios(base_price, n_scenarios=args.n_scenarios, seed=args.seed)
        all_scenarios.extend(scenarios)

    logger.info(f"Running {len(all_scenarios)} scenarios across {len(base_prices)} symbols")

    # Run each variant
    variants = ["none", "reclaim", "retest", "profit_funded", "atr_trail"]
    results = []
    for variant in variants:
        logger.info(f"Running variant: {variant}")
        result = run_variant(variant, all_scenarios, symbol="multi")
        results.append(result)
        logger.info(f"  {variant}: {result.total_trades} trades, "
                     f"net_pnl=${result.net_pnl:.2f}, "
                     f"sharpe={result.sharpe_ratio:.4f}")

    # Generate report
    report = generate_report(results, len(all_scenarios), list(base_prices.keys()))

    # Write report
    output_path.parent.mkdir(parents=True, exist_ok=True)
    atomic_write(output_path, report)

    logger.info(f"Report generated: {output_path}")

    # Also write JSON data for programmatic consumption
    json_path = output_path.with_suffix(".json")
    json_data = {
        "variants": {
            r.variant: {
                "total_trades": r.total_trades,
                "win_count": r.win_count,
                "loss_count": r.loss_count,
                "win_rate_pct": round(r.win_rate_pct, 2),
                "gross_pnl": round(r.gross_pnl, 4),
                "total_fees": round(r.total_fees, 4),
                "net_pnl": round(r.net_pnl, 4),
                "sharpe_ratio": round(r.sharpe_ratio, 6),
                "sortino_ratio": round(r.sortino_ratio, 6),
                "calmar_ratio": round(r.calmar_ratio, 6),
                "max_drawdown_usd": round(r.max_drawdown_usd, 4),
                "max_drawdown_pct": round(r.max_drawdown_pct, 2),
                "net_expectancy": round(r.net_expectancy, 6),
                "avg_tranche_count": round(r.avg_tranche_count, 2),
                "max_tranche_count": r.max_tranche_count,
                "min_tranche_count": r.min_tranche_count,
                "stopped_out_count": r.stopped_out_count,
                "single_trade_dependency_flagged": r.single_trade_dependency_flagged,
                "tranche_distribution": {str(k): v for k, v in r.tranche_distribution.items()},
                "avg_hold_secs": round(r.avg_hold_secs, 1),
            }
            for r in results
        },
        "metadata": {
            "n_scenarios": len(all_scenarios),
            "symbols": list(base_prices.keys()),
            "base_prices": {k: round(v, 2) for k, v in base_prices.items()},
        },
    }
    atomic_write(json_path, json.dumps(json_data, indent=2))
    logger.info(f"JSON data written to {json_path}")


if __name__ == "__main__":
    main()
