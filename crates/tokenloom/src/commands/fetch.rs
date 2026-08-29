//! `tokenloom fetch` / `tokenloom read` (PLAN.md §6, §8 example 3).

use crate::cache;
use crate::commands::App;
use tokenloom_core::{Config, FetchedPage};
use tokenloom_fetch::{FetchOptions, Fetcher, JinaClient};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: &Config,
    url: String,
    max_tokens: Option<usize>,
    max_chars: Option<usize>,
    no_cache: bool,
    no_reader: bool,
    allow_images: bool,
    wait: bool,
    json: bool,
) -> i32 {
    let app = match App::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("tokenloom: {e}");
            return 2;
        }
    };

    let store = match cache::open_store(&config.cache) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "cache disabled");
            None
        }
    };

    let jina = JinaClient::new(
        app.client.raw().clone(),
        &config.reader.jina_endpoint,
        config.reader.jina_rate_limit_rpm,
        read_jina_key(config),
        store.clone(),
    );
    // Authenticated tier (PLAN.md §6 fallback ladder, step 1).
    let jina_with_key = read_jina_key(config).map(|key| {
        JinaClient::new(
            app.client.raw().clone(),
            &config.reader.jina_endpoint,
            config.reader.jina_rate_limit_rpm.max(200),
            Some(key),
            None,
        )
    });

    let mut sanitize = tokenloom_sanitize::SanitizeOptions::from_config(
        &config.sanitizer,
        config.http.max_response_size_mb.saturating_mul(1024 * 1024),
    );
    if let Some(mc) = max_chars {
        sanitize.max_characters = mc;
    }

    let fetcher = Fetcher::new(
        app.client,
        jina,
        jina_with_key,
        store,
        sanitize,
        config.reader.enable_spa_detection,
        config.reader.enable_local_headless,
        config.reader.headless_timeout_ms,
        config.cache.ttl_seconds,
        config.cache.stale_revalidate_multiplier,
    );

    let opts = FetchOptions {
        no_cache,
        no_reader,
        wait,
        allow_images,
        max_chars: max_chars.unwrap_or(0),
    };

    match fetcher.fetch(&url, &opts).await {
        Ok(mut page) => {
            if let Some(budget) = max_tokens {
                let truncated = tokenloom_output::truncate_with_marker(&page.markdown, budget);
                page.is_truncated = truncated != page.markdown;
                page.markdown = truncated;
                page.estimated_tokens = budget.min(page.estimated_tokens);
            }
            if json {
                let v = tokenloom_output::fetched_page_to_json(&page);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into())
                );
            } else {
                println!("{}", tokenloom_output::format_fetch_markdown(&page));
            }
            // Degraded output still exits 0 so agent tool calls don't crash
            // (PLAN.md §6 Step 4, honest-LLM contract).
            0
        }
        Err(e) => {
            eprintln!("tokenloom: fetch failed: {e}");
            1
        }
    }
}

fn read_jina_key(config: &Config) -> Option<String> {
    let from_cfg = Some(config.reader.jina_api_key.clone()).filter(|k| !k.is_empty());
    from_cfg
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("JINA_API_KEY").ok().filter(|k| !k.is_empty()))
}

/// Fetch used by the MCP server (returns the page instead of printing).
pub async fn fetch_page(config: &Config, app: &App, url: &str) -> Result<FetchedPage, String> {
    let store = cache::open_store(&config.cache).unwrap_or(None);
    let jina = JinaClient::new(
        app.client.raw().clone(),
        &config.reader.jina_endpoint,
        config.reader.jina_rate_limit_rpm,
        read_jina_key(config),
        store.clone(),
    );
    let sanitize = tokenloom_sanitize::SanitizeOptions::from_config(
        &config.sanitizer,
        config.http.max_response_size_mb.saturating_mul(1024 * 1024),
    );
    let fetcher = Fetcher::new(
        // FetchClient is recreated cheaply per call here.
        tokenloom_fetch::FetchClient::new(&config.http).map_err(|e| e.to_string())?,
        jina,
        None,
        store,
        sanitize,
        config.reader.enable_spa_detection,
        config.reader.enable_local_headless,
        config.reader.headless_timeout_ms,
        config.cache.ttl_seconds,
        config.cache.stale_revalidate_multiplier,
    );
    fetcher
        .fetch(url, &FetchOptions::default())
        .await
        .map_err(|e| e.to_string())
}
