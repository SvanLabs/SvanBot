//! The write side: the operation lock, the swap journal and its repair, the scratch sweep, the
//! directory swap and the snapshot restore, against throwaway roots.

use super::fixture::{Fixture, git};
use crate::cli::rollback;
use crate::error::ReleaseError;
use crate::layout::Root;
use crate::swap::{self, Inject};
use crate::{journal, lock};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| format!("<{e}>"))
}

/// A root with commit A (its snapshot, a different dashboard) and a newer commit B installed.
struct Installed {
    f: Fixture,
    newer: String,
}

fn installed(tag: &str) -> Installed {
    let f = Fixture::new(tag);
    f.snapshot();
    std::fs::write(f.dir.join("README"), "y").unwrap();
    git(&f.dir, &["commit", "-q", "-am", "newer"]);
    let out = Command::new("git").arg("-C").arg(&f.dir).args(["rev-parse", "--short=7", "HEAD"]).output().unwrap();
    let newer = String::from_utf8(out.stdout).unwrap().trim().to_string();
    f.set(&f.dir, Some(&newer));
    std::fs::write(f.dir.join("target/release/.sv10-installed-commit"), format!("{newer}\n")).unwrap();
    std::fs::write(f.dir.join("web/dist/index.html"), "newer dashboard").unwrap();
    Installed { f, newer }
}

