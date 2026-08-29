//! TOML configuration schema & loaders (PLAN.md §9).
//!
//! Precedence (highest → lowest):
//! 1. CLI flags (applied by the caller on the returned `Config`)
//! 2. `TOKENLOOM_*` environment variables
//! 3. `./.tokenloom.toml` (local project override)
//! 4. `~/.config/tokenloom/config.toml` (user global)
//! 5. Built-in compiled defaults (this file)

use crate::error::TokenloomError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Built-in compiled defaults (PLAN.md §9 master example).
impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            http: HttpConfig::default(),
            sanitizer: SanitizerConfig::default(),
            reader: ReaderConfig::default(),
            cache: CacheConfig::default(),
            engines: EnginesConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub http: HttpConfig,
    pub sanitizer: SanitizerConfig,
    pub reader: ReaderConfig,
    pub cache: CacheConfig,
    pub engines: EnginesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub default_category: String,
    pub default_limit: usize,
    pub timeout_ms: u64,
    /// 0 = off, 1 = moderate, 2 = strict
    pub safe_search: u8,
    /// markdown | json | plain
    pub output_format: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_category: "general".into(),
            default_limit: 10,
            timeout_ms: 4000,
            safe_search: 1,
            output_format: "markdown".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub user_agent: String,
    pub max_response_size_mb: u64,
    pub connect_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub follow_redirects: bool,
    pub max_redirects: usize,
    /// Optional: "socks5://127.0.0.1:9050" or "http://proxy:8080"
    pub proxy: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: crate::USER_AGENT.into(),
            max_response_size_mb: 5,
            connect_timeout_ms: 2000,
            total_timeout_ms: 8000,
            follow_redirects: true,
            max_redirects: 5,
            proxy: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SanitizerConfig {
    pub allow_images: bool,
    /// inline | footnotes | strip
    pub link_format: String,
    pub max_characters: usize,
    pub escape_code_fences: bool,
    pub delimit_untrusted: bool,
}

