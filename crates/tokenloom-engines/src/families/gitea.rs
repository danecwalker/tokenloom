//! Gitea / Codeberg family via `/api/v1/repos/search` (PLAN.md §5.3).

use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct GiteaEngine {
    spec: EngineSpec,
    base: &'static str,
}

impl GiteaEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let base = match spec.name.as_str() {
            "gitea.com" => "https://gitea.com",
            _ => "https://codeberg.org",
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
impl Engine for GiteaEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!("{}/api/v1/repos/search", self.base);
        let json = json_get_with_params(
            http,
            &url,
            &[
                ("q", query.clean_query.clone()),
                ("limit", "15".into()),
                ("page", query.page.to_string()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        if let Some(items) = json.get("data").and_then(Value::as_array) {
            for item in items {
                let Some(name) = item.get("full_name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(url) = item.get("html_url").and_then(Value::as_str) else {
                    continue;
                };
                let desc = item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut r = result(&self.spec.name, self.category(), name, url, desc);
                if let Some(stars) = item.get("stars_count").and_then(Value::as_i64) {
                    r.metadata.insert("stars".into(), stars.to_string());
                }
                out.push(r);
            }
        }
        Ok(out)
    }
}
