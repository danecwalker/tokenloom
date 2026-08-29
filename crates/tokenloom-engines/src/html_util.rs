//! Small shared HTML text utilities for engines (entity decoding, tag
//! stripping) used when upstream APIs return HTML-escaped fields.

/// Strip HTML tags from a snippet, keeping text content.
pub fn strip_tags(html: &str) -> String {
    let frag = scraper::Html::parse_fragment(html);
    let text: String = frag.root_element().text().collect();
    normalize_ws(&text)
}

/// Decode the common HTML entities (`&amp;` `&quot;` `&#39;` …) that JSON
/// APIs frequently leave inside titles/snippets.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((replacement, len)) = parse_entity(&s[i..]) {
                out.push_str(&replacement);
                i += len;
                continue;
            }
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn parse_entity(rest: &str) -> Option<(String, usize)> {
    let end = rest.find(';')?;
    let entity = &rest[1..end];
    let replacement = match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "rsquo" => "’",
        "lsquo" => "‘",
        "ldquo" => "“",
        "rdquo" => "”",
        e if e.starts_with("#x") || e.starts_with("#X") => {
            let cp = u32::from_str_radix(&e[2..], 16).ok()?;
            return Some((char::from_u32(cp)?.to_string(), end + 1));
        }
        e if e.starts_with('#') => {
            let cp: u32 = e[1..].parse().ok()?;
            return Some((char::from_u32(cp)?.to_string(), end + 1));
        }
        _ => return None,
    };
    Some((replacement.to_string(), end + 1))
}

fn utf8_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Collapse runs of whitespace into single spaces and trim.
pub fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_entities_and_numeric_refs() {
        assert_eq!(
            decode_entities("A &amp; B &quot;C&quot; &#39;"),
            "A & B \"C\" '"
        );
        assert_eq!(decode_entities("x &#x2764; y"), "x ❤ y");
        assert_eq!(decode_entities("plain"), "plain");
    }

    #[test]
    fn strips_tags_and_normalizes() {
        assert_eq!(strip_tags("<b>bold</b> and <i>soft</i>"), "bold and soft");
        assert_eq!(normalize_ws(" a\n b   c "), "a b c");
    }
}
