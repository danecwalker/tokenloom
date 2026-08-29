//! StackExchange family (stackoverflow, askubuntu, superuser) via REST API
//! v2.3 (PLAN.md §5.3).

use crate::html_util::decode_entities;
use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct StackExchangeEngine {
    spec: EngineSpec,
    site: &'static str,
}

impl StackExchangeEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let site = match spec.name.as_str() {
            "askubuntu" => "askubuntu",
            "superuser" => "superuser",
            _ => "stackoverflow",
        };
        Self { spec, site }
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
impl Engine for StackExchangeEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let json = json_get_with_params(
            http,
            "https://api.stackexchange.com/2.3/search/advanced",
            &[
                ("order", "desc".into()),
                ("sort", "relevance".into()),
                ("q", query.clean_query.clone()),
                ("site", self.site.into()),
                ("pagesize", "15".into()),
                ("page", query.page.to_string()),
                ("filter", "!nNPvSNdWme".into()), // includes excerpt-ish body marker
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        if let Some(items) = json.get("items").and_then(Value::as_array) {
            for item in items {
                let Some(title) = item.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let Some(url) = item.get("link").and_then(Value::as_str) else {
                    continue;
                };
                let score = item.get("score").and_then(Value::as_i64).unwrap_or(0);
                let answers = item
                    .get("answer_count")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let answered = item
                    .get("is_answered")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut r = result(
                    &self.spec.name,
                    self.category(),
                    decode_entities(title),
                    url,
                    String::new(),
                );
                r.metadata.insert("score".into(), score.to_string());
                r.metadata.insert("answers".into(), answers.to_string());
                if answered {
                    r.metadata.insert("answered".into(), "true".into());
                }
                if let Some(ts) = item.get("creation_date").and_then(Value::as_i64) {
                    r.published_date = Some(unix_to_date(ts));
                }
                out.push(r);
            }
        }
        Ok(out)
    }
}

/// Format a unix timestamp as `YYYY-MM-DD`.
pub fn unix_to_date(ts: i64) -> String {
    // Days since epoch → civil date (Howard Hinnant's algorithm).
    let days = ts.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
