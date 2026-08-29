//! Brave Search specialist (search.brave.com scraping) — web / images /
//! videos / news variants (PLAN.md §5.4). Note: Brave applies aggressive
//! bot detection; failures are reported honestly (PLAN.md §15).

use crate::html_util::decode_entities;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use scraper::{Html, Selector};
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct BraveEngine {
    spec: EngineSpec,
    path: &'static str,
    category: Category,
}

impl BraveEngine {
    pub fn new(spec: EngineSpec) -> Self {
        match spec.name.as_str() {
            "brave_images" => Self {
                path: "/images?q={query}",
                category: Category::Images,
                spec,
            },
            "brave_videos" => Self {
                path: "/videos?q={query}",
                category: Category::Videos,
                spec,
            },
            "brave_news" => Self {
                path: "/news?q={query}",
                category: Category::News,
                spec,
            },
            _ => Self {
                path: "/search?q={query}",
                category: Category::General,
                spec,
            },
        }
    }
}

#[async_trait]
impl Engine for BraveEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://search.brave.com{}",
            self.path.replace(
                "{query}",
                &crate::spec::urlencoding_lite(&query.clean_query)
            )
        );
        let resp = http
            .get(&url)
            .timeout(self.timeout())
            .header("Accept", "text/html")
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(EngineError::RateLimited("HTTP 429".into()));
        }
        if status.as_u16() == 403 {
            return Err(EngineError::Blocked("HTTP 403 (bot detection)".into()));
        }
        if !status.is_success() {
            return Err(EngineError::Network(format!("HTTP {status}")));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let doc = Html::parse_document(&html);

        // Brave's markup churns; try a list of historically-stable selectors.
        let item_sel = Selector::parse(".snippet[data-type=\"web\"], .result, #results .snippet")
            .map_err(|_| EngineError::Parse("bad selectors".into()))?;
        let title_sel = Selector::parse(".title, .snippet-title, a .url, h2").unwrap();
        let link_sel = Selector::parse("a").unwrap();
        let desc_sel = Selector::parse(".desc, .snippet-description, .snippet-content").unwrap();

        let mut out = Vec::new();
        for item in doc.select(&item_sel) {
            let Some(link) = item.select(&link_sel).next() else {
                continue;
            };
            let Some(href) = link.value().attr("href") else {
                continue;
            };
            if !href.starts_with("http") || href.contains("brave.com") {
                continue;
            }
            let title = item
                .select(&title_sel)
                .next()
                .map(|t| decode_entities(t.text().collect::<String>().trim()))
                .unwrap_or_default();
            let snippet = item
                .select(&desc_sel)
                .next()
                .map(|d| decode_entities(d.text().collect::<String>().trim()))
                .unwrap_or_default();
            out.push(result(&self.spec.name, self.category, title, href, snippet));
            if out.len() >= 15 {
                break;
            }
        }
        if out.is_empty() {
            return Err(EngineError::Parse(
                "no results parsed (layout change?)".into(),
            ));
        }
        Ok(out)
    }
}
