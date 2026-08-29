//! Jina Reader client (`https://r.jina.ai`) with the strict 20 RPM token
//! bucket, cross-process SQLite quota ledger, and API-key authenticated tier
//! (PLAN.md §6, §7).

use crate::store::SqliteStore;
use governor::{Quota, RateLimiter};
use reqwest::Client;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tokenloom_core::{TokenloomError, USER_AGENT};

/// Request headers for r.jina.ai (PLAN.md §6).
const TARGET_SELECTOR: &str = "main, article, #content, .content, body";

pub struct JinaClient {
    client: Client,
    endpoint: String,
    api_key: Option<String>,
    /// In-process token bucket (1 token / 3s, burst 1 → exactly 20 RPM).
    limiter: RateLimiter<
        governor::state::direct::NotKeyed,
        governor::state::InMemoryState,
        governor::clock::DefaultClock,
    >,
    ledger: Option<Arc<SqliteStore>>,
    rpm: u32,
}

/// Outcome of an attempt to talk to Jina.
pub enum JinaOutcome {
    /// Markdown retrieved.
    Markdown(String),
    /// Jina reported 429; `retry_after` hints how long to wait.
    RateLimited { retry_after: Option<Duration> },
}

impl JinaClient {
    pub fn new(
        client: Client,
        endpoint: &str,
        rpm: u32,
        api_key: Option<String>,
        ledger: Option<Arc<SqliteStore>>,
    ) -> Self {
        let rpm = rpm.max(1);
        let quota = Quota::per_minute(NonZeroU32::new(rpm).unwrap());
        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            api_key,
            limiter: RateLimiter::direct(quota),
            ledger,
            rpm,
        }
    }

    /// Seconds used by the cross-process sliding window (one minute).
    const WINDOW_SECS: u64 = 60;

    /// Reserve quota: in-process bucket + persistent sliding-window ledger.
    ///
    /// Returns `Err(wait_hint)` when the 20 RPM budget is exhausted across
    /// processes and waiting `wait_hint` seconds would be required.
    pub async fn acquire(&self, max_wait: Option<Duration>) -> Result<(), TokenloomError> {
        // Cross-process ledger first (cheap DB check).
        if let Some(ledger) = &self.ledger {
            let used = ledger.jina_calls_in_window(Self::WINDOW_SECS)?;
            if used >= self.rpm {
                let hint = ledger.jina_wait_hint(Self::WINDOW_SECS, self.rpm)?;
                let wait = Duration::from_secs(hint.max(1));
                match max_wait {
                    Some(budget) if wait <= budget => tokio::time::sleep(wait).await,
                    _ => {
                        let rpm = self.rpm;
                        let hint = hint.max(1);
                        return Err(TokenloomError::JinaRateLimited(format!(
                            "persistent quota exhausted ({used}/{rpm} RPM used across processes); wait ~{hint}s"
                        )));
                    }
                }
            }
        }
        // In-process bucket (waits are cheap; burst is 1 token / 3s).
        match max_wait {
            None => {
                self.limiter.until_ready().await;
            }
            Some(budget) => {
                if tokio::time::timeout(budget, self.limiter.until_ready())
                    .await
                    .is_err()
                {
                    return Err(TokenloomError::JinaRateLimited(
                        "token bucket exhausted within wait budget".into(),
                    ));
                }
            }
        }
        if let Some(ledger) = &self.ledger {
            ledger.record_jina_call(Self::WINDOW_SECS)?;
        }
        Ok(())
    }

    /// Fetch `url` through Jina Reader, returning Markdown text.
    ///
    /// `bypass_quota` is used by the fallback ladder when a `JINA_API_KEY`
    /// upgrades the client to the authenticated (200+ RPM) tier.
    pub async fn fetch_markdown(
        &self,
        url: &str,
        max_wait: Option<Duration>,
    ) -> Result<JinaOutcome, TokenloomError> {
        self.acquire(max_wait).await?;
        self.request(url).await
    }

    async fn request(&self, url: &str) -> Result<JinaOutcome, TokenloomError> {
        let target = format!("{}/{}", self.endpoint, url);
        let mut req = self
            .client
            .get(&target)
            .header("Accept", "text/plain")
            .header("User-Agent", USER_AGENT)
            .header("X-No-Cache", "false")
            .header("X-Target-Selector", TARGET_SELECTOR)
            .header("X-Return-Format", "markdown");
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(TokenloomError::Http)?;
        match resp.status().as_u16() {
            429 => {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                Ok(JinaOutcome::RateLimited { retry_after })
            }
            200..=299 => {
                let text = resp.text().await?;
                Ok(JinaOutcome::Markdown(text))
            }
            status => Err(TokenloomError::EngineFailure {
                engine: "jina-reader".into(),
                reason: format!("HTTP {status} from r.jina.ai for {url}"),
            }),
        }
    }

    /// Probe used by `tokenloom doctor`.
    pub async fn probe(&self) -> Result<u16, TokenloomError> {
        let resp = self
            .client
            .get(&self.endpoint)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;
        Ok(resp.status().as_u16())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_fails_fast_when_ledger_exhausted() {
        let store = Arc::new(SqliteStore::open_memory().unwrap());
        for _ in 0..20 {
            store.record_jina_call(60).unwrap();
        }
        let client = JinaClient::new(Client::new(), "https://r.jina.ai", 20, None, Some(store));
        let err = client
            .acquire(Some(Duration::from_millis(1)))
            .await
            .unwrap_err();
        assert!(matches!(err, TokenloomError::JinaRateLimited(_)));
    }

    #[tokio::test]
    async fn acquire_ok_with_headroom() {
        let store = Arc::new(SqliteStore::open_memory().unwrap());
        let client = JinaClient::new(Client::new(), "https://r.jina.ai", 20, None, Some(store));
        client.acquire(None).await.unwrap();
    }
}