/// Run `attempt` until it succeeds. A lock outlives its holder by the moment another test's `fork`
/// still has the descriptor open before `exec` closes it, so a lock taken straight after a release
/// is retried rather than trusted to be free.
fn eventually<T, E>(mut attempt: impl FnMut() -> Result<T, E>) -> T {
    for _ in 0..100 {
        if let Ok(v) = attempt() {
            return v;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    attempt().unwrap_or_else(|_| panic!("still busy after two seconds"))
}

fn scratch_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect()
}

#[test]
fn the_operation_lock_is_exclusive_and_an_inherited_one_must_be_the_managed_file() {
    let f = Fixture::new("lock");
    let held = lock::acquire(&f.root, None).expect("the first holder gets it");
    assert_eq!(lock::acquire(&f.root, None).err(), Some(ReleaseError::LockBusy));
    assert_eq!(ReleaseError::LockBusy.to_string(), "another release, snapshot, or rollback operation is active");
    drop(held);
    eventually(|| lock::acquire(&f.root, None)); // released with its holder

    // A parent's descriptor: the right file, held, is accepted; any other file is not the lock.
    let lock_file = std::fs::File::create(f.dir.join("artifacts/release-operation.lock")).unwrap();
    eventually(|| lock_file.try_lock());
    let fd = lock_file.as_raw_fd().to_string();
    assert!(lock::acquire(&f.root, Some(&fd)).is_ok());
    let other = std::fs::File::create(f.dir.join("elsewhere")).unwrap();
    assert_eq!(lock::acquire(&f.root, Some(&other.as_raw_fd().to_string())).err(), Some(ReleaseError::LockForeign));
    assert_eq!(lock::acquire(&f.root, Some("999")).err(), Some(ReleaseError::LockForeign), "a closed descriptor names no file");
    // The managed file, but not locked by anyone we share it with.
    drop(lock_file);
    let unlocked = std::fs::File::open(f.dir.join("artifacts/release-operation.lock")).unwrap();
    let holder = std::fs::File::create(f.dir.join("artifacts/release-operation.lock")).unwrap();
    eventually(|| holder.try_lock());
    assert_eq!(lock::acquire(&f.root, Some(&unlocked.as_raw_fd().to_string())).err(), Some(ReleaseError::LockNotHeld));
}

#[test]
fn the_journal_is_written_in_the_shells_format() {
    let f = Fixture::new("journal");
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    let base = f.root.path().display().to_string();
    journal::Journal {
        staging: f.dir.join("target/.install-abc.XYZ123"),
        old_release: f.dir.join("target/.release.before-swap.7"),
        old_web: f.dir.join("web/.dist.before-swap.7"),
        have_release: true,
        have_web: false,
    }
    .write(&f.root)
    .unwrap();
    assert_eq!(
        read(journal::path(&f.root)),
        format!(
            "release_root={base}\nstaging={base}/target/.install-abc.XYZ123\nold_release={base}/target/.release.before-swap.7\nold_web={base}/web/.dist.before-swap.7\nhave_release=1\nhave_web=0\n"
        )
    );
}

#[test]
fn a_restore_installs_the_snapshot_logs_it_and_leaves_no_scratch() {
    let i = installed("restore");
    let f = &i.f;
    let mut out = String::new();
    rollback(&f.root, &args(&[&f.commit]), &[], &mut out).unwrap();
    assert_eq!(
        out,
        format!(
            "Verified release snapshot {}\nRestored {} from verified snapshot (previous {}). Watch the hot-swap log.\n",
            f.commit, f.commit, i.newer
        )
    );
    assert_eq!(read(f.dir.join("target/release/.sv10-installed-commit")).trim(), f.commit);
    assert_eq!(read(f.dir.join("web/dist/index.html")), "<html></html>");
    assert!(!journal::path(&f.root).exists());
    assert_eq!(scratch_names(&f.dir.join("target")), Vec::<String>::new());
    assert_eq!(scratch_names(&f.dir.join("web")), Vec::<String>::new());
    let log = read(f.dir.join("artifacts/releases.log"));
    let (stamp, rest) = log.trim_end().split_once(' ').unwrap();
    assert_eq!(rest, format!("{} rollback from {}", f.commit, i.newer));
    assert!(stamp.len() == 25 && stamp.as_bytes()[10] == b'T' && matches!(stamp.as_bytes()[19], b'+' | b'-'), "FT T%:z, got {stamp}");
    let mode = std::fs::metadata(f.dir.join("target/release/sv10-bot")).unwrap();
    assert_ne!(std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o111, 0, "the copy keeps its executable bits");
}

#[test]
fn an_injected_failure_after_the_executable_swap_puts_the_previous_sets_back() {
    let i = installed("inject");
    let f = &i.f;
    let elsewhere = f.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let hook = env(&[("SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP", "1"), ("SV10_SCRIPT_ROOT", elsewhere.to_str().unwrap())]);
    let err = rollback(&f.root, &args(&[&f.commit]), &hook, &mut String::new()).unwrap_err();
    assert_eq!(err, ReleaseError::Swap("injected failure after executable swap".into()));
    assert_eq!(read(f.dir.join("target/release/.sv10-installed-commit")).trim(), i.newer, "the previous executables are back");
    assert_eq!(read(f.dir.join("web/dist/index.html")), "newer dashboard");
    assert!(!journal::path(&f.root).exists(), "a swap that restored itself is not interrupted");
    assert_eq!(scratch_names(&f.dir.join("target")), Vec::<String>::new());
    assert!(!f.dir.join("artifacts/releases.log").exists(), "a failed restore logs nothing");
}

#[test]
fn a_failure_injection_is_refused_on_the_checkout_the_scripts_run_from() {
    let f = Fixture::new("forbid");
    let root = f.dir.to_str().unwrap();
    for hook in [
        env(&[("SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP", "1"), ("SV10_SCRIPT_ROOT", root)]),
        env(&[("SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP", "1")]),
    ] {
        let err = Inject::from_env(&f.root, &hook).unwrap_err();
        assert_eq!(err.to_string(), "test failure injection is forbidden on the real root");
    }
    assert_eq!(Inject::from_env(&f.root, &[]), Ok(Inject::Off));
}

/// The child of the kill test: with `SV10_KILL_ROOT` set it restores and is killed mid-swap; run
/// by itself it does nothing.
#[test]
fn kill_child() {
    let Some(dir) = std::env::var_os("SV10_KILL_ROOT") else { return };
    let root = Root::open(Path::new(&dir), std::env::var_os("HOME").map(std::path::PathBuf::from).as_deref()).unwrap();
    let commit = std::env::var("SV10_KILL_COMMIT").unwrap();
    let vars: Vec<_> = std::env::vars().collect();
    let _ = rollback(&root, &[commit], &vars, &mut String::new());
    unreachable!("the injected SIGKILL ends this process");
}

#[test]
fn a_swap_killed_halfway_is_repaired_to_the_previous_install() {
    let i = installed("kill");
    let f = &i.f;
    let elsewhere = f.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::swap::kill_child", "--nocapture"])
        .env("SV10_KILL_ROOT", &f.dir)
        .env("SV10_KILL_COMMIT", &f.commit)
        .env("HOME", std::env::temp_dir())
        .env("SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP", "1")
        .env("SV10_SCRIPT_ROOT", &elsewhere)
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "the child died of SIGKILL, not of a result");
    assert!(journal::path(&f.root).is_file(), "a killed swap leaves its journal");
    assert!(!f.dir.join("target/release").exists(), "and the tree is half installed: no executables at all");

    let mut out = String::new();
    rollback(&f.root, &args(&["--repair"]), &[], &mut out).unwrap();
    assert_eq!(out, "", "there was something to repair");
    assert_eq!(read(f.dir.join("target/release/.sv10-installed-commit")).trim(), i.newer);
    assert_eq!(read(f.dir.join("web/dist/index.html")), "newer dashboard");
    assert!(!journal::path(&f.root).exists());
    assert_eq!(scratch_names(&f.dir.join("target")), Vec::<String>::new(), "the staging tree went with the repair");
    let mut again = String::new();
    rollback(&f.root, &args(&["--repair"]), &[], &mut again).unwrap();
    assert_eq!(again, "No interrupted release swap to repair\n");
}

