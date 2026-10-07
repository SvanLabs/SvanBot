//! The directory swap (`swap_install` in `rollback.sh`): the staged executables and dashboard
//! replace `target/release` and `web/dist` with renames only, journalled first, so a kill at any
//! point is something [`crate::journal::repair`] can put right.

use crate::error::{ReleaseError, Result};
use crate::fsops::{remove_all, rename};
use crate::journal::{self, Journal};
use crate::layout::Root;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A failure the tests ask for after the executables are swapped, the one point where the tree is
/// half installed. Refused on the real checkout, so a stray variable cannot break a live install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inject {
    /// Nothing; the only value production uses.
    Off,
    /// `SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP=1`: fail, and restore the previous sets.
    FailAfterBinSwap,
    /// `SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP=1`: `SIGKILL` this process, leaving the journal.
    KillAfterBinSwap,
}

impl Inject {
    /// Read the two test variables. `SV10_SCRIPT_ROOT` is the checkout the scripts run from; an
    /// injection is allowed only when it is set and is not the root being installed into.
    pub fn from_env(root: &Root, env: &[(String, String)]) -> Result<Inject> {
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        let inject = if get("SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP") == Some("1") {
            Inject::KillAfterBinSwap
        } else if get("SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP") == Some("1") {
            Inject::FailAfterBinSwap
        } else {
            return Ok(Inject::Off);
        };
        let script_root = get("SV10_SCRIPT_ROOT").and_then(|p| std::fs::canonicalize(p).ok());
        if script_root.as_deref().is_none_or(|s| s == root.path()) {
            return Err(ReleaseError::Swap("test failure injection is forbidden on the real root".into()));
        }
        Ok(inject)
    }
}

fn dev(path: &Path) -> Result<u64> {
    std::fs::metadata(path).map(|m| m.dev()).map_err(|e| ReleaseError::Io(format!("cannot inspect {}: {e}", path.display())))
}

fn fail(root: &Root, message: &str) -> ReleaseError {
    // A failure that restored the previous sets by itself is not interrupted: drop the journal so
    // the next operation does not repair a tree that is already consistent.
    match journal::clear(root) {
        Ok(()) => ReleaseError::Swap(message.into()),
        Err(e) => e,
    }
}

struct Parked {
    old_release: PathBuf,
    old_web: PathBuf,
    failed_release: PathBuf,
    failed_web: PathBuf,
}

impl Parked {
    /// Put the previous sets back and clear the failed ones away.
    fn restore(&self, root: &Root) -> Result<()> {
        let (release, web) = (root.join("target/release"), root.join("web/dist"));
        if release.exists() {
            rename(&release, &self.failed_release)?;
        }
        if self.old_release.exists() {
            rename(&self.old_release, &release)?;
        }
        if self.old_web.exists() {
            if web.exists() {
                rename(&web, &self.failed_web)?;
            }
            rename(&self.old_web, &web)?;
        }
        remove_all(&self.failed_release)?;
        remove_all(&self.failed_web)
    }
}

/// Swap `staged_release` and `staged_web` (both inside one staging tree two levels down, as the
/// callers build it) in for the installed sets.
pub fn install(root: &Root, staged_release: &Path, staged_web: &Path, inject: Inject) -> Result<()> {
    let pid = std::process::id();
    let (release, web) = (root.join("target/release"), root.join("web/dist"));
    if dev(&root.join("target"))? != dev(&root.join("web"))? {
        return Err(ReleaseError::Swap("target and web are on different filesystems; atomic directory swap is unavailable".into()));
    }
    let parked = Parked {
        old_release: root.join(&format!("target/.release.before-swap.{pid}")),
        old_web: root.join(&format!("web/.dist.before-swap.{pid}")),
        failed_release: root.join(&format!("target/.release.failed-swap.{pid}")),
        failed_web: root.join(&format!("web/.dist.failed-swap.{pid}")),
    };
    for path in [&parked.old_release, &parked.old_web, &parked.failed_release, &parked.failed_web] {
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(ReleaseError::Swap(format!("swap scratch path exists: {}", path.display())));
        }
    }
    let (have_release, have_web) = (release.is_dir(), web.is_dir());
    if have_release != have_web {
        return Err(ReleaseError::Swap("installed executable and dashboard sets are incomplete".into()));
    }
    // The staging tree both callers build (`target/.install-<commit>.XXXXXX`); the journal names it
    // so a killed run's leftovers are removed with the repair.
    let staging = staged_release.parent().and_then(Path::parent).unwrap_or(staged_release).to_path_buf();
    Journal { staging, old_release: parked.old_release.clone(), old_web: parked.old_web.clone(), have_release, have_web }.write(root)?;
    if !have_release {
        if rename(staged_release, &release).is_err() {
            return Err(fail(root, "could not install the first executable set"));
        }
        if rename(staged_web, &web).is_err() {
            rename(&release, staged_release)?;
            return Err(fail(root, "could not install the first dashboard set"));
        }
        return journal::clear(root);
    }
    rename(&release, &parked.old_release)?;
    if inject == Inject::KillAfterBinSwap {
        sv10_rt::kill_self(); // a real SIGKILL at the worst point: no cleanup, the journal is the record
    }
    if rename(staged_release, &release).is_err() {
        rename(&parked.old_release, &release)?;
        return Err(fail(root, "could not install staged executable set"));
    }
    if inject == Inject::FailAfterBinSwap {
        parked.restore(root)?;
        return Err(fail(root, "injected failure after executable swap"));
    }
    if rename(&web, &parked.old_web).is_err() {
        parked.restore(root)?;
        return Err(fail(root, "could not stage the previous dashboard for replacement"));
    }
    if rename(staged_web, &web).is_err() {
        parked.restore(root)?;
        return Err(fail(root, "could not install staged dashboard set"));
    }
    // The swap is complete: drop the record before the old sets, so a kill from here on leaves the
    // new build installed rather than a journal that would roll it back.
    journal::clear(root)?;
    // The new build is installed. Previous sets that will not go are left for the next run's sweep
    // (`journal::sweep_scratch`): failing here reported a failed install of a build already live.
    for old in [&parked.old_release, &parked.old_web] {
        if let Err(e) = remove_all(old) {
            eprintln!("warning: the previous set was not removed: {e}");
        }
    }
    Ok(())
}
