//! `tokenloom engines list|show|test|enable|disable` (PLAN.md §8).

use crate::commands::App;
use tokenloom_core::{Category, Config, TokenloomError};
use tokenloom_engines::Registry;

pub async fn run(config: &Config, cmd: crate::EnginesCommands) -> i32 {
    let Ok(app) = App::new(config) else {
        eprintln!("tokenloom: failed to initialize");
        return 2;
    };
    match cmd {
        crate::EnginesCommands::List {
            category,
            implemented_only,
            json,
        } => list(&app.registry, category, implemented_only, json),
        crate::EnginesCommands::Show { name } => show(&app.registry, &name),
        crate::EnginesCommands::Test { name, query } => {
            test(&app, &name, query.as_deref().unwrap_or("test")).await
        }
        crate::EnginesCommands::Enable { name } => set_enabled(&name, true),
        crate::EnginesCommands::Disable { name } => set_enabled(&name, false),
    }
}

fn list(registry: &Registry, category: Option<String>, implemented_only: bool, json: bool) -> i32 {
    let cat = category.as_deref().and_then(Category::from_str);
    let specs: Vec<_> = registry
        .specs()
        .iter()
        .filter(|s| cat.is_none_or(|c| s.categories.contains(&c)))
        .filter(|s| !implemented_only || registry.is_implemented(&s.name))
        .collect();

    if json {
        let v: Vec<serde_json::Value> = specs
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "bang": format!("!{}", s.bang),
                    "family": s.family,
                    "categories": s.categories.iter().map(Category::as_str).collect::<Vec<_>>(),
                    "enabled": s.enabled,
                    "timeout_ms": s.timeout_ms,
                    "weight": s.weight,
                    "wave": s.wave,
                    "implemented": registry.is_implemented(&s.name),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into())
        );
        return 0;
    }

    println!(
        "{:<34} {:<12} {:<18} {:<14} {:<5} {:<5} STATUS",
        "ENGINE", "BANG", "FAMILY", "CATEGORIES", "DEF", "WAVE"
    );
    for s in &specs {
        let status = if registry.is_implemented(&s.name) {
            "ready"
        } else {
            "registered"
        };
        println!(
            "{:<34} {:<12} {:<18} {:<14} {:<5} {:<5} {}",
            truncate(&s.name, 33),
            format!("!{}", s.bang),
            truncate(&s.family, 17),
            truncate(
                &s.categories
                    .iter()
                    .map(Category::as_str)
                    .collect::<Vec<_>>()
                    .join(","),
                13
            ),
            if s.enabled { "on" } else { "off" },
            s.wave,
            status
        );
    }
    let total = registry.len();
    let implemented = registry
        .specs()
        .iter()
        .filter(|s| registry.is_implemented(&s.name))
        .count();
    eprintln!("\n{total} engines ({implemented} implemented, {total} registered) — {implemented_only_note}",
        implemented_only_note = if implemented_only { "filter: implemented only" } else { "all" });
    0
}

fn show(registry: &Registry, name: &str) -> i32 {
    let Some(s) = registry.get(name) else {
        eprintln!("tokenloom: unknown engine '{name}'");
        return 1;
    };
    println!("name:          {}", s.name);
    println!("display:       {}", s.display);
    println!("bang:          !{}", s.bang);
    println!("family:        {}", s.family);
    println!(
        "categories:    {}",
        s.categories
            .iter()
            .map(Category::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("enabled:       {}", if s.enabled { "on" } else { "off" });
    println!("timeout_ms:    {}", s.timeout_ms);
    println!("weight:        {}", s.weight);
    println!("paging:        {}", s.paging);
    println!("locale:        {}", s.locale);
    println!("safe_search:   {}", s.safe_search);
    println!("time_range:    {}", s.time_range);
    println!("wave:          {}", s.wave);
    println!("implemented:   {}", registry.is_implemented(name));
    if let Some(req) = &s.request {
        println!("request url:   {}", req.url);
    }
    0
}

async fn test(app: &App, name: &str, query: &str) -> i32 {
    if app.registry.get(name).is_none() {
        eprintln!("tokenloom: unknown engine '{name}' (see `tokenloom engines list`)");
        return 1;
    }
    let Some(engine) = app.registry.build(name) else {
        eprintln!(
            "tokenloom: engine '{name}' is registered but has no implementation yet (wave {})",
            app.registry.get(name).map(|s| s.wave).unwrap_or(0)
        );
        return 1;
    };
    let mut q = tokenloom_core::SearchQuery::new(query);
    q.clean_query = query.to_string();
    q.timeout = engine.timeout();
    let started = std::time::Instant::now();
    match engine.search(&q, app.client.raw()).await {
        Ok(results) => {
            println!(
                "✓ {} returned {} results in {}ms",
                name,
                results.len(),
                started.elapsed().as_millis()
            );
            for r in results.iter().take(3) {
                println!("  - [{}]({}) {}", r.title, r.url, r.snippet);
            }
            0
        }
        Err(e) => {
            println!("✗ {name} failed: {e}");
            1
        }
    }
}

fn set_enabled(name: &str, enabled: bool) -> i32 {
    // Validate the engine exists in the compiled registry.
    if let Ok(registry) = Registry::load() {
        if registry.get(name).is_none() {
            eprintln!("tokenloom: unknown engine '{name}'");
            return 1;
        }
    }
    let Some(path) = Config::user_config_path() else {
        eprintln!("tokenloom: cannot resolve user config path");
        return 2;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut value: toml::Value =
        toml::from_str(&text).unwrap_or(toml::Value::Table(Default::default()));
    let engines = value
        .as_table_mut()
        .expect("config root is a table")
        .entry("engines")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let overrides = engines
        .as_table_mut()
        .expect("engines is a table")
        .entry("overrides")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let entry = overrides
        .as_table_mut()
        .expect("overrides is a table")
        .entry(name.to_string())
        .or_insert_with(|| toml::Value::Table(Default::default()));
    entry
        .as_table_mut()
        .expect("override is a table")
        .insert("enabled".into(), toml::Value::Boolean(enabled));

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match toml::to_string_pretty(&value)
        .map_err(|e| TokenloomError::Config(e.to_string()))
        .and_then(|s| std::fs::write(&path, s).map_err(|e| TokenloomError::Config(e.to_string())))
    {
        Ok(()) => {
            println!(
                "{} {} in {}",
                if enabled { "enabled" } else { "disabled" },
                name,
                path.display()
            );
            0
        }
        Err(e) => {
            eprintln!("tokenloom: cannot write user config: {e}");
            1
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}
