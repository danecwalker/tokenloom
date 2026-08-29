//! DuckDuckGo family specialists (PLAN.md §5.3):
//! - `duckduckgo` / `duckduckgo_web`: html.duckduckgo.com HTML scraping
//! - `duckduckgo_definitions`: Instant Answer API (api.duckduckgo.com)
//! - `duckduckgo_extra`: images / videos / news via the vqd + i.js endpoints

use crate::families::stackexchange::unix_to_date;
use crate::html_util::decode_entities;
use crate::http_util::{json_get, text_get};
use crate::spec::{urlencoding_lite, EngineSpec};
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use std::time::Duration;
use tokenloom_core::{Category, SearchQuery, SearchResult};

/// Shared HTTP headers that DuckDuckGo expects.
fn ddg_headers() -> Vec<(&'static str, String)> {
    vec![("Accept-Language", "en-US,en;q=0.9".into())]
}

// ─────────────────────────────────────────────────────────────────────────────
// Web (html.duckduckgo.com/html)
// ─────────────────────────────────────────────────────────────────────────────

pub struct DdgHtmlEngine {
    spec: EngineSpec,
}

impl DdgHtmlEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }

    fn category(&self) -> Category {
        self.spec
            .categories
            .first()
            .copied()
            .unwrap_or(Category::General)
    }

    /// Unwrap DuckDuckGo redirect links (`//duckduckgo.com/l/?uddg=<encoded>`).
    fn unwrap_link(href: &str) -> String {
        if let Some(pos) = href.find("uddg=") {
            let rest = &href[pos + 5..];
            let end = rest.find('&').unwrap_or(rest.len());
            let decoded = percent_decode(&rest[..end]);
            if decoded.starts_with("http") {
                return decoded;
            }
        }
        if href.starts_with("//") {
            format!("https:{href}")
        } else if href.starts_with('/') {
            format!("https://duckduckgo.com{href}")
        } else {
            href.to_string()
        }
    }

    pub fn extract(&self, html: &str) -> Vec<SearchResult> {
        let doc = Html::parse_document(html);
        let mut out = Vec::new();
        let body_sel = Selector::parse(".result__body, .web-result, .result").unwrap();
        let link_sel = Selector::parse("a.result__a").unwrap();
        let snippet_sel = Selector::parse("a.result__snippet").unwrap();
        let date_sel = Selector::parse(".result__date").unwrap();

        for body in doc.select(&body_sel) {
            let Some(link) = body.select(&link_sel).next() else {
                continue;
            };
            let Some(href) = link.value().attr("href") else {
                continue;
            };
            let url = Self::unwrap_link(href);
            if !url.starts_with("http") || url.contains("duckduckgo.com/y.js") {
                continue; // ads
            }
            let title = decode_entities(&normalize_text(link.text().collect::<String>()));
            let snippet = body
                .select(&snippet_sel)
                .next()
                .map(|s| decode_entities(&normalize_text(s.text().collect::<String>())))
                .unwrap_or_default();
            let date = body
                .select(&date_sel)
                .next()
                .map(|d| normalize_text(d.text().collect::<String>()));
            let mut r = result(&self.spec.name, self.category(), title, url, snippet);
            r.published_date = date;
            out.push(r);
            if out.len() >= 20 {
                break;
            }
        }
        out
    }
}

