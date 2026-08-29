<div align="center">

# 🧵 tokenloom

**Weave the live web into your context window.**

A fast, safe, token-efficient Rust CLI for web search and page fetching —
built for LLMs, agents, and anyone tired of pasting HTML into prompts.

[![CI](https://github.com/danecwalker/tokenloom/actions/workflows/ci.yml/badge.svg)](https://github.com/danecwalker/tokenloom/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/danecwalker/tokenloom)](https://github.com/danecwalker/tokenloom/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`curl -fsSL https://raw.githubusercontent.com/danecwalker/tokenloom/main/install.sh | sh` — and your agents stop hallucinating URLs.

</div>

---

## Why

LLMs need the live web, but raw HTML wastes tokens and hides prompt injections.
**tokenloom** is one static binary that:

- 🔎 **Federates search** across up to **248 SearXNG-compatible engines** (10 category tabs — general, images, videos, news, map, music, it, science, files, social_media) in parallel, fused with **Reciprocal Rank Fusion** into dense, ranked Markdown or stable JSON v1
- 📄 **Turns any page into clean Markdown** through a **7-layer sanitiser**: SSRF-guarded transport → streaming pre-strip → spec-compliant parse → allowlist sanitisation → readability extraction → Markdown generation → LLM hardening (NFC, zero-width/bidi stripping, code-fence escaping, untrusted-content boundaries, token budgets)
- 🧠 **Handles SPAs honestly** — client-rendered pages are detected and routed through Jina Reader behind a strict **20 RPM token bucket** (cross-process, SQLite-backed), with a deterministic fallback ladder: API key → local headless Chrome → backoff → degraded static with an explicit warning
- 🛡️ **Never phones home to `169.254.169.254`** — DNS pinning blocks loopback, RFC1918, link-local, CGNAT and multicast on every redirect hop
- ⚡ **Starts in <20 ms** — zero Python, zero Docker, one binary

> Shipped engine parity mirrors SearXNG's configured engines: waves 1–3, honest
> per-engine status (`tokenloom engines list` never lies about what works).

## Install

### One-liner (macOS / Linux / Windows)

```bash
curl -fsSL https://raw.githubusercontent.com/danecwalker/tokenloom/main/install.sh | sh
```

Pin a version, choose a directory, or also install the DeepSeek Harness plugin:

```bash
TOKENLOOM_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/danecwalker/tokenloom/main/install.sh | sh -s -- --with-dsh-plugin
```

### From source

```bash
git clone https://github.com/danecwalker/tokenloom
cd tokenloom
cargo install --path crates/tokenloom
```

<details>
<summary>Optional: local headless-Chrome rendering</summary>

```bash
cargo install --path crates/tokenloom --features tokenloom-fetch/render
```

Lets the SPA fallback ladder render pages in a discovered Chrome/Chromium
(`$CHROME_PATH` is honoured) instead of degrading to the static shell.

</details>

## Quickstart

```bash
# Federated search → dense Markdown
tokenloom search "rust async runtime comparison"

# Bangs route to specific engines (SearXNG syntax)
tokenloom search "!arx quantum error correction"
tokenloom search "!ddg !news ukraine"
tokenloom search "!crates tokio" --json          # stable JSON v1 for agents

# Categories & limits
tokenloom search "vision transformer" --category science --limit 5

# Fetch any page as sanitised, LLM-ready Markdown
tokenloom fetch "https://news.ycombinator.com" --max-tokens 500
tokenloom read "https://example.com"             # `read` is an alias

# Inspect the registry & diagnose
tokenloom engines list --category science
tokenloom engines test duckduckgo
tokenloom bangs ddg
tokenloom doctor
```

Example output:

```markdown
# Search Results: "rust async runtime comparison"
*Queried 6 engines (crates.io, github, stackoverflow, duckduckgo, brave, reddit) in 342ms*

1. [Async runtimes - Tokio vs async-std vs smol](https://example.com/rust-runtimes)
   - **Engine:** `duckduckgo` | **Score:** 0.98
   - Comprehensive performance benchmark comparing Tokio, async-std, and smol…
```

## Use it as an MCP server

tokenloom ships a Model Context Protocol stdio server exposing `search` and
`fetch` tools. Add it to any MCP client:

**Claude Desktop / Cursor / generic** (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "tokenloom": {
      "command": "tokenloom",
      "args": ["mcp"]
    }
  }
}
```

The server implements `initialize`, `tools/list` and `tools/call` over
newline-delimited JSON-RPC 2.0 — no API keys, no network setup.

## Use it as DeepSeek Harness's search provider

tokenloom ships a [Cordis](https://github.com/danecwalker/tokenloom) plugin
(`integrations/dsh-plugin/`) that replaces the Harness `web_search` backend
with tokenloom's federated search.

**1. Install the binary + plugin in one shot**

```bash
curl -fsSL https://raw.githubusercontent.com/danecwalker/tokenloom/main/install.sh | sh -s -- --with-dsh-plugin
```

This drops `tokenloom` onto your `PATH` and installs the plugin to
`~/.dsh/profiles/node_modules/@dane/dsh-web-search-tokenloom`.

<details>
<summary>Manual plugin install</summary>

```bash
mkdir -p ~/.dsh/profiles/node_modules/@dane
cp -R integrations/dsh-plugin ~/.dsh/profiles/node_modules/@dane/dsh-web-search-tokenloom
```

</details>

**2. Wire it into the web seam**

Append to `~/.dsh/profiles/web/cordis.patch.yml`:

```yaml
- insert:
    - id: web-search-tokenloom
      name: '@dane/dsh-web-search-tokenloom'
      config:
        bin: tokenloom        # absolute path also works; $TOKENLOOM_BIN overrides
        maxResults: 10
        timeoutMs: 20000
        fetchTimeoutMs: 45000 # the SPA fallback ladder can run long

- id: web
  config:
    searchProvider: tokenloom   # web_search → federated RRF-ranked results
    fetchProvider: tokenloom    # web_fetch → the 7-layer sanitiser, not raw HTML

# some bundles ship with the fetch tool disabled — force it on and give the
# model the full field set (positive integers, all keys, so this works under
# both config-merge and config-replace semantics)
- id: tool-web
  config:
    search: true
    fetch: true
    searchMaxResults: 8
    searchMaxQueries: 4
    searchTimeoutMs: 30000
    fetchTimeoutMs: 45000
    fetchMaxOutputChars: 200000

# silence the providers you're replacing
- id: web-search-deepseek
  disabled: true
```

**3. Reload** — with `pnpm run dev:web` running, client plugins hot-reload;
otherwise refresh the GUI. Every `web_search` call now fans out across
tokenloom's engine registry and returns RRF-ranked, token-budgeted sources — and
`web_fetch` returns pages through the 7-layer sanitiser (SSRF guard, boilerplate
removal, prompt-injection hardening) as clean Markdown, instead of raw HTML.
The system prompt will tell the model to *follow up with web_fetch* once the
tool is registered — if it doesn't, the `tool-web` block above was the missing
piece.

**4. Tune it live** — Settings → Plugins → **tokenloom**: binary path, result
limit, search timeout, and fetch timeout are all editable in the settings card
and apply between operations (no restart). The card writes to the
`web-search-tokenloom:` section of `~/.dsh/settings.yaml`; the patch snippet
above is only the seeded default.

<details>
<summary>How it works under the hood</summary>

The plugin registers a provider into `ctx.web` that shells out to
`tokenloom search <query> --json --limit N` per request and maps the stable
JSON v1 envelope (`results[].url/title/snippet/published_date`,
`engines_failed[]`) onto the seam's source shape. Engine failures surface as
honest provider errors listing exactly which engines failed and why.

</details>

## Configuration

Precedence: CLI flags → `TOKENLOOM_*` env (`JINA_API_KEY` honoured) →
`./.tokenloom.toml` → `~/.config/tokenloom/config.toml` → built-in defaults.

| Key | Env | Default |
|---|---|---|
| `general.default_limit` | `TOKENLOOM_DEFAULT_LIMIT` | `10` |
| `general.timeout_ms` | `TOKENLOOM_TIMEOUT_MS` | `4000` |
| `http.proxy` | `TOKENLOOM_HTTP_PROXY` | — |
| `reader.jina_api_key` | `JINA_API_KEY` | — |
| `reader.jina_rate_limit_rpm` | `TOKENLOOM_JINA_RATE_LIMIT_RPM` | `20` |
| `reader.enable_local_headless` | — | `true` |
| `cache.db_path` | `TOKENLOOM_CACHE_DB` | `~/.cache/tokenloom/cache.db` |
| `cache.ttl_seconds` | `TOKENLOOM_CACHE_TTL_SECONDS` | `7200` |
| `engines.weights.<name>` | — | per-engine |
| `engines.overrides.<name>.enabled` | — | per-engine |

Inspect everything with `tokenloom config get`, one key with
`tokenloom config get http.proxy`.

## Security guarantees

| Invariant | How |
|---|---|
| **P1** No executable active content | Ammonia allowlist + pre-strip; `javascript:`/`data:` URIs and `on*` handlers die at Layer 4 |
| **P2** Bounded memory | 5 MB streaming byte cap (decompression bombs die mid-stream) + 10,000-node DOM cap |
| **P3** No SSRF | DNS pinning validates **every** resolved IP per hop — loopback, RFC1918, `169.254.169.254`, CGNAT `100.64/10`, multicast, ULA/link-local v6 are unreachble |
| **P4** Pure UTF-8 | NFC-normalised output, zero-width/bidi control characters stripped |
| **P5** Idempotent | `sanitise(sanitise(x)) ≡ sanitise(x)` — property-tested |

Prompt injection is mitigated, not solved: untrusted content is fenced, fenced
again (code-fence breakout is escaped), and wrapped in
`BEGIN/END_UNTRUSTED_CONTENT` boundaries so downstream models can see the seam.

## Engine status & extensibility

`engines.toml` registers **248 engines** transcribed from SearXNG's configured
engines (waves 1–3, 78 enabled by default). ~90 have working interpreters today
— family interpreters (MediaWiki, StackExchange, Discourse, Gitea, Lemmy,
Mastodon, HuggingFace…), declarative JSON/CSS engines (crates.io, GitHub, npm,
MDN, arXiv, PyPI…), and hand-built specialists (DuckDuckGo, Brave, Startpage,
Mojeek, Qwant…). The rest are listed honestly as `registered`.

Wire new engines **without recompiling** — add an `[[engines]]` entry with
`request`/`response` specs to your user config (format reference:
`crates/tokenloom-engines/src/builtin_specs.rs`).

## Development

```bash
cargo test --workspace              # 78 offline tests (wiremock engine harness)
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask sync-engines --check    # registry ↔ PLAN conformance gate
cargo check -p tokenloom-fetch --features render
cargo fuzz run fuzz_sanitizer       # fuzzing (nightly)
```

## License

MIT — see [LICENSE](LICENSE).
