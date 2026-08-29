//! Lemmy family (posts / comments / communities / users) via API v3
//! (PLAN.md §5.3).

use crate::html_util::strip_tags;
use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct LemmyEngine {
    spec: EngineSpec,
    kind: &'static str,
    base: &'static str,
}

impl LemmyEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let kind = match spec.name.as_str() {
            "lemmy_comments" => "Comments",
            "lemmy_communities" => "Communities",
            "lemmy_users" => "Users",
            _ => "Posts",
        };
        Self {
            kind,
            base: "https://lemmy.ml",
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
impl Engine for LemmyEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!("{}/api/v3/search", self.base);
        let json = json_get_with_params(
            http,
            &url,
            &[
                ("q", query.clean_query.clone()),
                ("type_", self.kind.into()),
                ("sort", "TopAll".into()),
                ("limit", "15".into()),
                ("page", query.page.to_string()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        match self.kind {
            "Posts" => {
                for post in json
                    .get("posts")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                {
                    let (Some(p), Some(community)) = (
                        post.get("post"),
                        post.get("community")
                            .and_then(|c| c.get("name"))
                            .and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    let Some(title) = p.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = p.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let url = p
                        .get("url")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{}/post/{id}", self.base));
                    let body = p.get("body").and_then(Value::as_str).unwrap_or("");
                    let mut r = result(
                        &self.spec.name,
                        self.category(),
                        title,
                        url,
                        strip_tags(body),
                    );
                    r.metadata.insert("community".into(), community.into());
                    out.push(r);
                }
            }
            "Comments" => {
                for c in json
                    .get("comments")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                {
                    let Some(comment) = c.get("comment") else {
                        continue;
                    };
                    let Some(content) = comment.get("content").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = comment.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let r = result(
                        &self.spec.name,
                        self.category(),
                        format!("Comment #{id}"),
                        format!("{}/comment/{id}", self.base),
                        strip_tags(content),
                    );
                    out.push(r);
                }
            }
            "Communities" => {
                for c in json
                    .get("communities")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                {
                    let Some(community) = c.get("community") else {
                        continue;
                    };
                    let Some(name) = community.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let title = community
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(name);
                    let desc = community
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let r = result(
                        &self.spec.name,
                        self.category(),
                        format!("{title} ({name})"),
                        format!("{}/c/{name}", self.base),
                        strip_tags(desc),
                    );
                    out.push(r);
                }
            }
            _ => {
                for u in json
                    .get("users")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                {
                    let Some(person) = u.get("person") else {
                        continue;
                    };
                    let Some(name) = person.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let display = person
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(name);
                    let bio = person.get("bio").and_then(Value::as_str).unwrap_or("");
                    let r = result(
                        &self.spec.name,
                        self.category(),
                        format!("{display} (@{name})"),
                        format!("{}/u/{name}", self.base),
                        strip_tags(bio),
                    );
                    out.push(r);
                }
            }
        }
        Ok(out)
    }
}
