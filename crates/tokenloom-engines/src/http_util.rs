//! Shared HTTP helpers for engine implementations.

use crate::trait_def::EngineError;
use serde_json::Value;
use std::time::Duration;

/// GET a URL and parse the body as JSON.
pub async fn json_get(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    headers: &[(&str, &str)],
) -> Result<Value, EngineError> {
    let text = text_get(http, url, timeout, headers).await?;
    serde_json::from_str(&text).map_err(|e| EngineError::Parse(format!("invalid JSON: {e}")))
}

/// GET a URL and return the body text.
pub async fn text_get(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    headers: &[(&str, &str)],
) -> Result<String, EngineError> {
    let mut req = http.get(url).timeout(timeout);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| EngineError::Network(e.to_string()))?;
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(EngineError::RateLimited("HTTP 429".into()));
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(EngineError::Blocked(format!("HTTP {status}")));
    }
    if !status.is_success() {
        return Err(EngineError::Network(format!("HTTP {status}")));
    }
    resp.text()
        .await
        .map_err(|e| EngineError::Network(e.to_string()))
}

/// GET with query parameters appended.
pub async fn json_get_with_params(
    http: &reqwest::Client,
    base: &str,
    params: &[(&str, String)],
    timeout: Duration,
    headers: &[(&str, &str)],
) -> Result<Value, EngineError> {
    let mut url = reqwest::Url::parse(base).map_err(|e| EngineError::Parse(e.to_string()))?;
    {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in params {
            pairs.append_pair(k, v);
        }
    }
    json_get(http, url.as_str(), timeout, headers).await
}