#[test]
fn a_journal_that_cannot_be_trusted_is_refused_before_anything_moves() {
    let i = installed("badjournal");
    let f = &i.f;
    let base = f.root.path().display().to_string();
    let write = |staging: &str, old_release: &str, root: &str| {
        std::fs::write(
            journal::path(&f.root),
            format!("release_root={root}\nstaging={staging}\nold_release={old_release}\nold_web={base}/web/.dist.before-swap.1\nhave_release=1\nhave_web=1\n"),
        )
        .unwrap();
    };
    let good = format!("{base}/target/.install-x.AAAAAA");
    let release = format!("{base}/target/.release.before-swap.1");
    write(&good, &release, "/somewhere/else");
    assert_eq!(journal::repair(&f.root), Err(ReleaseError::Journal("swap journal names another release root: /somewhere/else".into())));
    write(&good, "/etc/passwd", &base);
    assert_eq!(
        journal::repair(&f.root),
        Err(ReleaseError::Journal("swap journal names a path outside the release root: /etc/passwd".into()))
    );
    write(&good, &format!("{base}/target/../../x"), &base);
    assert!(matches!(journal::repair(&f.root), Err(ReleaseError::Journal(m)) if m.starts_with("swap journal names a path outside")));
    write(&format!("{base}/target/release"), &release, &base);
    assert_eq!(
        journal::repair(&f.root),
        Err(ReleaseError::Journal(format!("swap journal names an unexpected staging directory: {base}/target/release")))
    );
    assert_eq!(read(f.dir.join("target/release/.sv10-installed-commit")).trim(), i.newer, "the install was not touched");
    assert!(journal::path(&f.root).is_file(), "and the evidence is kept");
}

#[test]
fn the_sweep_removes_scratch_of_dead_swaps_only_and_never_while_a_journal_exists() {
    let f = Fixture::new("sweep");
    let dead = 999_999_999u32;
    let live = std::process::id();
    let made = |dir: &str, name: &str| {
        let p = f.dir.join(dir).join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let orphans = [
        made("web", &format!(".dist.before-swap.{dead}")),
        made("web", &format!(".dist.failed-swap.{dead}")),
        made("target", &format!(".release.before-swap.{dead}")),
        made("target", &format!(".release.failed-swap.{dead}")),
    ];
    let alive = made("web", &format!(".dist.before-swap.{live}"));
    let odd = made("web", ".dist.before-swap.notapid");
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    std::fs::write(journal::path(&f.root), "").unwrap();
    journal::sweep_scratch(&f.root).unwrap();
    assert!(orphans.iter().all(|p| p.exists()), "a journal owns its scratch");
    std::fs::remove_file(journal::path(&f.root)).unwrap();
    journal::sweep_scratch(&f.root).unwrap();
    assert!(orphans.iter().all(|p| !p.exists()), "dead swaps are cleared");
    assert!(alive.exists() && odd.exists(), "a live pid and a name that is no pid are left alone");
}

#[test]
fn a_first_install_has_nothing_to_park_and_a_dirty_checkout_is_named() {
    let f = Fixture::new("first");
    std::fs::write(f.dir.join(".gitignore"), "/target\n/web/dist\n").unwrap();
    git(&f.dir, &["add", ".gitignore"]);
    git(&f.dir, &["commit", "-q", "-m", "ignore outputs"]);
    std::fs::create_dir_all(f.dir.join("target")).unwrap();
    std::fs::create_dir_all(f.dir.join("web")).unwrap();
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    let stage = f.dir.join("target/.install-first.ABC123");
    f.set(&stage, Some(&f.commit));
    swap::install(&f.root, &stage.join("target/release"), &stage.join("web/dist"), Inject::Off).unwrap();
    assert_eq!(read(f.dir.join("web/dist/index.html")), "<html></html>");
    assert!(f.dir.join("target/release/sv10-bot").is_file() && !journal::path(&f.root).exists());

    let mut out = String::new();
    assert!(
        rollback(&f.root, &args(&["--validate-source-clean"]), &[], &mut out).is_ok(),
        "committed files and untracked outputs are clean"
    );
    std::fs::create_dir_all(f.dir.join("crates")).unwrap();
    std::fs::write(f.dir.join("crates/new.rs"), "x").unwrap();
    assert_eq!(rollback(&f.root, &args(&["--validate-source-clean"]), &[], &mut out), Err(ReleaseError::SourceDirty));
    std::fs::remove_dir_all(f.dir.join("crates")).unwrap();
    let live = std::process::id();
    std::fs::create_dir_all(f.dir.join(format!("web/.dist.before-swap.{live}"))).unwrap();
    std::fs::write(f.dir.join(format!("web/.dist.before-swap.{live}/index.html")), "x").unwrap();
    assert!(rollback(&f.root, &args(&["--validate-source-clean"]), &[], &mut out).is_ok(), "swap scratch is not a build input");
}

#[test]
fn a_restore_of_a_build_the_store_cannot_be_read_by_is_refused() {
    let i = installed("store");
    std::fs::write(i.f.dir.join("artifacts/data-format"), "2\n").unwrap();
    let err = rollback(&i.f.root, &args(&[&i.f.commit]), &[], &mut String::new()).unwrap_err();
    assert_eq!(err, ReleaseError::StoreTooNew { commit: i.f.commit.clone(), build: 1, store: 2 });
    assert!(err.to_string().starts_with(&format!("build {} reads data format 1 but the databases hold format 2", i.f.commit)));
    assert_eq!(read(i.f.dir.join("target/release/.sv10-installed-commit")).trim(), i.newer);
}
