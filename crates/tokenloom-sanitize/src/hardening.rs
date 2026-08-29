//! Layer 7 — LLM hardening & prompt-injection mitigation (PLAN.md §7).
//!
//! - Unicode NFC normalization (P4)
//! - Zero-width spaces, bidi overrides and control character stripping
//! - Markdown fence neutralization (prevents outer code-fence breakout)
//! - Token budget truncation with an explicit marker
//! - Optional `BEGIN/END_UNTRUSTED_CONTENT` enclosure

use unicode_normalization::UnicodeNormalization;

/// Characters removed unconditionally: zero-width spaces & joiners, BOM,
/// bidi direction overrides/isolates (PLAN.md §7 Layer 7).
fn is_invisible_control(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200D}'   // zero-width space/joiner/non-joiner
        | '\u{2060}'..='\u{2064}' // word joiner & invisible ops
        | '\u{FEFF}'              // BOM / zero-width no-break space
        | '\u{202A}'..='\u{202E}' // bidi embedding/override
        | '\u{2066}'..='\u{2069}' // bidi isolates
        | '\u{00AD}'              // soft hyphen
    )
}

fn is_disallowed_control(c: char) -> bool {
    // ASCII C0 except \n and \t, DEL, and C1 controls.
    (c.is_control() && c != '\n' && c != '\t') || ('\u{0080}'..='\u{009F}').contains(&c)
}

/// Options for the hardening pass.
#[derive(Debug, Clone)]
pub struct HardeningOptions {
    /// Escape runs of 3+ backticks so content cannot close an outer fence.
    pub escape_fences: bool,
    /// Wrap in untrusted-content boundary comments.
    pub delimit: bool,
    /// Maximum characters retained; `None` disables truncation.
    pub budget: Option<usize>,
}

impl Default for HardeningOptions {
    fn default() -> Self {
        Self {
            escape_fences: true,
            delimit: true,
            budget: None,
        }
    }
}

/// Apply all Layer-7 transforms. Idempotent: hardening the same input twice
/// yields the same output (PLAN.md §7 P5).
pub fn harden_markdown(markdown: &str, opts: &HardeningOptions) -> String {
    // NFC + invisible/control character stripping.
    let nfc: String = markdown.nfc().collect();
    let mut cleaned = String::with_capacity(nfc.len());
    for c in nfc.chars() {
        if is_invisible_control(c) || is_disallowed_control(c) {
            continue;
        }
        cleaned.push(c);
    }
    // Collapse 3+ consecutive blank lines into one blank line, then trim
    // trailing whitespace so budgets/delimiters apply to stable content.
    let cleaned = collapse_blank_lines(&cleaned);
    let cleaned = cleaned.trim_end().to_string();
    let cleaned = if opts.escape_fences {
        neutralize_fences(&cleaned)
    } else {
        cleaned
    };
    let trimmed_budget = match opts.budget {
        Some(max) => truncate_to_budget(&cleaned, max),
        None => cleaned,
    };
    if opts.delimit && !trimmed_budget.trim().is_empty() && !is_delimited(&trimmed_budget) {
        return format!(
            "<!-- BEGIN_UNTRUSTED_CONTENT -->\n{}\n<!-- END_UNTRUSTED_CONTENT -->",
            trimmed_budget.trim()
        );
    }
    trimmed_budget
}

fn is_delimited(s: &str) -> bool {
    s.contains("BEGIN_UNTRUSTED_CONTENT") || s.contains("END_UNTRUSTED_CONTENT")
}

