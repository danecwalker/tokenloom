//! Markdown templates for search results and fetched pages (PLAN.md §8).

use tokenloom_core::{FetchedPage, SearchResponse};

/// Dense, high-signal Markdown for federated search results (PLAN.md §8
/// example 1).
pub fn format_search_markdown(resp: &SearchResponse) -> String {
    // Display the clean query (bangs stripped) — they're routing, not content.
    let display_query = tokenloom_core::url_util::parse_bangs(&resp.query).clean_query;
    let display_query = if display_query.is_empty() {
        resp.query.as_str()
    } else {
        &display_query
    };
    let mut out = String::new();
    out.push_str(&format!("# Search Results: \"{display_query}\"\n"));
    out.push_str(&format!(
        "*Queried {} engines ({}) in {}ms*\n",
        resp.engines_queried.len(),
        resp.engines_queried.join(", "),
        resp.elapsed_ms
    ));

    if resp.results.is_empty() {
        out.push_str("\n_No results found._\n");
    } else {
        for (i, r) in resp.results.iter().enumerate() {
            out.push_str(&format!(
                "\n{}. [{}]({})\n",
                i + 1,
                escape_md(&r.title),
                r.url
            ));
            let mut sources = r
                .metadata
                .get("sources")
                .cloned()
                .unwrap_or(r.engine.clone());
            sources = sources
                .split(',')
                .next()
                .unwrap_or(r.engine.as_str())
                .to_string();
            out.push_str(&format!(
                "   - **Engine:** `{}` | **Score:** {:.2}\n",
                sources, r.score
            ));
            if let Some(date) = &r.published_date {
                if !date.is_empty() {
                    out.push_str(&format!("   - **Published:** {date}\n"));
                }
            }
            if !r.snippet.is_empty() {
                out.push_str(&format!("   - {}\n", collapse_snippet(&r.snippet)));
            }
        }
    }

    if !resp.engines_failed.is_empty() {
        out.push_str("\n---\n*Engine failures:*\n");
        for f in &resp.engines_failed {
            out.push_str(&format!(
                "- `{}`: {}{}\n",
                f.engine,
                f.error,
                if f.is_rate_limited {
                    " (rate limited)"
                } else {
                    ""
                }
            ));
        }
    }
    out
}

/// Markdown rendering of a fetched page with untrusted-content boundaries
/// (PLAN.md §8 example 3).
pub fn format_fetch_markdown(page: &FetchedPage) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n", escape_md(&page.title)));
    out.push_str(&format!("- **URL:** {}\n", page.final_url));
    if let Some(byline) = &page.byline {
        out.push_str(&format!("- **By:** {byline}\n"));
    }
    if let Some(published) = &page.published_time {
        out.push_str(&format!("- **Published:** {published}\n"));
    }
    if let Some(site) = &page.site_name {
        out.push_str(&format!("- **Site:** {site}\n"));
    }
    out.push_str(&format!(
        "- **Method:** `{}` | **Tokens:** ~{}\n",
        page.render_method.as_str(),
        page.estimated_tokens
    ));

    if let Some(warning) = &page.degradation_warning {
        out.push_str(&format!(
            "\n> [!WARNING]\n> **tokenloom Notice: Dynamic Render Unavailable**\n> {warning}\n"
        ));
    }

    out.push('\n');
    out.push_str(&page.markdown);
    out.push('\n');
    out
}

fn escape_md(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

fn collapse_snippet(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 300 {
        let cut: String = one_line.chars().take(300).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}
