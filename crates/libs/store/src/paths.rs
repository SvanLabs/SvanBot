//! Confining a name that arrives from outside the process to the directory it is meant to live in (#603).

use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// `name` resolved under `dir`, or an error naming it when it would leave `dir`.
///
/// The callers are the archive's restore and verification, whose file names come from a manifest
/// that is sealed by a sidecar hash in the same directory: the seal proves the file has not rotted,
/// not that its contents name anything sensible. A name holding `..`, an absolute path, or any
/// component that is not an ordinary one is refused rather than resolved and checked afterwards, so
/// there is no step at which a path outside `dir` exists.
pub fn confined(dir: &Path, name: &str) -> Result<PathBuf> {
    let rel = Path::new(name);
    if name.is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("{name}: the name leaves the archive directory");
    }
    Ok(dir.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_resolve_and_escaping_ones_are_refused() {
        let dir = Path::new("/tmp/archive");
        assert_eq!(confined(dir, "repo.bundle").unwrap(), dir.join("repo.bundle"));
        assert_eq!(confined(dir, "sub/thing.bin").unwrap(), dir.join("sub/thing.bin"));
        for name in ["../escaped.txt", "sub/../../escaped.txt", "/etc/passwd", "..", "", "./x"] {
            assert!(confined(dir, name).is_err(), "{name} was allowed");
        }
        // An interior `.` is normalized away by `Path` itself and names nothing outside `dir`.
        assert_eq!(confined(dir, "a/./b").unwrap(), dir.join("a/b"));
    }
}
