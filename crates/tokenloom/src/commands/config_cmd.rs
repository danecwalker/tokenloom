//! `tokenloom config path|get` (PLAN.md §8, §9).

use crate::EnginesCommands;
use tokenloom_core::Config;

pub fn run(config: &Config, cmd: crate::ConfigCommands) -> i32 {
    match cmd {
        crate::ConfigCommands::Path => {
            if let Some(p) = Config::user_config_path() {
                println!("user config:     {}", p.display());
            } else {
                println!("user config:     <unavailable>");
            }
            println!("project config:  ./.tokenloom.toml");
            if let Some(dir) = Config::cache_dir() {
                println!("cache directory: {}", dir.display());
            }
            0
        }
        crate::ConfigCommands::Get { key } => match config.get_value(key.as_deref()) {
            Some(v) => {
                println!("{v}");
                0
            }
            None => {
                eprintln!(
                    "tokenloom: unknown config key '{}'",
                    key.as_deref().unwrap_or("")
                );
                1
            }
        },
    }
}

/// Keep the enum import referenced (command tree lives in main.rs).
#[allow(dead_code)]
fn _enum_ref(_e: EnginesCommands) {}
