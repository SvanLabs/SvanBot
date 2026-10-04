//! Every refusal of the installer, one variant per class, with the message the shell scripts print
//! (operators and `docs/OPERATIONS.md` quote them, so the strings are part of the contract).

use std::fmt;

/// A refusal. `Display` is the message without the surface prefix (`rollback: `, `update: `).
#[derive(Debug, PartialEq, Eq)]
pub enum ReleaseError {
    /// The command line is wrong; carries the whole usage line.
    Usage(String),
    RootEmpty,
    RootMissing(String),
    RootUnsafe(String),
    RootNotGit,
    PathUnsafe(String),
    PathSymlink(String),
    PathEscapes(String),
    CommitEmpty,
    CommitUnresolved(String),
    NoInstalledCommit,
    SnapshotUnsafe(String),
    SnapshotManifestMissing(String),
    SpecialFile(String),
    ManifestHashEntry,
    ManifestPathUnsafe(String),
    ManifestIncomplete,
    ManifestMismatch,
    BinaryMissing(String),
    BinaryNotExecutable(String),
    BinaryVersionFailed(String),
    BinaryMalformed(String),
    BinaryBuildMismatch {
        name: String,
        got: String,
        want: String,
    },
    BinaryNoIdentity(String),
    BinaryNoMarker(String),
    SpaceSetting,
    SpaceUnreadable(String),
    SpaceLow {
        avail_mb: u64,
        root: String,
        required: u64,
    },
    LockBusy,
    ManifestWhitespace(String),
    ManifestEmpty,
    /// A refusal that is only ever raised in one place; carries the whole message.
    Refused(String),
    LockForeign,
    LockNotHeld,
    /// The swap journal cannot be trusted; carries the whole message.
    Journal(String),
    /// A directory swap could not complete; carries the whole message.
    Swap(String),
    SourceDirty,
    NoInstalledToRestore,
    StoreTooNew {
        commit: String,
        build: u64,
        store: u64,
    },
    /// A system call or file operation failed; carries what was being done.
    Io(String),
    /// A surface or flag that this binary has not taken over from the shell script yet.
    NotBuilt(String),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ReleaseError::*;
        match self {
            Usage(line) => write!(f, "{line}"),
            RootEmpty => write!(f, "release root is empty"),
            RootMissing(r) => write!(f, "release root does not exist: {r}"),
            RootUnsafe(r) => write!(f, "refusing unsafe release root: {r}"),
            RootNotGit => write!(f, "release root is not a git repository"),
            PathUnsafe(p) => write!(f, "unsafe managed path: {p}"),
            PathSymlink(p) => write!(f, "managed path contains a symlink: {p}"),
            PathEscapes(p) => write!(f, "managed path escapes repository: {p}"),
            CommitEmpty => write!(f, "commit is empty"),
            CommitUnresolved(c) => write!(f, "commit does not resolve in this repository: {c}"),
            NoInstalledCommit => write!(f, "no valid installed commit metadata"),
            SnapshotUnsafe(c) => write!(f, "snapshot does not exist safely: {c}"),
            SnapshotManifestMissing(c) => write!(f, "snapshot manifest missing: {c}"),
            SpecialFile(p) => write!(f, "symlink or special file is not allowed in a release tree: {p}"),
            ManifestHashEntry => write!(f, "invalid snapshot hash entry"),
            ManifestPathUnsafe(p) => write!(f, "unsafe snapshot manifest path: {p}"),
            ManifestIncomplete => write!(f, "release manifest is incomplete"),
            ManifestMismatch => write!(f, "release hash verification failed"),
            BinaryMissing(n) => write!(f, "required binary missing: {n}"),
            BinaryNotExecutable(n) => write!(f, "required binary is not executable: {n}"),
            BinaryVersionFailed(n) => write!(f, "{n} --version failed"),
            BinaryMalformed(n) => write!(f, "{n} --version is malformed"),
            BinaryBuildMismatch { name, got, want } => write!(f, "{name} identifies build {got}, expected {want}"),
            BinaryNoIdentity(n) => write!(f, "{n} does not identify its build commit"),
            BinaryNoMarker(n) => write!(f, "{n} has no build identity and no matching installed marker"),
            SpaceSetting => write!(f, "SV10_MIN_FREE_MB must be a whole number of megabytes"),
            SpaceUnreadable(r) => write!(f, "could not read free space for {r}"),
            SpaceLow { avail_mb, root, required } => {
                write!(f, "only {avail_mb} MB free on {root} and a release needs about {required} MB (SV10_MIN_FREE_MB)")
            }
            ManifestWhitespace(p) => write!(f, "release path contains whitespace: {p}"),
            ManifestEmpty => write!(f, "release manifest would be empty"),
            Refused(m) => write!(f, "{m}"),
            LockBusy => write!(f, "another release, snapshot, or rollback operation is active"),
            LockForeign => write!(f, "inherited release lock does not identify the managed lock file"),
            LockNotHeld => write!(f, "inherited release operation lock is not held"),
            Journal(m) | Swap(m) | Io(m) => write!(f, "{m}"),
            SourceDirty => write!(f, "uncommitted or untracked Rust/web build inputs"),
            NoInstalledToRestore => write!(f, "current installed commit cannot be resolved"),
            StoreTooNew { commit, build, store } => write!(
                f,
                "build {commit} reads data format {build} but the databases hold format {store} (compressed columns, 0229); stop the fleet, run ./target/release/archive unpack, then roll back"
            ),
            NotBuilt(what) => write!(f, "{what} is not built into sv10-release yet; use the shell script"),
        }
    }
}

impl std::error::Error for ReleaseError {}

pub type Result<T> = std::result::Result<T, ReleaseError>;
