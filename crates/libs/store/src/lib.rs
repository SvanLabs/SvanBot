//! Persistence for SvanBot: `store` (the main SQLite database), `packed` (compressed cold JSON
//! columns, 0229), `integrity` (startup checks,
//! sealed backups, quarantine and restore, per-hand content digests) and `archive` (daily, weekly
//! and monthly archives on a second disk with sealed manifests and verified restore).
//!
//! Part of the SvanBot workspace; `sv10-bot` re-exports both modules under its own paths.

#![warn(missing_docs)]

pub mod archive;
pub mod integrity;
pub mod packed;
pub mod store;