#[async_trait]
impl Engine for DdgHtmlEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = "https://html.duckduckgo.com/html/";
        let params = [
            ("q", query.clean_query.clone()),
            ("s", ((query.page.saturating_sub(1)) * 20).to_string()),
            ("kl", query.locale.clone().unwrap_or_else(|| "us-en".into())),
        ];
        let mut req = http
            .post(url)
            .form(&params)
            .timeout(self.timeout())
            .header("Accept", "text/html");
        for (k, v) in ddg_headers() {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(EngineError::RateLimited("HTTP 429 from DuckDuckGo".into()));
        }
        if status.as_u16() == 403 {
            return Err(EngineError::Blocked("HTTP 403 (bot check)".into()));
        }
        if !status.is_success() {
            return Err(EngineError::Network(format!("HTTP {status}")));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let results = self.extract(&html);
        if results.is_empty() {
            return Err(EngineError::Parse(
                "no results parsed (layout change or anomaly page)".into(),
            ));
        }
        Ok(results)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Instant Answers (api.duckduckgo.com)
// ─────────────────────────────────────────────────────────────────────────────

pub struct DdgIaEngine {
    spec: EngineSpec,
}

impl DdgIaEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for DdgIaEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            urlencoding_lite(&query.clean_query)
        );
        let json = json_get(http, &url, self.timeout(), &[]).await?;
        let mut out = Vec::new();

        if let (Some(heading), Some(url), Some(text)) = (
            json.get("Heading").and_then(Value::as_str),
            json.get("AbstractURL").and_then(Value::as_str),
            json.get("AbstractText").and_then(Value::as_str),
        ) {
            if !url.is_empty() {
                let source = json
                    .get("AbstractSource")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                out.push(result(
                    &self.spec.name,
                    Category::General,
                    heading,
                    url,
                    format!("{text} (via {source})"),
                ));
            }
        }
        if let Some(related) = json.get("RelatedTopics").and_then(Value::as_array) {
            for topic in related {
                // Flat entries have FirstURL/Text; grouped ones nest under Topics.
                if let (Some(url), Some(text)) = (
                    topic.get("FirstURL").and_then(Value::as_str),
                    topic.get("Text").and_then(Value::as_str),
                ) {
                    let mut r = result(
                        &self.spec.name,
                        Category::General,
                        text.split(" - ").next().unwrap_or(text),
                        url,
                        text,
                    );
                    r.metadata.insert("kind".into(), "related_topic".into());
                    out.push(r);
                } else if let Some(nested) = topic.get("Topics").and_then(Value::as_array) {
                    for t in nested.iter().take(5) {
                        if let (Some(url), Some(text)) = (
                            t.get("FirstURL").and_then(Value::as_str),
                            t.get("Text").and_then(Value::as_str),
                        ) {
                            let mut r = result(
                                &self.spec.name,
                                Category::General,
                                text.split(" - ").next().unwrap_or(text),
                                url,
                                text,
                            );
                            r.metadata.insert("kind".into(), "related_topic".into());
                            out.push(r);
                        }
                    }
                }
                if out.len() >= 12 {
                    break;
                }
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse("no instant answer available".into()));
        }
        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Images / Videos / News (vqd token + i.js / videos.js / news.js)
// ─────────────────────────────────────────────────────────────────────────────

pub struct DdgExtraEngine {
    spec: EngineSpec,
    kind: DdgKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DdgKind {
    Images,
    Videos,
    News,
}

impl DdgExtraEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let kind = match spec.name.as_str() {
            "duckduckgo_images" => DdgKind::Images,
            "duckduckgo_videos" => DdgKind::Videos,
            _ => DdgKind::News,
        };
        Self { spec, kind }
    }

    fn category(&self) -> Category {
        match self.kind {
            DdgKind::Images => Category::Images,
            DdgKind::Videos => Category::Videos,
            DdgKind::News => Category::News,
        }
    }

    async fn vqd(&self, http: &reqwest::Client, q: &str) -> Result<String, EngineError> {
        let url = format!(
            "https://duckduckgo.com/?q={}&iax=images&ia=images",
            urlencoding_lite(q)
        );
        let headers = ddg_headers();
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let html = text_get(http, &url, Duration::from_secs(5), &refs).await?;
        if let Some(pos) = html.find("vqd=\"") {
            let rest = &html[pos + 5..];
            let end = rest.find('"').unwrap_or(0);
            if end > 0 {
                return Ok(rest[..end].to_string());
            }
        }
        Err(EngineError::Parse("vqd token not found".into()))
    }

    fn endpoint(&self) -> &'static str {
        match self.kind {
            DdgKind::Images => "https://duckduckgo.com/i.js",
            DdgKind::Videos => "https://duckduckgo.com/videos.js",
            DdgKind::News => "https://duckduckgo.com/news.js",
        }
    }
}

