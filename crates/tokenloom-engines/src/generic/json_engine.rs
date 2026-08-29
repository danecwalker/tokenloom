//! Declarative JSON engine interpreter (PLAN.md §5.1): driven entirely by a
//! request spec (URL template + params + headers) and response spec (dotted
//! JSON paths). Powers ~30 registry engines.

use crate::spec::{render_template, EngineSpec, FieldSpec, RequestSpec};
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{SearchQuery, SearchResult};
use url::Url;

pub struct JsonEngine {
    spec: EngineSpec,
}

impl JsonEngine {
    pub fn new(spec: EngineSpec) -> Option<Self> {
        Some(Self { spec })
    }

    fn rendered_request(&self, query: &SearchQuery) -> Result<RequestSpec, EngineError> {
        let req = self
            .spec
            .request
            .as_ref()
            .ok_or_else(|| EngineError::Parse("missing request spec".into()))?
            .clone();
        let locale = query.locale.clone().unwrap_or_else(|| "en-US".into());
        Ok(RequestSpec {
            url: render_template(
                &req.url,
                &query.clean_query,
                query.page,
                &locale,
                query.safe_search,
                query.time_range.as_deref(),
            ),
            method: req.method,
            params: req
                .params
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        render_template(
                            &v,
                            &query.clean_query,
                            query.page,
                            &locale,
                            query.safe_search,
                            query.time_range.as_deref(),
                        ),
                    )
                })
                .collect(),
            headers: req.headers,
            body: req.body,
        })
    }

    pub fn extract(&self, json: &Value) -> Result<Vec<SearchResult>, EngineError> {
        let resp = self
            .spec
            .response
            .as_ref()
            .ok_or_else(|| EngineError::Parse("missing response spec".into()))?;
        let items: Vec<&Value> = match &resp.results_path {
            Some(path) => crate::json_path::get(json, path)
                .and_then(Value::as_array)
                .ok_or_else(|| EngineError::Parse(format!("results path '{path}' not found")))?
                .iter()
                .collect(),
            None => match json.as_array() {
                Some(a) => a.iter().collect(),
                None => {
                    return Err(EngineError::Parse(
                        "response is not an array and no results_path given".into(),
                    ))
                }
            },
        };

        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let title = field_string(item, &resp.title);
            let url = field_string(item, &resp.url);
            // An item without a usable URL/title pair is skipped.
            let Some(url) = url else { continue };
            let snippet = resp
                .snippet
                .as_ref()
                .and_then(|f| field_string(item, f))
                .unwrap_or_default();
            let thumbnail = resp.thumbnail.as_ref().and_then(|f| field_string(item, f));
            let date = resp.date.as_ref().and_then(|f| field_string(item, f));
            let mut metadata = std::collections::HashMap::new();
            for (k, f) in &resp.metadata {
                if let Some(v) = field_string(item, f) {
                    metadata.insert(k.clone(), v);
                }
            }
            let mut r = result(
                &self.spec.name,
                self.spec.categories[0],
                title.unwrap_or_default(),
                url,
                snippet,
            );
            r.published_date = date;
            r.thumbnail_url = thumbnail;
            r.metadata = metadata;
            out.push(r);
        }
        Ok(out)
    }
}

#[async_trait]
impl Engine for JsonEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let req = self.rendered_request(query)?;
        let mut url =
            Url::parse(&req.url).map_err(|e| EngineError::Parse(format!("bad url: {e}")))?;
        if !req.params.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &req.params {
                pairs.append_pair(k, v);
            }
        }
        let method = req.method.as_deref().unwrap_or("GET").to_uppercase();
        let mut http_req = match method.as_str() {
            "POST" => http.post(url),
            _ => http.get(url),
        }
        .timeout(self.timeout());
        for (k, v) in &req.headers {
            http_req = http_req.header(k.as_str(), v.as_str());
        }
        if let Some(body) = &req.body {
            http_req = http_req
                .body(body.clone())
                .header("Content-Type", "application/json");
        }
        http_req = http_req.header("Accept", "application/json");

        let resp = http_req
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(EngineError::RateLimited("HTTP 429".into()));
        }
        if !status.is_success() {
            return Err(EngineError::Network(format!("HTTP {status}")));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let json: Value =
            serde_json::from_str(&text).map_err(|e| EngineError::Parse(e.to_string()))?;
        self.extract(&json)
    }
}

/// Extract a field from a JSON item using a declarative [`FieldSpec`].
pub fn field_string(item: &Value, f: &FieldSpec) -> Option<String> {
    let raw = crate::json_path::get(item, &f.path).or_else(|| {
        f.fallback_path
            .as_deref()
            .and_then(|p| crate::json_path::get(item, p))
    })?;
    let mut s = match raw {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => return None,
        other => other.to_string(),
    };
    s = s.trim().to_string();
    if s.is_empty() {
        return None;
    }
    if let Some(prefix) = &f.prefix {
        s = format!("{prefix}{s}");
    }
    if f.strip_html {
        s = crate::html_util::strip_tags(&s);
    }
    Some(s)
}
