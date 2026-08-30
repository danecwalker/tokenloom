//! Per-type sanitisation for non-HTML response bodies (PLAN.md §7).
//!
//! Structured payloads must not go through the 7-layer HTML pipeline: there
//! is no markup to allowlist and no article to extract. Instead each kind
//! gets a strictly lighter pass:
//!
//! - **JSON** — verbatim passthrough: charset decode, LLM hardening, fenced
//!   as a `json` code block. No parse, no rewrite: the payload the server
//!   sent is what the model sees.
//! - **XML** — same light pass, fenced as `xml`; the first `<title>` element
//!   (RSS channel / Atom feed / SVG) is harvested for the document title.
//! - **Plain text families** (text, Markdown, CSV, YAML) — decode + harden;
//!   prose stays prose, structured text is fenced with its language tag.
//!
//! In all cases Layer 7 still applies in full: NFC normalisation,
//! zero-width/bidi/control character stripping, fence neutralisation, the
//! character budget and the optional untrusted-content boundaries — so a
//! hostile payload can neither smuggle invisible prompt-injection text nor
//! break out of the code fence.

use crate::hardening::{harden_markdown, HardeningOptions};
use crate::{decode_html, SanitizeOptions, SanitizedDocument};
use tokenloom_core::TokenloomError;

/// Sanitise a JSON response body (verbatim passthrough behind a hardened
/// `json` fence).
pub fn sanitize_json(
    raw: &[u8],
    header_charset: Option<&str>,
    opts: &SanitizeOptions,
) -> Result<SanitizedDocument, TokenloomError> {
    let text = decode_checked(raw, header_charset, opts)?;
    Ok(SanitizedDocument {
        title: "JSON response".to_string(),
        markdown: render_payload(&text, Some("json"), opts),
        text_length: visible_chars(&text),
        ..SanitizedDocument::default()
    })
}

/// Sanitise an XML response body (RSS, Atom, SVG, generic XML). The first
/// `<title>` element becomes the document title when present.
pub fn sanitize_xml(
    raw: &[u8],
    header_charset: Option<&str>,
    opts: &SanitizeOptions,
) -> Result<SanitizedDocument, TokenloomError> {
    // Honour the XML declaration's `encoding="…"` before falling back to
    // detection (decode_html already prefers the BOM over this value).
    let declared = raw
        .get(..512)
        .map(|head| String::from_utf8_lossy(head).into_owned())
        .and_then(|head| xml_declared_encoding(&head));
    let declared = declared.as_deref().or(header_charset);
    let text = decode_checked(raw, declared, opts)?;
    Ok(SanitizedDocument {
        title: xml_title(&text).unwrap_or_else(|| "XML document".to_string()),
        markdown: render_payload(&text, Some("xml"), opts),
        text_length: visible_chars(&text),
        ..SanitizedDocument::default()
    })
}

/// Sanitise a plain-text-family response body (text, Markdown, CSV, YAML).
/// `fence_lang` selects a code fence for structured text; `None` emits the
/// hardened payload as Markdown prose (text/plain, text/markdown).
pub fn sanitize_plaintext(
    raw: &[u8],
    header_charset: Option<&str>,
    fence_lang: Option<&str>,
    opts: &SanitizeOptions,
) -> Result<SanitizedDocument, TokenloomError> {
    let text = decode_checked(raw, header_charset, opts)?;
    Ok(SanitizedDocument {
        title: String::new(),
        markdown: render_payload(&text, fence_lang, opts),
        text_length: visible_chars(&text),
        ..SanitizedDocument::default()
    })
}

/// Byte-cap check + charset decode, shared by all structured kinds.
fn decode_checked(
    raw: &[u8],
    header_charset: Option<&str>,
    opts: &SanitizeOptions,
) -> Result<String, TokenloomError> {
    if (raw.len() as u64) > opts.max_bytes {
        return Err(TokenloomError::ResponseTooLarge {
            limit: opts.max_bytes,
        });
    }
    Ok(decode_html(raw, header_charset))
}

