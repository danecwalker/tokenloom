//! Persistent SQLite cache & cross-process quota ledger (PLAN.md §6,
//! *Persistent Quota Ledger & Caching*).
//!
//! - `page_cache`: canonical URL → markdown + freshness metadata (default TTL
//!   2 hours, stale-while-revalidate window via multiplier).
//! - `jina_quota_log`: timestamps of recent Jina Reader calls enforcing the
//!   20 RPM sliding window across independent CLI invocations.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokenloom_core::{RenderMethod, TokenloomError};

/// A cached page with its freshness information.
#[derive(Debug, Clone)]
pub struct CachedPage {
    pub canonical_url: String,
    pub title: String,
    pub markdown: String,
    pub render_method: RenderMethod,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub fetched_at: u64,
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl SqliteStore {
    /// Open (creating schema if needed). Parent directories are created.
    pub fn open(path: &Path) -> Result<Self, TokenloomError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)
            .map_err(|e| TokenloomError::Cache(format!("cannot open cache DB {path:?}: {e}")))?;
        Self::with_connection(conn)
    }

    /// In-memory store (tests / `--cache.enabled = false` paths).
    pub fn open_memory() -> Result<Self, TokenloomError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| TokenloomError::Cache(format!("cannot open memory cache: {e}")))?;
        Self::with_connection(conn)
    }

    fn with_connection(conn: Connection) -> Result<Self, TokenloomError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS page_cache (
                canonical_url TEXT PRIMARY KEY,
                title          TEXT NOT NULL DEFAULT '',
                markdown       TEXT NOT NULL,
                render_method  TEXT NOT NULL DEFAULT 'StaticDirect',
                etag           TEXT,
                last_modified  TEXT,
                fetched_at     INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS jina_quota_log (
                ts INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_jina_quota_ts ON jina_quota_log(ts);",
        )
        .map_err(|e| TokenloomError::Cache(format!("cannot init cache schema: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_page(&self, canonical_url: &str) -> Result<Option<CachedPage>, TokenloomError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TokenloomError::Cache("lock poisoned".into()))?;
        let row = conn
            .query_row(
                "SELECT canonical_url, title, markdown, render_method, etag, last_modified, fetched_at
                 FROM page_cache WHERE canonical_url = ?1",
                params![canonical_url],
                |r| {
                    let method: String = r.get(3)?;
                    Ok(CachedPage {
                        canonical_url: r.get(0)?,
                        title: r.get(1)?,
                        markdown: r.get(2)?,
                        render_method: parse_render_method(&method),
                        etag: r.get(4)?,
                        last_modified: r.get(5)?,
                        fetched_at: r.get::<_, i64>(6)? as u64,
                    })
                },
            )
            .optional()
            .map_err(|e| TokenloomError::Cache(format!("cache read failed: {e}")))?;
        Ok(row)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_page(
        &self,
        canonical_url: &str,
        title: &str,
        markdown: &str,
        render_method: RenderMethod,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<(), TokenloomError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TokenloomError::Cache("lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO page_cache (canonical_url, title, markdown, render_method, etag, last_modified, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(canonical_url) DO UPDATE SET
                title = excluded.title,
                markdown = excluded.markdown,
                render_method = excluded.render_method,
                etag = excluded.etag,
                last_modified = excluded.last_modified,
                fetched_at = excluded.fetched_at",
            params![
                canonical_url,
                title,
                markdown,
                render_method.as_str(),
                etag,
                last_modified,
                now_epoch() as i64
            ],
        )
        .map_err(|e| TokenloomError::Cache(format!("cache write failed: {e}")))?;
        Ok(())
    }

    /// Number of Jina calls recorded within the trailing `window_secs`.
    pub fn jina_calls_in_window(&self, window_secs: u64) -> Result<u32, TokenloomError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TokenloomError::Cache("lock poisoned".into()))?;
        let since = now_epoch().saturating_sub(window_secs) as i64;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jina_quota_log WHERE ts >= ?1",
                params![since],
                |r| r.get(0),
            )
            .map_err(|e| TokenloomError::Cache(format!("quota read failed: {e}")))?;
        Ok(count as u32)
    }

    /// Record a Jina call (pruning old rows in the same transaction).
    pub fn record_jina_call(&self, window_secs: u64) -> Result<(), TokenloomError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TokenloomError::Cache("lock poisoned".into()))?;
        let now = now_epoch() as i64;
        conn.execute("INSERT INTO jina_quota_log (ts) VALUES (?1)", params![now])
            .map_err(|e| TokenloomError::Cache(format!("quota write failed: {e}")))?;
        conn.execute(
            "DELETE FROM jina_quota_log WHERE ts < ?1",
            params![now.saturating_sub(window_secs as i64 * 2)],
        )
        .map_err(|e| TokenloomError::Cache(format!("quota prune failed: {e}")))?;
        Ok(())
    }

    /// Seconds until the next quota slot frees (0 if a slot is available now).
    pub fn jina_wait_hint(
        &self,
        window_secs: u64,
        max_per_window: u32,
    ) -> Result<u64, TokenloomError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TokenloomError::Cache("lock poisoned".into()))?;
        let now = now_epoch();
        let since = now.saturating_sub(window_secs) as i64;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jina_quota_log WHERE ts >= ?1",
                params![since],
                |r| r.get(0),
            )
            .map_err(|e| TokenloomError::Cache(format!("quota hint failed: {e}")))?;
        if (count as u32) < max_per_window {
            return Ok(0);
        }
        // Window full: when the oldest of the `max_per_window` most-recent
        // calls exits the window, a slot frees up.
        let oldest_in_window: Option<i64> = conn
            .query_row(
                "SELECT MIN(ts) FROM (
                    SELECT ts FROM jina_quota_log WHERE ts >= ?1
                    ORDER BY ts DESC LIMIT ?2
                 )",
                params![since, max_per_window],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| TokenloomError::Cache(format!("quota hint failed: {e}")))?
            .flatten();
        match oldest_in_window {
            None => Ok(0),
            Some(oldest) => Ok((oldest as u64 + window_secs).saturating_sub(now) + 1),
        }
    }
}

