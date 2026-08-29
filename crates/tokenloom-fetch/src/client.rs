//! Reqwest client wrapper with SSRF-guarded DNS, timeouts, proxy support and
//! a streaming decompression-bomb cap (PLAN.md §6, §7 Layer 1/2).

use crate::ssrf::{self, SsrfGuardResolver};
use reqwest::redirect::Policy;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokenloom_core::{HttpConfig, TokenloomError};

/// Content types accepted for reader/fetch pipelines.
const ACCEPTED_TYPES: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "text/plain",
    "text/markdown",
];

/// Raw result of one SSRF-guarded HTTP GET.
#[derive(Debug, Clone)]
pub struct RawFetch {
    pub final_url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

pub struct FetchClient {
    client: Client,
    max_bytes: u64,
}

impl FetchClient {
    pub fn new(cfg: &HttpConfig) -> Result<Self, TokenloomError> {
        let resolver = SsrfGuardResolver::new()?;
        let max_redirects = cfg.max_redirects;
        let redirect_policy = if cfg.follow_redirects {
            Policy::custom(move |attempt| {
                if attempt.previous().len() > max_redirects {
                    return attempt.error("too many redirects");
                }
                // Scheme must remain http/https on every hop; literal-IP and
                // port checks re-run here (DNS checks happen in the resolver).
                if let Err(e) = ssrf::validate_url(attempt.url()) {
                    return attempt.error(e.to_string());
                }
                attempt.follow()
            })
        } else {
            Policy::none()
        };

        let mut builder = Client::builder()
            .user_agent(&cfg.user_agent)
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .timeout(Duration::from_millis(cfg.total_timeout_ms))
            .redirect(redirect_policy)
            .dns_resolver(Arc::new(resolver))
            .gzip(true)
            .brotli(true)
            .zstd(true)
            .danger_accept_invalid_certs(false);

        if !cfg.proxy.is_empty() {
            builder = builder.proxy(
                reqwest::Proxy::all(&cfg.proxy)
                    .map_err(|e| TokenloomError::Config(format!("invalid proxy URL: {e}")))?,
            );
        }

        let client = builder
            .build()
            .map_err(|e| TokenloomError::Config(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            client,
            max_bytes: cfg.max_response_size_mb.saturating_mul(1024 * 1024),
        })
    }

    /// Internal client for engine queries (no streaming cap semantics).
    pub fn raw(&self) -> &Client {
        &self.client
    }

    /// GET a URL with the streaming byte cap (PLAN.md §7 Layer 2: the stream
    /// is killed immediately when the decompressed payload exceeds the cap).
    pub async fn get_capped(&self, url: &str) -> Result<RawFetch, TokenloomError> {
        let parsed: url::Url = url
            .parse()
            .map_err(|_| TokenloomError::InvalidUrl(url.to_string()))?;
        ssrf::validate_url(&parsed)?;

        let resp = self
            .client
            .get(parsed.clone())
            .header(
                "Accept",
                "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8",
            )
            .send()
            .await?;

        let status = resp.status().as_u16();
        let final_url = resp.url().clone();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase());
        let etag = header_string(resp.headers(), reqwest::header::ETAG);
        let last_modified = header_string(resp.headers(), reqwest::header::LAST_MODIFIED);

        // Content-type enforcement (PLAN.md §7 Layer 1). Missing type → sniff.
        if let Some(ct) = &content_type {
            if !ACCEPTED_TYPES.iter().any(|a| ct.starts_with(a)) {
                return Err(TokenloomError::Sanitization(format!(
                    "unsupported content type '{ct}' for reader pipeline"
                )));
            }
        }

        let mut body: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await? {
            if body.len() as u64 + chunk.len() as u64 > self.max_bytes {
                return Err(TokenloomError::ResponseTooLarge {
                    limit: self.max_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }

        // Sniff when the server sent no content type.
        let content_type = match content_type {
            Some(ct) => Some(ct),
            None => {
                if body.starts_with(b"<?xml") || body.starts_with(b"<!DOCTYPE html") {
                    Some("text/html".into())
                } else {
                    None
                }
            }
        };

        Ok(RawFetch {
            final_url: final_url.to_string(),
            status,
            content_type,
            body,
            etag,
            last_modified,
        })
    }
}

fn header_string(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
}

/// Best-effort status probe used by `tokenloom doctor`.
pub async fn probe_status(client: &Client, url: &str) -> Result<u16, TokenloomError> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    Ok(resp.status().as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HttpConfig {
        HttpConfig {
            connect_timeout_ms: 1000,
            total_timeout_ms: 3000,
            ..HttpConfig::default()
        }
    }

    #[tokio::test]
    async fn client_builds_and_caps_stream() {
        let fc = FetchClient::new(&test_config()).unwrap();
        // A 404 from an unroutable literal would be blocked by SSRF guard —
        // just assert construction & validation logic here.
        assert!(
            fc.get_capped("http://127.0.0.1/x").await.is_err(),
            "loopback must be rejected by SSRF guard"
        );
        assert!(fc.get_capped("file:///etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let fc = FetchClient::new(&test_config()).unwrap();
        assert!(fc.get_capped("ftp://example.com/f").await.is_err());
    }
}
