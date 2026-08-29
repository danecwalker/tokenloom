//! Federated search: parallel dispatch with per-engine timeouts, honest
//! failure reporting, URL-canonicalized dedup and Reciprocal Rank Fusion
//! (PLAN.md §5 *Deduplication & RRF*).

use crate::registry::Registry;
use crate::trait_def::EngineError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokenloom_core::{
    url_util, Category, EngineFailure, SearchQuery, SearchResponse, SearchResult,
};

/// Maximum engines queried in a single federated search.
pub const MAX_ENGINES_PER_QUERY: usize = 12;

pub struct Federator {
    registry: Arc<Registry>,
    client: reqwest::Client,
    weight_overrides: HashMap<String, f64>,
}

impl Federator {
    pub fn new(
        registry: Arc<Registry>,
        client: reqwest::Client,
        weight_overrides: HashMap<String, f64>,
    ) -> Self {
        Self {
            registry,
            client,
            weight_overrides,
        }
    }

    /// Select engines for the query (explicit engines > category set).
    fn select_engines(&self, query: &SearchQuery) -> (Vec<String>, Vec<EngineFailure>) {
        let mut selected: Vec<String> = Vec::new();
        let mut failures: Vec<EngineFailure> = Vec::new();

        for name in &query.engines {
            match self.registry.get(name) {
                None => failures.push(EngineFailure {
                    engine: name.clone(),
                    error: format!("unknown engine '{name}'"),
                    is_rate_limited: false,
                }),
                Some(spec) if !self.registry.is_implemented(name) => failures.push(EngineFailure {
                    engine: name.clone(),
                    error: format!(
                        "engine '{name}' is registered but not implemented yet (wave {})",
                        spec.wave
                    ),
                    is_rate_limited: false,
                }),
                Some(_) => selected.push(name.clone()),
            }
        }

        let cap = query.max_engines.unwrap_or(MAX_ENGINES_PER_QUERY);
        if selected.is_empty() {
            let category = query.category;
            for spec in self.registry.engines_for_category(category, false) {
                if selected.len() >= cap {
                    break;
                }
                selected.push(spec.name.clone());
            }
        } else {
            selected.truncate(cap);
        }
        (selected, failures)
    }

