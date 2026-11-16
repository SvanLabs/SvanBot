//! The two slots the updater swaps between: `current` and `previous`, as absolute symlinks into
//! the installed build directories. An update fills `current` and moves the build it replaced to
//! `previous`; a rollback points `current` back. A bot reads the slot while an update swaps it, so
//! each swap is one atomic rename.
// A test slot tree is a set of files and symlinks in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::fs;
use std::path::{Path, PathBuf};

use svanbot_runtime::slot::Slots;

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-slot-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An installed build: its own version-named directory, with a program in it.
fn install(root: &Path, version: &str) -> PathBuf {
    let dir = root.join(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("svanbot"), format!("{version}\n")).unwrap();
    fs::canonicalize(dir).unwrap()
}

#[test]
fn an_install_points_current_at_the_build_and_leaves_previous_empty() {
    let root = scratch("install");
    let slots = Slots::in_dir(&root);
    let build = install(&root, "2026.11.16-1");
    slots.install(&build).unwrap();
    // Absolute: the unit resolves `current` against systemd's working directory, not the tree's.
    let link = root.join("current");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), build);
    assert_eq!(slots.current(), Some(build.clone()));
    assert_eq!(slots.previous(), None);
    assert_eq!(
        fs::read_to_string(slots.binary().unwrap()).unwrap(),
        "2026.11.16-1\n",
        "the slot must hold the program the unit execs"
    );
}

#[test]
fn a_second_install_keeps_the_build_it_replaced_as_previous() {
    let root = scratch("replace");
    let slots = Slots::in_dir(&root);
    let first = install(&root, "2026.11.16-1");
    let second = install(&root, "2026.11.16-2");
    slots.install(&first).unwrap();
    slots.install(&second).unwrap();
    assert_eq!(slots.current(), Some(second));
    assert_eq!(slots.previous(), Some(first));
}

#[test]
fn a_rollback_switches_back_and_keeps_the_failed_build_as_previous() {
    let root = scratch("rollback");
    let slots = Slots::in_dir(&root);
    let old = install(&root, "2026.11.16-1");
    let new = install(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    slots.install(&new).unwrap();
    slots.roll_back().unwrap();
    assert_eq!(slots.current(), Some(old.clone()));
    // The build that failed is still installed and still in `previous`: another rollback, or an
    // operator's file copy, must be able to reach it.
    assert_eq!(slots.previous(), Some(new));
    assert!(old.join("svanbot").is_file());
}

#[test]
fn a_rollback_with_nothing_installed_before_changes_nothing() {
    let root = scratch("noprev");
    let slots = Slots::in_dir(&root);
    let only = install(&root, "2026.11.16-1");
    slots.install(&only).unwrap();
    assert!(matches!(
        slots.roll_back(),
        Err(svanbot_runtime::slot::SlotError::NoPrevious)
    ));
    assert_eq!(slots.current(), Some(only));
    assert_eq!(slots.previous(), None);
}

#[test]
fn an_install_of_a_build_that_is_not_there_is_refused() {
    let root = scratch("ghost");
    let slots = Slots::in_dir(&root);
    let ghost = root.join("never-built");
    let error = slots.install(&ghost).unwrap_err();
    assert!(
        error.to_string().contains("never-built"),
        "the error must name the build: {error}"
    );
    // Nothing was written: a slot pointing at nothing would leave the unit unable to exec.
    assert_eq!(slots.current(), None);
    assert!(!root.join("current").exists());
}

#[test]
fn a_build_removed_from_under_a_link_reads_as_no_build() {
    let root = scratch("dangling");
    let slots = Slots::in_dir(&root);
    let build = install(&root, "2026.11.16-1");
    slots.install(&build).unwrap();
    fs::remove_dir_all(&build).unwrap();
    assert_eq!(slots.current(), None);
    assert_eq!(slots.binary(), None);
}

#[test]
fn a_swap_leaves_no_stray_temporary_link() {
    let root = scratch("tidy");
    let slots = Slots::in_dir(&root);
    slots.install(install(&root, "2026.11.16-1")).unwrap();
    slots.install(install(&root, "2026.11.16-2")).unwrap();
    slots.roll_back().unwrap();
    let strays: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !matches!(name.as_str(), "current" | "previous"))
        .filter(|name| !name.starts_with("2026."))
        .collect();
    assert!(strays.is_empty(), "temporary links left behind: {strays:?}");
}

#[test]
fn installing_the_build_that_already_runs_keeps_previous_where_it_is() {
    // The updater re-runs a smoke test and reinstalls the same version: that must not push the
    // rollback target out, or a rollback would have nowhere to go.
    let root = scratch("same");
    let slots = Slots::in_dir(&root);
    let old = install(&root, "2026.11.16-1");
    let new = install(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    slots.install(&new).unwrap();
    slots.install(&new).unwrap();
    assert_eq!(slots.current(), Some(new));
    assert_eq!(slots.previous(), Some(old));
}
