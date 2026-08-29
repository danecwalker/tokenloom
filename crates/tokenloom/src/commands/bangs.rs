//! `tokenloom bangs` — list or search bang shortcuts (PLAN.md §8).

use crate::commands::App;
use tokenloom_core::Config;

pub fn run(config: &Config, pattern: Option<String>, json: bool) -> i32 {
    let Ok(app) = App::new(config) else {
        eprintln!("tokenloom: failed to initialize");
        return 2;
    };

    let needle = pattern
        .as_deref()
        .map(|p| p.trim_start_matches('!').to_lowercase());
    let mut rows: Vec<(String, String, String, bool)> = Vec::new();

    // Category bangs first.
    for c in tokenloom_core::Category::ALL {
        let bang = c.bang().trim_start_matches('!');
        if needle
            .as_deref()
            .is_none_or(|n| bang.contains(n) || c.as_str().contains(n))
        {
            rows.push((
                format!("!{bang}"),
                c.as_str().to_string(),
                "category".into(),
                true,
            ));
        }
    }
    // Engine bangs.
    for (name, bang, implemented) in app.registry.bangs() {
        if needle
            .as_deref()
            .is_none_or(|n| bang.contains(n) || name.contains(n))
        {
            rows.push((
                format!("!{bang}"),
                name.to_string(),
                "engine".into(),
                implemented,
            ));
        }
    }

    if json {
        let v: Vec<serde_json::Value> = rows
            .iter()
            .map(|(bang, name, kind, implemented)| {
                serde_json::json!({"bang": bang, "name": name, "kind": kind, "implemented": implemented})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into())
        );
        return 0;
    }

    for (bang, name, kind, implemented) in rows {
        let status = if implemented {
            ""
        } else {
            "  (registered only)"
        };
        println!("{bang:<14} {name:<36} {kind}{status}");
    }
    0
}
