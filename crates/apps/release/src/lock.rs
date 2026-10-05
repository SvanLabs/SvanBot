//! The operation lock (`acquire_operation_lock` in `rollback.sh`): one release, snapshot or
//! rollback at a time, taken on `artifacts/release-operation.lock`. A parent that already holds it
//! (the `release.sh` that calls `rollback.sh`) hands the descriptor down in `SV10_RELEASE_LOCK_FD`,
//! and this process then checks that the descriptor really is that file and still locked.

use crate::error::{ReleaseError, Result};
use crate::{journal, layout::Root};
use std::fs::File;

/// A held operation lock. Dropping it releases a lock this process took itself; one inherited from
/// a parent stays with the parent.
pub struct Operation {
    _file: Option<File>,
}

impl Operation {
    /// The descriptor of a lock this process took itself, for in-process steps to treat as inherited.
    pub fn fd(&self) -> Option<i32> {
        use std::os::fd::AsRawFd;
        self._file.as_ref().map(|f| f.as_raw_fd())
    }
}

/// Whether another process holds the lock right now. It asks and gives the lock straight back,
/// and repairs nothing: a caller uses it to step aside before it has touched anything.
pub fn busy(root: &Root) -> bool {
    let lock = root.join("artifacts/release-operation.lock");
    File::open(&lock).is_ok_and(|file| matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
}

/// Take the lock, then repair whatever an interrupted swap left behind. `inherited` is the value
/// of `SV10_RELEASE_LOCK_FD`.
pub fn acquire(root: &Root, inherited: Option<&str>) -> Result<Operation> {
    let dir = root.join("artifacts");
    std::fs::create_dir_all(&dir).map_err(|e| ReleaseError::Io(format!("cannot create {}: {e}", dir.display())))?;
    let lock = dir.join("release-operation.lock");
    let operation = match inherited.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) {
        Some(fd) => {
            let fd: i32 = fd.parse().map_err(|_| ReleaseError::LockForeign)?;
            let target = std::fs::canonicalize(format!("/proc/self/fd/{fd}")).map_err(|_| ReleaseError::LockForeign)?;
            if target != lock {
                return Err(ReleaseError::LockForeign);
            }
            if !sv10_rt::flock_held(fd).unwrap_or(false) {
                return Err(ReleaseError::LockNotHeld);
            }
            Operation { _file: None }
        }
        None => {
            let file = File::create(&lock).map_err(|e| ReleaseError::Io(format!("cannot open {}: {e}", lock.display())))?;
            file.try_lock().map_err(|e| match e {
                std::fs::TryLockError::WouldBlock => ReleaseError::LockBusy,
                std::fs::TryLockError::Error(e) => ReleaseError::Io(format!("cannot lock {}: {e}", lock.display())),
            })?;
            Operation { _file: Some(file) }
        }
    };
    journal::repair(root)?;
    Ok(operation)
}
