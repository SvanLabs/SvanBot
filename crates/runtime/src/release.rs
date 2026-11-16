//! A release, as the updater reads it before it installs anything.
//!
//! A staged release is a directory `svanbot-release install` has unpacked: the program `svanbot`,
//! a `version` manifest naming the version it is and the checksum its bytes must have, and the
//! `sha256` file the release publishes beside the program. Nothing is installed and no slot is
//! pointed anywhere until [`Release::verify`] has passed: the program's bytes must hash to the
//! checksum, and the program must be executable, or the next restart would fail to exec it.
//!
//! The version also becomes the name of the installed directory, so it is checked here to be a
//! plain name and not a path: a version like `../current` must never reach the install tree.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The program's file name, in a staged release and in an installed build.
pub const PROGRAM: &str = "svanbot";

/// Why a staged release is not one the updater will install.
#[derive(Debug)]
pub enum ReleaseError {
    /// The release's files could not be read.
    Io(io::Error),
    /// The `version` manifest is missing or does not name a version and a checksum.
    Manifest(String),
    /// The version cannot be a directory name in the install tree.
    BadVersion {
        version: String,
        reason: &'static str,
    },
    /// There is no program in the release.
    NoProgram(PathBuf),
    /// The program is not executable, so the unit could not run it.
    NotExecutable(PathBuf),
    /// The program's bytes do not hash to the checksum the release published.
    Checksum { expected: String, actual: String },
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "release: {error}"),
            Self::Manifest(why) => write!(f, "release: the version manifest {why}"),
            Self::BadVersion { version, reason } => {
                write!(f, "release: version {version:?} {reason}")
            }
            Self::NoProgram(path) => write!(f, "release: no program at {}", path.display()),
            Self::NotExecutable(path) => {
                write!(f, "release: {} is not executable", path.display())
            }
            Self::Checksum { expected, actual } => {
                write!(
                    f,
                    "release: checksum mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ReleaseError {}

impl From<io::Error> for ReleaseError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A staged release, read but not yet verified.
#[derive(Clone, Debug)]
pub struct Release {
    dir: PathBuf,
    version: String,
    checksum: String,
}

impl Release {
    /// Reads the staged release under `dir`, checking its shape but not its bytes.
    ///
    /// The bytes are checked by [`verify`](Self::verify), which is the caller's separate, explicit
    /// step: reading a release and trusting it are not the same act.
    pub fn at(dir: impl Into<PathBuf>) -> Result<Self, ReleaseError> {
        let dir = dir.into();
        let manifest = match fs::read_to_string(dir.join("version")) {
            Ok(manifest) => manifest,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ReleaseError::Manifest("is missing".into()));
            }
            Err(error) => return Err(error.into()),
        };
        let version = manifest_line(&manifest, "version")
            .ok_or_else(|| ReleaseError::Manifest("names no version".into()))?;
        check_version(&version)?;
        let program = dir.join(PROGRAM);
        if !program.is_file() {
            return Err(ReleaseError::NoProgram(program));
        }
        // The checksum comes from the manifest, which is inside the release; the release's own
        // `sha256` file is what the download published, and the two must agree.
        let checksum = manifest_line(&manifest, "sha256")
            .ok_or_else(|| ReleaseError::Manifest("names no checksum".into()))?;
        let published = published_checksum(&dir.join("sha256"))
            .ok_or_else(|| ReleaseError::Manifest("has no `sha256` beside the program".into()))?;
        if !published.eq_ignore_ascii_case(&checksum) {
            return Err(ReleaseError::Checksum {
                expected: published,
                actual: checksum,
            });
        }
        Ok(Self {
            dir,
            version,
            checksum,
        })
    }

    /// The version this release is.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The directory the release was unpacked into.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The program's path.
    pub fn program(&self) -> PathBuf {
        self.dir.join(PROGRAM)
    }

    /// Checks the program's bytes against the checksum and its mode against the exec the unit
    /// will do. A release that fails either is not installed.
    pub fn verify(&self) -> Result<(), ReleaseError> {
        let program = self.program();
        let bytes = fs::read(&program)?;
        let actual = digest(&bytes);
        if !actual.eq_ignore_ascii_case(&self.checksum) {
            return Err(ReleaseError::Checksum {
                expected: self.checksum.clone(),
                actual,
            });
        }
        let mode = fs::metadata(&program)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        if mode.mode() & 0o111 == 0 {
            return Err(ReleaseError::NotExecutable(program));
        }
        Ok(())
    }
}

/// The SHA-256 of `bytes`, lowercase hex, the form the release publishes.
pub fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The value of a `key\tvalue` line in the version manifest.
fn manifest_line(manifest: &str, key: &str) -> Option<String> {
    manifest
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The first field of a `sha256sum`-shaped file, which names the program it is for.
fn published_checksum(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let first = text.split_whitespace().next()?;
    (!first.is_empty()).then(|| first.to_owned())
}

/// Refuses a version that cannot be a directory name in the install tree: one that is empty, that
/// is dot or dot-dot, that contains a separator, or that would collide with a slot's own name.
fn check_version(version: &str) -> Result<(), ReleaseError> {
    let reason = if version.is_empty() {
        Some("is empty")
    } else if version == "." || version == ".." {
        Some("is not a directory name")
    } else if version.contains('/') || version.contains('\\') {
        Some("contains a path separator")
    } else if version.starts_with('.') {
        Some("starts with a dot")
    } else if version == "current" || version == "previous" {
        Some("is a slot's own name")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(ReleaseError::BadVersion {
            version: version.to_owned(),
            reason,
        }),
        None => Ok(()),
    }
}
