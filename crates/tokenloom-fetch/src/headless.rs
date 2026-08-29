//! Optional local headless Chrome/Chromium rendering (PLAN.md §6 Step 2,
//! §16.2) — behind the `render` feature flag so minimal builds stay
//! lightweight with zero browser dependencies.

/// A discovered headless-capable browser binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredBrowser {
    pub path: String,
}

/// Candidate locations for Chrome-family binaries (PLAN.md §6 Step 2).
#[cfg_attr(not(feature = "render"), allow(dead_code))]
const CANDIDATES: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/brave-browser",
    "/usr/bin/microsoft-edge",
    "/snap/bin/chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

/// Discover a Chrome/Chromium/Brave/Edge binary: `$CHROME_PATH` first.
#[cfg(feature = "render")]
pub fn discover_browser() -> Option<DiscoveredBrowser> {
    if let Ok(p) = std::env::var("CHROME_PATH") {
        if !p.is_empty() && std::path::Path::new(&p).exists() {
            return Some(DiscoveredBrowser { path: p });
        }
    }
    CANDIDATES
        .iter()
        .find(|c| std::path::Path::new(c).exists())
        .map(|c| DiscoveredBrowser {
            path: c.to_string(),
        })
}

/// Render `url` in a local headless browser (waits up to `timeout_ms`) and
/// dump the resulting DOM HTML for the sanitiser pipeline.
#[cfg(feature = "render")]
pub async fn render_dom(
    url: &str,
    timeout_ms: u64,
) -> Result<String, tokenloom_core::TokenloomError> {
    use std::time::Duration;
    use tokenloom_core::TokenloomError;

    let browser_path = discover_browser()
        .ok_or_else(|| TokenloomError::EngineFailure {
            engine: "headless".into(),
            reason: "no Chrome/Chromium binary found".into(),
        })?
        .path;

    // Headless by default; the executable is the browser we discovered.
    let config = chromiumoxide::BrowserConfig::with_executable(browser_path);

    let (mut browser, _handle) = tokio::time::timeout(
        Duration::from_millis(timeout_ms.max(1000)),
        chromiumoxide::Browser::launch(config),
    )
    .await
    .map_err(|_| TokenloomError::EngineFailure {
        engine: "headless".into(),
        reason: "browser launch timed out".into(),
    })?
    .map_err(|e| TokenloomError::EngineFailure {
        engine: "headless".into(),
        reason: format!("browser launch failed: {e}"),
    })?;

    let _guard = _handle; // keeps the connection handler alive
    let page =
        browser
            .new_page("about:blank")
            .await
            .map_err(|e| TokenloomError::EngineFailure {
                engine: "headless".into(),
                reason: format!("cannot open page: {e}"),
            })?;

    tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        let _ = page.goto(url).await;
        // Approximate `networkidle0`: give hydration a fixed settle window.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = page.wait_for_navigation().await;
    })
    .await
    .map_err(|_| TokenloomError::EngineFailure {
        engine: "headless".into(),
        reason: "render timed out".into(),
    })?;

    let html = page
        .content()
        .await
        .map_err(|e| TokenloomError::EngineFailure {
            engine: "headless".into(),
            reason: format!("DOM dump failed: {e}"),
        })?;

    let _ = browser.close().await;
    Ok(html)
}

/// Diagnostics for `tokenloom doctor` (available in all builds).
pub fn discovery_status() -> (&'static str, Option<String>) {
    #[cfg(feature = "render")]
    {
        match discover_browser() {
            Some(b) => ("render feature enabled", Some(b.path)),
            None => ("render feature enabled, no browser binary found", None),
        }
    }
    #[cfg(not(feature = "render"))]
    {
        (
            "built without `render` feature (recompile with --features render)",
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_status_never_panics() {
        let (msg, path) = discovery_status();
        assert!(!msg.is_empty());
        if let Some(p) = path {
            assert!(std::path::Path::new(&p).exists());
        }
    }
}
