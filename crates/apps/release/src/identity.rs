//! Which commit is installed, and what a commit name resolves to (`resolve_commit`,
//! `installed_commit` in `rollback.sh`).

use crate::error::{ReleaseError, Result};
use crate::gitops;
use std::path::Path;

/// The identity marker an installed set carries next to its executables.
pub const MARKER: &str = ".sv10-installed-commit";

/// `requested` as a seven-character commit hash, or why it is not one.
pub fn resolve_commit(root: &Path, requested: &str) -> Result<String> {
    if requested.is_empty() {
        return Err(ReleaseError::CommitEmpty);
    }
    let full = gitops::verify_commit(root, requested).ok_or_else(|| ReleaseError::CommitUnresolved(requested.to_string()))?;
    gitops::short7(root, &full)
}

/// The commit of the installed build: the marker file when it exists (a marker that does not
/// resolve is an error, as in the shell), else the newest `artifacts/releases.log` entry whose
/// second field resolves. `Ok(None)` when neither names a commit.
pub fn installed_commit(root: &Path) -> Result<Option<String>> {
    let marker = root.join("target/release").join(MARKER);
    if marker.is_file() {
        let text = std::fs::read_to_string(&marker).map_err(|e| ReleaseError::Io(format!("cannot read {}: {e}", marker.display())))?;
        let candidate: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        return resolve_commit(root, &candidate).map(Some);
    }
    let Ok(log) = std::fs::read_to_string(root.join("artifacts/releases.log")) else { return Ok(None) };
    for line in log.lines().rev() {
        let Some(candidate) = line.split_whitespace().nth(1) else { continue };
        if gitops::verify_commit(root, candidate).is_some() {
            return resolve_commit(root, candidate).map(Some);
        }
    }
    Ok(None)
}
