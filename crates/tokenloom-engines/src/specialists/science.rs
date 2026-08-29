//! Science specialists: arXiv (Atom API) and PubMed (E-utilities).

use crate::html_util::normalize_ws;
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use tokenloom_core::{Category, SearchQuery, SearchResult};

// ── arXiv ────────────────────────────────────────────────────────────────────

pub struct ArxivEngine {
    spec: EngineSpec,
}

impl ArxivEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for ArxivEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://export.arxiv.org/api/query?search_query=all:{}&start={}&max_results=15",
            crate::spec::urlencoding_lite(&query.clean_query),
            (query.page.saturating_sub(1)) * 15,
        );
        let resp = http
            .get(&url)
            .timeout(self.timeout())
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(EngineError::Network(format!("HTTP {status}")));
        }
        let xml = resp
            .text()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        parse_arxiv_atom(&xml, &self.spec.name)
    }
}

/// Parse the arXiv Atom feed (also used by offline conformance tests).
pub fn parse_arxiv_atom(xml: &str, engine: &str) -> Result<Vec<SearchResult>, EngineError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| EngineError::Parse(format!("invalid XML: {e}")))?;
    let mut out = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let title = entry
            .children()
            .find(|n| n.has_tag_name("title"))
            .and_then(|n| n.text())
            .map(normalize_ws)
            .unwrap_or_default();
        let id = entry
            .children()
            .find(|n| n.has_tag_name("id"))
            .and_then(|n| n.text())
            .unwrap_or("");
        let summary = entry
            .children()
            .find(|n| n.has_tag_name("summary"))
            .and_then(|n| n.text())
            .map(normalize_ws)
            .unwrap_or_default();
        let published = entry
            .children()
            .find(|n| n.has_tag_name("published"))
            .and_then(|n| n.text())
            .map(|t| t.chars().take(10).collect::<String>());
        if id.is_empty() || title.is_empty() {
            continue;
        }
        let id_short = id.rsplit("/abs/").next().unwrap_or(id).to_string();
        let url = format!("https://arxiv.org/abs/{id_short}");
        let mut r = result(engine, Category::Science, title, url, summary);
        r.published_date = published;
        r.metadata.insert("arxiv_id".into(), id_short);
        out.push(r);
    }
    if out.is_empty() {
        return Err(EngineError::Parse("no entries in Atom feed".into()));
    }
    Ok(out)
}

// ── PubMed ───────────────────────────────────────────────────────────────────

pub struct PubmedEngine {
    spec: EngineSpec,
}

impl PubmedEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for PubmedEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        // Step 1: esearch → PMIDs.
        let esearch = format!(
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term={}&retmax=15&retmode=json",
            crate::spec::urlencoding_lite(&query.clean_query),
        );
        let resp = http
            .get(&esearch)
            .timeout(self.timeout())
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(EngineError::Network(format!("HTTP {}", resp.status())));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EngineError::Parse(e.to_string()))?;
        let ids: Vec<String> = json
            .pointer("/esearchresult/idlist")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(vec![]);
        }

        // Step 2: esummary → metadata.
        let esummary = format!(
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi?db=pubmed&id={}&retmode=json",
            ids.join(","),
        );
        let resp = http
            .get(&esummary)
            .timeout(self.timeout())
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(EngineError::Network(format!("HTTP {}", resp.status())));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EngineError::Parse(e.to_string()))?;

        let mut out = Vec::new();
        for id in &ids {
            let Some(doc) = json.pointer(&format!("/result/{id}")) else {
                continue;
            };
            let title = doc
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let journal = doc
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let date = doc
                .get("pubdate")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let authors: Vec<String> = doc
                .get("authors")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.get("name").and_then(serde_json::Value::as_str))
                        .take(3)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut r = result(
                &self.spec.name,
                Category::Science,
                title,
                format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
                format!(
                    "{journal} · {} · {date}",
                    if authors.is_empty() {
                        "…".to_string()
                    } else {
                        format!("{}, et al.", authors.join(", "))
                    }
                ),
            );
            r.published_date = Some(date.to_string());
            r.metadata.insert("pmid".into(), id.clone());
            out.push(r);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arxiv_atom_offline() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <entry>
            <id>http://arxiv.org/abs/2401.00000v1</id>
            <title>Fault-Tolerant Quantum Computation with Surface Codes</title>
            <summary>We present a unified threshold analysis.</summary>
            <published>2026-01-15T00:00:00Z</published>
          </entry>
          <entry>
            <id>http://arxiv.org/abs/2402.00000v1</id>
            <title>A Second Paper</title>
            <summary>More science.</summary>
            <published>2026-02-01T00:00:00Z</published>
          </entry>
        </feed>"#;
        let results = parse_arxiv_atom(xml, "arxiv").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].title,
            "Fault-Tolerant Quantum Computation with Surface Codes"
        );
        assert_eq!(results[0].url, "https://arxiv.org/abs/2401.00000v1");
        assert_eq!(results[0].metadata["arxiv_id"], "2401.00000v1");
        assert_eq!(results[0].published_date.as_deref(), Some("2026-01-15"));
    }
}
