# M5 Integration Gate Evaluation — External Tool Research

**Date:** 2026-05-30  
**Context:** Zekt is a Rust binary for Solana perps trading (Flash Trade execution) with Python analysis pipeline. Paper/backtest only — no live execution yet. Uses reqwest, tokio, own strategy framework with regime detection (M4), risk management (M3), fee model (M2).

---

## 1. second-state/fintool

**Repo:** https://github.com/second-state/fintool  
**Stars:** 295 | **Forks:** 11 | **Language:** Rust 75%, Shell 25%  
**License:** MIT  
**Latest:** v0.1.6 (Mar 15, 2026) | 131 commits | 2 contributors (juntao + claude)

### What it is
A suite of Rust CLI tools for agentic trading across Hyperliquid, Binance, Coinbase, OKX, and Polymarket. Each exchange gets its own binary (`hyperliquid`, `binance`, etc.) plus a shared `fintool` for market intelligence (quotes, news, SEC filings) and a `backtest` binary for historical simulation with forward PnL analysis. All CLIs support `--json` mode for scripting. Designed as OpenClaw skill for AI agent integration.

### Integration Gate

#### (a) What bottleneck does this solve for Zekt?

**Marginal.** Zekt already has:
- Its own `flash_api.rs` REST client for Flash Trade prices/positions/tx builder
- Its own `hl_info.rs` for Hyperliquid Info API (positions, funding, fills, candles)
- Its own `backtest.rs` with candle replay engine

Fintool's **unique value** would be:
- Multi-exchange support (Binance, Coinbase, OKX) — but Zekt only trades on Flash Trade (Solana)
- LLM-enriched quote analysis via OpenAI — Zekt doesn't use LLMs in its trading loop
- SEC filings / news integration — irrelevant for crypto perps
- Polymarket prediction markets — outside Zekt's scope

The one area of overlap is the Hyperliquid binary, which uses `hyperliquid_rust_sdk` (EIP-712 signing). Zekt already has a working HL client via `hl_info.rs` using reqwest directly. Adding fintool would introduce an `ethers` dependency for a use case Zekt already covers.

#### (b) Is it testable in isolation?

**Yes, partially.** Fintool is a CLI tool — each binary can be tested independently via shell scripts (which the repo already does in `tests/`). The `--json` mode makes it scriptable. However:
- No Rust unit tests visible — testing is shell-based E2E only
- No `#[cfg(test)]` modules in the source
- The `backtest` binary uses Yahoo Finance / CoinGecko (public APIs, no auth needed) — good for isolated testing
- Live trading commands require funded wallets and API keys

#### (c) Can it be rolled back?

**Yes, cleanly.** Fintool is an external CLI binary, not a library dependency. It would be invoked as a subprocess or via `--json` mode. Removing it is just deleting the binary. No code changes to Zekt's Rust crate needed.

#### (d) What are the failure modes?

- **Network:** Depends on exchange APIs (Hyperliquid, Binance, etc.) and data sources (Yahoo Finance, CoinGecko, OpenAI). All subject to rate limits and downtime.
- **Dependency surface:** Heavy — pulls in `ethers`, `alloy`, `hyperliquid_rust_sdk`, `polymarket-client-sdk`. These are not Zekt's current dependencies.
- **Key management:** Requires wallet private keys in `~/.fintool/config.toml` — a security surface Zekt avoids by using Solana keypairs via `solana-sdk`.
- **Maintenance risk:** 2 contributors, largely AI-generated code. Last commit March 2026 — could be abandoned.
- **Version drift:** No semver guarantees. v0.1.6 could break CLI args in v0.2.0.

#### (e) Is it worth the dependency?

**No.** The overlap with Zekt's existing infrastructure is ~80%. The unique capabilities (multi-exchange, LLM quotes, SEC filings, Polymarket) are outside Zekt's scope. Adding a 7-binary CLI suite to gain marginally better HL access would increase attack surface, dependency count, and operational complexity without solving any actual bottleneck.

### Verdict: **REJECT**

Zekt already has better-integrated Rust implementations for the parts it needs (Flash Trade API, HL API, backtesting). Fintool's multi-exchange scope doesn't align with Zekt's single-execute-target (Flash Trade) pipeline. The LLM-enriched quotes and SEC filing features are irrelevant for crypto perps. No bottleneck is solved that isn't already handled.

