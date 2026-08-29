//! Wikimedia Commons family (images / videos / audio / files) via the
//! Commons `generator=search` API on namespace 6 (File:).

use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct CommonsEngine {
    spec: EngineSpec,
    filetype: Option<&'static str>,
}

impl CommonsEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let filetype = match spec.name.as_str() {
            "wikicommons.images" => Some("bitmap"),
            "wikicommons.videos" => Some("video"),
            "wikicommons.audio" => Some("audio"),
            _ => None,
        };
        Self { spec, filetype }
    }

    fn category(&self) -> Category {
        self.spec
            .categories
            .first()
            .copied()
            .unwrap_or(Category::Files)
    }
}

#[async_trait]
impl Engine for CommonsEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let mut gsrsearch = query.clean_query.clone();
        if let Some(ft) = self.filetype {
            gsrsearch = format!("{gsrsearch} filetype:{ft}");
        }
        let json = json_get_with_params(
            http,
            "https://commons.wikimedia.org/w/api.php",
            &[
                ("action", "query".into()),
                ("format", "json".into()),
                ("generator", "search".into()),
                ("gsrsearch", gsrsearch),
                ("gsrnamespace", "6".into()),
                ("gsrlimit", "15".into()),
                ("prop", "imageinfo".into()),
                ("iiprop", "url".into()),
                ("iiurlwidth", "400".into()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        if let Some(pages) = json
            .get("query")
            .and_then(|q| q.get("pages"))
            .and_then(Value::as_object)
        {
            for page in pages.values() {
                let Some(title) = page.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let Some(info) = page
                    .get("imageinfo")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                else {
                    continue;
                };
                let url = info
                    .get("descriptionurl")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if url.is_empty() {
                    continue;
                }
                let thumb = info
                    .get("thumburl")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let mut r = result(
                    &self.spec.name,
                    self.category(),
                    title.trim_start_matches("File:"),
                    url,
                    String::new(),
                );
                r.thumbnail_url = thumb;
                out.push(r);
            }
        }
        Ok(out)
    }
}
