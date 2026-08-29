//! CLI command implementations.

pub mod bangs;
pub mod config_cmd;
pub mod doctor;
pub mod engines;
pub mod fetch;
pub mod mcp;
pub mod search;

use std::sync::Arc;
use tokenloom_core::Config;
use tokenloom_engines::Registry;
use tokenloom_fetch::FetchClient;

/// Shared per-invocation context.
pub struct App {
    pub config: Config,
    pub registry: Arc<Registry>,
    pub client: FetchClient,
}

impl App {
    pub fn new(config: &Config) -> Result<Self, tokenloom_core::TokenloomError> {
        let registry = Registry::load()?;
        let client = FetchClient::new(&config.http)?;
        Ok(Self {
            config: config.clone(),
            registry: Arc::new(registry),
            client,
        })
    }
}
