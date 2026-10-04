//! Restoring a verified snapshot (`restore_snapshot` in `rollback.sh`): copy it to a staging tree
//! beside the install, verify the copy against the manifest, swap it in, and log the rollback.

use crate::binaries::{Identity, require_binaries};
use crate::error::{ReleaseError, Result};
use crate::fsops::{Scratch, copy_tree, make_temp_dir};
use crate::layout::Root;
use crate::swap::{self, Inject};
use crate::{identity, lock, manifest, snapshot, store_format};
use std::io::Write;
use std::process::Command;

/// A build that cannot read the stored data never gets installed: it would fail on every packed row.
pub fn require_readable_store(root: &Root, commit: &str) -> Result<()> {
    let (build, store) = (store_format::of_build(root.path(), commit), store_format::of_store(root.path()));
    if build >= store { Ok(()) } else { Err(ReleaseError::StoreTooNew { commit: commit.into(), build, store }) }
}

/// `date +%FT%T%:z`, the local time with its offset, as `releases.log` has always been written.
fn log_time() -> Result<String> {
    let out = Command::new("date").arg("+%FT%T%:z").output().map_err(|e| ReleaseError::Io(format!("cannot run date: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Restore the snapshot of `requested`; the success line is appended to `out`.
pub fn restore(root: &Root, requested: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let commit = identity::resolve_commit(root.path(), requested)?;
    root.validate_layout()?;
    let inherited = env.iter().find(|(k, _)| k == "SV10_RELEASE_LOCK_FD").map(|(_, v)| v.as_str());
    let _operation = lock::acquire(root, inherited)?;
    snapshot::verify(root, &commit)?;
    out.push_str(&format!("Verified release snapshot {commit}\n"));
    require_readable_store(root, &commit)?;
    let previous = identity::installed_commit(root.path())?.ok_or(ReleaseError::NoInstalledToRestore)?;
    let source = snapshot::dir(root, &commit);
    let stage = Scratch(make_temp_dir(&root.join("target"), &format!(".rollback-{commit}."))?);
    let (release, web) = (stage.0.join("target/release"), stage.0.join("web/dist"));
    for dir in [&release, &web] {
        std::fs::create_dir_all(dir).map_err(|e| ReleaseError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    copy_tree(&source.join("target/release"), &release)?;
    copy_tree(&source.join("web/dist"), &web)?;
    let sums = source.join("SHA256SUMS");
    let text = std::fs::read_to_string(&sums).map_err(|e| ReleaseError::Io(format!("cannot read {}: {e}", sums.display())))?;
    manifest::verify_tree(&stage.0, &text)?;
    require_binaries(root.path(), &release, Some(&commit), Identity::AllowMarker)?;
    swap::install(root, &release, &web, Inject::from_env(root, env)?)?;
    drop(stage);
    let log = root.join("artifacts/releases.log");
    let line = format!("{} {commit} rollback from {previous}\n", log_time()?);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| ReleaseError::Io(format!("cannot append to {}: {e}", log.display())))?;
    out.push_str(&format!("Restored {commit} from verified snapshot (previous {previous}). Watch the hot-swap log.\n"));
    Ok(())
}
