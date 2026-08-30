//! Reqwest client wrapper with SSRF-guarded DNS, timeouts, proxy support and
//! a streaming decompression-bomb cap (PLAN.md §6, §7 Layer 1/2).

use crate::ssrf::{self, SsrfGuardResolver};
use reqwest::redirect::Policy;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokenloom_core::{HttpConfig, TokenloomError};

/// How the reader pipeline should process a response body. Each kind gets
/// its own sanitisation pass — the full 7-layer HTML pipeline for HTML, and
/// progressively lighter passes for structured/text payloads (PLAN.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    /// HTML/XHTML — full 7-layer sanitiser + readability extraction.
    Html,
    /// JSON (incl. `+json` suffixes such as `application/ld+json`) —
    /// verbatim passthrough behind an LLM-hardened code fence.
    Json,
    /// XML (incl. `+xml` suffixes such as RSS/Atom/SVG) — light strip +
    /// hardened code fence.
    Xml,
    /// Plain text, Markdown, CSV, YAML, and source-code families
    /// (JS/TS/CSS/shell/Python/diff) — hardening only, no HTML parsing.
    Text,
}

impl ResponseKind {
    /// Classify a media type. Input must already be lowercased with
    /// parameters (`; charset=…`) stripped, as [`FetchClient::get_capped`]
    /// does. `None` means the reader pipeline cannot process this type.
    pub fn from_content_type(ct: &str) -> Option<Self> {
        // Order matters: `application/xhtml+xml` must win over the generic
        // `+xml` rule below.
        if ct == "text/html" || ct == "application/xhtml+xml" {
            return Some(ResponseKind::Html);
        }
        if ct == "application/json" || ct == "text/json" || ct.ends_with("+json") {
            return Some(ResponseKind::Json);
        }
        // JSON Lines / NDJSON streaming APIs get the same verbatim pass.
        if ct == "application/ndjson" || ct == "application/x-ndjson" {
            return Some(ResponseKind::Json);
        }
        if ct == "application/xml" || ct == "text/xml" || ct.ends_with("+xml") {
            return Some(ResponseKind::Xml);
        }
        if ct == "text/plain" || ct.ends_with("markdown") {
            return Some(ResponseKind::Text);
        }
        if ct == "text/csv" || ct == "application/csv" || ct == "text/tab-separated-values" {
            return Some(ResponseKind::Text);
        }
        if ct.ends_with("yaml") || ct.ends_with("yml") {
            return Some(ResponseKind::Text);
        }
        // Source-code families served with real media types (CDNs such as
        // unpkg/jsDelivr, some mirrors). Same light pass as text — only the
        // output fence differs.
        let code_families = [
            "text/javascript",
            "application/javascript",
            "application/x-javascript",
            "application/ecmascript",
            "application/typescript",
            "text/typescript",
            "text/css",
            "application/x-sh",
            "application/x-shellscript",
            "text/x-shellscript",
            "text/x-python",
            "application/x-python",
            "text/x-diff",
            "text/x-patch",
        ];
        if code_families.contains(&ct) {
            return Some(ResponseKind::Text);
        }
        None
    }

    /// Sniff the body when the server sent no `Content-Type` header.
    fn sniff(body: &[u8]) -> Self {
        let start = body
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(0);
        let rest = &body[start..];
        if rest.starts_with(b"{") || rest.starts_with(b"[") {
            return ResponseKind::Json;
        }
        let head = &rest[..rest.len().min(64)];
        let lower = head.to_ascii_lowercase();
        if lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html") {
            return ResponseKind::Html;
        }
        if rest.starts_with(b"<") {
            return ResponseKind::Xml;
        }
        ResponseKind::Text
    }

    /// Language tag for the output code fence. `None` emits the payload as
    /// (hardened) Markdown prose rather than a fenced block.
    pub fn fence_language(self, content_type: Option<&str>) -> Option<&'static str> {
        match self {
            ResponseKind::Json => Some("json"),
            ResponseKind::Xml => Some("xml"),
            ResponseKind::Html => None,
            ResponseKind::Text => {
                let ct = content_type.unwrap_or("text/plain");
                if ct.contains("csv") {
                    Some("csv")
                } else if ct.contains("yaml") || ct.contains("yml") {
                    Some("yaml")
                } else if ct.contains("tab-separated") {
                    Some("tsv")
                } else if ct.contains("javascript") || ct.contains("ecmascript") {
                    Some("javascript")
                } else if ct.contains("typescript") {
                    Some("typescript")
                } else if ct == "text/css" {
                    Some("css")
                } else if ct.contains("shell") || ct == "application/x-sh" {
                    Some("bash")
                } else if ct.contains("python") {
                    Some("python")
                } else if ct.contains("diff") || ct.contains("patch") {
                    Some("diff")
                } else {
                    // text/plain and text/markdown stay prose.
                    None
                }
            }
        }
    }
}

