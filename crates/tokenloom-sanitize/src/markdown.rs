//! Layer 6 — Markdown generation (`htmd`), plus link-format post-processing
//! (PLAN.md §7 Layer 6: clean GFM, tables, code language preservation).

use crate::LinkFormat;
use htmd::HtmlToMarkdown;

/// Convert an HTML fragment/document into GitHub-Flavored Markdown.
pub fn html_to_markdown(html: &str) -> String {
    let converter = HtmlToMarkdown::builder().build();
    converter.convert(html).unwrap_or_default()
}

/// Rewrite links in already-generated Markdown according to
/// [`LinkFormat`] (inline is a no-op — htmd emits inline links).
pub fn rewrite_links(markdown: &str, format: LinkFormat) -> String {
    match format {
        LinkFormat::Inline => markdown.to_string(),
        LinkFormat::Strip => strip_links(markdown),
        LinkFormat::Footnotes => footnotes(markdown),
    }
}

/// Remove `[text](url)` links, keeping the text.
fn strip_links(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let bytes = md.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some((text, end)) = parse_link(md, i) {
                out.push_str(&text);
                i = end;
                continue;
            }
        }
        // Advance one char (UTF-8 safe).
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&md[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Convert inline links to reference-style footnotes appended at the end.
fn footnotes(md: &str) -> String {
    let mut refs: Vec<String> = Vec::new();
    let mut out = String::with_capacity(md.len() + 256);
    let bytes = md.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some((text, url, end)) = parse_link_url(md, i) {
                let idx = match refs.iter().position(|u| u == &url) {
                    Some(existing) => existing + 1,
                    None => {
                        refs.push(url);
                        refs.len()
                    }
                };
                out.push_str(&text);
                out.push_str(&format!("[{idx}]"));
                i = end;
                continue;
            }
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&md[i..i + ch_len]);
        i += ch_len;
    }
    if !refs.is_empty() {
        out.push('\n');
        for (n, url) in refs.iter().enumerate() {
            out.push_str(&format!("\n[{}]: {url}", n + 1));
        }
        out.push('\n');
    }
    out
}

/// Parse `[text](url)` starting at `start` (which points at `[`).
/// Returns `(text, end_index_after_.)` or None if not a link.
fn parse_link(md: &str, start: usize) -> Option<(String, usize)> {
    parse_link_url(md, start).map(|(t, _u, e)| (t, e))
}

fn parse_link_url(md: &str, start: usize) -> Option<(String, String, usize)> {
    let bytes = md.as_bytes();
    debug_assert_eq!(bytes[start], b'[');
    let mut depth = 0usize;
    let mut i = start;
    // find matching ]
    let mut close = None;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;
    if close + 1 >= bytes.len() || bytes[close + 1] != b'(' {
        return None;
    }
    // find matching )
    let mut paren_depth = 1;
    let mut j = close + 2;
    let open_url = j;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 1,
            b'(' => paren_depth += 1,
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    let text = &md[start + 1..close];
                    let url = &md[open_url..j];
                    return Some((text.to_string(), url.to_string(), j + 1));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn utf8_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_html() {
        let md = html_to_markdown(
            "<h1>Title</h1><p>Hello <strong>world</strong></p><ul><li>a</li><li>b</li></ul>",
        );
        assert!(md.contains("# Title"), "{md}");
        assert!(md.contains("**world**"), "{md}");
        assert!(md.contains("a"), "{md}");
        assert!(md.contains("b"), "{md}");
    }

    #[test]
    fn converts_links_and_code() {
        let md = html_to_markdown("<p>See <a href=\"https://x.co\">X</a></p><pre><code class=\"language-rust\">let a = 1;</code></pre>");
        assert!(md.contains("[X](https://x.co)"), "{md}");
        assert!(md.contains("let a = 1;"), "{md}");
    }

    #[test]
    fn converts_tables() {
        let md = html_to_markdown(
            "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>",
        );
        assert!(md.contains("| A | B |"), "{md}");
        assert!(md.contains("| 1 | 2 |"), "{md}");
    }

    #[test]
    fn strip_links_keeps_text() {
        assert_eq!(strip_links("go [here](https://x.co) now"), "go here now");
        assert_eq!(strip_links("plain"), "plain");
        assert_eq!(strip_links("nested [a [b]](u) x"), "nested a [b] x");
    }

    #[test]
    fn footnotes_collect_refs() {
        let out = footnotes("a [x](https://a.co) b [y](https://b.co) c [z](https://a.co)");
        assert!(out.contains("[1]"));
        assert!(out.contains("[2]"));
        assert!(out.contains("[1]: https://a.co"));
        assert!(out.contains("[2]: https://b.co"));
    }
}
