//! Approximate token estimation & budget truncation (PLAN.md §8, §7 L7).

use tokenloom_core::estimate_tokens as core_estimate;

/// Re-exported core estimator (chars/4 + word blend heuristic).
pub fn estimate_tokens(text: &str) -> usize {
    core_estimate(text)
}

/// Truncate to a token budget with the explicit tokenloom marker.
pub fn truncate_with_marker(text: &str, max_tokens: usize) -> String {
    let tokens = estimate_tokens(text);
    if tokens <= max_tokens {
        return text.to_string();
    }
    // Convert the token budget back to an approximate char budget (~4 chars).
    let max_chars = max_tokens.saturating_mul(4);
    let remaining_chars = text.chars().count().saturating_sub(max_chars);
    let cut: String = text.chars().take(max_chars).collect();
    format!(
        "{}\n\n[... Remaining {} characters truncated by tokenloom token budget ...]",
        cut.trim_end(),
        remaining_chars
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimation_scales() {
        assert_eq!(estimate_tokens(""), 0);
        let short = "hello world";
        let longer = "word ".repeat(500);
        assert!(estimate_tokens(short) < estimate_tokens(&longer));
        assert!(estimate_tokens(&longer) > 100);
    }

    #[test]
    fn truncation_marker_included() {
        let text = "x".repeat(10_000);
        let out = truncate_with_marker(&text, 100);
        assert!(out.contains("truncated by tokenloom token budget"));
        assert!(out.chars().count() < 10_000);
        // Under budget → unchanged.
        assert_eq!(truncate_with_marker("tiny", 100), "tiny");
    }
}
