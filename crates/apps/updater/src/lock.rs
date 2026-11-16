//! One update at a time, across the whole box.
//!
//! Two `svanbot-release update` runs at once would both ask a bot to stop, both swap `current`, and
//! one would roll the other's swap back while it was still watching. The lock is a file in the
//! install tree, held for the length of the run, so a second run says so and stops rather than
//! fighting the first (`File::try_lock`, the same shape `svanbot download` uses for its store).
//!
//! It is not a lock the updater waits on: a window is short and a person is watching, so an update
//! already in flight is a thing to report, not a thing to queue behind.

use std::fs::File;
use std::path::Path;

/// The lock file, released when it drops at the end of the run.
pub(crate) struct Lock {
    _file: File,
}

impl Lock {
    /// Takes the tree's lock, or reports that an update is already running.
    pub(crate) fn take(tree: &Path) -> Result<Self, String> {
        let path = tree.join("update.lock");
        let file =
            File::create(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(
                "an update is already in flight: wait for it to finish before starting another"
                    .into(),
            ),
            Err(std::fs::TryLockError::Error(error)) => {
                Err(format!("cannot lock {}: {error}", path.display()))
            }
        }
    }
}
