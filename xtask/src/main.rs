//! xtask — developer tooling for the tokenloom workspace.
//!
//! Usage: `cargo xtask sync-engines [--check]`

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        "sync-engines" => {
            let check = args.next().as_deref() == Some("--check");
            sync_engines(check)
        }
        "help" | "--help" | "-h" => {
            println!("usage: cargo xtask <command>\n\ncommands:\n  sync-engines [--check]   validate engines.toml against PLAN.md Appendix A counts");
            Ok(())
        }
        other => bail!("unknown xtask command: {other}"),
    }
}

/// Validate the generated engine registry against the plan's stated counts:
/// 248 unique engines, waves 80/82/86, 78 enabled by default, 10 categories,
/// unique bangs. With `--check`, exit non-zero on any drift (CI gate).
fn sync_engines(check: bool) -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must live in the workspace root")?
        .to_path_buf();
    let path = root.join("engines.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;

    #[derive(serde::Deserialize, Debug)]
    struct EngineSpec {
        name: String,
        bang: String,
        #[allow(dead_code)]
        family: String,
        categories: Vec<String>,
        #[serde(default)]
        enabled: bool,
        wave: u8,
    }
    #[derive(serde::Deserialize)]
    struct File {
        engines: Vec<EngineSpec>,
    }

    let file: File = toml::from_str(&text).context("engines.toml is invalid TOML")?;
    let mut errors = Vec::new();

    // Unique engines & bangs.
    let mut names = std::collections::HashSet::new();
    let mut bangs = std::collections::HashSet::new();
    for e in &file.engines {
        if !names.insert(e.name.clone()) {
            errors.push(format!("duplicate engine name '{}'", e.name));
        }
        if !bangs.insert(e.bang.clone()) {
            errors.push(format!("duplicate bang '!{}'", e.bang));
        }
        if e.categories.is_empty() {
            errors.push(format!("engine '{}' has no categories", e.name));
        }
    }

    // Counts from PLAN.md Appendix A.
    if file.engines.len() != 248 {
        errors.push(format!(
            "expected 248 unique engines, found {}",
            file.engines.len()
        ));
    }
    let wave_counts = |w: u8| file.engines.iter().filter(|e| e.wave == w).count();
    if wave_counts(1) != 80 {
        errors.push(format!("wave 1: expected 80, found {}", wave_counts(1)));
    }
    if wave_counts(2) != 82 {
        errors.push(format!("wave 2: expected 82, found {}", wave_counts(2)));
    }
    if wave_counts(3) != 86 {
        errors.push(format!("wave 3: expected 86, found {}", wave_counts(3)));
    }
    let enabled = file.engines.iter().filter(|e| e.enabled).count();
    if enabled != 78 {
        errors.push(format!("enabled by default: expected 78, found {enabled}"));
    }

    let categories: std::collections::HashSet<&str> = [
        "general",
        "images",
        "videos",
        "news",
        "map",
        "music",
        "it",
        "science",
        "files",
        "social_media",
    ]
    .into_iter()
    .collect();
    let mut per_category: std::collections::HashMap<&str, usize> = Default::default();
    for e in &file.engines {
        for c in &e.categories {
            if !categories.contains(c.as_str()) {
                errors.push(format!("engine '{}' has unknown category '{c}'", e.name));
            }
            *per_category.entry(c.as_str()).or_default() += 1;
        }
    }

    // Generated-file freshness hint: the generator must have produced this.
    if !text.contains("GENERATED FILE") {
        errors.push("engines.toml was not produced by tools/gen_engines_toml.py".to_string());
    }

    println!(
        "engines.toml: {} engines | waves {}/{}/{} | enabled {enabled} | categories {}",
        file.engines.len(),
        wave_counts(1),
        wave_counts(2),
        wave_counts(3),
        per_category
            .iter()
            .map(|(c, n)| format!("{c}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if errors.is_empty() {
        println!("OK: engines.toml conforms to PLAN.md Appendix A");
        Ok(())
    } else if check {
        for e in &errors {
            eprintln!("DRIFT: {e}");
        }
        bail!("{} conformance error(s)", errors.len());
    } else {
        for e in &errors {
            eprintln!("warning: {e}");
        }
        Ok(())
    }
}
