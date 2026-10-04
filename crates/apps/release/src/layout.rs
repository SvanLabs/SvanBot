//! The release root and the paths the installer manages (`rollback.sh` `validate_root`,
//! `assert_managed_path`, `validate_layout`): no symlink anywhere on a managed path, nothing that
//! escapes the repository, and a root that is neither `/` nor the home directory.

use crate::error::{ReleaseError, Result};
use crate::gitops;
use std::path::{Path, PathBuf};

/// Root-relative paths the installer writes or locks, in the order the shell checks them.
pub const MANAGED: [&str; 10] = [
    "target",
    "target/release",
    "web",
    "web/dist",
    "artifacts",
    "artifacts/release-snapshots",
    "artifacts/release-operation.lock",
    "artifacts/release.lock",
    "artifacts/release.log",
    "artifacts/release-swap.journal",
];

/// A validated release root: an absolute, symlink-free path to a git checkout.
#[derive(Clone, Debug)]
pub struct Root {
    path: PathBuf,
}

impl Root {
    /// Validate `raw` as the release root. `home` is `$HOME` (the root may not be it).
    pub fn open(raw: &Path, home: Option<&Path>) -> Result<Root> {
        if raw.as_os_str().is_empty() {
            return Err(ReleaseError::RootEmpty);
        }
        if !raw.is_dir() {
            return Err(ReleaseError::RootMissing(raw.display().to_string()));
        }
        let path = raw.canonicalize().map_err(|e| ReleaseError::Io(format!("cannot resolve {}: {e}", raw.display())))?;
        let home = home.ok_or_else(|| ReleaseError::Io("HOME is not set".into()))?;
        let home = home.canonicalize().map_err(|e| ReleaseError::Io(format!("cannot resolve {}: {e}", home.display())))?;
        if path == Path::new("/") || path == home {
            return Err(ReleaseError::RootUnsafe(path.display().to_string()));
        }
        if !gitops::is_repository(&path) {
            return Err(ReleaseError::RootNotGit);
        }
        Ok(Root { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `root/rel`.
    pub fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }

    /// Refuse a managed path that is empty, absolute, contains `..`, passes through a symlink or
    /// resolves outside the root. A part that does not exist yet is fine.
    pub fn assert_managed_path(&self, rel: &str) -> Result<()> {
        if rel.is_empty() || rel.starts_with('/') || rel.contains("..") {
            return Err(ReleaseError::PathUnsafe(rel.to_string()));
        }
        let mut current = self.path.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            current.push(part);
            if std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(ReleaseError::PathSymlink(rel.to_string()));
            }
            if current.exists() {
                let resolved =
                    current.canonicalize().map_err(|e| ReleaseError::Io(format!("cannot resolve {}: {e}", current.display())))?;
                if !resolved.starts_with(&self.path) {
                    return Err(ReleaseError::PathEscapes(rel.to_string()));
                }
            }
        }
        Ok(())
    }

    /// `--validate-layout`: every managed path passes [`Root::assert_managed_path`].
    pub fn validate_layout(&self) -> Result<()> {
        MANAGED.iter().try_for_each(|rel| self.assert_managed_path(rel))
    }
}
