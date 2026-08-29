//! Declarative engine specifications loaded from `engines.toml`
//! (PLAN.md §5, *Registry & Sync Tooling*).

use serde::Deserialize;
use std::collections::BTreeMap;
use tokenloom_core::Category;

/// One engine row from the master registry. 248 of these exist; request /
/// response extraction specs are merged in from `builtin_specs` (and can be
/// supplied in user config) for declarative engines.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineSpec {
    pub name: String,
    pub display: String,
    pub bang: String,
    pub family: String,
    pub categories: Vec<Category>,
    #[serde(default)]
    pub enabled: bool,
    pub timeout_ms: u64,
    pub weight: f64,
    #[serde(default)]
    pub paging: bool,
    #[serde(default)]
    pub locale: bool,
    #[serde(default)]
    pub safe_search: bool,
    #[serde(default)]
    pub time_range: bool,
    pub wave: u8,
    #[serde(default)]
    pub request: Option<RequestSpec>,
    #[serde(default)]
    pub response: Option<ResponseSpec>,
}

/// HTTP request definition for declarative engines.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RequestSpec {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

/// Field mapping template: dotted JSON path or CSS selector, with an optional
/// prefix for relative URLs and a fallback path when the primary is missing.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldSpec {
    pub path: String,
    #[serde(default)]
    pub fallback_path: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub strip_html: bool,
}

/// Extraction rules for declarative engines.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseSpec {
    /// Dotted path to the results array (JSON engines).
    #[serde(default)]
    pub results_path: Option<String>,
    /// CSS item selector (CSS/XPath engines).
    #[serde(default)]
    pub item: Option<String>,
    pub title: FieldSpec,
    pub url: FieldSpec,
    #[serde(default)]
    pub snippet: Option<FieldSpec>,
    #[serde(default)]
    pub thumbnail: Option<FieldSpec>,
    #[serde(default)]
    pub date: Option<FieldSpec>,
    /// Extra metadata fields (name → field spec).
    #[serde(default)]
    pub metadata: BTreeMap<String, FieldSpec>,
}

/// Partial spec fragment used by `builtin_specs` and user config overlays.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SpecFragment {
    #[serde(default)]
    pub request: Option<RequestSpec>,
    #[serde(default)]
    pub response: Option<ResponseSpec>,
}

/// Apply a fragment's request/response onto a spec (fragment wins).
pub fn apply_fragment(spec: &mut EngineSpec, fragment: &SpecFragment) {
    if fragment.request.is_some() {
        spec.request = fragment.request.clone();
    }
    if fragment.response.is_some() {
        spec.response = fragment.response.clone();
    }
}

/// Substitute template placeholders in a URL or param value.
///
/// `{query}` substitutes the RAW query: when used in request params the HTTP
/// layer percent-encodes it (avoiding double-encoding). Use
/// `{query_encoded}` when embedding the query directly into a URL string.
pub fn render_template(
    template: &str,
    query: &str,
    page: u32,
    locale: &str,
    safe_search: u8,
    time_range: Option<&str>,
) -> String {
    template
        .replace("{query_encoded}", &urlencoding_lite(query))
        .replace("{query}", query)
        .replace("{page}", &(page.saturating_sub(1)).to_string())
        .replace("{page1}", &page.max(1).to_string())
        .replace("{locale}", locale)
        .replace("{safesearch}", &safe_search.to_string())
        .replace("{timerange}", time_range.unwrap_or(""))
}

/// Percent-encode everything outside the RFC 3986 unreserved set.
pub fn urlencoding_lite(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_render_placeholders() {
        assert_eq!(
            render_template(
                "https://x.co/s?q={query}&p={page}",
                "rust lang",
                2,
                "en",
                1,
                None
            ),
            "https://x.co/s?q=rust lang&p=1"
        );
        assert_eq!(
            render_template("/a/{query_encoded}/{locale}", "a&b", 1, "en-US", 0, None),
            "/a/a%26b/en-US"
        );
        assert_eq!(render_template("{query}", "a&b", 1, "en", 0, None), "a&b");
    }
}
