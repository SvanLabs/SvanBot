//! The installer behind `scripts/release.sh`, `scripts/update.sh` and `scripts/rollback.sh` (#742).
//!
//! Every behaviour keeps the bytes the shell scripts produce, on the screen and on disk, so either
//! side can operate on state the other wrote: snapshots and `SHA256SUMS`, the swap journal,
//! `release-progress.json`, `releases.log`, the identity marker and the lock files. The design is
//! recorded on #716; this crate takes the surfaces over one at a time, read-only ones first.

pub mod binaries;
pub mod cli;
pub mod error;
pub mod fsops;
pub mod gitops;
pub mod identity;
pub mod journal;
pub mod layout;
pub mod lock;
pub mod manifest;
pub mod restore;
pub mod snapshot;
pub mod space;
pub mod store_format;
pub mod swap;

#[cfg(test)]
mod tests;