impl Default for SanitizerConfig {
    fn default() -> Self {
        Self {
            allow_images: false,
            link_format: "inline".into(),
            max_characters: 50_000,
            escape_code_fences: true,
            delimit_untrusted: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReaderConfig {
    pub enable_spa_detection: bool,
    pub jina_endpoint: String,
    pub jina_rate_limit_rpm: u32,
    pub jina_api_key: String,
    pub enable_local_headless: bool,
    pub headless_timeout_ms: u64,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            enable_spa_detection: true,
            jina_endpoint: "https://r.jina.ai".into(),
            jina_rate_limit_rpm: 20,
            jina_api_key: String::new(),
            enable_local_headless: true,
            headless_timeout_ms: 6000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub enabled: bool,
    pub db_path: String,
    pub ttl_seconds: u64,
    /// Extra multiplier over `ttl_seconds` during which stale entries may be
    /// served when the network fails (stale-while-revalidate, PLAN.md §6).
    pub stale_revalidate_multiplier: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: "~/.cache/tokenloom/cache.db".into(),
            ttl_seconds: 7200,
            stale_revalidate_multiplier: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnginesConfig {
    pub weights: BTreeMap<String, f64>,
    pub overrides: BTreeMap<String, EngineOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct EngineOverride {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub weight: Option<f64>,
}

impl Config {
    /// Load config following the documented precedence chain.
    pub fn load(explicit_path: Option<&Path>) -> Result<Config, TokenloomError> {
        let mut cfg = Config::default();

        // 5 → 4: user global
        if let Some(p) = Self::user_config_path() {
            Self::merge_file(&mut cfg, &p, false)?;
        }
        // 3: local project override
        let local = PathBuf::from(".tokenloom.toml");
        Self::merge_file(&mut cfg, &local, false)?;
        // 1: explicit --config file wins over everything below the CLI flags
        if let Some(p) = explicit_path {
            Self::merge_file(&mut cfg, p, true)?;
        }
        // 2: environment variables
        cfg.apply_env();
        Ok(cfg)
    }

    fn merge_file(cfg: &mut Config, path: &Path, required: bool) -> Result<(), TokenloomError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if required {
                    return Err(TokenloomError::Config(format!(
                        "config file not found: {}",
                        path.display()
                    )));
                }
                return Ok(());
            }
            Err(e) => {
                return Err(TokenloomError::Config(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };
        let overlay: toml::Value = toml::from_str(&text).map_err(|e| {
            TokenloomError::Config(format!("invalid TOML in {}: {e}", path.display()))
        })?;
        // Value-tree merge: keys present in the overlay win, all others keep
        // their current (default or lower-precedence) value.
        let mut base = toml::Value::try_from(&*cfg)
            .map_err(|e| TokenloomError::Config(format!("cannot encode config: {e}")))?;
        merge_toml(&mut base, overlay);
        *cfg = base
            .try_into()
            .map_err(|e: toml::de::Error| TokenloomError::Config(format!("invalid config: {e}")))?;
        Ok(())
    }

    /// Overlay-merge: only keys present in `overlay` take effect.
    fn apply_env(&mut self) {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = env("TOKENLOOM_DEFAULT_CATEGORY") {
            self.general.default_category = v;
        }
        if let Some(v) = env("TOKENLOOM_DEFAULT_LIMIT").and_then(|v| v.parse().ok()) {
            self.general.default_limit = v;
        }
        if let Some(v) = env("TOKENLOOM_TIMEOUT_MS").and_then(|v| v.parse().ok()) {
            self.general.timeout_ms = v;
        }
        if let Some(v) = env("TOKENLOOM_SAFE_SEARCH").and_then(|v| v.parse().ok()) {
            self.general.safe_search = v;
        }
        if let Some(v) = env("TOKENLOOM_OUTPUT_FORMAT") {
            self.general.output_format = v;
        }
        if let Some(v) = env("TOKENLOOM_USER_AGENT") {
            self.http.user_agent = v;
        }
        if let Some(v) = env("TOKENLOOM_HTTP_PROXY") {
            self.http.proxy = v;
        }
        if let Some(v) = env("TOKENLOOM_MAX_RESPONSE_SIZE_MB").and_then(|v| v.parse().ok()) {
            self.http.max_response_size_mb = v;
        }
        // API keys are read from the environment or mode-0600 config files and
        // never echoed in error messages (PLAN.md §11).
        if let Ok(v) = std::env::var("JINA_API_KEY") {
            if !v.is_empty() {
                self.reader.jina_api_key = v;
            }
        }
        if let Some(v) = env("TOKENLOOM_JINA_ENDPOINT") {
            self.reader.jina_endpoint = v;
        }
        if let Some(v) = env("TOKENLOOM_JINA_RATE_LIMIT_RPM").and_then(|v| v.parse().ok()) {
            self.reader.jina_rate_limit_rpm = v;
        }
        if let Some(v) = env("TOKENLOOM_CACHE_DB") {
            self.cache.db_path = v;
        }
        if let Some(v) = env("TOKENLOOM_CACHE_TTL_SECONDS").and_then(|v| v.parse().ok()) {
            self.cache.ttl_seconds = v;
        }
    }

    /// Effective config directory: `$XDG_CONFIG_HOME/tokenloom` or `~/.config/tokenloom`.
    pub fn config_dir() -> Option<PathBuf> {
        base_dir("XDG_CONFIG_HOME", ".config").map(|p| p.join("tokenloom"))
    }

    /// Effective cache directory: `$XDG_CACHE_HOME/tokenloom` or `~/.cache/tokenloom`.
    pub fn cache_dir() -> Option<PathBuf> {
        base_dir("XDG_CACHE_HOME", ".cache").map(|p| p.join("tokenloom"))
    }

    /// `~/.config/tokenloom/config.toml` if resolvable.
    pub fn user_config_path() -> Option<PathBuf> {
        Self::config_dir().map(|p| p.join("config.toml"))
    }

    /// Resolved cache DB path (expands `~` and creates parent dirs lazily).
    pub fn cache_db_path(&self) -> PathBuf {
        expand_tilde(&self.cache.db_path)
    }

    /// Resolve a dotted config key like `http.proxy` for `tokenloom config get`.
    pub fn get_value(&self, key: Option<&str>) -> Option<String> {
        let value = toml::Value::try_from(self).ok()?;
        match key {
            None => Some(toml::to_string_pretty(&value).ok()?),
            Some(k) => {
                let mut cur = &value;
                for part in k.split('.') {
                    cur = cur.get(part)?;
                }
                Some(match cur {
                    toml::Value::Table(t) => toml::to_string_pretty(t).ok()?,
                    toml::Value::Array(a) => a
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => f.to_string(),
                    toml::Value::Boolean(b) => b.to_string(),
                    toml::Value::Datetime(d) => d.to_string(),
                })
            }
        }
    }
}

fn base_dir(env_var: &str, default_suffix: &str) -> Option<PathBuf> {
    if let Ok(v) = std::env::var(env_var) {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    home_dir().map(|h| h.join(default_suffix))
}

/// Recursive TOML value merge used by the config cascade.
fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge_toml(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Expand a leading `~` or `~user` (single-user systems only) in a path.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(h) = home_dir() {
            return h;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(h) = home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_plan() {
        let c = Config::default();
        assert_eq!(c.general.default_limit, 10);
        assert_eq!(c.general.timeout_ms, 4000);
        assert_eq!(c.http.max_response_size_mb, 5);
        assert_eq!(c.reader.jina_rate_limit_rpm, 20);
        assert_eq!(c.reader.jina_endpoint, "https://r.jina.ai");
        assert_eq!(c.cache.ttl_seconds, 7200);
        assert!(!c.sanitizer.allow_images);
        assert_eq!(c.sanitizer.max_characters, 50_000);
    }

    #[test]
    fn parses_plan_example() {
        let text = r#"
[general]
default_category = "science"
default_limit = 5
timeout_ms = 2500

[http]
proxy = "socks5://127.0.0.1:9050"

[sanitizer]
allow_images = true
link_format = "footnotes"

[reader]
jina_rate_limit_rpm = 200

[cache]
ttl_seconds = 60

[engines.weights]
wikipedia = 1.2

[engines.overrides]
google = { enabled = false, timeout_ms = 3000 }
google_scholar = { enabled = true }
"#;
        let c: Config = toml::from_str(text).unwrap();
        assert_eq!(c.general.default_category, "science");
        assert_eq!(c.general.default_limit, 5);
        assert_eq!(c.general.timeout_ms, 2500);
        assert_eq!(c.http.proxy, "socks5://127.0.0.1:9050");
        // untouched fields keep defaults
        assert_eq!(c.http.max_response_size_mb, 5);
        assert_eq!(c.reader.jina_endpoint, "https://r.jina.ai");
        assert_eq!(c.reader.jina_rate_limit_rpm, 200);
        assert!(c.sanitizer.allow_images);
        assert_eq!(c.engines.weights["wikipedia"], 1.2);
        assert_eq!(c.engines.overrides["google"].timeout_ms, Some(3000));
        assert_eq!(c.engines.overrides["google_scholar"].enabled, Some(true));
    }

    #[test]
    fn overlay_merges_only_present_keys() {
        let base = Config::default();
        let overlay: toml::Value = toml::from_str("[general]\ndefault_limit = 3").unwrap();
        let mut merged = base.clone();
        let mut base_v = toml::Value::try_from(&merged).unwrap();
        merge_toml(&mut base_v, overlay);
        merged = toml::Value::try_into(base_v).unwrap();
        assert_eq!(merged.general.default_limit, 3);
        assert_eq!(merged.general.timeout_ms, 4000);
        assert_eq!(merged.http.proxy, "");
    }

    #[test]
    fn get_value_dotted_key() {
        let c = Config::default();
        assert_eq!(c.get_value(Some("http.proxy")).unwrap(), "");
        assert_eq!(
            c.get_value(Some("reader.jina_rate_limit_rpm")).unwrap(),
            "20"
        );
        assert_eq!(c.get_value(Some("general.default_limit")).unwrap(), "10");
        assert!(c.get_value(Some("no.such.key")).is_none());
    }

    #[test]
    fn tilde_expansion() {
        std::env::set_var("HOME", "/Users/test");
        assert_eq!(
            expand_tilde("~/.cache/tokenloom/cache.db"),
            PathBuf::from("/Users/test/.cache/tokenloom/cache.db")
        );
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }
}
