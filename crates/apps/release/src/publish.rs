//! Making an installed set durable and putting a new one in: `create_snapshot`, `prune_snapshots`,
//! `install_release` and `preserve_unidentified` in `rollback.sh`.

use crate::binaries::{Identity, require_binaries};
use crate::error::{ReleaseError, Result};
use crate::fsops::{Scratch, copy_executables, copy_tree, make_temp_dir, remove_all, rename};
use crate::layout::Root;
use crate::swap::{self, Inject};
use crate::{identity, lock, manifest, snapshot};
use std::path::{Path, PathBuf};
use std::process::Command;

fn lock_of(env: &[(String, String)]) -> Option<&str> {
    env.iter().find(|(k, _)| k == "SV10_RELEASE_LOCK_FD").map(|(_, v)| v.as_str())
}

fn mkdirs(dirs: &[&Path]) -> Result<()> {
    for d in dirs {
        std::fs::create_dir_all(d).map_err(|e| ReleaseError::Io(format!("cannot create {}: {e}", d.display())))?;
    }
    Ok(())
}

fn write_marker(dir: &Path, commit: &str) -> Result<()> {
    let file = dir.join(identity::MARKER);
    std::fs::write(&file, format!("{commit}\n")).map_err(|e| ReleaseError::Io(format!("cannot write {}: {e}", file.display())))
}

/// `--snapshot <commit>`: a hash-verified copy of the installed set under
/// `artifacts/release-snapshots/<commit>`, then older ones pruned.
pub fn create_snapshot(root: &Root, requested: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let commit = identity::resolve_commit(root.path(), requested)?;
    root.validate_layout()?;
    let _operation = lock::acquire(root, lock_of(env))?;
    let (release, web) = (root.join("target/release"), root.join("web/dist"));
    require_binaries(root.path(), &release, Some(&commit), Identity::AllowMarker)?;
    if !web.is_dir() {
        return Err(ReleaseError::Refused("installed dashboard missing: web/dist".into()));
    }
    manifest::reject_special(&web)?;
    let parent = root.join("artifacts/release-snapshots");
    mkdirs(&[&parent])?;
    let target = parent.join(&commit);
    if target.exists() {
        snapshot::verify(root, &commit)?;
        out.push_str(&format!("Verified release snapshot {commit}\nRelease snapshot {commit} already exists; keeping it unchanged\n"));
        return Ok(());
    }
    let mut stage = Scratch(make_temp_dir(&parent, &format!(".{commit}.tmp."))?);
    let (sr, sw) = (stage.0.join("target/release"), stage.0.join("web/dist"));
    mkdirs(&[&sr, &sw])?;
    copy_executables(&release, &sr)?;
    write_marker(&sr, &commit)?;
    copy_tree(&web, &sw)?;
    manifest::reject_special(&sw)?;
    manifest::write(&stage.0)?;
    require_binaries(root.path(), &sr, Some(&commit), Identity::AllowMarker)?;
    rename(&stage.0, &target)?;
    stage.0 = PathBuf::new(); // moved: nothing left for the cleanup to remove
    snapshot::verify(root, &commit)?;
    out.push_str(&format!("Verified release snapshot {commit}\nCreated release snapshot {commit}\n"));
    prune(root, &commit, env, out)
}

/// Keep the newest `SV10_KEEP_SNAPSHOTS` snapshots (default 5); the one just made is never removed
/// and hidden staging directories are not snapshots.
fn prune(root: &Root, keep_commit: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let keep = env.iter().find(|(k, _)| k == "SV10_KEEP_SNAPSHOTS").map_or("5", |(_, v)| v.as_str());
    let keep: usize = keep
        .parse()
        .ok()
        .filter(|n| *n >= 2 && keep.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| ReleaseError::Refused("SV10_KEEP_SNAPSHOTS must be a whole number of at least 2".into()))?;
    let parent = root.join("artifacts/release-snapshots");
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&parent)
        .map_err(|e| ReleaseError::Io(format!("cannot read {}: {e}", parent.display())))?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    for (_, dir) in dirs.into_iter().skip(keep) {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name == keep_commit {
            continue;
        }
        remove_all(&dir)?;
        out.push_str(&format!("Pruned release snapshot {name}\n"));
    }
    Ok(())
}

