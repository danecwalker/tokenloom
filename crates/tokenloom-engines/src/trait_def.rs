//! Engine trait & capabilities (PLAN.md §5, *Engine Trait & Capabilities*).
//!
//! Every engine — family interpreter or declarative spec — implements this
//! async trait; the federation layer dispatches through `Box<dyn Engine>`.

use crate::spec::EngineSpec;
use async_trait::async_trait;
use std::fmt;
use tokenloom_core::{SearchQuery, SearchResult};

/// Feature capabilities advertised by an engine (mirrors the ✓/— columns of
/// PLAN.md Appendix A).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineCapabilities {
    pub paging: bool,
    pub locale: bool,
    pub safe_search: bool,
    pub time_range: bool,
    pub requires_api_key: bool,
}

/// Errors produced by a single engine query. The federation layer tolerates
/// them and reports each engine's status honestly (PLAN.md §15).
#[derive(Debug, Clone)]
pub enum EngineError {
    Network(String),
    RateLimited(String),
    Blocked(String),
    Parse(String),
    MissingApiKey,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Network(m) => write!(f, "network error: {m}"),
            EngineError::RateLimited(m) => write!(f, "rate limited: {m}"),
            EngineError::Blocked(m) => write!(f, "blocked by engine: {m}"),
            EngineError::Parse(m) => write!(f, "parse error: {m}"),
            EngineError::MissingApiKey => write!(f, "API key required but not configured"),
        }
    }
}

impl std::error::Error for EngineError {}

/// An executable search engine bound to its spec.
#[async_trait]
pub trait Engine: Send + Sync {
    /// The engine's registry spec (name, bang, family, capabilities…).
    fn spec(&self) -> &EngineSpec;

    /// Engine identifier.
    fn name(&self) -> &str {
        &self.spec().name
    }

    /// Primary bang shortcut (e.g. "ddg").
    fn bang(&self) -> &str {
        &self.spec().bang
    }

    fn capabilities(&self) -> EngineCapabilities {
        let s = self.spec();
        EngineCapabilities {
            paging: s.paging,
            locale: s.locale,
            safe_search: s.safe_search,
            time_range: s.time_range,
            requires_api_key: false,
        }
    }

    /// Per-engine timeout from the SearXNG-derived registry.
    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.spec().timeout_ms)
    }

    fn weight(&self) -> f64 {
        self.spec().weight
    }

    fn is_enabled_by_default(&self) -> bool {
        self.spec().enabled
    }

    /// Execute the query. Implementations receive the SSRF-guarded shared
    /// HTTP client and must not spawn their own transport.
    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError>;
}

/// Convenience: build a `SearchResult` with the engine/category defaults.
pub fn result(
    engine: &str,
    category: tokenloom_core::Category,
    title: impl Into<String>,
    url: impl Into<String>,
    snippet: impl Into<String>,
) -> SearchResult {
    SearchResult {
        title: title.into(),
        url: url.into(),
        snippet: snippet.into(),
        engine: engine.to_string(),
        category,
        score: 0.0,
        published_date: None,
        thumbnail_url: None,
        metadata: Default::default(),
    }
}
