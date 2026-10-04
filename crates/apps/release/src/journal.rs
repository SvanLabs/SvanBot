//! The swap journal (issue #726): the directory renames that make an install are five steps with no
//! transaction around them, so a SIGKILL between any two leaves the tree half installed. The
//! journal is written and flushed before the first rename, which accounts for every kill point: the
//! next operation, keepalive's repair or `rollback --repair` puts the previous verified sets back.
//! The file format is the shell's, so either side can repair what the other interrupted.

use crate::error::{ReleaseError, Result};
use crate::fsops::{remove_all, rename};
use crate::identity;
use crate::layout::Root;
use std::path::{Component, Path, PathBuf};

/// What a swap in progress has moved aside, and what it had before.
pub struct Journal {
    /// The staging tree both callers build (`target/.install-*` or `target/.rollback-*`).
    pub staging: PathBuf,
    /// Where the previous executables are parked while the new ones go in.
    pub old_release: PathBuf,
    /// Where the previous dashboard is parked.
    pub old_web: PathBuf,
    /// Whether there was an installed set before this swap.
    pub have_release: bool,
    /// Whether there was a dashboard before this swap.
    pub have_web: bool,
}

/// `artifacts/release-swap.journal`.
pub fn path(root: &Root) -> PathBuf {
    root.join("artifacts/release-swap.journal")
}

impl Journal {
    /// Write the journal durably (data, rename and directory flushed) before any rename it covers.
    pub fn write(&self, root: &Root) -> Result<()> {
        let text = format!(
            "release_root={}\nstaging={}\nold_release={}\nold_web={}\nhave_release={}\nhave_web={}\n",
            root.path().display(),
            self.staging.display(),
            self.old_release.display(),
            self.old_web.display(),
            u8::from(self.have_release),
            u8::from(self.have_web),
        );
        let file = path(root);
        sv10_rt::write_atomic(&file, &text).map_err(|e| ReleaseError::Io(format!("cannot write {}: {e}", file.display())))
    }
}

/// Drop the record. Called once the tree is consistent again, either way.
pub fn clear(root: &Root) -> Result<()> {
    let file = path(root);
    sv10_rt::remove_stale_file(&file).map_err(|e| ReleaseError::Io(format!("cannot remove {}: {e}", file.display())))
}

/// Remove the scratch directories of swaps whose process is gone. A kill between dropping the
/// journal and removing the previous sets leaves a `.before-swap` directory behind with no journal
/// to repair from, and untracked `web/.dist.before-swap.<pid>` reads as a build input, so every
/// later release would be refused. The pid in the name is the run that made it: a live one may be
/// mid-swap and is left alone.
pub fn sweep_scratch(root: &Root) -> Result<()> {
    // A journal owns its scratch directories: the repair puts them back, so clearing them here
    // would destroy the previous install it is about to restore.
    if path(root).is_file() {
        return Ok(());
    }
    for (dir, prefixes) in
        [("target", [".release.before-swap.", ".release.failed-swap."]), ("web", [".dist.before-swap.", ".dist.failed-swap."])]
    {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !prefixes.iter().any(|p| name.starts_with(p)) {
                continue;
            }
            let pid = name.rsplit('.').next().unwrap_or("");
            if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) || Path::new(&format!("/proc/{pid}")).is_dir() {
                continue;
            }
            remove_all(&entry.path())?;
        }
    }
    Ok(())
}

fn parse(text: &str) -> [Option<String>; 6] {
    let mut out: [Option<String>; 6] = Default::default();
    for line in text.lines() {
        let (key, value) = line.split_once('=').unwrap_or((line, line));
        let slot = match key {
            "release_root" => 0,
            "staging" => 1,
            "old_release" => 2,
            "old_web" => 3,
            "have_release" => 4,
            "have_web" => 5,
            _ => continue,
        };
        out[slot] = Some(value.to_string());
    }
    out
}

/// Restore whatever an interrupted swap left behind. Idempotent: the journal is removed only after
/// the previous sets are back, and a run killed mid-repair is repaired again by the next one.
pub fn repair(root: &Root) -> Result<()> {
    let file = path(root);
    if !file.is_file() {
        return sweep_scratch(root);
    }
    let text = std::fs::read_to_string(&file).map_err(|e| ReleaseError::Io(format!("cannot read {}: {e}", file.display())))?;
    let [recorded, staging, old_release, old_web, have_release, have_web] = parse(&text);
    let base = root.path();
    if recorded.as_deref().map(Path::new) != Some(base) {
        return Err(ReleaseError::Journal(format!("swap journal names another release root: {}", recorded.as_deref().unwrap_or("<none>"))));
    }
    // The shell checks the prefix only; a `..` component would let a damaged journal point out of the root.
    let inside = |p: &Path| p.starts_with(base) && !p.components().any(|c| c == Component::ParentDir);
    let [staging, old_release, old_web] = [staging, old_release, old_web].map(|v| PathBuf::from(v.unwrap_or_default()));
    for value in [&staging, &old_release, &old_web] {
        if !inside(value) {
            let shown = value.display().to_string();
            return Err(ReleaseError::Journal(format!(
                "swap journal names a path outside the release root: {}",
                if shown.is_empty() { "<none>" } else { &shown }
            )));
        }
    }
    let target = root.join("target");
    let name = staging.strip_prefix(&target).ok().and_then(Path::to_str).unwrap_or("");
    if staging.parent() != Some(target.as_path()) || !(name.starts_with(".install-") || name.starts_with(".rollback-")) {
        return Err(ReleaseError::Journal(format!("swap journal names an unexpected staging directory: {}", staging.display())));
    }
    eprintln!("rollback: repairing an interrupted release swap (journal {})", file.display());
    // A killed run's cleanup never ran, so its staging tree, and the copy about to be installed, is
    // still there. Removing both staged sets leaves the tree consistent either way.
    remove_all(&staging)?;
    let mut restored = false;
    let (release, web) = (root.join("target/release"), root.join("web/dist"));
    if have_release.as_deref() == Some("1") && old_release.exists() {
        remove_all(&release)?;
        rename(&old_release, &release)?;
        restored = true;
    } else if have_release.as_deref() == Some("0") && release.exists() {
        // A first install that never finished: there was no previous set, so none is installed again.
        remove_all(&release)?;
        restored = true;
    }
    if have_web.as_deref() == Some("1") && old_web.exists() {
        remove_all(&web)?;
        rename(&old_web, &web)?;
        restored = true;
    } else if have_web.as_deref() == Some("0") && web.exists() {
        remove_all(&web)?;
        restored = true;
    }
    clear(root)?;
    if restored {
        let now = identity::installed_commit(base).ok().flatten().unwrap_or_else(|| "unidentified".into());
        eprintln!("rollback: restored the previous install; installed commit is now {now}");
    }
    // The journal is gone: whatever scratch is left now belongs to a run that died earlier.
    sweep_scratch(root)
}
