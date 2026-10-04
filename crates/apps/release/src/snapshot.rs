//! Release snapshots: a hash-verified copy of an installed set, kept under
//! `artifacts/release-snapshots/<commit>/` so a rollback never trusts anything unverified.

use crate::binaries::{Identity, require_binaries};
use crate::error::{ReleaseError, Result};
use crate::layout::Root;
use crate::manifest;
use std::path::PathBuf;

/// `artifacts/release-snapshots/<commit>` under the root.
pub fn dir(root: &Root, commit: &str) -> PathBuf {
    root.join("artifacts/release-snapshots").join(commit)
}

fn is_plain(path: &std::path::Path, want_dir: bool) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_symlink() && if want_dir { m.is_dir() } else { m.is_file() })
}

/// `verify_snapshot`: the snapshot directory and manifest are plain (not symlinks), the trees hold no
/// symlink or special file, every manifest line is safe, the manifest and the files agree exactly,
/// and the binaries identify as `commit`. Prints nothing; the caller reports success.
pub fn verify(root: &Root, commit: &str) -> Result<()> {
    let snapshot = dir(root, commit);
    let manifest_path = snapshot.join("SHA256SUMS");
    if !is_plain(&snapshot, true) {
        return Err(ReleaseError::SnapshotUnsafe(commit.to_string()));
    }
    if !is_plain(&manifest_path, false) {
        return Err(ReleaseError::SnapshotManifestMissing(commit.to_string()));
    }
    manifest::reject_special(&snapshot.join("target/release"))?;
    manifest::reject_special(&snapshot.join("web/dist"))?;
    let text =
        std::fs::read_to_string(&manifest_path).map_err(|e| ReleaseError::Io(format!("cannot read {}: {e}", manifest_path.display())))?;
    manifest::parse_safe(&text)?;
    manifest::verify_tree(&snapshot, &text)?;
    require_binaries(root.path(), &snapshot.join("target/release"), Some(commit), Identity::AllowMarker)
}
