//! The few `git` questions the installer asks, through the `git` binary like the scripts do.

use crate::error::{ReleaseError, Result};
use std::path::Path;
use std::process::Command;

/// Run `git -C root args…`; the trimmed stdout on success, `None` on any failure.
pub fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether `root` is inside a git repository.
pub fn is_repository(root: &Path) -> bool {
    git(root, &["rev-parse", "--git-dir"]).is_some()
}

/// The full hash `rev` names as a commit, if it resolves.
pub fn verify_commit(root: &Path, rev: &str) -> Option<String> {
    git(root, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
}

/// The seven-character form of a full hash.
pub fn short7(root: &Path, full: &str) -> Result<String> {
    git(root, &["rev-parse", "--short=7", full]).ok_or_else(|| ReleaseError::CommitUnresolved(full.to_string()))
}

/// A file's contents at a commit (`git show <commit>:<path>`).
pub fn show(root: &Path, commit: &str, path: &str) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).arg("show").arg(format!("{commit}:{path}")).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
