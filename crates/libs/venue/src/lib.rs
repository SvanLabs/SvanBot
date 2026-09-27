//! Venue protocol logic for SvanBot, free of network I/O: `tracker` turns openpoker.ai frames
//! into the decision `Situation` and finished `HandSummary`s (live and replayed from archives), and
//! `statehash` verifies the server's `state_hash` for each table snapshot.
//!
//! Part of the SvanBot workspace; `sv10-bot` re-exports both modules under its own paths.

#![warn(missing_docs)]

pub mod statehash;
pub mod tracker;
