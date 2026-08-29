//! Typed error hierarchy (PLAN.md §10).

use thiserror::Error;

/// Convenience alias used across the workspace.
pub type Result<T> = std::result::Result<T, TokenloomError>;

/// Every failure mode `tokenloom` can produce.
#[derive(Error, Debug)]
pub enum TokenloomError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("SSRF violation: destination IP {ip} is in prohibited range")]
    SsrfBlocked { ip: std::net::IpAddr },

    #[error("URL scheme '{scheme}' is not allowed (only http/https)")]
    BadScheme { scheme: String },

    #[error("port {port} is in the browser bad-port blocklist")]
    BadPort { port: u16 },

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Engine '{engine}' failed: {reason}")]
    EngineFailure { engine: String, reason: String },

    #[error("Sanitisation error: {0}")]
    Sanitization(String),

    #[error("Jina Reader rate limit exhausted (20 RPM). Fallback status: {0}")]
    JinaRateLimited(String),

    #[error("Response body exceeded maximum allowed size of {limit} bytes")]
    ResponseTooLarge { limit: u64 },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("No enabled engines matched this query (category={category:?}, engines={engines:?})")]
    NoEngines {
        category: String,
        engines: Vec<String>,
    },
}

impl TokenloomError {
    /// Short machine-readable tag used in diagnostics and MCP responses.
    pub fn kind(&self) -> &'static str {
        match self {
            TokenloomError::Http(_) => "http",
            TokenloomError::SsrfBlocked { .. } => "ssrf_blocked",
            TokenloomError::BadScheme { .. } => "bad_scheme",
            TokenloomError::BadPort { .. } => "bad_port",
            TokenloomError::InvalidUrl(_) => "invalid_url",
            TokenloomError::EngineFailure { .. } => "engine_failure",
            TokenloomError::Sanitization(_) => "sanitization",
            TokenloomError::JinaRateLimited(_) => "jina_rate_limited",
            TokenloomError::ResponseTooLarge { .. } => "response_too_large",
            TokenloomError::Config(_) => "config",
            TokenloomError::Io(_) => "io",
            TokenloomError::Cache(_) => "cache",
            TokenloomError::NoEngines { .. } => "no_engines",
        }
    }
}
