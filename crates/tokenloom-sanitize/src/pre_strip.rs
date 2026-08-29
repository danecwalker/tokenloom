//! Layer 2 — streaming pre-strip via `lol_html` (PLAN.md §7).
//!
//! Dangerous and content-free elements are removed *before* full DOM tree
//! allocation: `<script>`, `<style>`, `<noscript>`, `<iframe>`, `<svg>`,
//! `<canvas>`, `<template>`, `<math>`, `<object>`, `<embed>` plus HTML
//! comments (including conditional IE blocks).

use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

/// Elements removed wholesale (with their content) before parsing.
pub const STRIPPED_ELEMENTS: &str =
    "script, style, noscript, iframe, svg, canvas, template, math, object, embed, noembed, xmp";

/// Strip executable/non-content markup from an HTML string.
pub fn pre_strip(html: &str) -> Result<String, String> {
    rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![element!(
                "script, style, noscript, iframe, svg, canvas, template, math, object, embed, noembed, xmp",
                |el| {
                    el.remove();
                    Ok(())
                }
            )],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_scripts_styles_and_comments() {
        let html = concat!(
            "<!DOCTYPE html><html><head>",
            "<script>alert('x')</script><style>body{color:red}</style>",
            "<!-- nav begins --><!--[if IE]><p>old</p><![endif]-->",
            "</head><body><p>Keep me</p>",
            "<iframe src=\"https://evil\"></iframe><noscript>enable js</noscript>",
            "<svg><circle/></svg><canvas></canvas>",
            "</body></html>"
        );
        let out = pre_strip(html).unwrap();
        assert!(out.contains("Keep me"));
        for bad in [
            "<script",
            "<style",
            "<!--",
            "<iframe",
            "<noscript",
            "<svg",
            "<canvas",
            "alert(",
        ] {
            assert!(!out.contains(bad), "output still contains {bad}: {out}");
        }
    }

    #[test]
    fn idempotent() {
        let html = "<div><p>a</p><script>x()</script></div>";
        let once = pre_strip(html).unwrap();
        let twice = pre_strip(&once).unwrap();
        assert_eq!(once, twice);
    }
}
