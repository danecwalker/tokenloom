//! Fetch pipeline orchestrator: static GET → SPA detection → Jina Reader →
//! the 5-step rate-limit fallback ladder (PLAN.md §6).

use crate::client::FetchClient;
use crate::jina::{JinaClient, JinaOutcome};
use crate::spa_detector;
use crate::store::{CachedPage, SqliteStore};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokenloom_core::{estimate_tokens, FetchedPage, RenderMethod, TokenloomError};
use tokenloom_sanitize::{
    decode_html, harden_markdown, hardening::HardeningOptions as HO, sanitize_document,
    SanitizeOptions,
};
use url::Url;

/// Per-fetch options supplied by the CLI.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// Bypass the page cache read/write.
    pub no_cache: bool,
    /// Disable SPA detection + Jina delegation (force static extraction).
    pub no_reader: bool,
    /// Allow queueing/backoff when rate-limited (PLAN.md §6 Step 3).
    pub wait: bool,
    /// `--allow-images` override for the sanitiser.
    pub allow_images: bool,
    /// Character budget applied to the final Markdown (0 = use config).
    pub max_chars: usize,
}

pub struct Fetcher {
    client: FetchClient,
    jina: JinaClient,
    jina_with_key: Option<JinaClient>,
    store: Option<Arc<SqliteStore>>,
    sanitize: SanitizeOptions,
    enable_spa_detection: bool,
    enable_local_headless: bool,
    headless_timeout_ms: u64,
    ttl_seconds: u64,
    stale_multiplier: u64,
}

/// Degradation warning emitted with the honest-LLM-contract static fallback
/// (PLAN.md §6 Step 4).
pub const DEGRADED_WARNING: &str = "This page appears to be a client-rendered Single Page Application (SPA). Jina Reader rate limits (20 RPM) were reached and no local headless browser was found. The content below represents the static HTML shell and may be incomplete.";