---

## 2. Senpi-ai/senpi-skills

**Repo:** https://github.com/Senpi-ai/senpi-skills  
**Stars:** 85 | **Forks:** 24 | **Language:** Python 100%  
**License:** MIT  
**Latest:** Active (1,208 commits, last commit May 29, 2026) | 11 contributors

### What it is
44+ agent skills for autonomous crypto trading on Hyperliquid. Organized into 16 producer archetypes across capabilities (trailing stops, market scanning, position management, fee optimization, regime classification). Requires the Senpi trading runtime (OpenClaw agent host) and Senpi MCP access token. Each skill is a self-contained directory with a Python producer, `runtime.yaml` config, and `SKILL.md` instructions.

**Key archetypes relevant to Zekt:**
- **Archetype 16 (Coyote):** Regime classifier — watches BTC trend + realized vol + cross-asset dispersion → TREND_UP / TREND_DOWN / CHOP
- **Archetype 15 (Lynx):** Self-tuning adaptive threshold — pulls own trade history, auto-raises MIN_SCORE when bottom buckets bleed
- **Archetype 7 (Vulture):** Momentum scalper with dynamic stop-loss and margin_pct sizing
- **DSL (Dynamic Stop Loss):** Configurable trailing stop with retrace, max_loss, hard_timeout
- **Fee optimizer:** Maker-preferred execution via FEE_OPTIMIZED_LIMIT

### Integration Gate

#### (a) What bottleneck does this solve for Zekt?

**Conceptual inspiration, not direct integration.** Senpi's strategy designs could inform Zekt's own strategy implementations:

1. **Coyote regime classifier** — Zekt already has M4 regime-aware entry filtering. Coyote's 3-regime approach (TREND_UP / TREND_DOWN / CHOP with vol-confirmation) is simpler than Zekt's multi-factor regime detector but the *pattern* of vol-confirmation on downside (crash = drop + vol spike, not slow grind) is worth studying.

2. **Lynx self-tuning** — Adaptive MIN_SCORE based on own trade history is a novel pattern Zekt doesn't have. Could inspire a Rust-based self-tuning mechanism.

3. **DSL (Dynamic Stop Loss)** — Zekt already has trailing stop, TP/SL, and time stop in `risk.rs`. Senpi's retrace-based DSL with staleness caps is a different approach worth comparing.

4. **Fee optimizer (FEE_OPTIMIZED_LIMIT)** — Maker-preferred execution. Zekt's fee model (M2) already tracks fees, but a maker-preference execution mode could reduce costs.

However, Senpi **executes on Hyperliquid only**, while Zekt executes on Flash Trade (Solana). The entire execution layer is incompatible.

#### (b) Is it testable in isolation?

**No, not easily.** Senpi skills require:
- OpenClaw agent host (Linux, Python 3.8+)
- Senpi MCP access token (proprietary service)
- Funded Hyperliquid wallet per strategy
- `senpi-trading-runtime` plugin + `senpi_runtime_helpers` SDK

The skills themselves are Python producers that call Senpi's MCP tools. They can't run standalone without the Senpi platform. Testing Coyote's regime classifier would require either extracting the pure Python logic or standing up the full Senpi runtime.

The repo does include tests (e.g., Coyote has 14 tests, Lynx has 19 tests), but they test the producer logic in isolation, not the end-to-end execution flow.

#### (c) Can it be rolled back?

**N/A for direct integration.** Since Senpi skills are Python producers for a different platform (Hyperliquid via OpenClaw), they wouldn't be integrated into Zekt's Rust codebase. If Zekt were to port concepts from Senpi, the port would be native Rust and fully reversible via git.

#### (d) What are the failure modes?

- **Platform lock-in:** Skills depend on Senpi MCP + OpenClaw runtime. Vendor-specific.
- **Hyperliquid-only execution:** No Flash Trade / Solana support. Completely different execution layer.
- **Python-only:** Zekt is Rust. Porting required.
- **Operational complexity:** 44 skills, 16 archetypes, fleet management, wallet isolation per strategy. Heavy operational burden.
- **API dependency:** Requires `SENPI_AUTH_TOKEN` — proprietary API that could change or be deprecated.

