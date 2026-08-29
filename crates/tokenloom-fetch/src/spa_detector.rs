//! SPA & client-rendered shell detection heuristics (PLAN.md §6).

/// Result of running the SPA classifier over one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaReport {
    pub is_spa: bool,
    pub reasons: Vec<&'static str>,
}

/// Known empty framework mount points (PLAN.md §6, *Empty Application Roots*).
const EMPTY_ROOTS: &[&str] = &[
    "id=\"root\"",
    "id=\"app\"",
    "id=\"__next\"",
    "id=\"app-root\"",
    "<app-root",
];

/// Common `<noscript>` fallback strings (PLAN.md §6, *NoScript Warning Blocks*).
const NOSCRIPT_STRINGS: &[&str] = &[
    "you need to enable javascript to run this app",
    "javascript is disabled in your browser",
    "please turn on javascript and refresh the page",
    "enable javascript and refresh the page to view",
    "this application requires javascript",
];

/// Classify a page as an SPA / client-rendered shell if any heuristic fires.
///
/// - `html` is the RAW server response (scripts still present — required by
///   the hydration-density heuristic).
/// - `visible_text_len` is the character count of the extracted visible text
///   (we pass the sanitised Markdown length).
pub fn detect(html: &str, visible_text_len: usize) -> SpaReport {
    let lower = html.to_lowercase();
    let html_len = html.len();
    let mut reasons = Vec::new();

    // 1. Byte & tag ratio: big document, almost no visible text.
    if html_len > 20 * 1024 && visible_text_len < 250 {
        reasons.push("large-html-with-almost-no-visible-text");
    }

    // 2. Empty application roots.
    for root in EMPTY_ROOTS {
        if let Some(idx) = lower.find(root) {
            // A mount point is "empty" when the element's content region ends
            // immediately (next non-whitespace is a closing tag) — heuristics
            // for the common framework shells.
            if looks_like_empty_root(&lower[idx..]) {
                if *root == "id=\"__next\"" {
                    // Next.js: only an SPA when __NEXT_DATA__ has no payload.
                    if !lower.contains("__next_data__") {
                        reasons.push("empty-next-root");
                    }
                } else {
                    reasons.push(match *root {
                        "id=\"app-root\"" | "<app-root" => "empty-angular-root",
                        _ => "empty-framework-root",
                    });
                }
                break;
            }
        }
    }

    // 3. NoScript warning strings present in the page text.
    for needle in NOSCRIPT_STRINGS {
        if lower.contains(needle) {
            reasons.push("noscript-fallback-warning");
            break;
        }
    }

    // 4. Hydration script density: many bundle scripts, no content tags.
    let script_count = lower.matches("<script").count();
    let content_tags = ["<p", "<article", "<h1", "<h2", "<h3"]
        .iter()
        .map(|t| lower.matches(t).count())
        .sum::<usize>();
    if script_count > 5 && content_tags == 0 {
        reasons.push("high-script-density-no-content-tags");
    }

    SpaReport {
        is_spa: !reasons.is_empty(),
        reasons,
    }
}

/// Heuristic: does the element starting at `start` (pointing at `id="..."`)
/// appear to contain no rendered content? We scan for the end of the opening
/// tag and check whether the first non-whitespace sequence afterwards is a
/// closing tag of the same element or a comment.
fn looks_like_empty_root(tail: &str) -> bool {
    let Some(open_end) = tail.find('>') else {
        return false;
    };
    let rest = tail[open_end + 1..].trim_start();
    rest.starts_with("</div")
        || rest.starts_with("</app-root")
        || rest.starts_with("</main")
        || rest.starts_with("<!--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_pages_are_not_spas() {
        let html = format!("<html><body><p>{}</p></body></html>", "word ".repeat(400));
        let report = detect(&html, 2000);
        assert!(!report.is_spa, "{:?}", report.reasons);
    }

    #[test]
    fn empty_react_root_is_spa() {
        let html = format!(
            "<html><head><script src=\"/bundle.{}.js\"></script></head><body><div id=\"root\"></div></body></html>",
            "a".repeat(21 * 1024)
        );
        let report = detect(&html, 3);
        assert!(report.is_spa);
        assert!(report.reasons.contains(&"empty-framework-root"));
        assert!(report
            .reasons
            .contains(&"large-html-with-almost-no-visible-text"));
    }

    #[test]
    fn next_with_data_is_not_spa() {
        // Pre-rendered Next page: __NEXT_DATA__ payload present.
        let html = format!(
            r#"<html><body><div id="__next"><article>Some server-rendered words {}</article></div><script id="__NEXT_DATA__" type="application/json">{{"props":{{}}}}</script></body></html>"#,
            "content words here ".repeat(30)
        );
        let report = detect(&html, 500);
        assert!(
            !report.reasons.contains(&"empty-next-root"),
            "{:?}",
            report.reasons
        );
    }

    #[test]
    fn noscript_warning_is_spa() {
        let html = "<html><body><noscript>You need to enable JavaScript to run this app.</noscript><div id=\"root\"></div></body></html>";
        let report = detect(html, 40);
        assert!(report.is_spa);
        assert!(report.reasons.contains(&"noscript-fallback-warning"));
    }

    #[test]
    fn script_density_is_spa() {
        let scripts = "<script src=\"x\"></script>".repeat(8);
        let html = format!("<html><body>{scripts}<div>  </div></body></html>");
        let report = detect(&html, 10);
        assert!(report.is_spa);
        assert!(report
            .reasons
            .contains(&"high-script-density-no-content-tags"));
    }

    #[test]
    fn small_content_pages_are_not_spas() {
        let html = "<html><body><p>Tiny but real.</p></body></html>";
        let report = detect(html, 15);
        assert!(!report.is_spa, "{:?}", report.reasons);
    }
}
