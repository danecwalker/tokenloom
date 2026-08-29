//! Declarative CSS/XPath engine interpreter (PLAN.md §5.2): driven by a
//! request spec and CSS selectors (`item`, `title`, `a@href`, …). The
//! `xpath` family is served by this interpreter with CSS-translated specs.

use crate::html_util::{decode_entities, normalize_ws};
use crate::spec::{render_template, EngineSpec, FieldSpec, RequestSpec};
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};
use tokenloom_core::{SearchQuery, SearchResult};
use url::Url;

pub struct CssEngine {
    spec: EngineSpec,
}

impl CssEngine {
    pub fn new(spec: EngineSpec) -> Option<Self> {
        (spec.response.is_some()).then_some(Self { spec })
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

    pub fn extract(&self, html: &str, base: &Url) -> Result<Vec<SearchResult>, EngineError> {
        let resp = self
            .spec
            .response
            .as_ref()
            .ok_or_else(|| EngineError::Parse("missing response spec".into()))?;
        let item_sel = resp
            .item
            .as_deref()
            .or(resp.results_path.as_deref())
            .ok_or_else(|| EngineError::Parse("missing item selector".into()))?;
        let selector = Selector::parse(item_sel)
            .map_err(|_| EngineError::Parse(format!("bad item selector '{item_sel}'")))?;
        let doc = Html::parse_document(html);

        let mut out = Vec::new();
        for item in doc.select(&selector) {
            let title = self.field_text(item, &resp.title);
            let Some(href) = self.field_attr(item, &resp.url, base) else {
                continue;
            };
            let snippet = resp.snippet.as_ref().and_then(|f| self.field_text(item, f));
            let thumbnail = resp
                .thumbnail
                .as_ref()
                .and_then(|f| self.field_attr(item, f, base));
            let date = resp.date.as_ref().and_then(|f| self.field_text(item, f));
            let mut metadata = std::collections::HashMap::new();
            for (k, f) in &resp.metadata {
                if let Some(v) = self.field_text(item, f) {
                    metadata.insert(k.clone(), v);
                }
            }
            let mut r = result(
                &self.spec.name,
                self.spec.categories[0],
                title.unwrap_or_default(),
                href,
                snippet.unwrap_or_default(),
            );
            r.published_date = date;
            r.thumbnail_url = thumbnail;
            r.metadata = metadata;
            out.push(r);
        }
        Ok(out)
    }

    /// Resolve a selector path within `item`; "@attr" alone → the item itself.
    fn select<'a>(&self, item: ElementRef<'a>, path: &str) -> Option<ElementRef<'a>> {
        let (sel_part, _) = split_attr(path);
        if sel_part.is_empty() {
            return Some(item);
        }
        let sel = Selector::parse(sel_part).ok()?;
        item.select(&sel).next()
    }

    fn field_text(&self, item: ElementRef<'_>, f: &FieldSpec) -> Option<String> {
        let el = self.select(item, &f.path)?;
        let text = normalize_ws(&el.text().collect::<String>());
        let text = decode_entities(&text);
        (!text.is_empty()).then_some(text)
    }

    fn field_attr(&self, item: ElementRef<'_>, f: &FieldSpec, base: &Url) -> Option<String> {
        let primary = self.field_attr_once(item, &f.path).or_else(|| {
            f.fallback_path
                .as_deref()
                .and_then(|p| self.field_attr_once(item, p))
        })?;
        let joined = base.join(&primary).ok()?.to_string();
        Some(joined)
    }

    fn field_attr_once(&self, item: ElementRef<'_>, path: &str) -> Option<String> {
        let (sel_part, attr) = split_attr(path);
        let attr = attr?;
        let el = if sel_part.is_empty() {
            item
        } else {
            let sel = Selector::parse(sel_part).ok()?;
            item.select(&sel).next()?
        };
        let value = el.value().attr(attr)?.to_string();
        (!value.is_empty()).then_some(value)
    }
}

/// Split `a.title@href` into ("a.title", Some("href")); `@href` → ("", Some("href"));
/// plain selector → (selector, None).
fn split_attr(path: &str) -> (&str, Option<&str>) {
    match path.rsplit_once('@') {
        Some((sel, attr)) if !attr.is_empty() => (sel, Some(attr)),
        _ => (path, None),
    }
}

#[async_trait]
impl Engine for CssEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let req = self.rendered_request(query)?;
        let base = Url::parse(&req.url).map_err(|e| EngineError::Parse(e.to_string()))?;
        let mut url = base.clone();
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &req.params {
                pairs.append_pair(k, v);
            }
        }
        let method = req.method.as_deref().unwrap_or("GET").to_uppercase();
        let mut http_req = match method.as_str() {
            "POST" => http.post(url).form(&req.params),
            _ => http.get(url),
        }
        .timeout(self.timeout());
        for (k, v) in &req.headers {
            http_req = http_req.header(k.as_str(), v.as_str());
        }
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
        let body = resp
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        self.extract(&body, &base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> EngineSpec {
        let frag: crate::spec::SpecFragment = toml::from_str(
            r#"
[request]
url = "https://engine.test/search"
[request.params]
q = "{query}"
[response]
item = "div.result"
[response.title]
path = "h2"
[response.url]
path = "a@href"
[response.snippet]
path = "p"
"#,
        )
        .unwrap();
        let request = frag.request.expect("request");
        let response = frag.response.expect("response");
        EngineSpec {
            name: "test".into(),
            display: "test".into(),
            bang: "tst".into(),
            family: "xpath".into(),
            categories: vec![tokenloom_core::Category::General],
            enabled: true,
            timeout_ms: 3000,
            weight: 1.0,
            paging: false,
            locale: false,
            safe_search: false,
            time_range: false,
            wave: 1,
            request: Some(request),
            response: Some(response),
        }
    }

    #[test]
    fn parses_html_results() {
        let engine = CssEngine::new(spec()).unwrap();
        let html = r#"
        <html><body>
        <div class="result"><h2>First</h2><a href="/one">link</a><p>about one</p></div>
        <div class="result"><h2>Second</h2><a href="/two">link</a><p>about two</p></div>
        </body></html>"#;
        let results = engine
            .extract(html, &"https://engine.test".parse().unwrap())
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First");
        assert_eq!(results[0].url, "https://engine.test/one");
        assert_eq!(results[1].snippet, "about two");
        assert_eq!(results[0].engine, "test");
    }
}
