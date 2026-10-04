//! One-time adoption of a legacy installed build (`adopt_legacy_identity` in `rollback.sh`): a build
//! installed before commits were stamped gets its identity marker once the running fleet proves it.

use crate::binaries::{Identity, REQUIRED, require_binaries};
use crate::error::{ReleaseError, Result};
use crate::identity::{MARKER, installed_commit, resolve_commit};
use crate::layout::Root;
use crate::{fleet, health, lock};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn refused(message: impl Into<String>) -> ReleaseError {
    ReleaseError::Refused(message.into())
}

pub fn adopt_legacy(root: &Root, requested: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let expected = resolve_commit(root.path(), requested)?;
    root.validate_layout()?;
    let _operation = lock::acquire(root, env.iter().find(|(k, _)| k == "SV10_RELEASE_LOCK_FD").map(|(_, v)| v.as_str()))?;
    let release = root.join("target/release");
    let marker = release.join(MARKER);
    if marker.exists() {
        return Err(refused("installed commit marker already exists"));
    }
    if installed_commit(root.path()).ok().flatten().as_deref() != Some(expected.as_str()) {
        return Err(refused(format!("release log does not identify legacy commit {expected}")));
    }
    for name in REQUIRED {
        let path = release.join(name);
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0) {
            return Err(refused(format!("legacy required binary missing or non-executable: {name}")));
        }
        let output = Command::new(&path).arg("--version").output().ok().filter(|o| o.status.success());
        let output = output.ok_or_else(|| refused(format!("{name} --version failed")))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut words = text.lines().next().unwrap_or("").split_whitespace();
        if (words.next(), words.next().is_some(), words.next()) != (Some(name), true, None) {
            return Err(refused(format!("{name} is not an unmarked legacy binary")));
        }
    }
    if !fleet::installed_bot_running(root.path()) {
        return Err(refused("no running process matches the installed legacy sv10-bot inode"));
    }
    let reported =
        health::commit_at(&health::url(root.path(), env)).map_err(|_| refused("could not read legacy build identity from local health"))?;
    let reported = resolve_commit(root.path(), &reported)?;
    if reported != expected {
        return Err(refused(format!("running health identifies {reported}, expected {expected}")));
    }
    let tmp = release.join(format!("{MARKER}.new.{}", std::process::id()));
    std::fs::write(&tmp, format!("{expected}\n")).map_err(|e| ReleaseError::Io(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &marker).map_err(|e| ReleaseError::Io(format!("cannot move {} into place: {e}", tmp.display())))?;
    require_binaries(root.path(), &release, Some(&expected), Identity::AllowMarker)?;
    out.push_str(&format!("Adopted verified legacy installed identity {expected}\n"));
    Ok(())
}
