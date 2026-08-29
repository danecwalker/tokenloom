//! `tokenloom search` (PLAN.md §8, examples 1 & 2).

use crate::commands::App;
use tokenloom_core::{url_util, Category, Config, SearchQuery, TokenloomError};
use tokenloom_engines::Registry;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: &Config,
    query_text: String,
    category: Option<String>,
    engines: Option<String>,
    limit: Option<usize>,
    page: u32,
    locale: Option<String>,
    time_range: Option<String>,
    safe_search: u8,
    timeout_ms: Option<u64>,
    json: bool,
    max_tokens: Option<usize>,
) -> i32 {
    let app = match App::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("tokenloom: {e}");
            return 2;
        }
    };

    // ── Bang parsing & routing (PLAN.md §5) ─────────────────────────────
    let parsed = url_util::parse_bangs(&query_text);
    let resolution = app.registry.resolve_bangs(&parsed);

    let category = resolution
        .category
        .or_else(|| category.as_deref().and_then(Category::from_str))
        .unwrap_or(Category::General);

    let mut engines_list: Vec<String> = resolution.engines.clone();
    if let Some(e) = &engines {
        for name in e.split(',') {
            let name = name.trim();
            if !name.is_empty() && !engines_list.contains(&name.to_string()) {
                engines_list.push(name.to_string());
            }
        }
    }

    // The primary bang reported in JSON output.
    let bang = parsed
        .bangs
        .first()
        .map(|b| format!("!{b}"))
        .or(engines_list.first().map(|_| {
            app.registry
                .get(&engines_list[0])
                .map(|s| format!("!{}", s.bang))
                .unwrap_or_default()
        }));

    let mut query = SearchQuery::new(query_text);
    query.clean_query = parsed.clean_query;
    query.bang = bang.clone();
    query.category = category;
    query.engines = engines_list;
    query.page = page.max(1);
    query.locale = locale.or_else(|| Some("en-US".into()));
    query.safe_search = safe_search.clamp(0, 2);
    query.time_range = time_range;
    query.limit = limit.unwrap_or(config.general.default_limit).max(1);
    query.timeout =
        std::time::Duration::from_millis(timeout_ms.unwrap_or(config.general.timeout_ms));

    // ── Federated dispatch + RRF ────────────────────────────────────────
    let federator = tokenloom_engines::Federator::new(
        app.registry.clone(),
        app.client.raw().clone(),
        config.engines.weights.clone().into_iter().collect(),
    );
    let response = federator.search(&query).await;

    // ── Output (PLAN.md §8) ─────────────────────────────────────────────
    if json {
        let v = tokenloom_output::search_response_to_json(&response);
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into())
        );
    } else {
        let md = tokenloom_output::format_search_markdown(&response);
        match max_tokens {
            Some(budget) => println!("{}", tokenloom_output::truncate_with_marker(&md, budget)),
            None => println!("{md}"),
        }
    }

    // Exit 0 even with engine failures when some results exist (LLM tool
    // contract, PLAN.md §6 Step 4); 1 only when nothing was produced.
    if response.results.is_empty() && !response.engines_failed.is_empty() {
        1
    } else {
        0
    }
}

/// Build a query (used by the MCP server too).
pub fn build_query(
    config: &Config,
    registry: &Registry,
    query_text: &str,
    category: Option<Category>,
    limit: Option<usize>,
) -> Result<SearchQuery, TokenloomError> {
    let parsed = url_util::parse_bangs(query_text);
    let resolution = registry.resolve_bangs(&parsed);
    let mut query = SearchQuery::new(query_text);
    query.clean_query = parsed.clean_query;
    query.bang = parsed.bangs.first().map(|b| format!("!{b}"));
    query.category = resolution
        .category
        .or(category)
        .unwrap_or(Category::General);
    query.engines = resolution.engines;
    query.limit = limit.unwrap_or(config.general.default_limit).max(1);
    query.timeout = std::time::Duration::from_millis(config.general.timeout_ms);
    Ok(query)
}
