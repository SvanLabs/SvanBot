//! The `svanbot` app's own pieces, kept out of `main` so they can be tested at their seams.
//!
//! An app does I/O, so it turns off the `clippy.toml` bans here, at this crate root (ADR 0003;
//! `main.rs` is the binary's own root and does the same there).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

pub mod dashboard;
pub mod learn;
pub mod live;
pub mod live_policy;

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in Unix milliseconds. The apps are where the clock lives (ADR 0003).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}
