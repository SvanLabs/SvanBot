//! Putting a verified staged release where `current` can point at it: `lib/<version>/`.
//!
//! A staged release arrives in a scratch directory — CI unpacks it under `/tmp` or an operator
//! `scp`s it in — and a slot must point at something that outlives that. Promotion copies the
//! verified program into a directory named for its version under the slot root, and hashes the
//! copy: what `current` ends up pointing at is the bytes that were verified, not the bytes that
//! were verified in a directory since removed.
//!
//! It is idempotent: promoting a version already in place re-verifies it and leaves it alone, so a
//! re-run of an update that failed after promotion does not have to start over.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use svanbot_runtime::release::{PROGRAM, Release, ReleaseError, digest};

/// Why a verified release could not be put in place.
#[derive(Debug)]
pub(crate) enum PromotError {
    /// The release's own reading failed.
    Release(ReleaseError),
    /// The copy could not be made, or a file in it could not be read.
    Io(PathBuf, io::Error),
    /// The copy's bytes are not the staged release's bytes: the disk changed them, so they are not
    /// installed.
    Corrupt {
        version: String,
        expected: String,
        actual: String,
    },
}

impl std::fmt::Display for PromotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Release(error) => write!(f, "{error}"),
            Self::Io(path, error) => write!(f, "{}: {error}", path.display()),
            Self::Corrupt {
                version,
                expected,
                actual,
            } => write!(
                f,
                "the copy of {version} hashes to {actual}, not {expected}: not installed"
            ),
        }
    }
}

impl std::error::Error for PromotError {}

impl From<ReleaseError> for PromotError {
    fn from(error: ReleaseError) -> Self {
        Self::Release(error)
    }
}

/// Copies the verified `staged` release into `lib/<version>`, checks the copy's bytes, and returns
/// the directory `current` may point at.
pub(crate) fn promote(staged: &Release, lib: &Path) -> Result<PathBuf, PromotError> {
    let version = staged.version().to_owned();
    let target = lib.join(&version);
    fs::create_dir_all(lib).map_err(|e| PromotError::Io(lib.to_path_buf(), e))?;
    // A version already installed is the same version: verify it and leave it, so a retry after a
    // failed window does not copy over the build the fleet is running.
    if target.is_dir() {
        let there = Release::at(&target)?;
        there.verify()?;
        if there.version() == version {
            return Ok(target);
        }
    }
    // A version-named directory is this version's: build it fresh rather than merge into whatever
    // is already there.
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| PromotError::Io(target.clone(), e))?;
    }
    fs::create_dir_all(&target).map_err(|e| PromotError::Io(target.clone(), e))?;
    let from = staged.dir().join(PROGRAM);
    let to = target.join(PROGRAM);
    fs::copy(&from, &to).map_err(|e| PromotError::Io(to.clone(), e))?;
    // The copy is a fresh program: the staged mode is not carried by `copy`, and a program systemd
    // cannot `exec` is not an installed build.
    let mut mode = fs::metadata(&to)
        .map_err(|e| PromotError::Io(to.clone(), e))?
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(&to, mode).map_err(|e| PromotError::Io(to.clone(), e))?;
    // The manifest travels with the program: `Release::at` reads it back, and it is what names the
    // version this directory is.
    for name in ["version", "sha256"] {
        let from = staged.dir().join(name);
        if from.is_file() {
            let to = target.join(name);
            fs::copy(&from, &to).map_err(|e| PromotError::Io(to, e))?;
        }
    }
    let expected = digest(&fs::read(&from).map_err(|e| PromotError::Io(from.clone(), e))?);
    let actual = digest(&fs::read(&to).map_err(|e| PromotError::Io(to.clone(), e))?);
    if actual != expected {
        let _ = fs::remove_dir_all(&target);
        return Err(PromotError::Corrupt {
            version,
            expected,
            actual,
        });
    }
    Ok(target)
}
