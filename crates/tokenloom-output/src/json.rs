//! Stable JSON v1 serialization for search and fetch outputs (PLAN.md §8
//! example 2).

use serde_json::{json, Value};
use tokenloom_core::{FetchedPage, SearchResponse};

/// Serialize a search response to the stable v1 JSON schema.
pub fn search_response_to_json(resp: &SearchResponse) -> Value {
    json!({
        "schema": "tokenloom/v1",
        "query": resp.query,
        "category": resp.category.as_str(),
        "bang": resp.bang,
        "results": resp.results.iter().map(|r| {
            json!({
                "title": r.title,
                "url": r.url,
                "snippet": r.snippet,
                "engine": r.engine,
                "category": r.category.as_str(),
                "score": r.score,
                "published_date": r.published_date,
                "thumbnail_url": r.thumbnail_url,
                "metadata": r.metadata,
            })
        }).collect::<Vec<_>>(),
        "total_results": resp.total_results,
        "engines_queried": resp.engines_queried,
        "engines_failed": resp.engines_failed.iter().map(|f| {
            json!({
                "engine": f.engine,
                "error": f.error,
                "is_rate_limited": f.is_rate_limited,
            })
        }).collect::<Vec<_>>(),
        "elapsed_ms": resp.elapsed_ms,
    })
}

/// Serialize a fetched page (adds the fetch-specific envelope).
pub fn fetched_page_to_json(page: &FetchedPage) -> Value {
    let mut v = serde_json::to_value(page).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("schema".into(), json!("tokenloom/v1"));
        obj.insert("render_method".into(), json!(page.render_method.as_str()));
    }
    v
}
