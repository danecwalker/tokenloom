//! MediaWiki family interpreter (PLAN.md §5.3): shared `action=query`
//! protocol for Wikipedia, Wiktionary, Wikibooks, Wikinews, Wikiquote,
//! Wikisource, Wikispecies, Wikiversity, Wikivoyage, Wikimini, Gentoo wiki,
//! NixOS wiki, Arch Wiki and the Free Software Directory. The `wikidata`
//! engine uses `wbsearchentities`.

use crate::html_util::{decode_entities, strip_tags};
use crate::http_util::json_get_with_params;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct MediaWikiEngine {
    spec: EngineSpec,
    domain: &'static str,
    wikidata: bool,
}

/// Domain mapping per registry engine name.
fn domain_for(name: &str) -> &'static str {
    match name {
        "wikipedia" => "en.wikipedia.org",
        "wikidata" => "www.wikidata.org",
        "wikibooks" => "en.wikibooks.org",
        "wikinews" => "en.wikinews.org",
        "wikiquote" => "en.wikiquote.org",
        "wikisource" => "en.wikisource.org",
        "wikispecies" => "species.wikimedia.org",
        "wikiversity" => "en.wikiversity.org",
        "wikivoyage" => "en.wikivoyage.org",
        "wiktionary" => "en.wiktionary.org",
        "wikimini" => "fr.wikimini.org",
        "gentoo" => "wiki.gentoo.org",
        "nixos_wiki" => "wiki.nixos.org",
        "arch_linux_wiki" => "wiki.archlinux.org",
        "free_software_directory" => "directory.fsf.org",
        _ => "en.wikipedia.org",
    }
}

impl MediaWikiEngine {
    pub fn new(spec: EngineSpec) -> Self {
        let wikidata = matches!(spec.family.as_str(), "wikidata") || spec.name == "wikidata";
        Self {
            domain: domain_for(&spec.name),
            wikidata,
            spec,
        }
    }

    fn category(&self) -> Category {
        self.spec
            .categories
            .first()
            .copied()
            .unwrap_or(Category::General)
    }

    fn article_url(&self, title: &str) -> String {
        format!("https://{}/wiki/{}", self.domain, title.replace(' ', "_"))
    }
}

#[async_trait]
impl Engine for MediaWikiEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let api = format!("https://{}/w/api.php", self.domain);
        if self.wikidata {
            let json = json_get_with_params(
                http,
                &api,
                &[
                    ("action", "wbsearchentities".into()),
                    ("search", query.clean_query.clone()),
                    (
                        "language",
                        query.locale.clone().unwrap_or_else(|| "en".into()),
                    ),
                    ("limit", "15".into()),
                    ("format", "json".into()),
                ],
                self.timeout(),
                &[],
            )
            .await?;
            let mut out = Vec::new();
            if let Some(items) = json.get("search").and_then(Value::as_array) {
                for item in items {
                    let label = item.get("label").and_then(Value::as_str).unwrap_or("");
                    let description = item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let Some(url) = item.get("concepturi").and_then(Value::as_str) else {
                        continue;
                    };
                    let mut r = result(&self.spec.name, self.category(), label, url, description);
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        r.metadata.insert("entity_id".into(), id.into());
                    }
                    out.push(r);
                }
            }
            Ok(out)
        } else {
            let json = json_get_with_params(
                http,
                &api,
                &[
                    ("action", "query".into()),
                    ("list", "search".into()),
                    ("srsearch", query.clean_query.clone()),
                    ("srlimit", "15".into()),
                    ("sroffset", (query.page.saturating_sub(1) * 15).to_string()),
                    ("format", "json".into()),
                ],
                self.timeout(),
                &[],
            )
            .await?;
            let mut out = Vec::new();
            let results = json
                .get("query")
                .and_then(|q| q.get("search"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &results {
                let title = item.get("title").and_then(Value::as_str).unwrap_or("");
                if title.is_empty() {
                    continue;
                }
                let snippet_html = item.get("snippet").and_then(Value::as_str).unwrap_or("");
                let snippet = decode_entities(&strip_tags(snippet_html));
                let mut r = result(
                    &self.spec.name,
                    self.category(),
                    title,
                    self.article_url(title),
                    snippet,
                );
                if let Some(ts) = item.get("timestamp").and_then(Value::as_str) {
                    r.published_date = Some(ts.to_string());
                }
                out.push(r);
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_mapping_and_urls() {
        let spec: EngineSpec = toml::from_str(
            r#"
name = "arch_linux_wiki"
display = "arch linux wiki"
bang = "al"
family = "archlinux"
categories = ["it"]
enabled = true
timeout_ms = 3000
weight = 1
wave = 2
"#,
        )
        .unwrap();
        let engine = MediaWikiEngine::new(spec);
        assert_eq!(engine.domain, "wiki.archlinux.org");
        assert_eq!(
            engine.article_url("Install Guide"),
            "https://wiki.archlinux.org/wiki/Install_Guide"
        );
    }
}
