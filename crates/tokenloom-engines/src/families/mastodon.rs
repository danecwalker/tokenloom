//! Mastodon family (hashtags / users) via API v2 search (PLAN.md §5.3).

use crate::html_util::strip_tags;
use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct MastodonEngine {
    spec: EngineSpec,
    hashtags: bool,
    base: &'static str,
}

impl MastodonEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self {
            hashtags: spec.name == "mastodon_hashtags",
            base: "https://mastodon.social",
            spec,
        }
    }

    fn category(&self) -> Category {
        self.spec
            .categories
            .first()
            .copied()
            .unwrap_or(Category::SocialMedia)
    }
}

#[async_trait]
impl Engine for MastodonEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!("{}/api/v2/search", self.base);
        let type_ = if self.hashtags {
            "hashtags"
        } else {
            "accounts"
        };
        let json = json_get_with_params(
            http,
            &url,
            &[
                ("q", query.clean_query.clone()),
                ("type", type_.into()),
                ("resolve", "false".into()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        if self.hashtags {
            for h in json
                .get("hashtags")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
            {
                let Some(name) = h.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let url = h
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{}/tags/{name}", self.base));
                let uses = h
                    .get("history")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|d| d.get("uses"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let mut r = result(
                    &self.spec.name,
                    self.category(),
                    format!("#{name}"),
                    url,
                    String::new(),
                );
                if let Some(uses) = uses {
                    r.metadata.insert("recent_uses".into(), uses);
                }
                out.push(r);
            }
        } else {
            for a in json
                .get("accounts")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
            {
                let Some(username) = a.get("username").and_then(Value::as_str) else {
                    continue;
                };
                let Some(url) = a.get("url").and_then(Value::as_str) else {
                    continue;
                };
                let display = a
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(username);
                let note = strip_tags(a.get("note").and_then(Value::as_str).unwrap_or(""));
                let r = result(
                    &self.spec.name,
                    self.category(),
                    format!("{display} (@{username})"),
                    url,
                    note,
                );
                out.push(r);
            }
        }
        Ok(out)
    }
}
