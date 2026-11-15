//! The venue's REST API (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02,
//! "Authentication", "REST API").
//!
//! A [`Rest`] is one API key and that key's limiter, and it cannot be cloned. Every request on
//! the key, retries included, waits its turn on the one limiter. A reply of 429 waits exactly what
//! its `Retry-After` says. A server error backs off. A refusal is final. A list endpoint is read
//! page by page with [`Rest::pages`], which stops at a short page and backs off on an empty one
//! before taking it as the end, so a download never spins on empty pages.

use std::fmt;
use std::time::{Duration, Instant};

use serde_json::Value;
use ureq::Agent;

use crate::limit::Limiter;

/// The venue's REST base.
pub const API: &str = "https://api.openpoker.ai/api";

/// Requests made for one call before it gives up on retries.
pub const MAX_ATTEMPTS: u32 = 6;

/// Tries of an empty page, backing off between them, before it is taken as the end.
pub const EMPTY_PAGE_TRIES: u32 = 3;

/// The longest backoff between two tries.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// How long to wait before trying again after a reply of `status`, on try `attempt` (from 0), or
/// `None` when trying again cannot help.
pub fn retry_wait(status: u16, retry_after: Option<&str>, attempt: u32) -> Option<Duration> {
    if attempt + 1 >= MAX_ATTEMPTS {
        return None;
    }
    match status {
        429 => Some(
            retry_after
                .and_then(|text| text.trim().parse::<u64>().ok())
                .map_or_else(|| backoff(attempt), Duration::from_secs),
        ),
        500..=599 => Some(backoff(attempt)),
        _ => None,
    }
}

/// 1 s, 2 s, 4 s, … up to a minute.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(6)).min(MAX_BACKOFF)
}

/// Why a call failed.
#[derive(Debug)]
pub enum RestError {
    /// The venue refused the key. Retrying with the same key cannot help.
    AuthFailed,
    /// A reply that retrying cannot change, or the last of the retries.
    Status(u16),
    /// The request did not get a reply.
    Transport(String),
    /// The reply was not the JSON the call expects.
    Body(String),
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "the venue refused the key"),
            Self::Status(status) => write!(f, "the venue answered {status}"),
            Self::Transport(why) => write!(f, "no reply: {why}"),
            Self::Body(why) => write!(f, "unreadable reply: {why}"),
        }
    }
}

impl std::error::Error for RestError {}

/// One API key, and the limiter every request on it shares.
pub struct Rest {
    agent: Agent,
    base: String,
    /// `Bearer <key>`. Never printed: `Debug` leaves it out.
    bearer: String,
    limiter: Limiter,
    sleep: Box<dyn FnMut(Duration)>,
}

impl fmt::Debug for Rest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rest")
            .field("base", &self.base)
            .field("key", &"<hidden>")
            .field("limiter", &self.limiter)
            .finish_non_exhaustive()
    }
}

impl Rest {
    /// The client for `key` at `base` (usually [`API`]), paced by `limiter`: the endpoint's limit
    /// for this key, such as 30 a minute for the hand history.
    pub fn new(base: &str, key: &str, limiter: Limiter) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            agent,
            base: base.trim_end_matches('/').to_owned(),
            bearer: format!("Bearer {key}"),
            limiter,
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Waits with `sleep` instead of the thread's sleep, so a test can see every wait.
    #[must_use]
    pub fn with_sleep(mut self, sleep: impl FnMut(Duration) + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// `GET` of `path` (for example `/me/active-game`), retried as [`retry_wait`] says.
    pub fn get(&mut self, path: &str) -> Result<Value, RestError> {
        let url = format!("{}{path}", self.base);
        let mut attempt = 0;
        loop {
            let wait = self.limiter.admit(Instant::now());
            (self.sleep)(wait);
            let mut reply = self
                .agent
                .get(&url)
                .header("Authorization", &self.bearer)
                .call()
                .map_err(|e| RestError::Transport(e.to_string()))?;
            let status = reply.status().as_u16();
            if (200..300).contains(&status) {
                let text = reply
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| RestError::Body(e.to_string()))?;
                return serde_json::from_str(&text).map_err(|e| RestError::Body(e.to_string()));
            }
            if status == 401 {
                return Err(RestError::AuthFailed);
            }
            let retry_after = reply
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok());
            let Some(wait) = retry_wait(status, retry_after, attempt) else {
                return Err(RestError::Status(status));
            };
            (self.sleep)(wait);
            attempt += 1;
        }
    }

    /// Reads the list at `path` `limit` items at a time. `items` finds the list in a page.
    pub fn pages<F>(&mut self, path: &str, limit: u32, items: F) -> Pager<'_, F>
    where
        F: Fn(&Value) -> Option<Vec<Value>>,
    {
        Pager {
            rest: self,
            path: path.to_owned(),
            limit,
            offset: 0,
            items,
            done: false,
        }
    }
}

/// A list endpoint, read a page at a time.
pub struct Pager<'a, F> {
    rest: &'a mut Rest,
    path: String,
    limit: u32,
    offset: u64,
    items: F,
    done: bool,
}

impl<F> Pager<'_, F>
where
    F: Fn(&Value) -> Option<Vec<Value>>,
{
    /// Starts at `offset` instead of 0: where an earlier read stopped.
    #[must_use]
    pub fn from(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }

    /// The offset of the next page.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The next page, or `None` at the end. A page shorter than the limit is the last one. An
    /// empty page is tried again, backing off, up to [`EMPTY_PAGE_TRIES`] times in all, and then
    /// taken as the end.
    pub fn next_page(&mut self) -> Result<Option<Vec<Value>>, RestError> {
        if self.done {
            return Ok(None);
        }
        let join = if self.path.contains('?') { '&' } else { '?' };
        let path = format!(
            "{}{join}limit={}&offset={}",
            self.path, self.limit, self.offset
        );
        for attempt in 0..EMPTY_PAGE_TRIES {
            if attempt > 0 {
                (self.rest.sleep)(backoff(attempt));
            }
            let page = self.rest.get(&path)?;
            let items = (self.items)(&page)
                .ok_or_else(|| RestError::Body(format!("no list in the page at {path}")))?;
            if !items.is_empty() {
                self.offset += items.len() as u64;
                self.done = items.len() < self.limit as usize;
                return Ok(Some(items));
            }
        }
        self.done = true;
        Ok(None)
    }
}