/// `--install <binary-dir> <web-dir> <commit>`: stage the build, write its identity and manifest,
/// check it strictly, and swap it in.
pub fn install(root: &Root, binaries: &str, web: &str, requested: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let commit = identity::resolve_commit(root.path(), requested)?;
    root.validate_layout()?;
    let _operation = lock::acquire(root, lock_of(env))?;
    for source in [binaries, web] {
        let path = Path::new(source);
        if !path.is_dir() {
            return Err(ReleaseError::Refused(format!("install source is not a directory: {source}")));
        }
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(ReleaseError::Refused(format!("install source is a symlink: {source}")));
        }
        let resolved = std::fs::canonicalize(path).map_err(|e| ReleaseError::Io(format!("cannot resolve {source}: {e}")))?;
        if !resolved.starts_with(root.path()) || resolved == root.path() {
            return Err(ReleaseError::Refused(format!("install source escapes repository: {source}")));
        }
        manifest::reject_special(path)?;
    }
    let stage = Scratch(make_temp_dir(&root.join("target"), &format!(".install-{commit}."))?);
    let (sr, sw) = (stage.0.join("target/release"), stage.0.join("web/dist"));
    mkdirs(&[&sr, &sw])?;
    copy_executables(Path::new(binaries), &sr)?;
    write_marker(&sr, &commit)?;
    copy_tree(Path::new(web), &sw)?;
    manifest::reject_special(&sr)?;
    manifest::reject_special(&sw)?;
    manifest::write(&stage.0)?;
    require_binaries(root.path(), &sr, Some(&commit), Identity::Strict)?;
    swap::install(root, &sr, &sw, Inject::from_env(root, env)?)?;
    drop(stage);
    out.push_str(&format!("Installed verified release set {commit}\n"));
    Ok(())
}

fn utc_stamp() -> Result<String> {
    let out =
        Command::new("date").args(["-u", "+%Y%m%dT%H%M%SZ"]).output().map_err(|e| ReleaseError::Io(format!("cannot run date: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `--preserve-unidentified`: a one-time escape for an installed build that names no commit. Keep a
/// verified copy under `artifacts/unidentified-builds/`, but only while a verified rollback
/// snapshot exists. Read-only on the installed tree.
pub fn preserve_unidentified(root: &Root, env: &[(String, String)], out: &mut String) -> Result<()> {
    root.validate_layout()?;
    let _operation = lock::acquire(root, lock_of(env))?;
    let (release, web) = (root.join("target/release"), root.join("web/dist"));
    if release.join(identity::MARKER).exists() {
        return Err(ReleaseError::Refused("installed release is identified; snapshot it with --snapshot instead".into()));
    }
    if !web.is_dir() {
        return Err(ReleaseError::Refused("installed dashboard missing: web/dist".into()));
    }
    manifest::reject_special(&release)?;
    manifest::reject_special(&web)?;
    let snapshots = root.join("artifacts/release-snapshots");
    let mut names: Vec<String> = std::fs::read_dir(&snapshots)
        .map(|d| d.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    let verified = names.into_iter().find(|n| !n.starts_with('.') && snapshot::verify(root, n).is_ok()).ok_or_else(|| {
        ReleaseError::Refused("no verified rollback snapshot exists; refusing to release over an unidentified build".into())
    })?;
    let parent = root.join("artifacts/unidentified-builds");
    mkdirs(&[&parent])?;
    let stamp = utc_stamp()?;
    let dest = parent.join(&stamp);
    if dest.exists() {
        return Err(ReleaseError::Refused(format!("preserved build already exists: {}", dest.display())));
    }
    let stage = Scratch(make_temp_dir(&parent, &format!(".{stamp}.tmp."))?);
    let (sr, sw) = (stage.0.join("target/release"), stage.0.join("web/dist"));
    mkdirs(&[&sr, &sw])?;
    copy_executables(&release, &sr)?;
    copy_tree(&web, &sw)?;
    let mut versions = String::new();
    for name in crate::binaries::REQUIRED {
        let bin = sr.join(name);
        if !bin.is_file() {
            return Err(ReleaseError::Refused(format!("unidentified build lacks {name}")));
        }
        let run = Command::new(&bin).arg("--version").output().ok().filter(|o| o.status.success());
        let run = run.ok_or_else(|| ReleaseError::BinaryVersionFailed(name.into()))?;
        versions.push_str(&String::from_utf8_lossy(&run.stdout));
        versions.push_str(&String::from_utf8_lossy(&run.stderr));
    }
    let file = stage.0.join("VERSIONS");
    std::fs::write(&file, versions).map_err(|e| ReleaseError::Io(format!("cannot write {}: {e}", file.display())))?;
    manifest::write(&stage.0)?;
    rename(&stage.0, &dest)?;
    std::mem::forget(stage); // moved into place: the cleanup must not touch it
    out.push_str(&format!("Rollback target stays verified snapshot {verified}\nPreserved unidentified build at {}\n", dest.display()));
    Ok(())
}