fn parse_render_method(s: &str) -> RenderMethod {
    match s {
        "JinaReader" => RenderMethod::JinaReader,
        "LocalHeadless" => RenderMethod::LocalHeadless,
        "DegradedStatic" => RenderMethod::DegradedStatic,
        "Cache" => RenderMethod::Cache,
        _ => RenderMethod::StaticDirect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_cache_roundtrip() {
        let store = SqliteStore::open_memory().unwrap();
        assert!(store.get_page("https://example.com").unwrap().is_none());
        store
            .put_page(
                "https://example.com",
                "Example",
                "# hi",
                RenderMethod::StaticDirect,
                Some("\"v1\""),
                None,
            )
            .unwrap();
        let page = store.get_page("https://example.com").unwrap().unwrap();
        assert_eq!(page.title, "Example");
        assert_eq!(page.markdown, "# hi");
        assert_eq!(page.render_method, RenderMethod::StaticDirect);
        assert_eq!(page.etag.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn jina_quota_window() {
        let store = SqliteStore::open_memory().unwrap();
        assert_eq!(store.jina_calls_in_window(60).unwrap(), 0);
        for _ in 0..3 {
            store.record_jina_call(60).unwrap();
        }
        assert_eq!(store.jina_calls_in_window(60).unwrap(), 3);
        // Window exhausted → hint > 0 for a 20-per-60s budget (3 << 20 gives 0).
        assert_eq!(store.jina_wait_hint(60, 20).unwrap(), 0);
        assert!(store.jina_wait_hint(60, 3).unwrap() > 0);
    }
}
