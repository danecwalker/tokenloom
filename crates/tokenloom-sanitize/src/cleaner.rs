//! Layer 4 — allowlist HTML sanitisation via `ammonia` (PLAN.md §7).

use crate::SanitizeOptions;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use url::Url;

/// Structural tags preserved through sanitisation (PLAN.md §7 Layer 4).
pub const ALLOWED_TAGS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "article",
    "section",
    "blockquote",
    "pre",
    "code",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "a",
    "em",
    "strong",
    "del",
    "hr",
    "dl",
    "dt",
    "dd",
    "br",
    "img",
];

/// Allowed URL schemes for `href`/`src` (javascript:/data: URIs die here).
pub const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Sanitise an HTML fragment/string with the strict allowlist. Relative URLs
/// are rewritten against `base_url` so Markdown links come out absolute.
pub fn sanitize_html_string(html: &str, base_url: &Url, opts: &SanitizeOptions) -> String {
    let mut tags: HashSet<&str> = ALLOWED_TAGS.iter().copied().collect();
    if !opts.allow_images {
        tags.remove("img");
    }

    let mut builder = ammonia::Builder::default();
    builder
        .tags(tags)
        // Boilerplate containers are removed WITH their content, not merely
        // unwrapped — menus/footers/cookie bars must not leak into reader
        // output (PLAN.md §7 Layer 5).
        .clean_content_tags(HashSet::from([
            "nav", "header", "footer", "aside", "form", "button", "select", "option", "datalist",
            "fieldset", "dialog", "figure",
        ]))
        .url_relative(ammonia::UrlRelative::RewriteWithBase(base_url.clone()))
        .url_schemes(ALLOWED_SCHEMES.iter().copied().collect())
        .attribute_filter(|_element, attribute, value| {
            // Drop inline event handlers and style attributes defensively;
            // the tag/attribute allowlist already excludes them, this makes
            // the invariant explicit and future-proof (PLAN.md §11).
            // The returned value REPLACES the attribute value.
            match attribute {
                "onload" | "onerror" | "onclick" | "onmouseover" | "style" => None,
                _ => Some(Cow::Owned(value.to_owned())),
            }
        })
        .tag_attributes(HashMap::from([
            ("a", HashSet::from(["href", "title"])),
            ("img", HashSet::from(["src", "alt", "title"])),
        ]))
        .link_rel(Some("noopener noreferrer"))
        .generic_attributes(HashSet::new());

    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        "https://example.com/page".parse().unwrap()
    }

    fn opts() -> SanitizeOptions {
        SanitizeOptions::default()
    }

    #[test]
    fn strips_event_handlers_and_scripts() {
        let html =
            r#"<p onclick="steal()">hi</p><script>x()</script><a href="javascript:evil()">c</a>"#;
        let out = sanitize_html_string(html, &base(), &opts());
        assert!(out.contains("hi"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("javascript:"));
    }

    #[test]
    fn relative_urls_resolved_against_base() {
        let html = r#"<a href="/docs/page">Docs</a>"#;
        let out = sanitize_html_string(html, &base(), &opts());
        assert!(
            out.contains(r#"href="https://example.com/docs/page""#),
            "{out}"
        );
    }

    #[test]
    fn images_dropped_unless_allowed() {
        let html = r#"<p>x</p><img src="/a.png" alt="a">"#;
        let out = sanitize_html_string(html, &base(), &opts());
        assert!(!out.contains("<img"));
        let mut o = opts();
        o.allow_images = true;
        let out = sanitize_html_string(html, &base(), &o);
        assert!(out.contains("<img"), "{out}");
    }

    #[test]
    fn unknown_tags_flattened() {
        let html = "<div><font>me</font><p>keep</p></div>";
        let out = sanitize_html_string(html, &base(), &opts());
        assert!(out.contains("keep"));
        assert!(out.contains("me"));
        assert!(!out.contains("<div"));
    }

    #[test]
    fn boilerplate_containers_removed_with_content() {
        let html = r#"<body>
            <nav><a href="/a">Menu</a> items</nav>
            <header>cookie notice</header>
            <main><article><p>real content</p></article></main>
            <footer>© junk</footer>
        </body>"#;
        let out = sanitize_html_string(html, &base(), &opts());
        assert!(out.contains("real content"));
        for junk in ["Menu", "cookie notice", "© junk"] {
            assert!(!out.contains(junk), "boilerplate leaked: {junk}");
        }
    }
}
