//! `tokenloom doctor` — self-tests (PLAN.md §10, *Observability*).

use crate::commands::App;
use tokenloom_core::Config;

pub async fn run(config: &Config) -> i32 {
    println!("tokenloom doctor");
    println!("───────────────");
    let mut failures = 0usize;

    // 1. HTTP client / SSRF guard construction.
    let app_ok = App::new(config).ok();
    match &app_ok {
        Some(_) => println!("✓ HTTP client built with SSRF-guarded DNS resolver"),
        None => {
            println!("✗ failed to build HTTP client (DNS resolver?)");
            failures += 1;
        }
    }

    // 2. DNS resolution of a public host through the guarded path.
    if let Some(app) = &app_ok {
        match tokenloom_fetch::client::probe_status(app.client.raw(), "https://example.com/").await
        {
            Ok(_) => println!("✓ DNS + HTTPS connectivity (example.com)"),
            Err(e) => {
                println!("✗ connectivity check failed: {e}");
                failures += 1;
            }
        }
    }

    // 3. SSRF self-test: literal metadata IP & localhost must be rejected.
    for (url, label) in [
        (
            "http://169.254.169.254/latest/meta-data/",
            "cloud metadata blocked",
        ),
        ("http://127.0.0.1:1/", "loopback blocked"),
    ] {
        match tokenloom_fetch::validate_url(&url.parse().unwrap()) {
            Err(_) => println!("✓ SSRF guard: {label}"),
            Ok(()) => {
                println!("✗ SSRF guard: {url} was NOT rejected");
                failures += 1;
            }
        }
    }

    // 4. Engine pings (quick, small budget).
    if let Some(app) = &app_ok {
        for name in ["duckduckgo", "wikipedia"] {
            let Some(engine) = app.registry.build(name) else {
                println!("✗ engine {name}: no implementation");
                failures += 1;
                continue;
            };
            let mut q = tokenloom_core::SearchQuery::new("tokenloom doctor");
            q.clean_query = "tokenloom doctor".into();
            q.timeout = std::time::Duration::from_secs(6);
            match tokio::time::timeout(
                std::time::Duration::from_secs(8),
                engine.search(&q, app.client.raw()),
            )
            .await
            {
                Ok(Ok(results)) => println!("✓ engine {name}: {} results", results.len()),
                Ok(Err(e)) => {
                    println!("△ engine {name}: {e} (may be network-dependent)");
                }
                Err(_) => {
                    println!("△ engine {name}: timed out (may be network-dependent)");
                }
            }
        }
    }

    // 5. Jina Reader reachability & quota state.
    {
        let jina = tokenloom_fetch::JinaClient::new(
            app_ok
                .as_ref()
                .map(|a| a.client.raw().clone())
                .unwrap_or_default(),
            &config.reader.jina_endpoint,
            config.reader.jina_rate_limit_rpm,
            (!config.reader.jina_api_key.is_empty()).then(|| config.reader.jina_api_key.clone()),
            None,
        );
        match jina.probe().await {
            Ok(status) => println!(
                "✓ Jina Reader reachable (HTTP {status}) at {}",
                config.reader.jina_endpoint
            ),
            Err(e) => println!("△ Jina Reader unreachable: {e}"),
        }
    }

    // 6. Persistent cache & quota ledger.
    match crate::cache::open_store(&config.cache) {
        Ok(Some(store)) => {
            let used = store.jina_calls_in_window(60).unwrap_or(0);
            println!(
                "✓ cache DB at {} (jina calls last minute: {used}/{}; ttl {}s)",
                crate::cache::db_path(&config.cache).display(),
                config.reader.jina_rate_limit_rpm,
                config.cache.ttl_seconds
            );
        }
        _ => println!("△ cache disabled or unavailable"),
    }

    // 7. Headless browser discovery.
    let (msg, path) = tokenloom_fetch::headless::discovery_status();
    match path {
        Some(p) => println!("✓ headless browser: {msg} → {p}"),
        None => println!("△ headless browser: {msg}"),
    }

    println!();
    if failures == 0 {
        println!("All critical checks passed.");
        0
    } else {
        println!("{failures} critical check(s) failed.");
        1
    }
}
