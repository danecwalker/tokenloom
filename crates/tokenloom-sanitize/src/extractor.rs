//! Layer 5 — main-content extraction (readability via `dom_smoothie`)
//! with boilerplate removal, metadata extraction and the DOM node cap
//! (PLAN.md §7 Layers 3 & 5).

use crate::markdown::{html_to_markdown, rewrite_links};
use crate::SanitizeOptions;
use scraper::{Html, Selector};
use url::Url;

/// Extracted article + metadata.
#[derive(Debug, Clone, Default)]
pub struct ExtractedArticle {
    pub title: String,
    pub byline: Option<String>,
    pub published_time: Option<String>,
    pub site_name: Option<String>,
    pub markdown: String,
    pub text_length: usize,
}

/// Errors surfaced by extraction (mapped into `TokenloomError::Sanitization`).
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("DOM exceeded maximum node count of {0}")]
    TooManyNodes(usize),
}

/// Extract the main content of an already-sanitised HTML document.
///
/// Strategy:
/// 1. Parse with html5ever (spec-compliant) and enforce the node cap.
/// 2. Try readability (`dom_smoothie`) for boilerplate-stripped main content.
/// 3. On failure (thin pages like link listings), fall back to the whole
///    sanitised document converted to Markdown.
pub fn extract_article(
    sanitized_html: &str,
    base_url: &Url,
    opts: &SanitizeOptions,
) -> Result<ExtractedArticle, ExtractError> {
    let dom = Html::parse_document(sanitized_html);
    let meta = metadata_from_dom(&dom);
    extract_article_with_meta(sanitized_html, meta, base_url, opts)
}

/// Like [`extract_article`] but with metadata extracted from an earlier
/// pipeline stage — metadata must be harvested BEFORE ammonia strips
/// `<title>`/`<meta>` tags (PLAN.md §7 L5).
pub fn extract_article_with_meta(
    sanitized_html: &str,
    meta: DomMeta,
    base_url: &Url,
    opts: &SanitizeOptions,
) -> Result<ExtractedArticle, ExtractError> {
    let dom = Html::parse_document(sanitized_html);
    let node_count = dom.tree.nodes().count();
    if node_count > opts.max_nodes {
        return Err(ExtractError::TooManyNodes(opts.max_nodes));
    }

    let fallback_meta = meta;

    // Layer 5: readability main-content extraction.
    match dom_smoothie::Readability::new(sanitized_html, Some(base_url.as_str()), None) {
        Ok(mut readability) => match readability.parse() {
            Ok(article) => {
                let content_html = article.content.to_string();
                let markdown = rewrite_links(&html_to_markdown(&content_html), opts.link_format);
                // Prefer document metadata (<title>/og:title): readability
                // can promote an <h1> whose inline-block markup has no
                // inter-word whitespace, mangling the text. Metadata titles
                // are author-controlled and reliably spaced.
                let title = if fallback_meta.title.trim().is_empty() {
                    article.title
                } else {
                    fallback_meta.title
                };
                let text_length = article
                    .text_content
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .count();
                Ok(ExtractedArticle {
                    title,
                    byline: article.byline.or(fallback_meta.byline),
                    published_time: article.published_time.or(fallback_meta.published_time),
                    site_name: article.site_name.or(fallback_meta.site_name),
                    markdown,
                    text_length,
                })
            }
            Err(e) => {
                tracing::debug!(error = %e, "readability found no article; using full document");
                fallback(sanitized_html, fallback_meta, opts)
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "readability init failed; using full document");
            fallback(sanitized_html, fallback_meta, opts)
        }
    }
}

fn fallback(
    sanitized_html: &str,
    meta: DomMeta,
    opts: &SanitizeOptions,
) -> Result<ExtractedArticle, ExtractError> {
    let markdown = rewrite_links(&html_to_markdown(sanitized_html), opts.link_format);
    let text_length = markdown.chars().filter(|c| !c.is_whitespace()).count();
    Ok(ExtractedArticle {
        title: meta.title,
        byline: meta.byline,
        published_time: meta.published_time,
        site_name: meta.site_name,
        markdown,
        text_length,
    })
}

#[derive(Debug, Default, Clone)]
pub struct DomMeta {
    pub title: String,
    pub byline: Option<String>,
    pub published_time: Option<String>,
    pub site_name: Option<String>,
}

/// Metadata extraction: `<title>`, OpenGraph / Twitter Cards (PLAN.md §7 L5).
/// Must run on HTML that still contains `<head>`.
pub fn metadata_from_dom(dom: &Html) -> DomMeta {
    let mut meta = DomMeta::default();

    if let Ok(title_sel) = Selector::parse("title") {
        if let Some(t) = dom.select(&title_sel).next() {
            meta.title = t.text().collect::<String>().trim().to_string();
        }
    }

    let prop = |dom: &Html, key: &str| -> Option<String> {
        let sel =
            Selector::parse(&format!("meta[property=\"{key}\"], meta[name=\"{key}\"")).ok()?;
        dom.select(&sel)
            .find_map(|m| m.value().attr("content"))
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
    };

    if let Some(t) = prop(dom, "og:title").or_else(|| prop(dom, "twitter:title")) {
        meta.title = t;
    }
    meta.byline = prop(dom, "author")
        .or_else(|| prop(dom, "article:author"))
        .or_else(|| prop(dom, "twitter:creator"));
    meta.published_time = prop(dom, "article:published_time")
        .or_else(|| prop(dom, "date"))
        .or_else(|| prop(dom, "dcterms.date"));
    meta.site_name = prop(dom, "og:site_name");
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        "https://example.com/a".parse().unwrap()
    }

    #[test]
    fn extracts_article_and_metadata() {
        let html = r#"
        <html><head><title>Old Title</title>
        <meta property="og:site_name" content="Example News">
        <meta name="author" content="Jane Doe">
        </head><body>
        <nav>Menu menu menu</nav>
        <article><h1>The Real Story</h1>
        <p>This is the main content of the article with plenty of text here.</p>
        <p>Another paragraph with more meaningful words and useful detail.</p>
        </article>
        <footer>footer junk</footer>
        </body></html>"#;
        let out = extract_article(html, &base(), &SanitizeOptions::default()).unwrap();
        assert!(out.markdown.contains("main content"), "{:?}", out.markdown);
        assert_eq!(out.site_name.as_deref(), Some("Example News"));
        assert_eq!(out.byline.as_deref(), Some("Jane Doe"));
        assert!(out.text_length > 40);
    }

    #[test]
    fn falls_back_on_thin_pages() {
        let html = "<html><head><title>HN</title></head><body><ol><li><a href=\"https://a.co\">Link one</a></li><li><a href=\"https://b.co\">Link two</a></li></ol></body></html>";
        let out = extract_article(html, &base(), &SanitizeOptions::default()).unwrap();
        assert!(out.markdown.contains("Link one"), "{:?}", out.markdown);
        assert!(out.markdown.contains("https://b.co"));
        assert_eq!(out.title, "HN");
    }

    #[test]
    fn node_cap_enforced() {
        let o = SanitizeOptions {
            max_nodes: 10,
            ..Default::default()
        };
        let html = "<p>".to_string() + &"x</p><p>".repeat(50) + "x</p>";
        assert!(matches!(
            extract_article(&html, &base(), &o),
            Err(ExtractError::TooManyNodes(10))
        ));
    }
}
