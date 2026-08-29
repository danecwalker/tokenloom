# tokenloom — Implementation Plan

> A fast, safe, token-efficient Rust CLI for web search and web-page fetching, designed for LLMs and AI agent pipelines.
> Implements federated search across all **248 SearXNG-configured engines** (10 category tabs), an **enterprise-grade sanitiser** on page fetches that renders clean **Markdown**, an **SPA detection & Jina Reader fallback** (20 RPM bucket), and an **extensible rate-limit endgame ladder**.

---

## Table of Contents

1. [Executive Summary & Core Contract](#1-executive-summary--core-contract)
2. [Goals & Non-Goals](#2-goals--non-goals)
3. [Architecture & Workspace Layout](#3-architecture--workspace-layout)
4. [Data Model & Type System](#4-data-model--type-system)
5. [Search Engine Subsystem](#5-search-engine-subsystem)
   - [Engine Trait & Capabilities](#engine-trait--capabilities)
   - [Module Families & Declarative Engines](#module-families--declarative-engines)
   - [Registry & Sync Tooling](#registry--sync-tooling)
   - [Bangs & Category Routing](#bangs--category-routing)
   - [Deduplication & Reciprocal Rank Fusion](#deduplication--reciprocal-rank-fusion)
   - [Implementation Waves](#implementation-waves)
6. [Page Fetch & Reader Subsystem](#6-page-fetch--reader-subsystem)
   - [Fetch Pipeline Overview](#fetch-pipeline-overview)
   - [SPA & Shell Detection Heuristics](#spa--shell-detection-heuristics)
   - [Jina Reader Fallback (r.jina.ai @ 20 RPM)](#jina-reader-fallback-rjinaai--20-rpm)
   - [Rate-Limit Endgame: The Fallback Ladder](#rate-limit-endgame-the-fallback-ladder)
   - [Persistent Quota Ledger & Caching](#persistent-quota-ledger--caching)
7. [Robust Sanitiser Pipeline](#7-robust-sanitiser-pipeline)
   - [Defense-in-Depth Layered Architecture](#defense-in-depth-layered-architecture)
   - [Layer 1: Transport & SSRF Guard](#layer-1-transport--ssrf-guard)
   - [Layer 2: Pre-Strip & Streaming Caps](#layer-2-pre-strip--streaming-caps)
   - [Layer 3: DOM Parser & Charset Handling](#layer-3-dom-parser--charset-handling)
   - [Layer 4: Allowlist HTML Sanitisation (Ammonia)](#layer-4-allowlist-html-sanitisation-ammonia)
   - [Layer 5: Main Content Extraction (Readability)](#layer-5-main-content-extraction-readability)
   - [Layer 6: Markdown Generation (htmd)](#layer-6-markdown-generation-htmd)
   - [Layer 7: LLM Hardening & Prompt Injection Mitigation](#layer-7-llm-hardening--prompt-injection-mitigation)
   - [Sanitiser Guarantees & Invariants](#sanitiser-guarantees--invariants)
8. [CLI Design & Ergonomics](#8-cli-design--ergonomics)
   - [Command Tree](#command-tree)
   - [Output Formats (Markdown Default + JSON)](#output-formats-markdown-default--json)
   - [Flags & Environment Variables](#flags--environment-variables)
9. [Configuration](#9-configuration)
10. [Error Handling, Observability & Diagnostics](#10-error-handling-observability--diagnostics)
11. [Security & Threat Model](#11-security--threat-model)
12. [Testing, Fuzzing & Conformance Strategy](#12-testing-fuzzing--conformance-strategy)
13. [Milestones, Phasing & Acceptance Criteria](#13-milestones-phasing--acceptance-criteria)
14. [Dependency Catalog](#14-dependency-catalog)
15. [Risks & Mitigations](#15-risks--mitigations)
16. [Open Technical Decisions](#16-open-technical-decisions)
17. [Appendix A — SearXNG Engine Registry](#appendix-a--searxng-engine-registry)

---

## 1. Executive Summary & Core Contract

`tokenloom` is a single, zero-external-runtime Rust binary purpose-built for LLM consumption (agents, chat loops, research scripts, MCP tool servers). It delivers two primary capabilities and one fallback ladder:

```
                  ┌──────────────────────────────────────────────────────────┐
                  │                 tokenloom CLI Interface                   │
                  │   - Default output: Clean, structured Markdown           │
                  │   - Programmatic: --json (stable v1 schema)              │
                  │   - Token-budget-aware, anti-injection hardened          │
                  └─────────────┬──────────────────────────────┬─────────────┘
                                │                              │
                  ┌─────────────┴─────────────┐  ┌─────────────┴─────────────┐
                  │      tokenloom search      │  │       tokenloom fetch      │
                  │  Federated multi-engine   │  │   Safe URL → Markdown     │
                  │  querying across 248      │  │   extractor & reader      │
                  │  SearXNG-compatible       │  │                           │
                  │  engines with RRF ranking │  │                           │
                  └─────────────┬─────────────┘  └─────────────┬─────────────┘
                                │                              │
                                │                     [Static GET + Guard]
                                │                              │
                                │                 [SPA Heuristic Triggered?]
                                │                    /                   \
                                │                 (No)                   (Yes)
                                │                  │                       │
                                │           [Sanitiser & MD]       [r.jina.ai Reader]
                                │                  │                 (20 RPM Bucket)
                                │                  │                       │
                                │                  │             [429 Rate Limited?]
                                │                  │                /             \
                                │                  │             (No)             (Yes)
                                │                  │              │                 │
                                │                  │        [MD Output]   [Fallback Ladder]
                                │                  │                       (Local Headless /
                                │                  │                        Backoff / Cache /
                                │                  │                        Degraded Static)
                                └──────────────────┴───────────────────────┘
```

### Core Functions Contract
1. **Search Results (`tokenloom search "<query>"`):**
   - Queries multiple configured engines in parallel with individual timeouts and weights.
   - Formats results into **dense, high-signal Markdown** (default) or **versioned JSON** (`--json`).
   - Supports SearXNG bangs (e.g. `!ddg`, `!arx`, `!gh`, `!w`, `!news`).
   - Merges and deduplicates results using Reciprocal Rank Fusion (RRF).

2. **Page Fetcher (`tokenloom fetch <url>` / `tokenloom read <url>`):**
   - Converts any web page into **clean, LLM-friendly Markdown**.
   - Runs a 7-layer robust sanitiser: SSRF protection, streaming byte caps, script/style/iframe removal, allowlist sanitisation, readability main-content extraction, DOM-to-Markdown conversion, and LLM-hardening (stripping control chars, zero-width spaces, bidi overrides, and fence-escaping).

3. **SPA & Dynamic Page Fallback (`r.jina.ai`):**
   - Detects JavaScript-rendered SPAs (React/Vue/Next/Nuxt empty root shells, `<noscript>` warnings, high script-to-text ratios, low extracted word counts).
   - Routes SPAs to `https://r.jina.ai/<url>` with a client-side token bucket strictly enforcing **20 RPM** (unauthenticated tier).

4. **Rate-Limit Endgame (Beyond Jina):**
   - When `r.jina.ai` returns `429 Too Many Requests` or exhausts the token bucket, executes a deterministic degradation ladder:
     1. **Cache check:** Return cached markdown if fresh or within stale-while-revalidate window.
     2. **Configured API Key:** If `JINA_API_KEY` is provided, bump to the authenticated tier.
     3. **Local Headless Chrome (Feature flag `render`):** Spawn local headless Chrome/Chromium via DevTools protocol (`chromiumoxide`) to evaluate JS locally.
     4. **Backoff & Wait:** If `--wait` or token budget allows, queue with exponential backoff and jitter.
     5. **Degraded Static Fallback:** If all else fails, return the best-effort sanitised static HTML with explicit `[tokenloom warning: dynamic render unavailable; showing static HTML shell]` header so LLMs understand context was degraded.

---

## 2. Goals & Non-Goals

### Goals
- **Markdown by Default:** Output pure, readable, high-density Markdown optimized for LLM token usage (no noisy navigation, footers, tracking scripts, CSS noise).
- **Parity with SearXNG Engines:** Support configuration and querying of all **248 engines** listed in SearXNG's *Configured Engines* documentation across all 10 category tabs (`general`, `images`, `videos`, `news`, `map`, `music`, `it`, `science`, `files`, `social_media`).
- **Zero Python/Docker Runtime:** Self-contained native Rust binary with fast startup (<20ms).
- **Extreme Fetch Safety:** Comprehensive protection against SSRF, DNS rebinding, decompression bombs, parser differentials, and prompt injection attempts embedded in crawled HTML.
- **Polite & Deterministic Rate Limiting:** Built-in rate limiting per domain, including strict 20 RPM for Jina Reader and persistent cross-process rate-limit state.

### Non-Goals
- Web UI or browser server (this is strictly a CLI and library; an MCP server mode is supported for stdio tool-use).
- Full browser engine written from scratch in Rust (we leverage HTTP + Jina / DevTools protocol).
- Bypassing Cloudflare/DataDome CAPTCHAs via illegal/adversarial methods (engines that aggressively block scrapers default to `off`, mirroring SearXNG defaults, and status is honestly reported).

---

## 3. Architecture & Workspace Layout

The repository is structured as a Cargo workspace separating the core data types, network/fetch layer, sanitiser/reader pipeline, engine registry/federation, output formatters, and the CLI binary.

```
tokenloom/
├── Cargo.toml                      # Workspace root
├── Cargo.lock
├── PLAN.md                         # This implementation plan
├── README.md
├── engines.toml                    # Declarative master engine registry (248 engines)
├── xtask/                          # Developer tools (e.g., sync-engines from upstream)
│   ├── Cargo.toml
│   └── src/main.rs
└── crates/
    ├── tokenloom-core/              # Common types, errors, config, URL normalization
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── error.rs            # Typed error hierarchy (thiserror)
    │       ├── config.rs           # TOML config schema & loaders
    │       ├── model.rs            # SearchResult, SearchQuery, FetchedPage
    │       └── url_util.rs         # Canonicalization, scheme checking, bang extraction
    │
    ├── tokenloom-fetch/             # Safe HTTP client, SSRF guard, SPA detector, Jina client
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── client.rs           # Reqwest wrapper with timeouts & proxy support
    │       ├── ssrf.rs             # DNS resolution pinning & private IP blocklist
    │       ├── spa_detector.rs     # Heuristics for detecting client-rendered pages
    │       ├── jina.rs             # r.jina.ai client with 20 RPM token bucket
    │       ├── headless.rs         # Optional local headless Chrome renderer
    │       └── fallback.rs         # The fallback ladder orchestrator
    │
    ├── tokenloom-sanitize/          # The 7-layer sanitiser & Markdown converter
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── pre_strip.rs        # Streaming tag remover (lol_html)
    │       ├── cleaner.rs          # Ammonia HTML allowlist sanitiser
    │       ├── extractor.rs        # Readability / DOM content extractor
    │       ├── markdown.rs         # HTML to clean Markdown transformer
    │       └── hardening.rs        # LLM prompt injection & unicode normalization
    │
    ├── tokenloom-engines/           # Engine trait, generic interpreters, 248 engine specs
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── trait_def.rs        # Engine trait & EngineCapabilities
    │       ├── registry.rs         # Engine registry & lookup
    │       ├── federation.rs       # Parallel dispatcher, timeout manager & RRF ranker
    │       ├── generic/            # Shared engine interpreters
    │       │   ├── json_engine.rs  # Declarative JSONPath engine
    │       │   ├── css_engine.rs   # Declarative CSS selector / scraper engine
    │       │   ├── xpath_engine.rs # Declarative XPath engine
    │       │   ├── mediawiki.rs    # MediaWiki family (Wikipedia, Wikibooks, etc.)
    │       │   ├── stackexchange.rs# StackExchange family (SO, AskUbuntu, SuperUser)
    │       │   ├── discourse.rs    # Discourse forum family
    │       │   ├── gitea.rs        # Gitea / Codeberg family
    │       │   ├── lemmy.rs        # Lemmy community family
    │       │   └── mastodon.rs     # Mastodon / Fediverse family
    │       └── specialists/        # Hand-crafted engines (DuckDuckGo, Brave, Startpage, etc.)
    │
    ├── tokenloom-output/            # LLM-optimized Markdown & JSON formatters
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── markdown.rs         # Search results & fetch page Markdown templates
    │       ├── json.rs             # Stable JSON schema serializers
    │       └── token_budget.rs     # Approximate token estimators & truncation markers
    │
    └── tokenloom-cli/               # CLI entrypoint (clap derive) & MCP server mode
        ├── Cargo.toml
        └── src/
            ├── main.rs
            ├── commands/
            │   ├── search.rs       # `tokenloom search`
            │   ├── fetch.rs        # `tokenloom fetch` / `read`
            │   ├── engines.rs      # `tokenloom engines list|test|show`
            │   ├── bangs.rs        # `tokenloom bangs`
            │   ├── doctor.rs       # `tokenloom doctor`
            │   └── mcp.rs          # `tokenloom mcp` (stdio MCP tool server)
            └── cache.rs            # SQLite cache & persistent quota ledger
```

---

## 4. Data Model & Type System

### Core Search Types (`tokenloom-core`)

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    General,
    Images,
    Videos,
    News,
    Map,
    Music,
    It,
    Science,
    Files,
    SocialMedia,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub raw_query: String,
    pub clean_query: String,
    pub bang: Option<String>,
    pub category: Category,
    pub engines: Vec<String>,
    pub page: u32,
    pub locale: Option<String>,
    pub safe_search: u8, // 0 = off, 1 = moderate, 2 = strict
    pub time_range: Option<String>, // day, week, month, year
    pub limit: usize,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
    pub category: Category,
    pub score: f64,
    pub published_date: Option<String>,
    pub thumbnail_url: Option<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub category: Category,
    pub results: Vec<SearchResult>,
    pub total_results: usize,
    pub engines_queried: Vec<String>,
    pub engines_failed: Vec<EngineFailure>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineFailure {
    pub engine: String,
    pub error: String,
    pub is_rate_limited: bool,
}
```

### Page Fetch & Reader Types (`tokenloom-core`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedPage {
    pub requested_url: String,
    pub final_url: String,
    pub title: String,
    pub byline: Option<String>,
    pub published_time: Option<String>,
    pub site_name: Option<String>,
    pub markdown: String,
    pub text_length: usize,
    pub estimated_tokens: usize,
    pub is_truncated: bool,
    pub render_method: RenderMethod,
    pub degradation_warning: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMethod {
    /// Direct static HTTP GET + sanitiser & readability extraction
    StaticDirect,
    /// Rendered via Jina Reader (https://r.jina.ai)
    JinaReader,
    /// Rendered via local headless Chrome/Chromium
    LocalHeadless,
    /// Degraded fallback static HTML after SPA render failure
    DegradedStatic,
    /// Cached response from local SQLite
    Cache,
}
```

---

## 5. Search Engine Subsystem

### Engine Trait & Capabilities

Every engine implements the async `Engine` trait:

```rust
use async_trait::async_trait;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub paging: bool,
    pub locale: bool,
    pub safe_search: bool,
    pub time_range: bool,
    pub requires_api_key: bool,
}

#[async_trait]
pub trait Engine: Send + Sync {
    /// Engine identifier (e.g., "duckduckgo", "crates", "arxiv")
    fn name(&self) -> &'static str;

    /// Supported bang shortcuts (e.g., "!ddg", "!crates")
    fn bangs(&self) -> &'static [&'static str];

    /// Primary and secondary categories
    fn categories(&self) -> &'static [Category];

    /// Engine capabilities
    fn capabilities(&self) -> EngineCapabilities;

    /// Default timeout specified in SearXNG configuration
    fn default_timeout(&self) -> Duration;

    /// Default weight for ranking
    fn default_weight(&self) -> f64;

    /// Shipped default state (true = enabled by default, false = opt-in)
    fn is_enabled_by_default(&self) -> bool;

    /// Execute search query
    async fn search(&self, query: &SearchQuery, client: &reqwest::Client) -> Result<Vec<SearchResult>, EngineError>;
}
```

### Module Families & Declarative Engines

SearXNG implements hundreds of engines by sharing Python modules. In `tokenloom`, we replicate this architecture through **Declarative Engine Interpreters** plus dedicated specialist implementations:

1. **Declarative JSON Engine (`json_engine`):**
   - Configured via TOML.
   - Defines endpoint URL template, query parameters, headers, and JSONPath expressions for title, URL, snippet, and pagination.
   - Powers ~35 engines (e.g., `packagist`, `openalex`, `mankier`, `openairedatasets`).

2. **Declarative CSS / Scraper Engine (`css_engine` / `xpath_engine`):**
   - Configured via TOML.
   - Defines request URL, method, and CSS/XPath selectors for result items, title, link href, and snippet text.
   - Powers ~40 engines (e.g., `searchmysite`, `bitbucket`, `anaconda`, `pub.dev`).

3. **Family Interpreters:**
   - **MediaWiki Family:** Shared query protocol against `/w/api.php?action=opensearch` or `query` for `wikipedia`, `wikidata`, `wikibooks`, `wikinews`, `wikiquote`, `wikisource`, `wikispecies`, `wikiversity`, `wikivoyage`, `archlinux`, `gentoo`, `nixos wiki`.
   - **StackExchange Family:** REST API v2.3 integration for `stackoverflow`, `askubuntu`, `superuser`.
   - **Discourse Family:** Search endpoint `/search.json?q=` for `caddy.community`, `discuss.python`, `pi-hole.community`.
   - **Gitea Family:** Search API for `codeberg`, `gitea.com`.
   - **Lemmy Family:** API v3 search for `lemmy posts`, `lemmy comments`, `lemmy communities`, `lemmy users`.
   - **Mastodon Family:** API v2 search for `mastodon hashtags`, `mastodon users`.
   - **HuggingFace Family:** Hub API for `huggingface`, `huggingface datasets`, `huggingface spaces`.
   - **DuckDuckGo Family:** Specialist scrapers/APIs for `duckduckgo`, `duckduckgo images`, `duckduckgo videos`, `duckduckgo news`, `duckduckgo definitions`.

4. **Hand-Crafted Specialists:**
   - Dedicated modules for high-traffic engines with specific session/cookie/token needs: `brave`, `startpage`, `mojeek`, `qwant`, `google`, `bing`, `github`, `crates.io`, `arxiv`, `pubmed`, `hackernews`, `openstreetmap`.

### Registry & Sync Tooling

The complete registry is stored in `engines.toml` in the repository root. At compile time, `build.rs` or `lazy_static` loads and validates all 248 engine definitions. An `xtask` tool (`cargo xtask sync-engines`) fetches the latest SearXNG documentation tables and checks for schema drift.

### Bangs & Category Routing

Query bang syntax is parsed with high fidelity:
- **Engine bangs:** `!ddg query`, `query !crates`, `!arx quantum computing`
- **Category bangs:** `!news artificial intelligence`, `!images rust logo`, `!it reqwest`, `!science crispr`
- **Multi-bang support:** `!ddg !news ukraine` routes to DuckDuckGo news.
- Bangs are stripped from the clean query passed to the underlying engine.

### Deduplication & Reciprocal Rank Fusion (RRF)

When querying multiple engines simultaneously:
1. **URL Canonicalization:** Strip UTM parameters, trailing slashes, tracking fragments, `www.` prefixes, and normalize scheme to `https`.
2. **RRF Scoring Formula:**
   $$\text{Score}(d) = \sum_{e \in E} w_e \cdot \frac{1}{k + \text{rank}_e(d)}$$
   where $k = 60$, $w_e$ is the engine's configured weight, and $\text{rank}_e(d)$ is the 1-based rank of result $d$ returned by engine $e$.
3. **Snippet Merging:** When multiple engines return the same URL, combine or select the longest high-quality snippet and record all source engines in the metadata.

### Implementation Waves

The 248 engines are implemented across 3 structured waves:

- **Wave 1 (80 engines — Core & Shared Families):**
  - Engine framework + Declarative JSON/CSS interpreters
  - Shared families: MediaWiki (10), StackExchange (3), Discourse (3), Gitea (2), Lemmy (4), Mastodon (2), HuggingFace (3), WikiCommons (4)
  - Core web engines: DuckDuckGo (+news/images/videos/definitions), Brave, Startpage, Mojeek, Qwant, Wikipedia, Wikidata
- **Wave 2 (82 engines — Stable JSON/API Long Tail):**
  - Developer & Package registries: Crates.io, PyPI, NPM, Docker Hub, Pkg.go.dev, Lib.rs, Hex, MetaCPAN, GitHub, GitLab, SourceHut, MDN, Mankier, NVD, Microsoft Learn
  - Science: ArXiv, PubMed, Semantic Scholar, CrossRef, OpenAlex, PDBe, OpenAIRE
  - Media & Geodata: OpenStreetMap, Photon, OpenVerse, Pexels, Unsplash, Artic, Genius, Radio Browser, Bandcamp, SoundCloud, Deezer, MixCloud, Dailymotion, Vimeo, PeerTube
  - Translation & Dictionaries: Lingva, DictZone, MyMemory, Mozhi, Currency
- **Wave 3 (86 engines — HTML-Scraped / Adversarial / Regional):**
  - Big search engines requiring anti-bot resilience: Google (+images/videos/news/scholar/cse), Bing (+images/videos/news), Yahoo, Yandex, Mojeek News, Seznam
  - Regional engines: Baidu (+images/kaifa), Sogou (+images/videos/wechat), Quark (+images), Naver (+images/videos/news), 360Search (+videos)
  - Torrents, Apps, Forum scrapers: 1337x, BT4G, BTDigg, PirateBay, APKMirror, F-Droid, Apple App Store, Google Play, Boardreader, Crowdview

---

## 6. Page Fetch & Reader Subsystem

The fetch pipeline converts arbitrary URLs into dense, structured, LLM-ready Markdown.

### Fetch Pipeline Overview

```
URL Input
   │
   ▼
[ 1. Transport & SSRF Validation ] ──► Reject private IPs, invalid schemes, bad ports
   │
   ▼
[ 2. Cache Check (SQLite) ] ─────────► Return cached Markdown if fresh
   │
   ▼
[ 3. Static HTTP GET ] ──────────────► Stream with 5MB cap, decompression bomb guard
   │
   ▼
[ 4. SPA Detection Heuristics ]
   │
   ├── (Not an SPA) ─────────────────► [ 5. Robust Sanitiser & Readability ] ──► [ 6. Markdown Generator ]
   │                                                                                    │
   └── (SPA Detected)                                                                   │
          │                                                                             ▼
          ▼                                                                     [ 7. LLM Hardening ]
   [ Jina Reader: r.jina.ai ] ◄── (20 RPM Token Bucket)                                 │
          │                                                                             ▼
          ├── (Success 200) ───────────────────────────────────────────────────► Clean Markdown Output
          │
          └── (429 Rate Limited / Error)
                 │
                 ▼
          [ Rate-Limit Fallback Ladder ]
                 │
                 ├── 1. API Key Auth (JINA_API_KEY if present)
                 ├── 2. Local Headless Chrome (chromiumoxide)
                 ├── 3. Exponential Backoff Queue (if --wait)
                 └── 4. Best-Effort Degraded Static HTML with Warning
```

### SPA & Shell Detection Heuristics

A page is classified as an SPA / client-rendered shell if any of the following conditions trigger:

1. **Byte & Tag Ratio:** Total body HTML size > 20KB, but visible extracted text < 250 characters.
2. **Empty Application Roots:** Matches empty single-page framework mount points:
   - `<div id="root"></div>` or `<div id="app"></div>`
   - `<div id="__next"></div>` (where `__NEXT_DATA__` contains no pre-rendered page content)
   - `<app-root></app-root>` (Angular)
3. **NoScript Warning Blocks:** Extracted text contains common fallback strings:
   - *"You need to enable JavaScript to run this app"*
   - *"JavaScript is disabled in your browser"*
   - *"Please turn on JavaScript and refresh the page"*
4. **Hydration Script Density:** More than 5 `<script>` bundle tags with virtually no paragraph (`<p>`), article (`<article>`), or heading (`<h1>..<h3>`) tags in the body.

### Jina Reader Fallback (`r.jina.ai` @ 20 RPM)

When an SPA is detected, `tokenloom` delegates page rendering to Jina Reader:

- **Target Endpoint:** `https://r.jina.ai/<target_url>`
- **Request Headers:**
  ```http
  Accept: text/plain
  User-Agent: tokenloom/0.1.0 (+https://github.com/danewalker/tokenloom)
  X-No-Cache: false
  X-Target-Selector: main, article, #content, .content, body
  ```
- **Rate-Limiter Specification:**
  - Token bucket algorithm implemented via `governor` crate.
  - Rate: Exactly **20 requests per minute** (1 token every 3000ms, max burst 1).
  - Scope: Global per-process and synchronized via SQLite quota table across multiple CLI invocations.

### Rate-Limit Endgame: The Fallback Ladder

When `r.jina.ai` returns HTTP `429 Too Many Requests` or local quota is depleted, `tokenloom` resolves the fetch via a strict 5-step fallback ladder:

```
                  ┌──────────────────────────────────────────────┐
                  │          r.jina.ai 429 Rate Limited          │
                  └──────────────────────┬───────────────────────┘
                                         │
                                         ▼
                     ┌───────────────────────────────────────┐
                     │ Step 1: Check JINA_API_KEY in Env/Cfg │
                     └───────────────────┬───────────────────┘
                                         │
                         ┌───────────────┴───────────────┐
                     (Present)                       (Missing)
                         │                               │
                         ▼                               ▼
             ┌───────────────────────┐       ┌───────────────────────┐
             │ Retry with Auth Token │       │ Step 2: Headless      │
             │ (High-RPM Tier)       │       │ Chromium Available?   │
             └───────────────────────┘       └───────────┬───────────┘
                                                         │
                                         ┌───────────────┴───────────────┐
                                     (Yes)                             (No)
                                         │                               │
                                         ▼                               ▼
                             ┌───────────────────────┐       ┌───────────────────────┐
                             │ Spawn Local Headless  │       │ Step 3: Is --wait or  │
                             │ Browser via DevTools  │       │ Backoff Budget Set?   │
                             │ (chromiumoxide)       │       └───────────┬───────────┘
                             └───────────────────────┘                   │
                                                         ┌───────────────┴───────────────┐
                                                     (Yes)                             (No)
                                                         │                               │
                                                         ▼                               ▼
                                             ┌───────────────────────┐       ┌───────────────────────┐
                                             │ Queue Request & Sleep │       │ Step 4: Degraded      │
                                             │ per Retry-After       │       │ Static HTML Fallback  │
                                             └───────────────────────┘       │ (With LLM Disclaimer) │
                                                                             └───────────────────────┘
```

1. **Step 1 — Authenticated Jina Tier:**
   - If `JINA_API_KEY` is configured in `~/.config/tokenloom/config.toml` or environment, attach `Authorization: Bearer <key>` which upgrades rate limits beyond the unauthenticated 20 RPM limit.
2. **Step 2 — Local Headless Chrome (`--features render`):**
   - If compiled with local rendering support and a Chrome/Chromium/Brave/Edge binary is discovered on the host (`$CHROME_PATH`, `/usr/bin/google-chrome`, `/Applications/Google Chrome.app`, etc.):
   - Launch an ephemeral headless browser process via `chromiumoxide`.
   - Wait for `networkidle0` or DOM selector stability (max 5s timeout).
   - Dump HTML DOM and pass directly through Layer 4–7 of the Sanitiser Pipeline.
3. **Step 3 — Exponential Backoff Queue:**
   - If the user invoked the CLI with `--wait` or specified a `--timeout` >= 10s:
   - Inspect the `Retry-After` header from `r.jina.ai` (defaulting to exponential backoff with jitter: $T = \min(60, 2^n + \text{rand}(0, 2))$).
   - Await quota replenishment and retry.
4. **Step 4 — Degraded Static Fallback (Honest LLM Contract):**
   - If headless browser is unavailable and waiting is not viable:
   - Extract whatever raw text and metadata was returned by the initial static GET.
   - Run through the Sanitiser and prefix the generated Markdown with an explicit machine-readable disclaimer block:
     ```markdown
     > [!WARNING]
     > **tokenloom Notice: Dynamic Render Unavailable**
     > This page appears to be a client-rendered Single Page Application (SPA).
     > Jina Reader rate limits (20 RPM) were reached and no local headless browser was found.
     > The content below represents the static HTML shell and may be incomplete.
     ```
   - Exit with status code `0` (so LLM tool calling does not crash), but set `"render_method": "DegradedStatic"` and `"degradation_warning"` in JSON output.

### Persistent Quota Ledger & Caching

To ensure multiple CLI invocations (e.g. from an agent loop) do not hammer `r.jina.ai` with burst 429s:
- **SQLite Storage (`~/.cache/tokenloom/cache.db`):**
  - Table `jina_quota_log`: Stores timestamps of recent Jina calls to enforce the 20 RPM sliding window across independent processes.
  - Table `page_cache`: Stores canonical URL, HTTP ETag, Last-Modified, fetch timestamp, and resulting Markdown (default TTL: 2 hours).

---

## 7. Robust Sanitiser Pipeline

The sanitiser is the security and quality core of `tokenloom`. It protects LLM systems from **remote code execution**, **SSRF**, **decompression bombs**, **HTML injection**, and **prompt injection attacks** while producing clean, dense Markdown.

```
Incoming HTTP Stream
   │
   ▼
[ Layer 1: Transport & SSRF Guard ]
   │ • DNS pinning to verified public IPs (Hickory DNS)
   │ • Reject RFC1918, Loopback, Link-Local (169.254.169.254), Carrier-Grade NAT
   │ • Enforce text/html or text/plain content-type
   │
   ▼
[ Layer 2: Streaming Pre-Strip (lol_html) ]
   │ • 5 MB total byte cap (kill stream immediately on overflow)
   │ • Streaming removal of <script>, <style>, <noscript>, <iframe>, <svg>, <canvas>
   │ • Strip HTML comments and conditional IE blocks
   │
   ▼
[ Layer 3: Parser & Charset Normalization ]
   │ • Robust charset detection (HTTP header -> <meta> tag -> chardetng)
   │ • html5ever DOM tree building with max 10,000 node cap
   │
   ▼
[ Layer 4: Allowlist HTML Sanitisation (Ammonia) ]
   │ • Strict tag allowlist: p, h1-h6, article, section, blockquote, pre, code,
   │   ul, ol, li, table, thead, tbody, tr, th, td, a, em, strong, del
   │ • Strip all event handlers (on*), style attributes, javascript: and data: URIs
   │ • URL normalization & absolute link resolution
   │
   ▼
[ Layer 5: Main Content Extraction (Readability / dom_smoothie) ]
   │ • Boilerplate & navigation removal (nav, header, footer, aside, ads)
   │ • Text-to-tag density scoring
   │ • Metadata extraction (title, author, published date, site name)
   │
   ▼
[ Layer 6: Markdown Generation (htmd) ]
   │ • Convert structured DOM to clean CommonMark
   │ • Table formatting (clean GFM tables)
   │ • Code block language preservation
   │ • Link formatting modes: inline, footnote references, or stripped
   │
   ▼
[ Layer 7: LLM Hardening & Prompt Injection Mitigation ]
   │ • Unicode normalization (NFC) & zero-width / bidi control character stripping
   │ • Markdown fence escaping (prevent malicious code block breakout)
   │ • Untrusted content enclosure delimiters (`<<<UNTRUSTED_EXTERNAL_CONTENT>>>`)
   │ • Token budget truncation with explicit `[... Content Truncated ...]` marker
   │
   ▼
Final Clean LLM-Ready Markdown Output
```

### Layer 1: Transport & SSRF Guard
- Custom DNS resolver via `hickory-resolver`.
- Resolves hostnames before TCP connect and validates that **all** returned IP addresses are routable public addresses.
- **Blocked Ranges:**
  - `0.0.0.0/8`, `127.0.0.0/8` (Loopback)
  - `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` (RFC1918 Private)
  - `169.254.0.0/16` (Link-Local / AWS/GCP Instance Metadata endpoint)
  - `100.64.0.0/10` (Shared Transition Space / CGNAT)
  - `224.0.0.0/4`, `240.0.0.0/4` (Multicast / Reserved)
  - `::1/128`, `fc00::/7`, `fe80::/10` (IPv6 Loopback, ULA, Link-Local)
- Pin connection directly to verified IP; reject redirects that resolve to prohibited CIDR blocks.

### Layer 2: Pre-Strip & Streaming Caps
- Maximum response size capped at **5 MB** (configurable).
- Streaming tokenization using Cloudflare's `lol_html`: drops `<script>`, `<style>`, `<template>`, `<svg>`, `<math>`, `<object>`, `<embed>`, `<iframe>`, and `<canvas>` content **before** full DOM tree allocation.

### Layer 3: DOM Parser & Charset Handling
- Decodes encoding using `encoding_rs` and `chardetng` if BOM/meta tags are absent.
- Parses HTML using `html5ever` (spec-compliant HTML5 parsing).
- Enforces a hard DOM limit of 10,000 nodes to prevent DOM-tree memory exhaustion.

### Layer 4: Allowlist HTML Sanitisation (Ammonia)
- Strict tag allowlist preserving only structural content:
  `p, h1, h2, h3, h4, h5, h6, article, section, blockquote, pre, code, ul, ol, li, table, thead, tbody, tr, th, td, a, em, strong, del, hr, dl, dt, dd`
- All other tags flattened or stripped.
- Attributes allowed: `href` on `<a>` (only `http`, `https`, `mailto`), `src` on `<img>` (if images enabled), `title`, `alt`.
- All `on*` inline JavaScript handlers, `style="..."` attributes, `javascript:` URLs, and `data:` URIs are aggressively stripped.

### Layer 5: Main Content Extraction (Readability)
- Rust port of Mozilla Readability algorithm (`dom_smoothie` or native implementation).
- Removes menus, cookie notices, sidebars, headers, footers, advertisement containers, and social share widgets.
- Extracts document metadata: OpenGraph / Twitter Cards title, author, publish date.

### Layer 6: Markdown Generation (`htmd`)
- Generates clean GitHub-Flavored Markdown.
- Preserves code blocks with language tags (` ```rust `, ` ```python `).
- Normalizes whitespace, paragraph breaks, and lists.
- Converts HTML tables into clean Markdown tables.

### Layer 7: LLM Hardening & Prompt Injection Mitigation
- **Unicode Sanitisation:** Strips zero-width spaces (`U+200B..U+200D`, `U+FEFF`), bidi override characters (`U+202A..U+202E`, `U+2066..U+2069`), and invalid ASCII control characters.
- **Fence Neutralization:** Any triple-backtick sequence (` ``` `) found within extracted content is safely escaped or indented so it cannot prematurely close an LLM prompt's outer code fence.
- **Delimited Container:** Content is clearly wrapped in untrusted boundary blocks:
  ```markdown
  # Page Title: Rust 1.95 Release Notes
  - Source URL: https://blog.rust-lang.org/2026/04/14/Rust-1.95.0.html
  - Rendered Via: StaticDirect
  - Content Hash: sha256:8f4c2...

  <!-- BEGIN_UNTRUSTED_CONTENT -->
  ... Sanitised Markdown ...
  <!-- END_UNTRUSTED_CONTENT -->
  ```
- **Token Budget Truncation:** Enforces user-specified `--max-tokens` or `--max-chars` with a clear suffix `[... Remaining 4,200 characters truncated by tokenloom token budget ...]`.

### Sanitiser Guarantees & Invariants
- **P1 (No Executable Active Content):** Output contains zero `<script>`, `onload=`, `javascript:`, or CSS expressions.
- **P2 (No Unbounded Memory):** Strict byte streaming caps (5MB) and node count caps (10,000 nodes).
- **P3 (No SSRF Leakage):** Localhost and cloud metadata services (`169.254.169.254`) can never be queried.
- **P4 (Pure Valid UTF-8):** Output is guaranteed to be valid UTF-8, normalized to Unicode NFC.
- **P5 (Deterministic Idempotency):** $\text{Sanitise}(\text{Sanitise}(x)) \equiv \text{Sanitise}(x)$.

---

## 8. CLI Design & Ergonomics

### Command Tree

```bash
tokenloom
├── search <QUERY>          # Federated search across engines (Markdown output default)
├── fetch <URL>             # Fetch page, sanitise, and convert to Markdown
├── read <URL>              # Alias for fetch
├── engines
│   ├── list                # List all 248 configured engines with status & categories
│   ├── show <ENGINE>       # Display detailed capabilities, bang, weight, timeout
│   ├── test <ENGINE>       # Test connectivity and live parsing for an engine
│   ├── enable <ENGINE>     # Enable an engine in user config
│   └── disable <ENGINE>    # Disable an engine in user config
├── bangs [PATTERN]         # List or search all supported !bang shortcuts
├── doctor                  # Check DNS, connectivity, Jina quota, and headless browser
├── config
│   ├── path                # Show active config file path
│   └── get [KEY]           # Read config value
└── mcp                     # Launch Model Context Protocol (MCP) stdio server
```

### Command Examples & Output Formats

#### 1. Search (Markdown Default)
```bash
$ tokenloom search "rust async runtime comparison" --category it --limit 5
```
**Output:**
```markdown
# Search Results: "rust async runtime comparison"
*Queried 6 engines (crates.io, github, stackoverflow, duckduckgo, brave, reddit) in 342ms*

1. [Async runtimes - Tokio vs async-std vs smol](https://example.com/rust-runtimes)
   - **Engine:** `duckduckgo` | **Score:** 0.98
   - Comprehensive performance benchmark comparing Tokio, async-std, and smol across I/O throughput and latency.

2. [tokio - crates.io: Rust Package Registry](https://crates.io/crates/tokio)
   - **Engine:** `crates.io` | **Score:** 0.89
   - An event-driven, non-blocking I/O platform for writing asynchronous applications with the Rust programming language.

...
```

#### 2. Search (JSON for Agents / Tools)
```bash
$ tokenloom search "!arx quantum error correction" --limit 2 --json
```
```json
{
  "query": "quantum error correction",
  "category": "science",
  "bang": "!arx",
  "results": [
    {
      "title": "Fault-Tolerant Quantum Computation with Surface Codes",
      "url": "https://arxiv.org/abs/2401.00000",
      "snippet": "We present a unified threshold analysis for surface codes...",
      "engine": "arxiv",
      "category": "science",
      "score": 1.0,
      "published_date": "2026-01-15",
      "metadata": { "arxiv_id": "2401.00000" }
    }
  ],
  "total_results": 1,
  "engines_queried": ["arxiv"],
  "engines_failed": [],
  "elapsed_ms": 184
}
```

#### 3. Page Fetch / Reader (Markdown)
```bash
$ tokenloom fetch "https://news.ycombinator.com" --max-tokens 500
```
**Output:**
```markdown
# Hacker News
- **URL:** https://news.ycombinator.com
- **Method:** `StaticDirect` | **Tokens:** ~420

<!-- BEGIN_UNTRUSTED_CONTENT -->
1. [Rust 1.95.0 Released](https://blog.rust-lang.org) (84 comments)
2. [Show HN: tokenloom – Web search CLI for LLMs](https://github.com) (142 comments)
3. [DeepSeek-V3 Technical Report](https://arxiv.org) (310 comments)
<!-- END_UNTRUSTED_CONTENT -->
```

#### 4. Engine Management
```bash
$ tokenloom engines list --category science
$ tokenloom engines test duckduckgo
$ tokenloom bangs !ddg
```

---

## 9. Configuration

Config search path:
1. CLI flags (`--config <path>`)
2. Environment variables (`TOKENLOOM_*`)
3. `./.tokenloom.toml` (Local project override)
4. `~/.config/tokenloom/config.toml` (User global config)
5. Built-in compiled defaults

### Master `config.toml` Example

```toml
[general]
default_category = "general"
default_limit = 10
timeout_ms = 4000
safe_search = 1               # 0 = off, 1 = moderate, 2 = strict
output_format = "markdown"    # markdown | json | plain

[http]
user_agent = "tokenloom/0.1.0 (+https://github.com/danewalker/tokenloom)"
max_response_size_mb = 5
connect_timeout_ms = 2000
total_timeout_ms = 8000
follow_redirects = true
max_redirects = 5
proxy = ""                    # Optional: "socks5://127.0.0.1:9050" or "http://proxy:8080"

[sanitizer]
allow_images = false
link_format = "inline"        # inline | footnotes | strip
max_characters = 50000
escape_code_fences = true
delimit_untrusted = true

[reader]
enable_spa_detection = true
jina_endpoint = "https://r.jina.ai"
jina_rate_limit_rpm = 20
jina_api_key = ""             # Optional: upgrades to 200+ RPM
enable_local_headless = true  # Requires chrome/chromium installed
headless_timeout_ms = 6000

[cache]
enabled = true
db_path = "~/.cache/tokenloom/cache.db"
ttl_seconds = 7200            # 2 hours

[engines.weights]
duckduckgo = 1.0
brave = 1.0
wikipedia = 1.2
github = 1.1

[engines.overrides]
# Enable engines that are disabled by default
google = { enabled = false, timeout_ms = 3000 }
bing = { enabled = false }
google_scholar = { enabled = true }
```

---

## 10. Error Handling, Observability & Diagnostics

### Typed Error Hierarchy (`tokenloom-core`)

```rust
#[derive(thiserror::Error, Debug)]
pub enum TokenloomError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("SSRF violation: destination IP {ip} is in prohibited range")]
    SsrfBlocked { ip: std::net::IpAddr },

    #[error("Engine '{engine}' failed: {reason}")]
    EngineFailure { engine: String, reason: String },

    #[error("Sanitisation error: {0}")]
    Sanitization(String),

    #[error("Jina Reader rate limit exhausted (20 RPM). Fallback status: {0}")]
    JinaRateLimited(String),

    #[error("Response body exceeded maximum allowed size of {0} bytes")]
    ResponseTooLarge(usize),

    #[error("Configuration error: {0}")]
    Config(String),
}
```

### Observability
- Integration with `tracing` and `tracing-subscriber`.
- Pass `--verbose` (`-v`, `-vv`, `-vvv`) to emit structured trace logs to `stderr` while keeping `stdout` clean for piped Markdown/JSON.
- `tokenloom doctor` command performs self-tests:
  - DNS resolution & SSRF filter verification.
  - Test ping to default engines (`duckduckgo`, `wikipedia`, `crates.io`).
  - Test connectivity to `https://r.jina.ai` and remaining quota check.
  - Headless Chromium binary discovery check.

---

## 11. Security & Threat Model

| Threat Vector | Mitigation in `tokenloom` |
|---|---|
| **Server-Side Request Forgery (SSRF)** | Custom DNS resolver pins IPs before connect; blocks private, loopback, link-local, and AWS metadata (`169.254.169.254`) ranges on every hop. |
| **Prompt Injection via Crawled Web Pages** | All fetched content is parsed through Ammonia allowlist, stripped of control characters, sanitized for markdown code-fence escapes, and enclosed in clear untrusted boundaries. |
| **Decompression Bombs (Zip / Gzip Bombs)** | Streaming byte counter terminates HTTP reads immediately at 5 MB decompressed payload cap. |
| **DOM Parser DoS (Deep Nesting)** | `html5ever` parser enforces a strict 10,000 node cap before DOM traversal. |
| **API Key Leakage** | API keys and credentials are only read from environment variables or mode-0600 config files; never echoed in CLI error messages. |
| **Parser Differential Attacks** | Two-pass sanitisation: pre-strip via `lol_html` followed by full spec-compliant `html5ever` parse and `ammonia` allowlisting. |

---

## 12. Testing, Fuzzing & Conformance Strategy

### 1. Unit Tests
- Fast, offline tests for URL canonicalization, bang parsing, RRF scoring, SPA heuristics, and config parsing.

### 2. Sanitiser Golden File & Corpus Tests
- Maintain a test corpus of 100+ real-world HTML files (news sites, SPAs, documentation, Wikipedia, malicious test payloads).
- Test with `insta` snapshot testing to guarantee stable Markdown output across versions.

### 3. Fuzzing (`cargo-fuzz` / `libFuzzer`)
- Target 1: `fuzz_sanitizer` — arbitrary byte streams fed to the 7-layer sanitiser to guarantee no panics, unbounded allocations, or invariant violations.
- Target 2: `fuzz_ssrf_resolver` — feed adversarial IP representations, hex IPs, octal IPs, IPv6-mapped IPv4 addresses to verify blocklist integrity.
- Target 3: `fuzz_spa_detector` — test SPA heuristic classifier against malformed HTML.

### 4. WireMock Engine Conformance Harness
- Offline engine tests using `wiremock` recording engine HTTP responses.
- Guarantees engine parser stability in CI without making outbound network calls.

### 5. Live Probes (`tokenloom engines test`)
- Diagnostic CLI command for verifying live engine parsing against upstream changes.

---

## 13. Milestones, Phasing & Acceptance Criteria

```
┌────────┐     ┌────────┐     ┌────────┐     ┌────────┐     ┌────────┐     ┌────────┐
│  M0    │ ──► │  M1    │ ──► │  M2    │ ──► │  M3    │ ──► │  M4    │ ──► │  M5    │
│ Setup  │     │ Core & │     │ Sanitize     │ Engines│     │ Fetch  │     │ Search │
│ & CI   │     │ SSRF   │     │ & MD   │     │ Wave 1 │     │ & Jina │     │ CLI UX │
└────────┘     └────────┘     └────────┘     └────────┘     └────────┘     └────────┘
                                                                               │
                               ┌────────┐     ┌────────┐     ┌────────┐        │
                               │  M8    │ ◄── │  M7    │ ◄── │  M6    │ ◄──────┘
                               │ Polish │     │ Wave 3 │     │ Wave 2 │
                               │ & MCP  │     │ Engines│     │ Engines│
                               └────────┘     └────────┘     └────────┘
```

### Milestone M0: Workspace Scaffolding & CI
- Cargo workspace with all crates created.
- GitHub Actions CI (cargo check, clippy, cargo test, fmt, MSRV 1.85+).
- `engines.toml` master data file generated from SearXNG documentation.

### Milestone M1: Core Types, Config & SSRF-Guarded HTTP
- `tokenloom-core` models and config loader.
- `tokenloom-fetch` secure HTTP client with Hickory DNS resolution and IP blocklist.
- Unit tests covering SSRF edge cases (IPv6 mapped, 169.254, loopbacks).

### Milestone M2: Robust Sanitiser & Markdown Generator
- `tokenloom-sanitize` implementation (Layers 1–7).
- `lol_html` pre-strip, `ammonia` allowlist, readability extractor, `htmd` Markdown conversion.
- Snapshot tests and initial `cargo-fuzz` targets.

### Milestone M3: Engine Framework & Wave 1 Engines (80 Engines)
- `Engine` trait, registry, parallel dispatcher, RRF ranking.
- Declarative JSON/CSS interpreters.
- Implement Wave 1 engines: DuckDuckGo family, Brave, Startpage, Mojeek, Qwant, MediaWiki family, StackExchange, Discourse, Gitea, Lemmy, Mastodon.

### Milestone M4: Page Fetcher, SPA Detection & Jina Reader
- `tokenloom fetch` command.
- SPA detection heuristics.
- Jina Reader client with 20 RPM token bucket.
- Rate-limit fallback ladder (local headless Chrome via `chromiumoxide` + degraded fallback).
- Persistent SQLite quota ledger & page caching.

### Milestone M5: Search CLI Polish & Markdown Output
- `tokenloom search` command with bang routing, category filtering, and clean Markdown output.
- `--json` output with stable v1 schema.
- Token budget limiting and truncation indicators.

### Milestone M6: Wave 2 Engines (82 Engines — Stable APIs)
- Package managers (Crates.io, PyPI, NPM, Docker Hub, etc.).
- Science engines (ArXiv, PubMed, Semantic Scholar, CrossRef, etc.).
- Media & Maps (OpenStreetMap, Bandcamp, SoundCloud, PeerTube, etc.).

### Milestone M7: Wave 3 Engines (86 Engines — HTML / Regional)
- Google, Bing, Yahoo, Yandex, Baidu, Sogou, Naver, 360Search.
- Torrents, Apps, and forum scrapers (shipped off by default).
- `tokenloom engines test` live diagnostic suite.

### Milestone M8: MCP Server Mode, Doctor & Release Engineering
- `tokenloom mcp` stdio server exposing `search` and `fetch` tools to LLMs.
- `tokenloom doctor` diagnostics.
- Cross-platform release binaries (Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64).
- Man pages and shell completions.

---

## 14. Dependency Catalog

| Crate | Purpose |
|---|---|
| `tokio` (features = ["full"]) | Async runtime for parallel engine queries and HTTP |
| `reqwest` (features = ["rustls-tls", "stream", "gzip", "brotli", "zstd"]) | HTTP client with modern TLS and compression |
| `hickory-resolver` | Custom DNS resolver for SSRF IP verification |
| `clap` (features = ["derive", "env"]) | CLI argument parsing |
| `serde`, `serde_json`, `toml` | Serialization and configuration parsing |
| `ammonia` | HTML allowlist sanitiser |
| `lol_html` | Low-overhead streaming HTML rewriter |
| `html5ever`, `scraper` | Spec-compliant HTML parsing and CSS selection |
| `dom_smoothie` / `readability` | Readability-style article and main content extraction |
| `htmd` | HTML to clean Markdown transformer |
| `governor` | Token bucket rate limiting (20 RPM for Jina) |
| `rusqlite` (features = ["bundled"]) | SQLite cache & persistent quota ledger |
| `chromiumoxide` (optional feature `render`) | Headless Chrome DevTools protocol driver |
| `thiserror`, `anyhow` | Typed error management |
| `tracing`, `tracing-subscriber` | Structured logging and diagnostics |
| `unicode-normalization` | Unicode NFC normalization for anti-injection |
| `wiremock` (dev-dependency) | HTTP mock server for offline engine testing |
| `insta` (dev-dependency) | Golden snapshot testing for sanitiser output |
| `proptest` (dev-dependency) | Property-based testing for parser invariants |

---

## 15. Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| **Upstream Search Engine HTML Changes** | Engine scrapers break over time. | 1. Declarative TOML engine specs allow updating selectors without re-compiling.<br>2. `tokenloom engines test` quickly flags broken engines.<br>3. RRF federated search tolerates individual engine failures gracefully. |
| **Jina Reader Rate Limiting (20 RPM)** | High-frequency agent loops get blocked on SPAs. | 1. Persistent SQLite token bucket prevents bursting into 429s.<br>2. Fallback ladder automatically tries local headless Chrome or caches.<br>3. Degraded static HTML fallback ensures the LLM receives context rather than a fatal error. |
| **Aggressive Anti-Bot IP Bans (Google/Bing)** | IP gets blocked when searching directly. | High-risk engines are disabled by default (matching SearXNG); user can configure proxies (`[http.proxy]`) or use stable API-backed engines (`duckduckgo`, `brave`, `wikipedia`). |
| **SSRF / Cloud Metadata Vulnerability** | Attacker tricks CLI into fetching AWS/GCP internal metadata. | IP resolution pinning rejects RFC1918 and link-local ranges (`169.254.169.254`) before the TCP handshake occurs on initial request and every redirect hop. |

---

## 16. Open Technical Decisions

1. **Readability Engine Selection:** Evaluate `dom_smoothie` (pure Rust Readability port) vs a custom tree-density algorithm. Decision gate in Milestone M2 based on benchmark corpus accuracy.
2. **Local Headless Chrome Dependency:** Keep `chromiumoxide` behind an optional Cargo feature (`--features render`) so minimal builds remain lightweight with zero C++/browser dependencies.
3. **MCP Tool Integration:** Implement Model Context Protocol over stdio in Milestone M8 using `rmcp` or lightweight JSON-RPC to make `tokenloom` natively usable as an LLM desktop tool.

---
## Appendix A — Engine registry (source: SearXNG *Configured Engines*)

**248 unique engines** across **10 category tabs**; **78 enabled by default**, mirroring the disabled-state column of the SearXNG tables; implemented through **161 module families**. Engines listed under several tabs (e.g. `youtube` in *videos* + *music*) are implemented once.

Legend — **Default**: shipped state in this CLI (`on` = enabled out of the box, `off` = opt-in, mirroring SearXNG's *Disabled* column). Flags: ✓ supported upstream, — unsupported, n/a = not applicable to the engine kind (instant-answer / currency / URL-search engines). **Wave** = proposed implementation phase (1 = framework + core; 2 = stable JSON/API long tail; 3 = fragile / HTML-scraped / regional, always opt-in and best-effort).


### Tab `!general` (61 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| brave | `!br` | `brave` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| duckduckgo | `!ddg` | `duckduckgo` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| ddg definitions | `!ddd` | `duckduckgo_definitions` | off | 3.0 | 2 | — | — | — | — | 1 |
| duckduckgo web | `!ddgw` | `duckduckgo_web` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| crowdview | `!cv` | `json_engine` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| encyclosearch | `!es` | `json_engine` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| wiby | `!wib` | `json_engine` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| wikibooks | `!wb` | `mediawiki` | off | 3.0 | 0.5 | ✓ | — | — | — | 1 |
| wikiquote | `!wq` | `mediawiki` | off | 3.0 | 0.5 | ✓ | — | — | — | 1 |
| wikisource | `!ws` | `mediawiki` | off | 3.0 | 0.5 | ✓ | — | — | — | 1 |
| wikispecies | `!wsp` | `mediawiki` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| wikiversity | `!wv` | `mediawiki` | off | 3.0 | 0.5 | ✓ | — | — | — | 1 |
| wikivoyage | `!wy` | `mediawiki` | off | 3.0 | 0.5 | ✓ | — | — | — | 1 |
| mojeek | `!mjk` | `mojeek` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| qwant | `!qw` | `qwant` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| startpage | `!sp` | `startpage` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| wikidata | `!wd` | `wikidata` | on | 3.0 | 2 | — | ✓ | — | — | 1 |
| wikipedia | `!wp` | `wikipedia` | on | 3.0 | 1.0 | — | ✓ | — | — | 1 |
| abcnyheter (NO) | `!abc` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| ayo | `!ayo` | `xpath` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| fastbot | `!fa` | `xpath` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| fynd | `!fynd` | `xpath` | off | 3.0 | 1.0 | ✓ | — | ✓ | — | 1 |
| gabanza | `!gab` | `xpath` | off | 4 | 1.0 | — | — | — | — | 1 |
| reloado (DE) | `!rel` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| searchch (CH) | `!sch` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| searchmysite | `!sms` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| wikimini (FR) | `!wkmn` | `xpath` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| zapmeta | `!zpm` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| bpb (DE) | `!bpb` | `bpb` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| currency | `!cc` | `currency_convert` | on | 3.0 | 100 | n/a | — | — | — | 2 |
| dictzone | `!dc` | `dictzone` | on | 3.0 | 100 | n/a | — | — | — | 2 |
| google cse | `!goc` | `google_cse` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 2 |
| lingva | `!lv` | `lingva` | on | 6.0 | 1.0 | n/a | — | — | — | 2 |
| mozhi | `!mz` | `mozhi` | off | 4.0 | 1.0 | n/a | — | — | — | 2 |
| openlibrary | `!ol` | `openlibrary` | off | 10 | 1.0 | ✓ | — | — | — | 2 |
| tagesschau (DE) | `!ts` | `tagesschau` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| tineye | `!tin` | `tineye` | off | 9.0 | 1.0 | n/a | — | — | — | 2 |
| mymemory translated | `!tl` | `translated` | on | 5.0 | 100 | n/a | — | — | — | 2 |
| 360search (ZH) | `!360so` | `360search` | off | 20.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| baidu (ZH) | `!bd` | `baidu` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| bing | `!bi` | `bing` | off | 3.0 | 1.0 | — | ✓ | ✓ | — | 3 |
| boardreader | `!boa` | `boardreader` | off | 3.0 | 1.0 | ✓ | ✓ | — | ✓ | 3 |
| fireball | `!fire` | `fireball` | off | 3.0 | 1.0 | — | — | ✓ | — | 3 |
| gmx | `!gmx` | `gmx` | off | 3.0 | 1.0 | ✓ | — | ✓ | ✓ | 3 |
| google | `!go` | `google` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| mwmbl | `!mwm` | `mwmbl` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| naver (KO) | `!nvr` | `naver` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| privacywall | `!pw` | `privacywall` | off | 3.0 | 1.0 | — | ✓ | ✓ | ✓ | 3 |
| quark (ZH) | `!qk` | `quark` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| resulthunter | `!reh` | `resulthunter` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| infospace | `!ifs` | `s1search` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| searchtoday | `!std` | `s1search` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| seznam (CZ) | `!szn` | `seznam` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| sogou (ZH) | `!sogou` | `sogou` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| tusksearch | `!tu` | `tusksearch` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| vuhuv | `!vu` | `vuhuv` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| wolframalpha | `!wa` | `wolframalpha_noapi` | off | 6.0 | 1.0 | — | — | — | — | 3 |
| yacy | `!ya` | `yacy` | off | 5.0 | 1.0 | ✓ | — | — | — | 3 |
| yahoo | `!yh` | `yahoo` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| yandex | `!yd` | `yandex` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| yep | `!yep` | `yep` | off | 3.0 | 1.0 | — | ✓ | ✓ | — | 3 |

### Tab `!images` (49 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| brave.images | `!brimg` | `brave` | on | 3.0 | 1.0 | — | ✓ | ✓ | — | 1 |
| duckduckgo images | `!ddi` | `duckduckgo_extra` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| mojeek images | `!mjkimg` | `mojeek` | off | 3.0 | 1.0 | — | ✓ | ✓ | ✓ | 1 |
| qwant images | `!qwi` | `qwant` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| startpage images | `!spi` | `startpage` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| wikicommons.images | `!wci` | `wikicommons` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| artic | `!arc` | `artic` | on | 4.0 | 1.0 | ✓ | — | — | — | 2 |
| artstation | `!as` | `artstation` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| cara | `!ca` | `cara` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| devicons | `!di` | `devicons` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| findthatmeme | `!ftm` | `findthatmeme` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| flaticon | `!fli` | `flaticon` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| flickr | `!fl` | `flickr_noapi` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| frinkiac | `!frk` | `frinkiac` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| giphy | `!gif` | `giphy` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| google cse images | `!goci` | `google_cse` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 2 |
| lucide | `!luc` | `lucide` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| material icons | `!mi` | `material_icons` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| openverse | `!opv` | `openverse` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| pexels | `!pe` | `pexels` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| pixabay images | `!pixi` | `pixabay` | off | 3.0 | 1.0 | ✓ | — | ✓ | ✓ | 2 |
| selfhst icons | `!si` | `selfhst` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| unsplash | `!us` | `unsplash` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| uxwing | `!ux` | `uxwing` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| 500px | `!500` | `500px` | off | 5 | 1.0 | ✓ | — | — | — | 3 |
| adobe stock | `!asi` | `adobe_stock` | off | 6 | 1.0 | ✓ | — | — | — | 3 |
| baidu images (ZH) | `!bdi` | `baidu` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| bing images | `!bii` | `bing_images` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| findfiles images | `!fifi` | `findfiles` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| google images | `!goi` | `google_images` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| imgur | `!img` | `imgur` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| ipernity | `!ip` | `ipernity` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| library of congress | `!loc` | `loc` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| magnific | `!mag` | `magnific` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| naver images (KO) | `!nvri` | `naver` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| picjumbo | `!pj` | `picjumbo` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| pinterest | `!pin` | `pinterest` | on | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| privacywall images | `!pwi` | `privacywall` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| public domain image archive | `!pdia` | `public_domain_image_archive` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| quark images (ZH) | `!qki` | `quark` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| resulthunter images | `!rehi` | `resulthunter` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| shopify stock | `!shs` | `shopify_stock` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| sogou images | `!sogoui` | `sogou_images` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| stocksnap | `!sto` | `stocksnap` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| tusksearch images | `!tui` | `tusksearch` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| vuhuv images | `!vui` | `vuhuv` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| 1x | `!1x` | `www1x` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| yacy images | `!yai` | `yacy` | off | 5.0 | 1.0 | ✓ | — | — | — | 3 |
| yandex images | `!ydi` | `yandex` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |

### Tab `!videos` (32 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| brave.videos | `!brvid` | `brave` | on | 3.0 | 1.0 | — | ✓ | ✓ | — | 1 |
| duckduckgo videos | `!ddv` | `duckduckgo_extra` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| qwant videos | `!qwv` | `qwant` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| wikicommons.videos | `!wcv` | `wikicommons` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| bitchute | `!bit` | `bitchute` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| media.ccc.de | `!c3tv` | `ccc_media` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| dailymotion | `!dm` | `dailymotion` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 2 |
| mediathekviewweb (DE) | `!mvw` | `mediathekviewweb` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| odysee | `!od` | `odysee` | off | 3.0 | 1.0 | ✓ | ✓ | — | ✓ | 2 |
| peertube | `!ptb` | `peertube` | off | 6.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 2 |
| pixabay videos | `!pixv` | `pixabay` | off | 3.0 | 1.0 | ✓ | — | ✓ | ✓ | 2 |
| rumble | `!ru` | `rumble` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| sepiasearch | `!sep` | `sepiasearch` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 2 |
| vimeo | `!vm` | `vimeo` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| youtube | `!yt` | `youtube_noapi` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| 360search videos | `!360sov` | `360search_videos` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| acfun (ZH) | `!acf` | `acfun` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| adobe stock video | `!asv` | `adobe_stock` | off | 6 | 1.0 | ✓ | — | — | — | 3 |
| bilibili | `!bil` | `bilibili` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| bing videos | `!biv` | `bing_videos` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| findfiles videos | `!fifv` | `findfiles` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| fireball videos | `!firev` | `fireball` | off | 3.0 | 1.0 | — | — | ✓ | — | 3 |
| google play movies | `!gpm` | `google_play` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| google videos | `!gov` | `google_videos` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| ina (FR) | `!in` | `ina` | off | 6.0 | 1.0 | ✓ | — | — | — | 3 |
| iqiyi (ZH) | `!iq` | `iqiyi` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| naver videos (KO) | `!nvrv` | `naver` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| niconico (JA) | `!nico` | `niconico` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| privacywall videos | `!pwv` | `privacywall` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 3 |
| sogou videos (ZH) | `!sogouv` | `sogou_videos` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| tusksearch videos | `!tuv` | `tusksearch` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| vuhuv videos | `!vuv` | `vuhuv` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |

### Tab `!news` (16 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| brave.news | `!brnews` | `brave` | on | 3.0 | 1.0 | — | ✓ | ✓ | — | 1 |
| duckduckgo news | `!ddn` | `duckduckgo_extra` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| wikinews | `!wn` | `mediawiki` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| mojeek news | `!mjknews` | `mojeek` | off | 3.0 | 1.0 | — | ✓ | ✓ | ✓ | 1 |
| qwant news | `!qwn` | `qwant` | off | 3.0 | 1.0 | ✓ | ✓ | ✓ | — | 1 |
| startpage news | `!spn` | `startpage` | on | 3.0 | 1.0 | ✓ | ✓ | ✓ | ✓ | 1 |
| ansa (IT) | `!ans` | `ansa` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| il post (IT) | `!pst` | `il_post` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| reuters | `!reu` | `reuters` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| tagesschau (DE) | `!ts` | `tagesschau` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| bing news | `!bin` | `bing_news` | on | 3.0 | 1.0 | ✓ | ✓ | — | ✓ | 3 |
| fireball news | `!firen` | `fireball` | off | 3.0 | 1.0 | — | — | ✓ | — | 3 |
| google news | `!gon` | `google_news` | on | 3.0 | 1.0 | ✓ | ✓ | — | — | 3 |
| naver news (KO) | `!nvrn` | `naver` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| sogou wechat (ZH) | `!sogouw` | `sogou_wechat` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| tusksearch news | `!tun` | `tusksearch` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |

### Tab `!map` (3 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| apple maps | `!apm` | `apple_maps` | off | 5.0 | 1.0 | — | — | — | — | 2 |
| openstreetmap | `!osm` | `openstreetmap` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| photon | `!ph` | `photon` | on | 3.0 | 1.0 | — | — | — | — | 2 |

### Tab `!music` (11 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| wikicommons.audio | `!wca` | `wikicommons` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| bandcamp | `!bc` | `bandcamp` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| deezer | `!dz` | `deezer` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| genius | `!gen` | `genius` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| mixcloud | `!mc` | `mixcloud` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| radio browser | `!rb` | `radio_browser` | on | 3.0 | 1.0 | ✓ | ✓ | — | — | 2 |
| soundcloud | `!sc` | `soundcloud` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| youtube | `!yt` | `youtube_noapi` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| adobe stock audio | `!asa` | `adobe_stock` | off | 6 | 1.0 | ✓ | — | — | — | 3 |
| findfiles music | `!fifm` | `findfiles` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| yandex music | `!ydm` | `yandex_music` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |

### Tab `!it` (44 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| caddy.community | `!caddy` | `discourse` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 1 |
| discuss.python | `!dpy` | `discourse` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 1 |
| pi-hole.community | `!pi` | `discourse` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 1 |
| codeberg | `!cb` | `gitea` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| gitea.com | `!gitea` | `gitea` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| huggingface | `!hf` | `huggingface` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| huggingface datasets | `!hfd` | `huggingface` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| huggingface spaces | `!hfs` | `huggingface` | off | 3.0 | 1.0 | — | — | — | — | 1 |
| mankier | `!man` | `json_engine` | on | 3.0 | 1.0 | — | — | — | — | 1 |
| mdn | `!mdn` | `json_engine` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| packagist | `!pack` | `json_engine` | off | 5.0 | 1.0 | ✓ | — | — | — | 1 |
| free software directory | `!fsd` | `mediawiki` | off | 5.0 | 1.0 | ✓ | — | — | — | 1 |
| gentoo | `!ge` | `mediawiki` | on | 10 | 1.0 | ✓ | — | — | — | 1 |
| nixos wiki | `!nixw` | `mediawiki` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| askubuntu | `!ubuntu` | `stackexchange` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| stackoverflow | `!st` | `stackexchange` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| superuser | `!su` | `stackexchange` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| anaconda | `!conda` | `xpath` | off | 6.0 | 1.0 | ✓ | — | — | — | 1 |
| bitbucket | `!bb` | `xpath` | off | 4.0 | 1.0 | ✓ | — | — | — | 1 |
| habrahabr | `!habr` | `xpath` | off | 4.0 | 1.0 | ✓ | — | — | — | 1 |
| hoogle | `!ho` | `xpath` | on | 3.0 | 1.0 | — | — | — | — | 1 |
| lobste.rs | `!lo` | `xpath` | off | 5.0 | 1.0 | — | — | — | — | 1 |
| pub.dev | `!pd` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| rubygems | `!rbg` | `xpath` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| alpine linux packages | `!alp` | `alpinelinux` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| arch linux wiki | `!al` | `archlinux` | on | 3.0 | 1.0 | ✓ | ✓ | — | — | 2 |
| cachy os packages | `!cos` | `cachy_os` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| crates.io | `!crates` | `crates` | off | 6.0 | 1.0 | ✓ | — | — | — | 2 |
| docker hub | `!dh` | `docker_hub` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| github | `!gh` | `github` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| gitlab | `!gl` | `gitlab` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| hackernews | `!hn` | `hackernews` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 2 |
| hex | `!hex` | `hex` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| lib.rs | `!lrs` | `lib_rs` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| metacpan | `!cpan` | `metacpan` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| microsoft learn | `!msl` | `microsoft_learn` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| npm | `!npm` | `npm` | off | 5.0 | 1.0 | ✓ | — | — | — | 2 |
| national vulnerability database | `!nvd` | `nvd` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| ollama | `!ollama` | `ollama` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| pkg.go.dev | `!pgo` | `pkg_go_dev` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| pypi | `!pypi` | `pypi` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| sourcehut | `!srht` | `sourcehut` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| voidlinux | `!void` | `voidlinux` | off | 3.0 | 1.0 | — | — | — | — | 2 |
| baidu kaifa (ZH) | `!bdk` | `baidu` | off | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |

### Tab `!science` (10 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| openairedatasets | `!oad` | `json_engine` | on | 5.0 | 1.0 | ✓ | — | — | — | 1 |
| openairepublications | `!oap` | `json_engine` | on | 5.0 | 1.0 | ✓ | — | — | — | 1 |
| wikispecies | `!wsp` | `mediawiki` | off | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| arxiv | `!arx` | `arxiv` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| crossref | `!cr` | `crossref` | off | 30 | 1.0 | ✓ | — | — | — | 2 |
| openalex | `!oa` | `openalex` | off | 5.0 | 1.0 | ✓ | — | — | — | 2 |
| pdbe | `!pdb` | `pdbe` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| pubmed | `!pub` | `pubmed` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| semantic scholar | `!se` | `semantic_scholar` | on | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| google scholar | `!gos` | `google_scholar` | on | 3.0 | 1.0 | ✓ | ✓ | — | ✓ | 3 |

### Tab `!files` (17 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| wikicommons.files | `!wcf` | `wikicommons` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| library genesis | `!lg` | `xpath` | off | 7.0 | 1.0 | — | — | — | — | 1 |
| openrepos | `!or` | `xpath` | off | 4.0 | 1.0 | ✓ | — | — | — | 1 |
| apple app store | `!aps` | `apple_app_store` | off | 3.0 | 1.0 | — | — | ✓ | — | 2 |
| fdroid | `!fd` | `fdroid` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| nyaa | `!nt` | `nyaa` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| solidtorrents | `!solid` | `solidtorrents` | on | 4.0 | 1.0 | ✓ | — | — | — | 2 |
| 1337x | `!1337x` | `1337x` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| annas archive | `!aa` | `annas_archive` | off | 5 | 1.0 | ✓ | ✓ | — | — | 3 |
| apk mirror | `!apkm` | `apkmirror` | off | 4.0 | 1.0 | ✓ | — | — | — | 3 |
| bt4g | `!bt4g` | `bt4g` | on | 3.0 | 1.0 | ✓ | — | — | ✓ | 3 |
| btdigg | `!bt` | `btdigg` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| findfiles | `!fif` | `findfiles` | off | 3.0 | 1.0 | ✓ | — | — | — | 3 |
| google play apps | `!gpa` | `google_play` | off | 3.0 | 1.0 | — | — | — | — | 3 |
| kickass | `!kc` | `kickass` | on | 4.0 | 1.0 | ✓ | — | — | — | 3 |
| piratebay | `!tpb` | `piratebay` | on | 3.0 | 1.0 | — | — | — | — | 3 |
| tokyotoshokan | `!tt` | `tokyotoshokan` | off | 6.0 | 1.0 | ✓ | — | — | — | 3 |

### Tab `!social_media` (9 engines)

| Engine | !bang | Module family | Default | Timeout | Weight | Paging | Locale | Safe search | Time range | Wave |
|---|---|---|---|---|---|---|---|---|---|---|
| lemmy comments | `!lecom` | `lemmy` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| lemmy communities | `!leco` | `lemmy` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| lemmy posts | `!lepo` | `lemmy` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| lemmy users | `!leus` | `lemmy` | on | 3.0 | 1.0 | ✓ | — | — | — | 1 |
| mastodon hashtags | `!mah` | `mastodon` | on | 3.0 | 1.0 | — | — | — | — | 1 |
| mastodon users | `!mau` | `mastodon` | on | 3.0 | 1.0 | — | — | — | — | 1 |
| 9gag | `!9g` | `9gag` | off | 3.0 | 1.0 | ✓ | — | — | — | 2 |
| tootfinder | `!toot` | `tootfinder` | on | 3.0 | 1.0 | — | — | — | — | 2 |
| boardreader | `!boa` | `boardreader` | off | 3.0 | 1.0 | ✓ | ✓ | — | ✓ | 3 |