/// Raw result of one SSRF-guarded HTTP GET.
#[derive(Debug, Clone)]
pub struct RawFetch {
    pub final_url: String,
    pub status: u16,
    pub content_type: Option<String>,
    /// Which sanitisation pipeline the body must go through.
    pub kind: ResponseKind,
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

        // Content-type enforcement (PLAN.md §7 Layer 1). Unknown types are
        // rejected before the body is streamed; known types select the
        // sanitisation pipeline. Missing type → sniff after the read.
        let header_kind = content_type
            .as_deref()
            .and_then(ResponseKind::from_content_type);
        if let (Some(ct), None) = (content_type.as_deref(), header_kind) {
            return Err(TokenloomError::Sanitization(format!(
                "unsupported content type '{ct}' for reader pipeline \
                 (supported: html, xhtml, json, xml, yaml, csv, plain text)"
            )));
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

        // Resolve the kind: header-declared, else sniffed from the body.
        let kind = header_kind.unwrap_or_else(|| ResponseKind::sniff(&body));

        Ok(RawFetch {
            final_url: final_url.to_string(),
            status,
            content_type,
            kind,
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

    #[test]
    fn classifies_content_types() {
        use ResponseKind::*;
        let cases: &[(&str, ResponseKind)] = &[
            ("text/html", Html),
            ("application/xhtml+xml", Html),
            ("application/json", Json),
            ("text/json", Json),
            ("application/ld+json", Json),
            ("application/vnd.api+json", Json),
            ("application/problem+json", Json),
            ("application/xml", Xml),
            ("text/xml", Xml),
            ("application/rss+xml", Xml),
            ("application/atom+xml", Xml),
            ("image/svg+xml", Xml),
            ("text/plain", Text),
            ("text/markdown", Text),
            ("text/x-markdown", Text),
            ("text/csv", Text),
            ("application/csv", Text),
            ("text/tab-separated-values", Text),
            ("application/yaml", Text),
            ("application/x-yaml", Text),
            ("text/yaml", Text),
            ("application/ndjson", Json),
            ("application/x-ndjson", Json),
            ("text/javascript", Text),
            ("application/javascript", Text),
            ("application/x-javascript", Text),
            ("application/ecmascript", Text),
            ("application/typescript", Text),
            ("text/typescript", Text),
            ("text/css", Text),
            ("application/x-sh", Text),
            ("application/x-shellscript", Text),
            ("text/x-shellscript", Text),
            ("text/x-python", Text),
            ("application/x-python", Text),
            ("text/x-diff", Text),
            ("text/x-patch", Text),
        ];
        for (ct, want) in cases {
            assert_eq!(
                ResponseKind::from_content_type(ct),
                Some(*want),
                "content type {ct}"
            );
        }
        // The reader pipeline genuinely can't process these.
        for ct in [
            "image/png",
            "application/pdf",
            "application/octet-stream",
            "video/mp4",
        ] {
            assert_eq!(
                ResponseKind::from_content_type(ct),
                None,
                "content type {ct}"
            );
        }
    }

    #[test]
    fn fence_language_policy() {
        assert_eq!(ResponseKind::Json.fence_language(None), Some("json"));
        assert_eq!(ResponseKind::Xml.fence_language(None), Some("xml"));
        assert_eq!(ResponseKind::Html.fence_language(None), None);
        // Prose-like text stays prose; structured text is fenced.
        assert_eq!(ResponseKind::Text.fence_language(Some("text/plain")), None);
        assert_eq!(
            ResponseKind::Text.fence_language(Some("text/markdown")),
            None
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("text/csv")),
            Some("csv")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("application/x-yaml")),
            Some("yaml")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("application/javascript")),
            Some("javascript")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("application/typescript")),
            Some("typescript")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("text/css")),
            Some("css")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("application/x-sh")),
            Some("bash")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("text/x-python")),
            Some("python")
        );
        assert_eq!(
            ResponseKind::Text.fence_language(Some("text/x-diff")),
            Some("diff")
        );
    }

    #[test]
    fn sniffs_bodies_without_content_type() {
        assert_eq!(ResponseKind::sniff(br#"{"a": 1}"#), ResponseKind::Json);
        assert_eq!(ResponseKind::sniff(b"\r\n\t [1, 2]"), ResponseKind::Json);
        assert_eq!(
            ResponseKind::sniff(b"<?xml version=\"1.0\"?><rss/>"),
            ResponseKind::Xml
        );
        assert_eq!(
            ResponseKind::sniff(b"<!DOCTYPE html><html>"),
            ResponseKind::Html
        );
        assert_eq!(
            ResponseKind::sniff(b"<!doctype HTML><html><body>"),
            ResponseKind::Html
        );
        assert_eq!(ResponseKind::sniff(b"just some words"), ResponseKind::Text);
        assert_eq!(ResponseKind::sniff(b""), ResponseKind::Text);
    }
}
