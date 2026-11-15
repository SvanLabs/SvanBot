//! The send limiter. The venue takes at most 20 messages a second on one connection and drops the
//! rest with `rate_limited` (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02). A
//! dropped action costs the turn, so the client waits rather than sends past the limit.
//!
//! The clock is passed in, which keeps the limiter testable without sleeping.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// At most `limit` messages in any `window`, one connection's worth.
#[derive(Clone, Debug)]
pub struct Limiter {
    limit: usize,
    window: Duration,
    /// When each recent message went, oldest first.
    sent: VecDeque<Instant>,
}

impl Limiter {
    /// `limit` messages per `window`. A limit of 0 is taken as 1.
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit: limit.max(1),
            window,
            sent: VecDeque::new(),
        }
    }

    /// The venue's limit, 20 a second, with a tenth of a second's margin on the window for the
    /// gap between our clock and the venue's.
    pub fn venue() -> Self {
        Self::new(20, Duration::from_millis(1100))
    }

    /// Admits one message that is ready at `now`, and says how long to wait before sending it.
    /// The message counts from when it goes, `now` plus the wait.
    pub fn admit(&mut self, now: Instant) -> Duration {
        while self.sent.front().is_some_and(|&at| at + self.window <= now) {
            self.sent.pop_front();
        }
        let goes = match self.sent.len() < self.limit {
            true => now,
            false => self.sent[self.sent.len() - self.limit] + self.window,
        };
        self.sent.push_back(goes);
        goes - now
    }
}
