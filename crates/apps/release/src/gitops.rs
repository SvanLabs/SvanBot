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

/// The build inputs that differ from the commit, as `git status --porcelain` lists them (empty when
/// clean). The release's own half-moved dashboard scratch is not a build input: an interrupted swap
/// must not read as a dirty checkout (the sweep and the journal repair clear it).
pub fn dirty_inputs(root: &Path) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all", "--"])
        .args(["crates", "Cargo.toml", "Cargo.lock", "build.rs", ".cargo", "rust-toolchain", "rust-toolchain.toml", "web"])
        .args([":(exclude)web/.dist.before-swap.*", ":(exclude)web/.dist.failed-swap.*"])
        .output()
        .map_err(|e| ReleaseError::Io(format!("cannot run git: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
}