/// Replace runs of 3 or more backticks with escaped backticks so embedded
/// content can never close (or open) a Markdown code fence (PLAN.md §7).
pub fn neutralize_fences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0usize;
    for c in s.chars() {
        if c == '`' {
            run += 1;
        } else {
            if run >= 3 {
                for _ in 0..run {
                    out.push_str("\\`");
                }
            } else {
                for _ in 0..run {
                    out.push('`');
                }
            }
            run = 0;
            out.push(c);
        }
    }
    if run >= 3 {
        for _ in 0..run {
            out.push_str("\\`");
        }
    } else {
        for _ in 0..run {
            out.push('`');
        }
    }
    out
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0usize;
    for line in s.split('\n') {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 2 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Split('\n') adds a trailing newline we may have doubled; normalize.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Budget marker appended when content is truncated (PLAN.md §7 Layer 7).
pub const TRUNCATION_MARKER_PREFIX: &str = "[... Remaining ";
pub const TRUNCATION_MARKER_SUFFIX: &str = " characters truncated by tokenloom token budget ...]";

/// Truncate to `max` characters on a char boundary, appending the explicit
/// truncation marker when content was cut.
pub fn truncate_to_budget(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let remaining = s.chars().count() - max;
    format!(
        "{}{}{}{}",
        cut.trim_end(),
        if cut.trim_end().is_empty() {
            ""
        } else {
            "\n\n"
        },
        TRUNCATION_MARKER_PREFIX,
        // e.g. "[... Remaining 4,200 characters truncated ...]"
        format_number_with_commas(remaining),
        // trailing suffix appended below
    ) + TRUNCATION_MARKER_SUFFIX
}

fn format_number_with_commas(n: usize) -> String {
    let raw = n.to_string();
    let mut out = String::new();
    let bytes = raw.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> HardeningOptions {
        HardeningOptions {
            delimit: false,
            ..Default::default()
        }
    }

    #[test]
    fn strips_zero_width_and_bidi() {
        let input = "a\u{200B}b\u{202E}reversed\u{202C}c\u{FEFF}d\u{2066}iso\u{2069}e";
        let out = harden_markdown(input, &plain());
        assert_eq!(out.trim(), "abreversedcdisoe");
    }

    #[test]
    fn normalizes_to_nfc() {
        // Decomposed é (e + combining acute) must become single codepoint.
        let input = "cafe\u{301}";
        let out = harden_markdown(input, &plain());
        assert_eq!(out.trim(), "café");
        assert_eq!(out.trim().chars().filter(|c| *c == 'é').count(), 1);
    }

    #[test]
    fn neutralizes_fences() {
        let out = neutralize_fences("```rust\nlet x = 1;\n```");
        assert!(out.contains("\\`\\`\\`rust"));
        assert!(!out.contains("```"));
        // Short backtick runs are preserved.
        assert_eq!(neutralize_fences("a `code` b"), "a `code` b");
    }

    #[test]
    fn control_characters_stripped() {
        let input = "a\u{00}b\u{1F}c\u{7F}d\u{85}e\tf";
        let out = harden_markdown(input, &plain());
        assert_eq!(out.trim(), "abcde\tf");
    }

    #[test]
    fn budget_truncation_marker() {
        let long = "x".repeat(1000);
        let out = harden_markdown(
            &long,
            &HardeningOptions {
                delimit: false,
                budget: Some(100),
                ..Default::default()
            },
        );
        assert!(out.starts_with("xxx"));
        assert!(
            out.contains("[... Remaining 900 characters truncated by tokenloom token budget ...]")
        );
        assert!(out.chars().count() < 1000);
    }

    #[test]
    fn delimiting_is_idempotent() {
        let opts = HardeningOptions::default();
        let once = harden_markdown("hello", &opts);
        let twice = harden_markdown(&once, &opts);
        assert_eq!(once, twice);
        assert!(once.starts_with("<!-- BEGIN_UNTRUSTED_CONTENT -->"));
        assert!(once.ends_with("<!-- END_UNTRUSTED_CONTENT -->"));
    }

    #[test]
    fn hardening_is_idempotent_on_hostile_input() {
        let opts = HardeningOptions::default();
        let samples = [
            "\u{FFFD}\u{0B}\u{1C}", // replacement char + stray C0 controls
            "``````",
            "\u{200D}\u{202D}text",
        ];
        for s in samples {
            let once = harden_markdown(s, &opts);
            let twice = harden_markdown(&once, &opts);
            assert_eq!(once, twice);
        }
    }
}
