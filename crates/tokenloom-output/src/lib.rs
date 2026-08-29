//! `tokenloom-output` — LLM-optimized Markdown & JSON formatters
//! (PLAN.md §8, *Output Formats*).

pub mod json;
pub mod markdown;
pub mod token_budget;

pub use json::{fetched_page_to_json, search_response_to_json};
pub use markdown::{format_fetch_markdown, format_search_markdown};
pub use token_budget::{estimate_tokens, truncate_with_marker};