    /// Run the federated search and fuse results with RRF (k = 60).
    pub async fn search(&self, query: &SearchQuery) -> SearchResponse {
        let started = Instant::now();
        let (engine_names, mut failures) = self.select_engines(query);

        if engine_names.is_empty() {
            return SearchResponse {
                query: query.raw_query.clone(),
                category: query.category,
                bang: query.bang.clone(),
                results: vec![],
                total_results: 0,
                engines_queried: vec![],
                engines_failed: failures,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
        }

        // Dispatch all engines in parallel; each is capped by its own
        // registry timeout (PLAN.md §5).
        let mut handles = Vec::with_capacity(engine_names.len());
        for name in &engine_names {
            let Some(engine) = self.registry.build(name) else {
                continue;
            };
            let query = query.clone();
            let client = self.client.clone();
            let timeout = engine.timeout();
            handles.push(tokio::spawn(async move {
                let result = tokio::time::timeout(timeout, engine.search(&query, &client)).await;
                (engine.name().to_string(), result)
            }));
        }

        let mut engines_queried = Vec::new();
        let mut per_engine: Vec<(String, Result<Vec<SearchResult>, EngineError>)> = Vec::new();
        for handle in handles {
            let Ok((name, outcome)) = handle.await else {
                failures.push(EngineFailure {
                    engine: "<unknown>".into(),
                    error: "engine task panicked".into(),
                    is_rate_limited: false,
                });
                continue;
            };
            match outcome {
                Ok(Ok(results)) => {
                    if !results.is_empty() {
                        engines_queried.push(name.clone());
                        per_engine.push((name, Ok(results)));
                    } else {
                        engines_queried.push(name.clone());
                    }
                }
                Ok(Err(e)) => failures.push(EngineFailure {
                    is_rate_limited: matches!(
                        e,
                        EngineError::RateLimited(_) | EngineError::Blocked(_)
                    ),
                    engine: name.clone(),
                    error: e.to_string(),
                }),
                Err(_) => failures.push(EngineFailure {
                    engine: name.clone(),
                    error: "engine timed out".to_string(),
                    is_rate_limited: false,
                }),
            }
        }

        let results = fuse(
            &per_engine,
            &self.weight_overrides,
            query.category,
            query.limit,
        );
        failures.retain(|f| !engines_queried.contains(&f.engine));
        engines_queried.sort();
        failures.sort_by(|a, b| a.engine.cmp(&b.engine));

        SearchResponse {
            query: query.raw_query.clone(),
            category: query.category,
            bang: query.bang.clone(),
            total_results: results.len(),
            results,
            engines_queried,
            engines_failed: failures,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }
}

/// Reciprocal Rank Fusion (PLAN.md §5):
/// Score(d) = Σ_e w_e / (k + rank_e(d)) with k = 60.
pub fn fuse(
    per_engine: &[(String, Result<Vec<SearchResult>, EngineError>)],
    weight_overrides: &HashMap<String, f64>,
    category: Category,
    limit: usize,
) -> Vec<SearchResult> {
    const K: f64 = 60.0;

    struct Group {
        best: SearchResult,
        score: f64,
        sources: Vec<String>,
        snippet: String,
    }

    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (engine_name, outcome) in per_engine {
        let Ok(results) = outcome else { continue };
        let weight = weight_overrides.get(engine_name).copied().unwrap_or(1.0);
        for (rank, r) in results.iter().enumerate() {
            let key =
                url_util::canonicalize_url(&r.url).unwrap_or_else(|| format!("nourl:{}", r.title));
            let contribution = weight / (K + (rank as f64 + 1.0));
            if let Some(group) = groups.get_mut(&key) {
                group.score += contribution;
                if !group.sources.contains(engine_name) {
                    group.sources.push(engine_name.clone());
                }
                if r.snippet.len() > group.snippet.len() {
                    group.snippet = r.snippet.clone();
                    group.best.snippet = r.snippet.clone();
                }
                // Prefer the richer title.
                if r.title.len() > group.best.title.len() && !r.title.is_empty() {
                    group.best.title = r.title.clone();
                }
            } else {
                let mut r = r.clone();
                r.category = category;
                let sources = vec![engine_name.clone()];
                let snippet = r.snippet.clone();
                groups.insert(
                    key.clone(),
                    Group {
                        best: r,
                        score: contribution,
                        sources,
                        snippet,
                    },
                );
                order.push(key);
            }
        }
    }

    let mut fused: Vec<SearchResult> = order
        .iter()
        .filter_map(|key| groups.remove(key))
        .map(|mut g| {
            g.best.score = (g.score * 100.0).round() / 100.0;
            g.best
                .metadata
                .insert("sources".into(), g.sources.join(","));
            g.best
        })
        .collect();
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(engine: &str, title: &str, url: &str) -> SearchResult {
        result_for_test(engine, title, url)
    }

    fn result_for_test(engine: &str, title: &str, url: &str) -> SearchResult {
        SearchResult {
            title: title.into(),
            url: url.into(),
            snippet: format!("snippet from {engine}"),
            engine: engine.into(),
            category: Category::General,
            score: 0.0,
            published_date: None,
            thumbnail_url: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn rrf_ranks_multi_engine_hits_first() {
        let per_engine = vec![
            (
                "a".into(),
                Ok(vec![
                    r("a", "Shared", "https://x.co/1"),
                    r("a", "Only A", "https://x.co/2"),
                ]),
            ),
            (
                "b".into(),
                Ok(vec![
                    r("b", "Shared B", "https://x.co/1"),
                    r("b", "Only B", "https://x.co/3"),
                ]),
            ),
        ];
        let fused = fuse(&per_engine, &HashMap::new(), Category::General, 10);
        assert_eq!(fused[0].url, "https://x.co/1");
        assert!(fused[0].score > fused[1].score);
        assert_eq!(fused[0].metadata["sources"], "a,b");
        // Snippet merging keeps the longest snippet seen for the URL.
        assert!(!fused[0].snippet.is_empty());
        // Canonicalization: www + tracking params collapse.
        let per_engine = vec![(
            "a".into(),
            Ok(vec![
                r("a", "U1", "https://www.x.co/1?utm_source=t"),
                r("a", "U2", "https://x.co/1"),
            ]),
        )];
        let fused = fuse(&per_engine, &HashMap::new(), Category::General, 10);
        assert_eq!(fused.len(), 1, "tracking/www variants must dedup");
    }

    #[test]
    fn weights_modify_rrf() {
        let mut weights = HashMap::new();
        weights.insert("heavy".to_string(), 5.0);
        let per_engine = vec![
            (
                "heavy".into(),
                Ok(vec![
                    r("heavy", "H1", "https://h.co/1"),
                    r("heavy", "H2", "https://h.co/2"),
                ]),
            ),
            ("light".into(), Ok(vec![r("light", "L1", "https://l.co/1")])),
        ];
        let fused = fuse(&per_engine, &weights, Category::General, 10);
        // heavy's #1 (5/61) outranks light's #1 (1/61) and heavy #2 (5/62).
        assert_eq!(fused[0].url, "https://h.co/1");
    }
}
