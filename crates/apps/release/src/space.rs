//! The disk preflight (`check_space`): a release refuses to start when the root's volume is short.

use crate::error::{ReleaseError, Result};
use std::path::Path;

/// Megabytes a release needs free when `SV10_MIN_FREE_MB` is unset.
pub const DEFAULT_MIN_FREE_MB: u64 = 4096;

/// Check `required_mb` (or `SV10_MIN_FREE_MB`, or 4096) against the space free for `root`.
/// `setting` is the raw value of `SV10_MIN_FREE_MB`. Returns the line to print on success.
pub fn check(root: &Path, setting: Option<&str>) -> Result<String> {
    let required = match setting {
        None => DEFAULT_MIN_FREE_MB,
        Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s.parse().map_err(|_| ReleaseError::SpaceSetting)?,
        Some(_) => return Err(ReleaseError::SpaceSetting),
    };
    let avail = sv10_rt::free_bytes(root).ok_or_else(|| ReleaseError::SpaceUnreadable(root.display().to_string()))?;
    check_bytes(root, avail, required)
}

/// The comparison alone, for tests: `avail` bytes against `required` megabytes.
pub fn check_bytes(root: &Path, avail: u64, required: u64) -> Result<String> {
    let avail_kb = avail / 1024;
    if avail_kb < required.saturating_mul(1024) {
        return Err(ReleaseError::SpaceLow { avail_mb: avail_kb / 1024, root: root.display().to_string(), required });
    }
    Ok(format!("{} MB free on {} (>= {} MB)", avail_kb / 1024, root.display(), required))
}
