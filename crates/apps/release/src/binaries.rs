//! The executables a release must carry and what they must say about themselves
//! (`require_binaries` in `rollback.sh`).

use crate::error::{ReleaseError, Result};
use crate::identity::{MARKER, resolve_commit};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// The programs every installed set must contain.
pub const REQUIRED: [&str; 3] = ["sv10-bot", "learner", "analyst"];

/// How a build without its own commit in `--version` is judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// The matching `.sv10-installed-commit` next to the files may stand in (snapshots, rollbacks).
    AllowMarker,
    /// The binary must name its own commit (a fresh install).
    Strict,
}

/// Check each required binary in `base`: present, executable, `--version` is
/// `<program> <version> [<build commit>]`, and (when `expected` is given) names that commit.
pub fn require_binaries(root: &Path, base: &Path, expected: Option<&str>, mode: Identity) -> Result<()> {
    let expected = expected.map(|e| resolve_commit(root, e)).transpose()?;
    let marker: Option<String> =
        std::fs::read_to_string(base.join(MARKER)).ok().map(|t| t.chars().filter(|c| !c.is_whitespace()).collect());
    for name in REQUIRED {
        let path = base.join(name);
        let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file()).ok_or_else(|| ReleaseError::BinaryMissing(name.into()))?;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(ReleaseError::BinaryNotExecutable(name.into()));
        }
        let out = Command::new(&path).arg("--version").output().ok().filter(|o| o.status.success());
        let out = out.ok_or_else(|| ReleaseError::BinaryVersionFailed(name.into()))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut words = text.lines().next().unwrap_or("").split_whitespace();
        let (program, version, build, extra) = (words.next(), words.next(), words.next(), words.next());
        if program != Some(name) || version.is_none() || extra.is_some() {
            return Err(ReleaseError::BinaryMalformed(name.into()));
        }
        let Some(want) = expected.as_deref() else { continue };
        match build {
            Some(got) if got != want => {
                return Err(ReleaseError::BinaryBuildMismatch { name: name.into(), got: got.into(), want: want.into() });
            }
            Some(_) => {}
            None if mode == Identity::Strict => return Err(ReleaseError::BinaryNoIdentity(name.into())),
            None if marker.as_deref() != Some(want) => return Err(ReleaseError::BinaryNoMarker(name.into())),
            None => {}
        }
    }
    Ok(())
}
