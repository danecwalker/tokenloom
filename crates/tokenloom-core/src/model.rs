//! Core data model (PLAN.md §4).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// The 10 SearXNG category tabs (PLAN.md §2, Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

impl Category {
    pub const ALL: [Category; 10] = [
        Category::General,
        Category::Images,
        Category::Videos,
        Category::News,
        Category::Map,
        Category::Music,
        Category::It,
        Category::Science,
        Category::Files,
        Category::SocialMedia,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Category::General => "general",
            Category::Images => "images",
            Category::Videos => "videos",
            Category::News => "news",
            Category::Map => "map",
            Category::Music => "music",
            Category::It => "it",
            Category::Science => "science",
            Category::Files => "files",
            Category::SocialMedia => "social_media",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Category> {
        let norm = s.trim().to_lowercase().replace('-', "_");
        Some(match norm.as_str() {
            "general" => Category::General,
            "images" => Category::Images,
            "videos" => Category::Videos,
            "news" => Category::News,
            "map" | "maps" => Category::Map,
            "music" => Category::Music,
            "it" => Category::It,
            "science" => Category::Science,
            "files" => Category::Files,
            "social_media" | "social media" => Category::SocialMedia,
            _ => return None,
        })
    }

    /// Canonical `!bang` for the category (PLAN.md §5, *Bangs & Category Routing*).
    pub fn bang(&self) -> &'static str {
        match self {
            Category::General => "!general",
            Category::Images => "!images",
            Category::Videos => "!videos",
            Category::News => "!news",
            Category::Map => "!map",
            Category::Music => "!music",
            Category::It => "!it",
            Category::Science => "!science",
            Category::Files => "!files",
            Category::SocialMedia => "!social_media",
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Category {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Category {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Category::from_str(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown category '{s}'")))
    }
}

/// A fully-resolved federated search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub raw_query: String,
    pub clean_query: String,
    pub bang: Option<String>,
    pub category: Category,
    pub engines: Vec<String>,
    pub page: u32,
    pub locale: Option<String>,
    /// 0 = off, 1 = moderate, 2 = strict (PLAN.md §4).
    pub safe_search: u8,
    /// day, week, month, year
    pub time_range: Option<String>,
    pub limit: usize,
    pub timeout: Duration,
}

impl SearchQuery {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw_query = raw.into();
        Self {
            raw_query: raw_query.clone(),
            clean_query: raw_query,
            bang: None,
            category: Category::General,
            engines: Vec::new(),
            page: 1,
            locale: None,
            safe_search: 1,
            time_range: None,
            limit: 10,
            timeout: Duration::from_millis(4000),
        }
    }
}

/// One federated result after deduplication and RRF ranking.
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
    pub metadata: HashMap<String, String>,
}

/// Per-engine failure reported honestly in output (PLAN.md §4, §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineFailure {
    pub engine: String,
    pub error: String,
    pub is_rate_limited: bool,
}

/// Top-level federated search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub category: Category,
    /// Bang(s) that were resolved for this query, e.g. "!arx".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bang: Option<String>,
    pub results: Vec<SearchResult>,
    pub total_results: usize,
    pub engines_queried: Vec<String>,
    pub engines_failed: Vec<EngineFailure>,
    pub elapsed_ms: u64,
}

/// How a page's Markdown was ultimately produced (PLAN.md §4, §6).
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

impl RenderMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            RenderMethod::StaticDirect => "StaticDirect",
            RenderMethod::JinaReader => "JinaReader",
            RenderMethod::LocalHeadless => "LocalHeadless",
            RenderMethod::DegradedStatic => "DegradedStatic",
            RenderMethod::Cache => "Cache",
        }
    }
}

/// Approximate token count for LLM budgets (~4 chars/token heuristic with a
/// word-boundary refinement; PLAN.md §8 *token_budget*).
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count() as f64;
    let words = text.split_whitespace().count() as f64;
    if text.is_empty() {
        0
    } else {
        // Blend: chars/4 tracks prose+markup, words*1.33 bounds it for
        // symbol-dense text; the average stays close to real tokenizers.
        ((chars / 4.0) * 0.6 + (words * 1.33) * 0.4) as usize
    }
}

/// A fetched page converted to LLM-ready Markdown (PLAN.md §4, §6).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_roundtrip() {
        for c in Category::ALL {
            assert_eq!(Category::from_str(c.as_str()), Some(c));
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
            assert_eq!(serde_json::to_string(&c).unwrap(), json);
        }
        assert_eq!(
            Category::from_str("Social-Media"),
            Some(Category::SocialMedia)
        );
        assert_eq!(Category::from_str("bogus"), None);
    }

    #[test]
    fn render_method_names() {
        let m: RenderMethod = serde_json::from_str("\"JinaReader\"").unwrap();
        assert_eq!(m, RenderMethod::JinaReader);
        assert_eq!(m.as_str(), "JinaReader");
    }

    #[test]
    fn search_response_json_shape() {
        let resp = SearchResponse {
            query: "quantum error correction".into(),
            category: Category::Science,
            bang: Some("!arx".into()),
            results: vec![SearchResult {
                title: "Fault-Tolerant Quantum Computation".into(),
                url: "https://arxiv.org/abs/2401.00000".into(),
                snippet: "We present a unified threshold analysis".into(),
                engine: "arxiv".into(),
                category: Category::Science,
                score: 1.0,
                published_date: Some("2026-01-15".into()),
                thumbnail_url: None,
                metadata: HashMap::from([("arxiv_id".into(), "2401.00000".into())]),
            }],
            total_results: 1,
            engines_queried: vec!["arxiv".into()],
            engines_failed: vec![],
            elapsed_ms: 184,
        };
        let v: serde_json::Value = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["category"], "science");
        assert_eq!(v["bang"], "!arx");
        assert_eq!(v["results"][0]["engine"], "arxiv");
        assert_eq!(v["engines_queried"][0], "arxiv");
        assert_eq!(v["elapsed_ms"], 184);
    }
}
