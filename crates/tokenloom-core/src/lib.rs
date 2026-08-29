//! `tokenloom-core` — common types, errors, configuration and URL utilities
//! shared by every other crate in the workspace.
//!
//! See `PLAN.md` §3 (Architecture), §4 (Data Model) and §9 (Configuration).

pub mod config;
pub mod error;
pub mod model;
pub mod url_util;

pub use config::{
    CacheConfig, Config, EnginesConfig, GeneralConfig, HttpConfig, ReaderConfig, SanitizerConfig,
};
pub use error::{Result, TokenloomError};
pub use model::{
    estimate_tokens, Category, EngineFailure, FetchedPage, RenderMethod, SearchQuery,
    SearchResponse, SearchResult,
};

/// User agent used for all outbound HTTP traffic (PLAN.md §6, §9).
pub const USER_AGENT: &str = "tokenloom/0.1.0 (+https://github.com/danewalker/tokenloom)";
