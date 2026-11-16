//! `svanbot-release`, run as an operator runs it: install a staged release, read the panel, and
//! hear the refusals that keep an update from happening by accident.
//!
//! Every test drives the real binary with a cleared environment and an install tree in a temporary
//! directory, so what is asserted is the exit code and the words a person would read. The one
//! thing not exercised here is a swap against systemd: that needs a unit on the box, and the window
//! it drives is covered by `svanbot_runtime::update`'s own tests.
//!
//! A test install tree is a set of files in a fresh temporary directory, and the binary is an app,
//! so the bans on files are off here as they are in the app itself.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use svanbot_runtime::release::digest;

const STAGED: &[u8] = b"a program\n";

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A staged release under `dir`: an executable program, the manifest naming its version and
/// checksum, and the `sha256` file the download publishes beside it.
fn stage(dir: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let program = dir.join("svanbot");
    fs::write(&program, STAGED).unwrap();
    let mut mode = fs::metadata(&program).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(&program, mode).unwrap();
    fs::write(
        dir.join("version"),
        format!("version\t{version}\nsha256\t{}\n", digest(STAGED)),
    )
    .unwrap();
    fs::write(dir.join("sha256"), format!("{}  svanbot\n", digest(STAGED))).unwrap();
    dir.to_path_buf()
}

/// Runs `svanbot-release` with `args`, a cleared environment, and the install tree at `tree`.
fn run(tree: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot-release"))
        .args(args)
        .env_clear()
        .env("SVANBOT_LIB", tree)
        .env("HOME", tree)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn install_verifies_a_staged_release_and_promotes_it_into_the_tree() {
    let tree = scratch("install");
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    let (code, out, err) = run(&tree, &["install", staged.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(out.contains("installed 2026.11.16-1"), "{out}");
    // The build is where a slot can point at it, and it is executable there: the unit execs this.
    let installed = tree.join("2026.11.16-1").join("svanbot");
    assert_eq!(fs::read(&installed).unwrap(), STAGED);
    let mode = fs::metadata(&installed).unwrap().permissions();
    assert_ne!(
        std::os::unix::fs::PermissionsExt::mode(&mode) & 0o111,
        0,
        "the installed program must be executable"
    );
    // Installing touched no slot: a release can be staged long before a window is found.
    assert!(
        !tree.join("current").exists(),
        "install must not swap a slot"
    );
}

#[test]
fn install_refuses_a_release_whose_bytes_changed_after_its_manifest() {
    let tree = scratch("tampered");
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    fs::write(staged.join("svanbot"), b"a different program\n").unwrap();
    let (code, _out, err) = run(&tree, &["install", staged.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("checksum mismatch"), "{err}");
    assert!(
        !tree.join("2026.11.16-1").exists(),
        "a release that did not verify must not be in the tree"
    );
}

#[test]
fn install_without_a_directory_says_what_it_needs() {
    let tree = scratch("nodir");
    let (code, _out, err) = run(&tree, &["install"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("directory a release was unpacked into"),
        "{err}"
    );
}

#[test]
fn status_reads_the_tree_and_says_which_slots_point_where() {
    let tree = scratch("status");
    let (code, out, err) = run(&tree, &["status"]);
    assert_eq!(code, Some(0), "{err}");
    // An install tree with nothing in it says so rather than failing: `status` is a read.
    assert!(
        out.contains(&format!("tree     {}", tree.display())),
        "{out}"
    );
    assert!(out.contains("current  -"), "{out}");
    assert!(out.contains("previous -"), "{out}");
    // No canary named: the panel still answers what is installed and says what it cannot read.
    assert!(out.contains("canary   -"), "{out}");
}

#[test]
fn status_is_what_runs_when_no_subcommand_is_given() {
    let tree = scratch("default");
    let (code, out, _err) = run(&tree, &[]);
    assert_eq!(code, Some(0));
    assert!(out.contains("tree     "), "{out}");
}

#[test]
fn status_lists_a_build_that_was_installed_but_never_swapped_in() {
    let tree = scratch("unswapped");
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    let (code, _out, err) = run(&tree, &["install", staged.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{err}");
    let (code, out, _err) = run(&tree, &["status"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("installed 2026.11.16-1"), "{out}");
    // It is installed but not current: this is exactly the state an operator checks before a window.
    assert!(out.contains("current  -"), "{out}");
    assert!(out.contains("previous -"), "{out}");
}

#[test]
fn status_does_not_list_a_staged_release_as_an_installed_build() {
    let tree = scratch("onlystaged");
    // A staged release is what `deploy/install.sh` leaves in the tree, named for what it is and not
    // for a version: it is not a build a window can point at until `install` verifies and promotes it.
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    assert!(staged.is_dir());
    let (code, out, _err) = run(&tree, &["status"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("installed -"), "{out}");
    assert!(!out.contains("installed 2026.11.16-1"), "{out}");
    // And with nothing installed but a staged release, there is no build to swap to.
    let (code, _out, err) = run(&tree, &["update"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("no build is installed"), "{err}");
}

#[test]
fn update_with_nothing_installed_says_so() {
    let tree = scratch("nobuild");
    let (code, _out, err) = run(&tree, &["update"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("no build is installed"), "{err}");
}

#[test]
fn update_needs_the_canary_named_before_it_touches_a_bot() {
    let tree = scratch("nocanary");
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    let (code, _out, err) = run(&tree, &["install", staged.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{err}");
    // A build is installed, so the next refusal is the missing canary: the one bot that takes
    // every update first is named, never guessed.
    let (code, _out, err) = run(&tree, &["update", "2026.11.16-1"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("SVANBOT_CANARY"), "{err}");
}

#[test]
fn update_refuses_a_version_that_is_not_installed() {
    let tree = scratch("noversion");
    let staged = stage(&tree.join("staged"), "2026.11.16-1");
    let (code, _out, err) = run(&tree, &["install", staged.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{err}");
    let (code, _out, err) = run(&tree, &["update", "2026.11.17-1"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("is not installed"), "{err}");
}

#[test]
fn rollback_with_nothing_to_go_back_to_says_so() {
    let tree = scratch("norollback");
    // No `previous` slot at all: the harm guard and an operator get the same answer, and no slot
    // is written for a rollback that has nowhere to land.
    let (code, _out, err) = run(&tree, &["rollback"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("nothing to roll back to"), "{err}");
}

#[test]
fn an_unknown_subcommand_prints_the_usage() {
    let tree = scratch("unknown");
    let (code, _out, err) = run(&tree, &["frobnicate"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("unknown subcommand `frobnicate`"), "{err}");
    assert!(err.contains("usage: svanbot-release"), "{err}");
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    let tree = scratch("help");
    let (code, out, _err) = run(&tree, &["help"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("usage: svanbot-release"), "{out}");
    // The four subcommands a person or CI types are all named in the help.
    for verb in ["status", "install", "update", "rollback"] {
        assert!(out.contains(verb), "help must name {verb}: {out}");
    }
}
