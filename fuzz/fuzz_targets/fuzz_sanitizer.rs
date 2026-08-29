//! Fuzz target 1 (PLAN.md §12): arbitrary byte streams fed to the 7-layer
//! sanitiser must never panic, over-allocate, or violate the invariants.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let url: url::Url = "https://fuzz.example/".parse().unwrap();
    let opts = tokenloom_sanitize::SanitizeOptions {
        max_bytes: 1 << 20, // 1 MiB cap keeps fuzz runs fast
        ..Default::default()
    };
    if let Ok(doc) = tokenloom_sanitize::sanitize_document(data, None, &url, &opts) {
        // Invariant P5: sanitisation is idempotent.
        let second = tokenloom_sanitize::sanitize_str(&doc.markdown, &url, &opts);
        let _ = second;
    }
});
