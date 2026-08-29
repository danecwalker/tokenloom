//! Qwant specialist via the v3 JSON API (api.qwant.com) — web / images /
//! videos / news (PLAN.md §5.4). Unauthenticated requests require
//! browser-like headers and are frequently 403'd; reported honestly.

use crate::http_util::text_get;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct QwantEngine {
    spec: EngineSpec,
    kind: &'static str,
    category: Category,
}

impl QwantEngine {
    pub fn new(spec: EngineSpec) -> Self {
        match spec.name.as_str() {
            "qwant_images" => Self {
                kind: "images",
                category: Category::Images,
                spec,
            },
            "qwant_videos" => Self {
                kind: "videos",
                category: Category::Videos,
                spec,
            },
            "qwant_news" => Self {
                kind: "news",
                category: Category::News,
                spec,
            },
            _ => Self {
                kind: "web",
                category: Category::General,
                spec,
            },
        }
    }
}

#[async_trait]
impl Engine for QwantEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let locale = query.locale.clone().unwrap_or_else(|| "en_US".into());
        let url = format!(
            "https://api.qwant.com/v3/search/{}?count=15&locale={}&offset={}&q={}&device=desktop&safesearch=1",
            self.kind,
            locale,
            (query.page.saturating_sub(1)) * 15,
            crate::spec::urlencoding_lite(&query.clean_query),
        );
        let headers = [
            ("Referer", "https://www.qwant.com/"),
            ("Accept", "application/json"),
        ];
        let text = text_get(
            http,
            &url,
            Duration::from_millis(self.spec.timeout_ms),
            &headers,
        )
        .await?;
        let json: Value =
            serde_json::from_str(&text).map_err(|e| EngineError::Parse(e.to_string()))?;
        if json.get("status").and_then(Value::as_str) != Some("success") {
            return Err(EngineError::Blocked("API status != success".into()));
        }

        let mut out = Vec::new();
        let mainline = json
            .pointer("/data/result/items/mainline")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for group in &mainline {
            let group_type = group.get("type").and_then(Value::as_str).unwrap_or("");
            if !group_type.is_empty()
                && group_type != self.kind
                && !(self.kind == "web" && group_type == "web")
            {
                continue;
            }
            let Some(items) = group.get("items").and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                let Some(title) = item.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let Some(url) = item.get("url").and_then(Value::as_str) else {
                    continue;
                };
                let mut r = result(
                    &self.spec.name,
                    self.category,
                    title,
                    url,
                    item.get("desc")
                        .or_else(|| item.get("description"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                r.thumbnail_url = item
                    .get("media")
                    .or_else(|| item.get("thumbnail"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(src) = item
                    .get("source")
                    .and_then(|s| s.get("name"))
                    .and_then(Value::as_str)
                {
                    r.metadata.insert("source".into(), src.into());
                }
                out.push(r);
                if out.len() >= 15 {
                    break;
                }
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse("no results in mainline".into()));
        }
        Ok(out)
    }
}
