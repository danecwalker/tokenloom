//! Startpage specialist (www.startpage.com/sp/search scraping).

use crate::html_util::decode_entities;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use scraper::{Html, Selector};
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct StartpageEngine {
    spec: EngineSpec,
    path: &'static str,
    category: Category,
}

impl StartpageEngine {
    pub fn new(spec: EngineSpec) -> Self {
        match spec.name.as_str() {
            "startpage_images" => Self {
                path: "/sp/search?query={query}&cat=images",
                category: Category::Images,
                spec,
            },
            "startpage_news" => Self {
                path: "/sp/search?query={query}&cat=news",
                category: Category::News,
                spec,
            },
            _ => Self {
                path: "/sp/search?query={query}",
                category: Category::General,
                spec,
            },
        }
    }
}

#[async_trait]
impl Engine for StartpageEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://www.startpage.com{}",
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

        let item_sel = Selector::parse(".w-gl__result, .result").unwrap();
        let title_sel = Selector::parse("h2, .w-gl__result-title").unwrap();
        let link_sel = Selector::parse("a.w-gl__result-url, a.result-link, a").unwrap();
        let desc_sel = Selector::parse(".w-gl__description, .description, p").unwrap();

        let mut out = Vec::new();
        for item in doc.select(&item_sel) {
            let Some(link) = item.select(&link_sel).next() else {
                continue;
            };
            let Some(href) = link.value().attr("href") else {
                continue;
            };
            if !href.starts_with("http") {
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