fn visible_chars(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Harden the payload (Layer 7) and emit it, fenced when a language tag is
/// given. The payload is hardened *before* the fence is added so backtick
/// runs inside it are neutralised and cannot close the wrapper.
fn render_payload(content: &str, fence_lang: Option<&str>, opts: &SanitizeOptions) -> String {
    let Some(lang) = fence_lang else {
        // Prose path: identical to the HTML pipeline's Layer-7 output.
        return harden_markdown(
            content,
            &HardeningOptions {
                escape_fences: opts.escape_code_fences,
                delimit: opts.delimit_untrusted,
                budget: Some(opts.max_characters),
            },
        );
    };
    let hardened = harden_markdown(
        content,
        &HardeningOptions {
            escape_fences: opts.escape_code_fences,
            delimit: false,
            budget: Some(opts.max_characters),
        },
    );
    if hardened.trim().is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(hardened.len() + 32);
    if opts.delimit_untrusted {
        out.push_str("<!-- BEGIN_UNTRUSTED_CONTENT -->\n");
    }
    out.push_str("```");
    out.push_str(lang);
    out.push('\n');
    out.push_str(hardened.trim_end());
    out.push_str("\n```");
    if opts.delimit_untrusted {
        out.push_str("\n<!-- END_UNTRUSTED_CONTENT -->");
    }
    out
}

/// `encoding="…"` from an XML declaration (first 512 bytes), if any.
fn xml_declared_encoding(head: &str) -> Option<String> {
    let lower = head.to_ascii_lowercase();
    let idx = lower.find("encoding=")?;
    let rest = head[idx + "encoding=".len()..].trim_start();
    let (label, _) = crate::take_quoted_or_token(rest);
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

/// Best-effort title: the text of the first `<title>…</title>` element
/// (RSS channel title, Atom feed title, SVG title, …).
fn xml_title(text: &str) -> Option<String> {
    let open = text.find("<title")?;
    let inner_start = open + text[open..].find('>')? + 1;
    let inner_end = inner_start + text[inner_start..].find("</title")?;
    let inner = &text[inner_start..inner_end];
    // Decode entities via the HTML fragment parser (scraper is already in
    // the tree and parses hostile fragments safely).
    let frag = scraper::Html::parse_fragment(inner);
    let raw: String = frag.root_element().text().collect();
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        None
    } else {
        Some(title.chars().take(200).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> SanitizeOptions {
        SanitizeOptions::default()
    }

    fn undelimited() -> SanitizeOptions {
        SanitizeOptions {
            delimit_untrusted: false,
            ..SanitizeOptions::default()
        }
    }

    #[test]
    fn json_passes_through_verbatim_behind_fence() {
        let body = br#"{"name":"MiniMax-H3","likes":42}"#;
        let doc = sanitize_json(body, None, &undelimited()).unwrap();
        assert_eq!(doc.title, "JSON response");
        assert_eq!(
            doc.markdown,
            "```json\n{\"name\":\"MiniMax-H3\",\"likes\":42}\n```"
        );
    }

    #[test]
    fn json_is_delimited_by_default() {
        let doc = sanitize_json(br#"{"a":1}"#, None, &opts()).unwrap();
        assert!(doc
            .markdown
            .starts_with("<!-- BEGIN_UNTRUSTED_CONTENT -->\n```json\n"));
        assert!(doc
            .markdown
            .ends_with("\n```\n<!-- END_UNTRUSTED_CONTENT -->"));
    }

    #[test]
    fn json_invisible_chars_stripped() {
        // Zero-width/bidi injection inside a JSON string value must not
        // survive (the literal chars stay, the control chars go).
        let body = "{\"a\":\"x\u{200B}y\u{202E}rev\u{202C}z\"}";
        let doc = sanitize_json(body.as_bytes(), None, &undelimited()).unwrap();
        assert!(doc.markdown.contains("xyrevz"), "{:?}", doc.markdown);
        assert!(!doc.markdown.contains('\u{200B}'));
        assert!(!doc.markdown.contains('\u{202E}'));
        assert!(!doc.markdown.contains('\u{202C}'));
    }

    #[test]
    fn json_backtick_runs_cannot_break_the_fence() {
        let body = br#"{"code":"```python\nprint(1)\n```"}"#;
        let doc = sanitize_json(body, None, &undelimited()).unwrap();
        // The payload's fence runs are escaped, the wrapper fence is intact.
        assert!(doc.markdown.starts_with("```json\n"));
        assert!(doc.markdown.ends_with("\n```"));
        assert!(doc.markdown.contains("\\`\\`\\`python"));
    }

    #[test]
    fn json_budget_truncates_with_marker() {
        let mut o = opts();
        o.max_characters = 10;
        let doc = sanitize_json(br#"{"k":"aaaaaaaaaaaaaaaaaaaaaaaa"}"#, None, &o).unwrap();
        assert!(doc.markdown.contains("truncated by tokenloom"));
    }

    #[test]
    fn xml_title_extracted_with_entities() {
        let body = br#"<?xml version="1.0"?><rss><channel><title>AT &amp; T News</title><item><title>skip me</title></item></channel></rss>"#;
        let doc = sanitize_xml(body, None, &undelimited()).unwrap();
        assert_eq!(doc.title, "AT & T News");
        assert!(doc.markdown.starts_with("```xml\n"));
    }

    #[test]
    fn xml_without_title_falls_back() {
        let body = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
        let doc = sanitize_xml(body, None, &undelimited()).unwrap();
        assert_eq!(doc.title, "XML document");
    }

    #[test]
    fn xml_declared_encoding_honoured() {
        // Latin-1 bytes for "café" declared via the XML declaration.
        let mut body = b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?><note>".to_vec();
        body.extend_from_slice(b"caf\xe9</note>");
        let doc = sanitize_xml(&body, None, &undelimited()).unwrap();
        assert!(doc.markdown.contains("café"), "{:?}", doc.markdown);
    }

    #[test]
    fn plaintext_markdown_stays_prose_and_hardened() {
        let doc = sanitize_plaintext(b"# Heading\n\ntext", None, None, &undelimited()).unwrap();
        assert_eq!(doc.markdown, "# Heading\n\ntext");
        assert!(!doc.markdown.contains("```"));
    }

    #[test]
    fn plaintext_csv_is_fenced() {
        let doc = sanitize_plaintext(b"a,b\n1,2\n", Some("text/csv"), Some("csv"), &undelimited())
            .unwrap();
        assert_eq!(doc.markdown, "```csv\na,b\n1,2\n```");
    }

    #[test]
    fn empty_payload_yields_empty_markdown() {
        let doc = sanitize_json(b"   \n", None, &opts()).unwrap();
        assert_eq!(doc.markdown, "");
    }

    #[test]
    fn byte_cap_enforced() {
        let mut o = opts();
        o.max_bytes = 8;
        assert!(sanitize_json(b"123456789", None, &o).is_err());
    }
}
