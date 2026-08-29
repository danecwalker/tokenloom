//! Discourse forum family (caddy.community, discuss.python, pi-hole.community)
//! via `/search.json` (PLAN.md §5.3).

use crate::html_util::{decode_entities, strip_tags};
use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct DiscourseEngine {
    spec: EngineSpec,
    base: &'static str,
}

impl DiscourseEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let base = match spec.name.as_str() {
            "caddy.community" => "https://caddy.community",
            "discuss.python" => "https://discuss.python.org",
            "pi-hole.community" => "https://pi-hole.community",
            _ => "https://meta.discourse.org",
        };
        Self { spec, base }
    }

    fn category(&self) -> Category {
        self.spec
            .categories
            .first()
            .copied()
            .unwrap_or(Category::It)
    }
}

#[async_trait]
impl Engine for DiscourseEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!("{}/search.json", self.base);
        let json = json_get_with_params(
            http,
            &url,
            &[
                ("q", query.clean_query.clone()),
                ("page", query.page.to_string()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        // topics: id → (title, slug)
        let mut topics: HashMap<i64, (String, String)> = HashMap::new();
        if let Some(arr) = json.get("topics").and_then(Value::as_array) {
            for t in arr {
                let id = t.get("id").and_then(Value::as_i64).unwrap_or(0);
                let title = t
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let slug = t
                    .get("slug")
                    .and_then(Value::as_str)
                    .unwrap_or("topic")
                    .to_string();
                topics.insert(id, (title, slug));
            }
        }

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Some(posts) = json.get("posts").and_then(Value::as_array) {
            for p in posts {
                let topic_id = p.get("topic_id").and_then(Value::as_i64).unwrap_or(0);
                if !seen.insert(topic_id) {
                    continue;
                }
                let Some((title, slug)) = topics.get(&topic_id) else {
                    continue;
                };
                let url = format!("{}/t/{slug}/{topic_id}", self.base);
                let blurb = p.get("blurb").and_then(Value::as_str).unwrap_or("");
                let r = result(
                    &self.spec.name,
                    self.category(),
                    decode_entities(title),
                    url,
                    decode_entities(&strip_tags(blurb)),
                );
                out.push(r);
                if out.len() >= 15 {
                    break;
                }
            }
        }
        Ok(out)
    }
}
