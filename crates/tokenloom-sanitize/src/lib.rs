//! `tokenloom-sanitize` — the 7-layer robust sanitiser & Markdown generator
//! (PLAN.md §7).
//!
//! Pipeline:
//! 0. charset normalization (HTTP header → BOM/meta → chardetng)
//! 1. transport & SSRF guard lives in `tokenloom-fetch`
//! 2. streaming pre-strip via `lol_html` (script/style/iframe/… removal)
//! 3. spec-compliant HTML5 parse (html5ever via `scraper`) with node cap
//! 4. allowlist sanitisation (ammonia)
//! 5. main-content extraction (readability via `dom_smoothie`)
//! 6. Markdown generation (htmd)
//! 7. LLM hardening (NFC, zero-width/bidi strip, fence escaping, budget)
//!
//! Non-HTML payloads (JSON, XML, plain-text families) skip layers 2–6 and
//! take the lighter per-type passes in [`structured`] instead — Layer 7
//! hardening always applies.

pub mod cleaner;
pub mod extractor;
pub mod hardening;
pub mod markdown;
pub mod pre_strip;
pub mod structured;

pub use hardening::{harden_markdown, HardeningOptions};
pub use structured::{sanitize_json, sanitize_plaintext, sanitize_xml};

use crate::cleaner::sanitize_html_string;
use crate::extractor::extract_article_with_meta;
use tokenloom_core::TokenloomError;
use url::Url;

/// How links are rendered in the final Markdown (PLAN.md §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFormat {
    Inline,
    Footnotes,
    Strip,
}

impl LinkFormat {
    pub fn from_config(s: &str) -> Self {
        match s {
            "footnotes" => LinkFormat::Footnotes,
            "strip" | "none" => LinkFormat::Strip,
            _ => LinkFormat::Inline,
        }
    }
}

/// Options controlling the sanitiser pipeline.
#[derive(Debug, Clone)]
pub struct SanitizeOptions {
    /// Maximum accepted raw response size in bytes (Layer 2, PLAN.md §7 L2).
    pub max_bytes: u64,
    /// Maximum DOM nodes before falling back to text-only extraction (Layer 3).
    pub max_nodes: usize,
    /// Maximum characters of Markdown output (Layer 7 budget).
    pub max_characters: usize,
    pub allow_images: bool,
    pub link_format: LinkFormat,
    pub escape_code_fences: bool,
    /// Wrap content in `BEGIN/END_UNTRUSTED_CONTENT` boundaries.
    pub delimit_untrusted: bool,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024,
            max_nodes: 10_000,
            max_characters: 50_000,
            allow_images: false,
            link_format: LinkFormat::Inline,
            escape_code_fences: true,
            delimit_untrusted: true,
        }
    }
}

impl SanitizeOptions {
    pub fn from_config(c: &tokenloom_core::config::SanitizerConfig, max_bytes: u64) -> Self {
        SanitizeOptions {
            max_bytes,
            max_nodes: 10_000,
            max_characters: c.max_characters,
            allow_images: c.allow_images,
            link_format: LinkFormat::from_config(&c.link_format),
            escape_code_fences: c.escape_code_fences,
            delimit_untrusted: c.delimit_untrusted,
        }
    }
}

/// Result of the sanitiser pipeline for one document.
#[derive(Debug, Clone, Default)]
pub struct SanitizedDocument {
    pub title: String,
    pub byline: Option<String>,
    pub published_time: Option<String>,
    pub site_name: Option<String>,
    pub markdown: String,
    /// Visible text length in characters (approximate token proxy input).
    pub text_length: usize,
}

/// Decode raw bytes into a Rust string using the documented charset chain:
/// BOM → HTTP header charset → `<meta>` tag → `chardetng` detection → UTF-8
/// lossy (PLAN.md §7 Layer 3).
pub fn decode_html(bytes: &[u8], header_charset: Option<&str>) -> String {
    // BOM sniffing (encoding_rs handles this internally when we pass the full
    // buffer, but we need the label decision first).
    let bom_label = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some("utf-8")
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        Some("utf-16")
    } else {
        None
    };

    // Try explicit labels first.
    for label in [bom_label, header_charset].into_iter().flatten() {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (text, _, had_errors) = enc.decode(bytes);
            if !had_errors {
                return text.into_owned();
            }
        }
    }

    // Look for <meta charset=...> / http-equiv content-type in the first 2 KiB.
    let head = &bytes[..bytes.len().min(2048)];
    let head_text = String::from_utf8_lossy(head);
    let meta = meta_charset(&head_text);
    if let Some(label) = meta {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (text, _, had_errors) = enc.decode(bytes);
            if !had_errors {
                return text.into_owned();
            }
        }
    }

    // Charset detection fallback.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let enc = detector.guess(None, true);
    let (text, _, _) = enc.decode(bytes);
    text.into_owned()
}

