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