impl Fetcher {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: FetchClient,
        jina: JinaClient,
        jina_with_key: Option<JinaClient>,
        store: Option<Arc<SqliteStore>>,
        sanitize: SanitizeOptions,
        enable_spa_detection: bool,
        enable_local_headless: bool,
        headless_timeout_ms: u64,
        ttl_seconds: u64,
        stale_multiplier: u64,
    ) -> Self {
        Self {
            client,
            jina,
            jina_with_key,
            store,
            sanitize,
            enable_spa_detection,
            enable_local_headless,
            headless_timeout_ms,
            ttl_seconds,
            stale_multiplier,
        }
    }

    /// Fetch a URL and produce an LLM-ready `FetchedPage`.
    pub async fn fetch(
        &self,
        url: &str,
        opts: &FetchOptions,
    ) -> Result<FetchedPage, TokenloomError> {
        let started = Instant::now();
        let parsed: Url = url
            .parse()
            .map_err(|_| TokenloomError::InvalidUrl(url.to_string()))?;
        crate::ssrf::validate_url(&parsed)?;
        let canonical = tokenloom_core::url_util::canonicalize_url(parsed.as_str())
            .unwrap_or_else(|| parsed.to_string());

        let mut sanitize = self.sanitize.clone();
        if opts.allow_images {
            sanitize.allow_images = true;
        }
        if opts.max_chars > 0 {
            sanitize.max_characters = opts.max_chars;
        }

        // ── Step 2 of the pipeline: cache check (PLAN.md §6) ──────────────
        if !opts.no_cache {
            if let Some(store) = &self.store {
                if let Some(hit) = self.cache_lookup(store, &canonical)? {
                    return Ok(self.page_from_cache(hit, started.elapsed().as_millis() as u64));
                }
            }
        }

        // ── Step 3: static GET with SSRF guard + streaming cap ────────────
        let raw = match self.client.get_capped(parsed.as_str()).await {
            Ok(raw) => raw,
            Err(network_err) => {
                // Stale-while-revalidate: serve a stale cache entry when the
                // network path fails (PLAN.md §6, fallback ladder step 1).
                if let Some(store) = &self.store {
                    if let Some(hit) = store.get_page(&canonical)? {
                        let age = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                            .saturating_sub(hit.fetched_at);
                        if age <= self.ttl_seconds.saturating_mul(self.stale_multiplier) {
                            tracing::warn!(age, "network fetch failed; serving stale cache entry");
                            let mut page =
                                self.page_from_cache(hit, started.elapsed().as_millis() as u64);
                            page.requested_url = parsed.to_string();
                            return Ok(page);
                        }
                    }
                }
                return Err(network_err);
            }
        };

        // ── Step 4: sanitise statically (always — needed for detection and
        //    for the degraded fallback) ─────────────────────────────────────
        let header_charset: Option<&str> = None;
        let doc = sanitize_document(
            &raw.body,
            header_charset,
            &Url::parse(&raw.final_url).unwrap_or(parsed.clone()),
            &sanitize,
        )?;
        let visible_chars = doc.markdown.chars().count();

        // ── Step 5: SPA detection & Jina delegation ────────────────────────
        let decoded = decode_html(&raw.body, header_charset);
        let spa = self.enable_spa_detection
            && !opts.no_reader
            && raw
                .content_type
                .as_deref()
                .unwrap_or("text/html")
                .contains("html")
            && spa_detector::detect(&decoded, visible_chars).is_spa;

        let mut render_method = RenderMethod::StaticDirect;
        let mut degradation_warning = None;
        let mut markdown = doc.markdown.clone();
        let mut title = doc.title.clone();

        if spa {
            // Delegate to Jina Reader, then the fallback ladder on 429.
            let mut outcome = self.jina.fetch_markdown(parsed.as_str(), None).await;

            if let Ok(JinaOutcome::RateLimited { .. }) = outcome {
                // Ladder step 1: authenticated Jina tier (PLAN.md §6).
                if let Some(authed) = &self.jina_with_key {
                    if let Ok(md) = authed.fetch_markdown(parsed.as_str(), None).await {
                        outcome = Ok(md);
                    }
                }
            }

            match outcome {
                Ok(JinaOutcome::Markdown(md)) => {
                    render_method = RenderMethod::JinaReader;
                    let hardened = harden_markdown(
                        &md,
                        &HO {
                            escape_fences: sanitize.escape_code_fences,
                            delimit: sanitize.delimit_untrusted,
                            budget: Some(sanitize.max_characters),
                        },
                    );
                    markdown = hardened;
                    title = title_or_first_heading(&markdown, &title);
                }
                Ok(JinaOutcome::RateLimited { retry_after }) => {
                    // Ladder steps 2–4 (PLAN.md §6).
                    let resolved = self
                        .rate_limit_ladder(
                            parsed.as_str(),
                            retry_after,
                            opts,
                            &sanitize,
                            &mut title,
                        )
                        .await;
                    match resolved {
                        LadderResult::LocalHeadless(md) => {
                            render_method = RenderMethod::LocalHeadless;
                            markdown = md;
                        }
                        LadderResult::WaitedAndSucceeded(md) => {
                            render_method = RenderMethod::JinaReader;
                            markdown = md;
                        }
                        LadderResult::Degraded => {
                            render_method = RenderMethod::DegradedStatic;
                            degradation_warning = Some(DEGRADED_WARNING.to_string());
                            markdown = format!(
                                "> [!WARNING]\n> **tokenloom Notice: Dynamic Render Unavailable**\n> {DEGRADED_WARNING}\n\n{}",
                                doc.markdown
                            );
                        }
                    }
                }
                Err(e) => {
                    // Jina unreachable — degrade honestly rather than fail.
                    render_method = RenderMethod::DegradedStatic;
                    degradation_warning = Some(format!(
                        "Jina Reader was unreachable ({e}); showing the static HTML extraction."
                    ));
                }
            }
        }

        // ── Step 6: persist to cache & assemble the page ───────────────────
        if !opts.no_cache {
            if let Some(store) = &self.store {
                let _ = store.put_page(
                    &canonical,
                    &title,
                    &markdown,
                    render_method,
                    raw.etag.as_deref(),
                    raw.last_modified.as_deref(),
                );
            }
        }

        let text_length = markdown.chars().filter(|c| !c.is_whitespace()).count();
        Ok(FetchedPage {
            requested_url: parsed.to_string(),
            final_url: raw.final_url,
            title,
            byline: doc.byline,
            published_time: doc.published_time,
            site_name: doc.site_name,
            estimated_tokens: estimate_tokens(&markdown),
            markdown,
            text_length,
            is_truncated: false,
            render_method,
            degradation_warning,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn cache_lookup(
        &self,
        store: &SqliteStore,
        canonical: &str,
    ) -> Result<Option<CachedPage>, TokenloomError> {
        let Some(hit) = store.get_page(canonical)? else {
            return Ok(None);
        };
        let age = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(hit.fetched_at);
        if age <= self.ttl_seconds {
            return Ok(Some(hit));
        }
        // Stale entries are refreshed on next call; only served when the
        // network path fails (handled by callers re-invoking with the flag).
        Ok(None)
    }

    async fn rate_limit_ladder(
        &self,
        url: &str,
        retry_after: Option<Duration>,
        opts: &FetchOptions,
        sanitize: &SanitizeOptions,
        title: &mut String,
    ) -> LadderResult {
        let _ = self.headless_timeout_ms; // consumed by the render feature path
                                          // Step 2: local headless Chrome via DevTools protocol (feature `render`).
        if self.enable_local_headless {
            #[cfg(feature = "render")]
            if let Ok(dom) = crate::headless::render_dom(url, self.headless_timeout_ms).await {
                if let Ok(doc) =
                    tokenloom_sanitize::sanitize_str(&dom, &Url::parse(url).unwrap(), sanitize)
                {
                    *title = doc.title.clone();
                    return LadderResult::LocalHeadless(doc.markdown);
                }
            }
        }

        // Step 3: exponential backoff queue when a wait budget exists
        // (PLAN.md §6: T = min(60, 2^n + rand(0,2)), capped here at 60s).
        if opts.wait {
            let mut delay = retry_after.unwrap_or(Duration::from_secs(1));
            for _ in 0..2 {
                if delay > Duration::from_secs(60) {
                    break;
                }
                tokio::time::sleep(delay).await;
                if let Ok(JinaOutcome::Markdown(md)) = self.jina.fetch_markdown(url, None).await {
                    let hardened = harden_markdown(
                        &md,
                        &HO {
                            escape_fences: sanitize.escape_code_fences,
                            delimit: sanitize.delimit_untrusted,
                            budget: Some(sanitize.max_characters),
                        },
                    );
                    *title = title_or_first_heading(&hardened, title);
                    return LadderResult::WaitedAndSucceeded(hardened);
                }
                delay = Duration::from_secs((delay.as_secs() * 2).min(60));
            }
        }

        // Step 4: honest degraded static fallback.
        LadderResult::Degraded
    }

    fn page_from_cache(&self, hit: CachedPage, elapsed_ms: u64) -> FetchedPage {
        FetchedPage {
            requested_url: hit.canonical_url.clone(),
            final_url: hit.canonical_url.clone(),
            title: hit.title.clone(),
            byline: None,
            published_time: None,
            site_name: None,
            markdown: hit.markdown.clone(),
            text_length: hit.markdown.chars().filter(|c| !c.is_whitespace()).count(),
            estimated_tokens: estimate_tokens(&hit.markdown),
            is_truncated: false,
            render_method: RenderMethod::Cache,
            degradation_warning: None,
            elapsed_ms,
        }
    }
}

// LocalHeadless is only produced when the `render` feature is compiled in.
#[allow(dead_code)]
enum LadderResult {
    LocalHeadless(String),
    WaitedAndSucceeded(String),
    Degraded,
}

fn title_or_first_heading(markdown: &str, fallback: &str) -> String {
    for line in markdown.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            let rest = rest.trim();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_fallback_prefers_heading() {
        assert_eq!(
            title_or_first_heading("intro\n# Real Title\nmore", "Old"),
            "Real Title"
        );
        assert_eq!(title_or_first_heading("no headings", "Old"), "Old");
    }
}
