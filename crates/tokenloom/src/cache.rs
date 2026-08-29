//! Cache handling for CLI commands (PLAN.md §6 *Persistent Quota Ledger*).

use std::path::PathBuf;
use std::sync::Arc;
use tokenloom_core::{CacheConfig, TokenloomError};
use tokenloom_fetch::SqliteStore;

/// Resolve the cache DB path, expanding `~`.
pub fn db_path(cfg: &CacheConfig) -> PathBuf {
    tokenloom_core::config::expand_tilde(&cfg.db_path)
}

/// Open the persistent store unless caching is disabled.
pub fn open_store(cfg: &CacheConfig) -> Result<Option<Arc<SqliteStore>>, TokenloomError> {
    if !cfg.enabled {
        return Ok(None);
    }
    let store = SqliteStore::open(&db_path(cfg))?;
    Ok(Some(Arc::new(store)))
}
