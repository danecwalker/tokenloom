//! HuggingFace family (models / datasets / spaces) via the Hub API
//! (PLAN.md §5.3).

use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct HfEngine {
    spec: EngineSpec,
    kind: &'static str,
}

impl HfEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let kind = match spec.name.as_str() {
            "huggingface_datasets" => "datasets",
            "huggingface_spaces" => "spaces",
            _ => "models",
        };
        Self { spec, kind }
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
impl Engine for HfEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!("https://huggingface.co/api/{}", self.kind);
        let json = json_get_with_params(
            http,
            &url,
            &[
                ("search", query.clean_query.clone()),
                ("limit", "15".into()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        let Some(items) = json.as_array() else {
            return Ok(out);
        };
        for item in items {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                continue;
            };
            let likes = item.get("likes").and_then(Value::as_i64).unwrap_or(0);
            let downloads = item.get("downloads").and_then(Value::as_i64);
            let pipeline = item.get("pipeline_tag").and_then(Value::as_str);
            let mut r = result(
                &self.spec.name,
                self.category(),
                id,
                format!("https://huggingface.co/{id}"),
                String::new(),
            );
            r.metadata.insert("likes".into(), likes.to_string());
            if let Some(d) = downloads {
                r.metadata.insert("downloads".into(), d.to_string());
            }
            if let Some(p) = pipeline {
                r.metadata.insert("pipeline_tag".into(), p.into());
            }
            out.push(r);
        }
        Ok(out)
    }
}
