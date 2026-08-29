//! `tokenloom-fetch` — safe HTTP client, SSRF guard, SPA detector, Jina
//! Reader client with the 20 RPM token bucket, and the rate-limit fallback
//! ladder (PLAN.md §6, §7 Layer 1).

pub mod client;
pub mod fallback;
pub mod headless;
pub mod jina;
pub mod spa_detector;
pub mod ssrf;
pub mod store;

pub use client::{FetchClient, RawFetch};
pub use fallback::{FetchOptions, Fetcher, DEGRADED_WARNING};
pub use jina::JinaClient;
pub use ssrf::{ip_is_blocked, validate_url};
pub use store::{CachedPage, SqliteStore};