#[async_trait]
impl Engine for DdgExtraEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let vqd = self.vqd(http, &query.clean_query).await?;
        let mut url = reqwest::Url::parse(self.endpoint()).unwrap();
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("l", &query.locale.clone().unwrap_or_else(|| "us-en".into()));
            pairs.append_pair("o", "json");
            pairs.append_pair("q", &query.clean_query);
            pairs.append_pair("vqd", &vqd);
            pairs.append_pair("p", "1");
            if self.kind == DdgKind::News {
                pairs.append_pair("df", query.time_range.as_deref().unwrap_or(""));
            }
        }
        let headers = ddg_headers();
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let text = text_get(http, url.as_str(), self.timeout(), &refs).await?;
        let items: Value = serde_json::from_str(&text)
            .map_err(|e| EngineError::Parse(format!("invalid JSON: {e}")))?;
        let Some(items) = items.as_array() else {
            return Err(EngineError::Parse("unexpected JSON shape".into()));
        };

        let mut out = Vec::new();
        for item in items {
            let r = match self.kind {
                DdgKind::Images => {
                    let (Some(title), Some(image)) = (
                        item.get("title").and_then(Value::as_str),
                        item.get("image").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    let page = item.get("url").and_then(Value::as_str).unwrap_or(image);
                    let mut r =
                        result(&self.spec.name, self.category(), title, page, String::new());
                    r.thumbnail_url = item
                        .get("thumbnail")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    r.metadata.insert("image".into(), image.into());
                    if let Some(w) = item.get("width").and_then(Value::as_i64) {
                        r.metadata.insert("width".into(), w.to_string());
                    }
                    if let Some(h) = item.get("height").and_then(Value::as_i64) {
                        r.metadata.insert("height".into(), h.to_string());
                    }
                    r
                }
                DdgKind::Videos => {
                    let Some(title) = item.get("title").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(url) = item.get("url").and_then(Value::as_str) else {
                        continue;
                    };
                    let mut r = result(
                        &self.spec.name,
                        self.category(),
                        title,
                        url,
                        item.get("description")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    );
                    r.thumbnail_url = item
                        .get("images")
                        .and_then(|i| i.get("medium"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(p) = item.get("publisher").and_then(Value::as_str) {
                        r.metadata.insert("publisher".into(), p.into());
                    }
                    if let Some(d) = item.get("duration").and_then(Value::as_str) {
                        r.metadata.insert("duration".into(), d.into());
                    }
                    r
                }
                DdgKind::News => {
                    let (Some(title), Some(url)) = (
                        item.get("title").and_then(Value::as_str),
                        item.get("url").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    let mut r = result(
                        &self.spec.name,
                        self.category(),
                        title,
                        url,
                        item.get("excerpt").and_then(Value::as_str).unwrap_or(""),
                    );
                    r.thumbnail_url = item
                        .get("image")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(src) = item.get("source").and_then(Value::as_str) {
                        r.metadata.insert("source".into(), src.into());
                    }
                    if let Some(ts) = item.get("date").and_then(Value::as_i64) {
                        r.published_date = Some(unix_to_date(ts));
                    } else if let Some(rel) = item.get("relative_time").and_then(Value::as_str) {
                        r.published_date = Some(rel.to_string());
                    }
                    r
                }
            };
            out.push(r);
            if out.len() >= 20 {
                break;
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse("no results parsed".into()));
        }
        Ok(out)
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn normalize_text(s: String) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_ddg_redirect_links() {
        assert_eq!(
            DdgHtmlEngine::unwrap_link(
                "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=abc"
            ),
            "https://example.com/a"
        );
        assert_eq!(
            DdgHtmlEngine::unwrap_link("//example.com/x"),
            "https://example.com/x"
        );
        assert_eq!(
            DdgHtmlEngine::unwrap_link("https://direct.example.com"),
            "https://direct.example.com"
        );
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
    }
}
