//! The two slots a build lives in: `current` and `previous`.
//!
//! A slot is an **absolute symlink** into the directory of one installed build, and `current` is
//! the slot the fleet runs: `deploy/svanbot@.service` execs `~/.local/lib/svanbot/current/svanbot`.
//! An update installs the new build into its own version-named directory and then points `current`
//! at it, keeping the build it replaced in `previous`; a rollback points `current` back at
//! `previous` and keeps the failed build in `previous`, so a second rollback goes forward again
//! (`GLOSSARY.md`: update, slot, rollback).
//!
//! Each swap is a symlink written under a temporary name and then renamed over the slot's name,
//! which replaces it in one step. A bot that resolves the path while an update swaps it sees one
//! build or the other, never a half-written link, and a bot already running keeps its own files:
//! nothing deletes an installed build here.
//!
//! Absolute is not a preference: the supervisor's exec resolves the link against whatever working
//! directory the supervisor gives the bot, so a relative link would point somewhere else entirely.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A slot's name in the tree, and the symlink that stands for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// The build the fleet runs.
    Current,
    /// The build `current` replaced: the one a rollback returns to.
    Previous,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Previous => "previous",
        }
    }
}

/// Why a slot could not be read or swapped.
#[derive(Debug)]
pub enum SlotError {
    /// The slot tree could not be read or written.
    Io(io::Error),
    /// The build a slot was asked to point at is not one that is installed.
    NotInstalled(PathBuf),
    /// There is no build in `previous` to roll back to.
    NoPrevious,
}

impl fmt::Display for SlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "slot: {error}"),
            Self::NotInstalled(path) => {
                write!(f, "slot: {} is not an installed build", path.display())
            }
            Self::NoPrevious => write!(f, "slot: there is no previous build to roll back to"),
        }
    }
}

impl std::error::Error for SlotError {}

impl From<io::Error> for SlotError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The tree that holds the installed builds and the two slot symlinks. The real one is
/// `~/.local/lib/svanbot`; everything here works on whatever directory it is given, so a test can
/// use a temporary one.
#[derive(Clone, Debug)]
pub struct Slots {
    root: PathBuf,
}

impl Slots {
    /// The slots under `root`, whether or not `root` exists yet.
    pub fn in_dir(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The tree's directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory of the build in `current`, or `None` when the slot is empty.
    ///
    /// A symlink whose build is gone reads as empty: a slot that points at a path that is not a
    /// directory is not a build a bot can run.
    pub fn current(&self) -> Option<PathBuf> {
        self.build(Slot::Current)
    }

    /// The directory of the build in `previous`, the one a rollback returns to.
    pub fn previous(&self) -> Option<PathBuf> {
        self.build(Slot::Previous)
    }

    /// The program `current` holds: what the supervisor execs.
    pub fn binary(&self) -> Option<PathBuf> {
        self.current().map(|dir| dir.join("svanbot"))
    }

    /// Makes an installed build the one that runs, and keeps the build it replaced in `previous`.
    ///
    /// `build` must be a directory that exists: pointing a slot at anything else would leave the
    /// supervisor unable to exec, so it is refused rather than written.
    pub fn install(&self, build: impl AsRef<Path>) -> Result<(), SlotError> {
        let build = build.as_ref();
        if !build.is_dir() {
            return Err(SlotError::NotInstalled(build.to_path_buf()));
        }
        let build = fs::canonicalize(build)?;
        fs::create_dir_all(&self.root)?;
        let replaced = self.current().filter(|running| running != &build);
        self.write_link(Slot::Current, &build)?;
        // After the swap, so there is no moment with nothing in `current`, and after the rollback
        // target is read, so an install that fails to write `previous` still leaves the new build
        // running with the old one installed beside it.
        if let Some(replaced) = replaced {
            self.write_link(Slot::Previous, &replaced)?;
        }
        Ok(())
    }

    /// Points `current` back at `previous`, and keeps the build it replaced in `previous`, so a
    /// rollback that turns out to be wrong rolls forward the same way.
    pub fn roll_back(&self) -> Result<(), SlotError> {
        let previous = self.previous().ok_or(SlotError::NoPrevious)?;
        let failed = self.current();
        self.write_link(Slot::Current, &previous)?;
        if let Some(failed) = failed {
            self.write_link(Slot::Previous, &failed)?;
        }
        Ok(())
    }

    /// The build in `slot`, resolved to an absolute path.
    fn build(&self, slot: Slot) -> Option<PathBuf> {
        let target = fs::read_link(self.link(slot)).ok()?;
        let target = if target.is_absolute() {
            target
        } else {
            // A link written by hand or by an older tool: resolved against the tree, but never
            // handed out in its relative form, which would mean something else to the reader.
            self.root.join(target)
        };
        target.is_dir().then_some(target)
    }

    /// Writes `slot` as an absolute symlink to `build`, atomically: a temporary link in the same
    /// directory, then a rename over the slot's name, which replaces it in a single step.
    fn write_link(&self, slot: Slot, build: &Path) -> Result<(), SlotError> {
        let temporary = self.root.join(format!(".{}.new", slot.name()));
        let _ = fs::remove_file(&temporary);
        std::os::unix::fs::symlink(build, &temporary)?;
        match fs::rename(&temporary, self.link(slot)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(SlotError::Io(error))
            }
        }
    }

    /// Where `slot`'s symlink lives.
    fn link(&self, slot: Slot) -> PathBuf {
        self.root.join(slot.name())
    }
}
