//! Fuzz target 3 (PLAN.md §12): the SPA heuristic classifier must never
//! panic on malformed HTML and must be deterministic.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let report = tokenloom_fetch::spa_detector::detect(&html, html.len() / 7);
    // Determinism: same input → same decision.
    let again = tokenloom_fetch::spa_detector::detect(&html, html.len() / 7);
    assert_eq!(report, again);
});