#### (e) Is it worth the dependency?

**As a reference, yes. As a dependency, no.** Senpi's strategy designs are well-documented with clear thesis explanations, test coverage, and operational patterns. The regime classification (Coyote), self-tuning (Lynx), and fee optimization patterns are worth studying for Zekt's own implementations. But the execution layer incompatibility (Hyperliquid vs Flash Trade) and platform dependency (OpenClaw + Senpi MCP) make direct integration impossible.

**Recommended approach:** Study the Senpi producer patterns and port useful concepts (regime classification, self-tuning thresholds, maker-preferred execution) into Zekt's Rust strategy framework. No code dependency.

### Verdict: **DEFER** (study concepts, don't integrate)

Senpi-skills is the most relevant of the three repos to Zekt's problem domain. The strategy archetypes, regime classification, self-tuning, and fee optimization patterns are directly applicable. But the entire execution layer targets Hyperliquid (not Flash Trade), requires a proprietary runtime (OpenClaw + Senpi MCP), and is Python-only. The value is in the *design patterns*, not the code. Extract concepts, port to Rust, integrate into Zekt's existing strategy trait.

**Specific things worth porting:**
- Coyote's vol-confirmed regime classification (3-regime with crash = drop + vol spike heuristic)
- Lynx's self-tuning MIN_SCORE from own trade history (adaptive threshold based on ROE by score bucket)
- FEE_OPTIMIZED_LIMIT maker-preference pattern
- margin_pct budget-relative sizing (Zekt's sizing is fixed notional)

---

## 3. chrisworsey55/atlas-gic

**Repo:** https://github.com/chrisworsey55/atlas-gic  
**Stars:** 1.9k | **Forks:** 350 | **Language:** Python 100%  
**License:** Custom (proprietary core, architecture-only open source)  
**Latest:** 7 commits, last May 27, 2026 | 2 contributors (chrisworsey55 + claude)

### What it is
ATLAS is a multi-agent AI trading system that uses Karpathy-style autoresearch to self-improve prompts through market feedback. 25+ agents debate markets daily across 4 layers (Macro → Sector Desks → Superinvestors → Decision). The worst-performing agent gets its prompt rewritten; if performance improves, the git commit survives; otherwise, git revert. Running live with real capital.

**Key concepts:**
- **Autoresearch loop:** Sharpe ratio is the loss function. Agent prompts are the weights. No GPU needed.
- **Darwinian weights:** Top quartile agents get louder (+5%/day), bottom quartile get quieter (-5%/day).
- **Agent spawning:** System autonomously creates new specialist agents when it detects knowledge gaps.
- **PRISM (All Seasons):** Separate cohorts trained on distinct market regimes (bull, crisis, rate tightening, recovery, euphoria).
- **JANUS meta-layer:** Algorithmically weights cohorts by recent accuracy → emergent regime detector.
- **Soros reflexivity engine:** 5 feedback loops modelling how prices change fundamentals.
- **MiroFish swarm simulation:** Trains agents on simulated futures.

### Integration Gate

#### (a) What bottleneck does this solve for Zekt?

**Conceptual only — meta-learning methodology.** ATLAS addresses a problem Zekt doesn't have yet: *how to automatically improve strategy parameters over time*. Zekt's strategies have fixed parameters from blueprint JSON. ATLAS shows how to:
- Use performance feedback to evolve strategy parameters (Zekt: could auto-tune strategy thresholds)
- Detect knowledge gaps and create new strategy variants (Zekt: could auto-generate new strategy configs)
- Weight multiple strategies by recent performance (Zekt: could weight the 5 strategies dynamically instead of running them equally)

However, ATLAS targets **equities** (stocks via Alpaca, prediction markets via Kalshi), not crypto perps. And its "trained prompts are proprietary" — the actual valuable part (the evolved agent prompts) is not in the repo.

#### (b) Is it testable in isolation?

**No.** The repo contains architecture documentation, result files, placeholder prompts, and Python skeleton code. The actual implementation is proprietary:
- Trained agent prompts are NOT included
- API integration code is NOT included  
- Position management logic is NOT included
- Risk management rules are NOT included
- Deployment configuration is NOT included

The `src/` directory contains `janus.py` and a `mirofish/` directory, but the core modules (`agents/backtest_loop.py`, `agents/eod_cycle.py`, etc.) are described in README only — not present in the repo. This is essentially a **paid SaaS product** (atlasagents.co, $49-$499/month) with an open-source architecture doc.

#### (c) Can it be rolled back?

**N/A — nothing to integrate.** Since the core implementation is proprietary and not in the repo, there's nothing to integrate into Zekt. If Zekt were to implement its own autoresearch loop inspired by ATLAS, that would be original Rust code fully under Zekt's control.

#### (d) What are the failure modes?

- **Vaporware risk:** 7 commits total. The repo is primarily a marketing page for atlasagents.co. The "source code" is architecture docs + placeholder prompts.
- **LLM dependency:** Every decision requires Claude Sonnet API calls. Cost: ~$50-80 per 18-month backtest. Ongoing API costs for live trading.
- **Equity-focused:** Targets stocks (Alpaca), prediction markets (Kalshi). No crypto perps support.
- **Over-engineering:** 25 agents across 4 layers for daily EOD decisions. Zekt operates on 5m-1h candles with sub-second signal detection. Latency mismatch.
- **Proprietary core:** The trained prompts (the actual value) are not open source. What's open is the *methodology*, which is well-documented in the README.

#### (e) Is it worth the dependency?

**No.** There's no dependency to adopt — the actual code is proprietary. The value is entirely in the *ideas*: autoresearch loops, Darwinian weighting, PRISM regime-specific training, agent spawning. These are design patterns that could inspire Zekt's own meta-learning layer, but they'd need to be implemented from scratch in Rust.

### Verdict: **REJECT**

ATLAS is a well-marketed architecture document for a paid SaaS product, not an integrable open-source tool. The repo has 7 commits, no testable code, and the core implementation (trained prompts, execution logic, risk management) is explicitly proprietary. The autoresearch concept is interesting but over-engineered for Zekt's use case (Zekt runs 5 strategies on crypto perps, not 25 agents on equities). The LLM-in-the-loop approach (Claude Sonnet for every decision) is fundamentally incompatible with Zekt's sub-second signal detection requirement.

**One concept worth noting:** The JANUS meta-layer's "emergent regime detection from cohort weight differentials" is clever and could inform a future Zekt meta-strategy that weights its 5 strategies based on recent per-regime performance. But this is a design pattern to implement natively, not code to import.

---

## Summary Table

| Repo | Verdict | Value for Zekt | Integration Effort | Risk |
|------|---------|----------------|-------------------|------|
| **second-state/fintool** | **REJECT** | Low — 80% overlap with existing infra | Low (external CLI) | Low (no code changes) |
| **Senpi-ai/senpi-skills** | **DEFER** | Medium — useful design patterns | High (Python→Rust port, platform mismatch) | Medium (concept extraction only) |
| **chrisworsey55/atlas-gic** | **REJECT** | Low — architecture doc, proprietary code | N/A (nothing to integrate) | N/A |

## Overall Recommendation

**None of these repos warrant direct code integration.** Zekt's Rust codebase is more specialized and better suited to its Flash Trade execution target than any of these tools.

**Actionable next steps:**
1. **Study Senpi's Coyote regime classifier** — the vol-confirmed 3-regime pattern (TREND_UP / TREND_DOWN / CHOP) is a simpler alternative to Zekt's M4 multi-factor regime detector. Compare approaches and cherry-pick the vol-confirmation heuristic.
2. **Study Senpi's Lynx self-tuning** — adaptive MIN_SCORE based on own trade history is a novel pattern Zekt doesn't have. Consider adding a self-tuning mechanism to Zekt's strategy parameters.
3. **Study ATLAS's autoresearch loop** — the idea of "prompts are weights, Sharpe is loss function, git commit/revert as optimization" could inspire a Zekt-native parameter optimization layer that runs during backtesting.
4. **Skip fintool entirely** — Zekt already has better-integrated Rust implementations for everything fintool offers in Zekt's domain.
