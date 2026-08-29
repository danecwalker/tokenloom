//! URL canonicalization, scheme validation and bang parsing (PLAN.md §4, §5).

use crate::model::Category;
use serde::{Deserialize, Serialize};

/// Tracking/query parameters stripped during canonicalization (PLAN.md §5,
/// *Deduplication & RRF*).
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "gclid",
    "fbclid",
    "mc_eid",
    "mc_cid",
    "msclkid",
    "dclid",
    "twclid",
    "igshid",
    "yclid",
    "ved",
    "ei",
    "gs_l",
    "s_kwcid",
];

/// Result of parsing the `!bang` tokens out of a raw query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BangParse {
    /// Query with all bangs removed.
    pub clean_query: String,
    /// Bang tokens as written, lowercased, in order (e.g. ["ddg", "news"]).
    pub bangs: Vec<String>,
}

/// Outcome of resolving parsed bangs against the engine registry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BangResolution {
    pub engines: Vec<String>,
    pub category: Option<Category>,
}

/// Parse `!bang` tokens out of a query (PLAN.md §5):
/// - Engine bangs: `!ddg query`, `query !crates`, `!arx quantum computing`
/// - Category bangs: `!news artificial intelligence`
/// - Multi-bang: `!ddg !news ukraine`
pub fn parse_bangs(query: &str) -> BangParse {
    let mut bangs = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for token in query.split_whitespace() {
        let t = token.trim_start_matches('!');
        // Bang tokens are a single "!"-prefixed word (bangs contain no spaces).
        if token.len() > 1
            && token.starts_with('!')
            && !t.is_empty()
            && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            bangs.push(t.to_lowercase());
        } else {
            kept.push(token);
        }
    }
    BangParse {
        clean_query: kept.join(" "),
        bangs,
    }
}

/// Canonicalization used as the deduplication key before RRF fusion
/// (PLAN.md §5): strip tracking params + fragment, drop `www.`, normalize
/// scheme to https and remove trailing slash (except on the root path).
pub fn canonicalize_url(input: &str) -> Option<String> {
    let mut url = url::Url::parse(input).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    let _ = url.set_scheme("https");
    if let Some(host) = url.host_str().map(str::to_owned) {
        if let Some(stripped) = host.strip_prefix("www.") {
            let _ = url.set_host(Some(stripped));
        }
    }
    url.set_fragment(None);
    let query = url.query().map(|q| {
        let filtered: Vec<(String, String)> = url::form_urlencoded::parse(q.as_bytes())
            .filter(|(k, _)| {
                let k = k.to_lowercase();
                !TRACKING_PARAMS.contains(&k.as_str())
            })
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if filtered.is_empty() {
            String::new()
        } else {
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(filtered)
                .finish()
        }
    });
    match query.as_deref() {
        Some("") => url.set_query(None),
        Some(q) => url.set_query(Some(q)),
        None => {}
    }
    let mut s = url.to_string();
    if url.query().is_none() && s.ends_with('/') {
        s.pop();
    }
    Some(s)
}

/// True if the token is a bare category bang (`!news`, `!images`, ...).
pub fn category_from_bang(bang: &str) -> Option<Category> {
    let norm = bang.trim_start_matches('!').to_lowercase();
    Category::from_str(&norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_parsing_positions_and_multi_bangs() {
        let p = parse_bangs("!ddg rust async");
        assert_eq!(p.clean_query, "rust async");
        assert_eq!(p.bangs, vec!["ddg"]);

        let p = parse_bangs("quantum error correction !arx");
        assert_eq!(p.clean_query, "quantum error correction");
        assert_eq!(p.bangs, vec!["arx"]);

        let p = parse_bangs("!DDG !NEWS ukraine");
        assert_eq!(p.clean_query, "ukraine");
        assert_eq!(p.bangs, vec!["ddg", "news"]);

        // Not a bang: "!" alone, or email-like tokens.
        let p = parse_bangs("! hello a@b.com");
        assert_eq!(p.clean_query, "! hello a@b.com");
        assert!(p.bangs.is_empty());
    }

    #[test]
    fn canonicalization_strips_tracking_and_normalizes() {
        let cases = [
            (
                "http://www.Example.com/a/b/?utm_source=x&utm_medium=y&id=7#top",
                "https://example.com/a/b/?id=7",
            ),
            ("https://example.com/", "https://example.com"),
            ("https://example.com/path/", "https://example.com/path"),
            (
                "https://example.com/p?gclid=abc&keep=1",
                "https://example.com/p?keep=1",
            ),
            ("https://EXAMPLE.com/Camel/", "https://example.com/Camel"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                canonicalize_url(input).as_deref(),
                Some(expected),
                "{input}"
            );
        }
        assert_eq!(canonicalize_url("ftp://example.com"), None);
    }

    #[test]
    fn category_bangs_resolve() {
        assert_eq!(category_from_bang("!news"), Some(Category::News));
        assert_eq!(
            category_from_bang("social_media"),
            Some(Category::SocialMedia)
        );
        assert_eq!(category_from_bang("!ddg"), None);
    }
}