fn meta_charset(head: &str) -> Option<String> {
    let lower = head.to_ascii_lowercase();
    for marker in ["charset=", "charset ="] {
        if let Some(idx) = lower.find(marker) {
            let rest = &head[idx + marker.len()..];
            let rest = rest.trim_start();
            let (raw, _) = take_quoted_or_token(rest);
            if !raw.is_empty() {
                return Some(raw);
            }
        }
    }
    None
}

pub(crate) fn take_quoted_or_token(s: &str) -> (String, usize) {
    let mut chars = s.char_indices();
    if let Some((_, first)) = chars.next() {
        let quote = if first == '"' || first == '\'' {
            Some(first)
        } else {
            None
        };
        let mut out = String::new();
        let mut consumed = 0usize;
        for (i, c) in chars {
            match quote {
                Some(q) if c == q => {
                    consumed = i + c.len_utf8();
                    break;
                }
                None if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                    out.push(c);
                }
                None => {
                    consumed = i;
                    break;
                }
                _ => {
                    consumed = i + c.len_utf8();
                    break;
                }
            }
        }
        return (out.to_lowercase(), consumed);
    }
    (String::new(), 0)
}

/// Run the full 7-layer pipeline over a decoded HTML document.
///
/// Layers 0–3 have already been applied if you arrive via
/// `tokenloom-fetch`'s streaming fetch (byte cap + pre-strip); this function is
/// idempotent and re-runs them safely (PLAN.md §7 P5).
pub fn sanitize_document(
    raw: &[u8],
    header_charset: Option<&str>,
    base_url: &Url,
    opts: &SanitizeOptions,
) -> Result<SanitizedDocument, TokenloomError> {
    if (raw.len() as u64) > opts.max_bytes {
        return Err(TokenloomError::ResponseTooLarge {
            limit: opts.max_bytes,
        });
    }
    let decoded = decode_html(raw, header_charset);
    sanitize_str(&decoded, base_url, opts)
}

/// Sanitise an already-decoded HTML string (also the entry point for
/// hardening already-converted Markdown from Jina, via [`harden_markdown`]).
pub fn sanitize_str(
    html: &str,
    base_url: &Url,
    opts: &SanitizeOptions,
) -> Result<SanitizedDocument, TokenloomError> {
    // Layer 2: streaming pre-strip (removes scripts, styles, comments, …).
    let pre_stripped = pre_strip::pre_strip(html)
        .map_err(|e| TokenloomError::Sanitization(format!("pre-strip failed: {e}")))?;

    // Metadata must be harvested before ammonia strips <title>/<meta>.
    let meta = {
        let dom = scraper::Html::parse_document(&pre_stripped);
        extractor::metadata_from_dom(&dom)
    };

    // Layer 4: allowlist sanitisation.
    let allowed = sanitize_html_string(&pre_stripped, base_url, opts);

    // Layer 3+: parse with html5ever (via scraper) & enforce node cap.
    // Layer 5: readability extraction with boilerplate removal.
    // Layer 6: DOM → Markdown.
    let doc = extract_article_with_meta(&allowed, meta, base_url, opts)
        .map_err(|e| TokenloomError::Sanitization(format!("extraction failed: {e}")))?;

    // Layer 7: LLM hardening + budget truncation.
    let markdown = harden_markdown(
        &doc.markdown,
        &hardening::HardeningOptions {
            escape_fences: opts.escape_code_fences,
            delimit: opts.delimit_untrusted,
            budget: Some(opts.max_characters),
        },
    );

    Ok(SanitizedDocument {
        title: doc.title,
        byline: doc.byline,
        published_time: doc.published_time,
        site_name: doc.site_name,
        markdown,
        text_length: doc.text_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> SanitizeOptions {
        SanitizeOptions::default()
    }

    #[test]
    fn charset_meta_detection() {
        let html = b"<html><head><meta charset=\"windows-1252\"></head><body>caf\xe9</body></html>";
        let doc = sanitize_document(html, None, &"https://example.com".parse().unwrap(), &opts())
            .unwrap();
        assert!(doc.markdown.contains("café"), "{:?}", doc.markdown);
    }

    #[test]
    fn bom_utf8_wins() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("<p>héllo</p>".as_bytes());
        let doc = sanitize_document(
            &bytes,
            None,
            &"https://example.com".parse().unwrap(),
            &opts(),
        )
        .unwrap();
        assert!(doc.markdown.contains("héllo"));
    }

    #[test]
    fn byte_cap_enforced() {
        let big = vec![b'a'; 1024];
        let mut o = opts();
        o.max_bytes = 1024;
        let mut bigger = big.clone();
        bigger.push(b'b');
        assert!(
            sanitize_document(&bigger, None, &"https://example.com".parse().unwrap(), &o).is_err()
        );
    }
}
