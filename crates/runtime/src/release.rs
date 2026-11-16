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

/// Orders two versions the way a release is versioned, not the way a name is spelled.
///
/// A version read as text is wrong the moment a segment's digit count differs: `1.9.0` sorts after
/// `1.10.0`, and `2026.11.9` after `2026.11.16`. Either one makes `update` pick a build that is
/// older than the one it meant to take — a downgrade no crash loop catches, because the build it
/// lands on works. So the numbers are compared as numbers, segment by segment, and a segment is
/// only compared as text when a number cannot be read from it, which a release tag never is (the
/// workflow refuses anything but `N.N.N`) but a hand-made build directory can be.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut left, mut right) = (segments(a), segments(b));
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            // A version with segments left is the newer one: `1.0.0.1` came after `1.0.0`.
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ordering = x.cmp(&y);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

/// The newest of `versions`, or `None` for an empty set: the build a window with no version named
/// should take.
pub fn newest<I, S>(versions: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    versions
        .into_iter()
        .max_by(|a, b| version_cmp(a.as_ref(), b.as_ref()))
        .map(|version| version.as_ref().to_owned())
}

/// One version as the things to compare: each dot-separated part is a number where it is all
/// digits and text where it is not, and a part that is text sorts below any number.
fn segments(version: &str) -> impl Iterator<Item = Segment> {
    version.split('.').map(|part| match part.parse::<u64>() {
        Ok(number) => Segment::Number(number),
        Err(_) => Segment::Text(part.to_owned()),
    })
}

/// One dot-separated part of a version: a number where it reads as one, its own text where not.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    /// An all-digits part, compared as the number it is.
    Number(u64),
    /// A part that is not all digits, compared as text.
    Text(String),
}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Segment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            // A numbered part is a release's own shape, so it outranks a part that is not one:
            // `1.0.0` is a version and `nightly` is not, and the release is what a window wants.
            (Self::Number(_), Self::Text(_)) => Ordering::Greater,
            (Self::Text(_), Self::Number(_)) => Ordering::Less,
        }
    }
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
