//! A release, as the updater reads it: the version a build is, and the checksum that says its
//! bytes are the ones CI published.
//!
//! A staged release is what `svanbot-release install` unpacks: the program, the manifest naming the
//! version it is, and the archive's checksum. Verification is the step that stands between a
//! download and the fleet: nothing is installed, and `current` is never pointed at anything, until
//! the program's bytes hash to the checksum the release published.
// A test release is a set of files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::fs;
use std::path::PathBuf;

use svanbot_runtime::release::{Release, ReleaseError, digest, newest, version_cmp};

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-release-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A staged release: an executable `svanbot` with `bytes` in it, and the manifest that names it.
fn stage(dir: &PathBuf, version: &str, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("svanbot"), bytes).unwrap();
    let mut mode = fs::metadata(dir.join("svanbot")).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(dir.join("svanbot"), mode).unwrap();
    fs::write(
        dir.join("version"),
        format!("version\t{version}\nsha256\t{}\n", digest(bytes)),
    )
    .unwrap();
    fs::write(dir.join("sha256"), format!("{}  svanbot\n", digest(bytes))).unwrap();
    dir.clone()
}

#[test]
fn a_staged_release_reads_its_version_and_verifies() {
    let root = scratch("good");
    let staged = stage(&root, "2026.11.16-1", b"a program");
    let release = Release::at(&staged).unwrap();
    assert_eq!(release.version(), "2026.11.16-1");
    release.verify().unwrap();
}

#[test]
fn a_program_whose_bytes_changed_after_the_manifest_is_refused() {
    let root = scratch("tampered");
    let staged = stage(&root, "2026.11.16-1", b"a program");
    fs::write(staged.join("svanbot"), b"a different program").unwrap();
    let release = Release::at(&staged).unwrap();
    let error = release.verify().unwrap_err();
    assert!(
        matches!(error, ReleaseError::Checksum { .. }),
        "tampering must be caught as a checksum mismatch: {error:?}"
    );
    // Nothing about a refusal is silent: the message names both hashes so an operator can compare
    // them against the release page.
    let said = error.to_string();
    assert!(said.contains(&digest(b"a program")) && said.contains(&digest(b"a different program")));
}

#[test]
fn a_release_with_no_manifest_is_not_a_release() {
    let root = scratch("nomanifest");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("svanbot"), b"a program").unwrap();
    let error = Release::at(&root).unwrap_err();
    assert!(matches!(error, ReleaseError::Manifest(_)), "{error:?}");
}

#[test]
fn a_release_with_no_program_is_not_a_release() {
    let root = scratch("noprogram");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("version"), "version\t2026.11.16-1\n").unwrap();
    let error = Release::at(&root).unwrap_err();
    assert!(matches!(error, ReleaseError::NoProgram { .. }), "{error:?}");
}

#[test]
fn a_manifest_that_names_no_version_is_refused() {
    let root = scratch("noversion");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("svanbot"), b"a program").unwrap();
    fs::write(root.join("version"), "sha256\tdeadbeef\n").unwrap();
    let error = Release::at(&root).unwrap_err();
    assert!(matches!(error, ReleaseError::Manifest(_)), "{error:?}");
}

#[test]
fn a_program_that_cannot_be_run_is_not_installed() {
    // The file is there and the checksum matches, but the mode is not executable: the supervisor
    // would fail to exec it, so verification refuses it here rather than at the next restart.
    let root = scratch("nomode");
    let staged = stage(&root, "2026.11.16-1", b"a program");
    let mut mode = fs::metadata(staged.join("svanbot")).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o644);
    fs::set_permissions(staged.join("svanbot"), mode).unwrap();
    let release = Release::at(&staged).unwrap();
    let error = release.verify().unwrap_err();
    assert!(
        matches!(error, ReleaseError::NotExecutable { .. }),
        "{error:?}"
    );
}

#[test]
fn an_executable_program_verifies() {
    let root = scratch("exec");
    let staged = stage(&root, "2026.11.16-1", b"a program");
    let mut mode = fs::metadata(staged.join("svanbot")).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(staged.join("svanbot"), mode).unwrap();
    let release = Release::at(&staged).unwrap();
    assert!(release.verify().is_ok());
}

#[test]
fn a_version_is_safe_to_use_as_a_directory_name() {
    // The version becomes the name of the installed directory, so anything that could escape the
    // install tree or collide with a slot's name is refused before it is a path.
    let root = scratch("badversion");
    for bad in [
        "../escape",
        "current",
        "previous",
        "a/b",
        ".",
        "..",
        ".hidden",
    ] {
        let staged = stage(&root.join("staged"), bad, b"a program");
        let error = Release::at(&staged).unwrap_err();
        assert!(
            matches!(error, ReleaseError::BadVersion { .. }),
            "{bad:?} must be refused: {error:?}"
        );
    }
}

#[test]
fn a_release_that_names_no_version_at_all_is_refused() {
    // An empty value in the manifest is not a version that slipped through: it reads as no version,
    // which is the same refusal an absent manifest gets.
    let root = scratch("emptyversion");
    let staged = stage(&root, "", b"a program");
    let error = Release::at(&staged).unwrap_err();
    assert!(matches!(error, ReleaseError::Manifest(_)), "{error:?}");
}

#[test]
fn a_version_in_the_shape_ci_publishes_is_accepted() {
    let root = scratch("goodversion");
    for good in ["2026.11.16-1", "v11.0.0", "v11.0.0-rc.1", "11.0.0"] {
        let staged = stage(&root.join(good), good, b"a program");
        assert_eq!(Release::at(&staged).unwrap().version(), good);
    }
}

#[test]
fn versions_order_by_their_numbers_and_not_as_text() {
    use std::cmp::Ordering::*;
    // Choosing a build means choosing the newest, and a name read as text gets that wrong the
    // moment a segment's digit count differs: `1.9.0` sorts after `1.10.0`, and `2026.11.9` after
    // `2026.11.16`. Either is a silent downgrade, so the comparison is by number.
    for (older, newer) in [
        ("1.9.0", "1.10.0"),
        ("2026.11.9", "2026.11.16"),
        ("11.0.0", "11.0.1"),
        ("11.0.0", "11.1.0"),
        ("11.0.0", "12.0.0"),
        ("9.9.9", "10.0.0"),
    ] {
        assert_eq!(
            version_cmp(older, newer),
            Less,
            "{older} is older than {newer}"
        );
        assert_eq!(
            version_cmp(newer, older),
            Greater,
            "{newer} is newer than {older}"
        );
    }
    assert_eq!(version_cmp("11.0.0", "11.0.0"), Equal);
}

#[test]
fn a_version_a_number_cannot_rank_is_still_ordered_and_never_dropped() {
    // Releases are `N.N.N` — the workflow refuses anything else — but a build directory is named
    // however it was made. One a number cannot rank is ordered by its text rather than ignored,
    // and a rankable name outranks it, so `update` still lands on a release when one is installed.
    use std::cmp::Ordering::*;
    assert_eq!(version_cmp("nightly", "nightly"), Equal);
    assert_eq!(version_cmp("nightly", "1.0.0"), Less);
    assert_eq!(version_cmp("2.0.0", "nightly"), Greater);
}

#[test]
fn sorting_by_the_version_comparator_puts_the_newest_last() {
    // The order `update` reads to pick the build it swaps to, and `status` reads to list what is
    // installed: both sort by version, so the newest is the last of the list rather than whatever
    // a text sort put there.
    let mut versions = vec!["2026.11.9", "1.9.0", "2026.11.16", "1.10.0"];
    versions.sort_by(|a, b| version_cmp(a, b));
    assert_eq!(versions, ["1.9.0", "1.10.0", "2026.11.9", "2026.11.16"]);
}

#[test]
fn the_newest_of_a_set_of_versions_is_the_one_a_window_should_take() {
    // What `update` with no version points `current` at. Picking wrong is a downgrade no crash
    // loop catches, because the build it lands on is a good build — just an older one.
    assert_eq!(newest(["1.10.0", "1.9.0"]).as_deref(), Some("1.10.0"));
    assert_eq!(newest(["1.9.0", "1.10.0"]).as_deref(), Some("1.10.0"));
    assert_eq!(
        newest(["2026.11.16", "2026.11.9"]).as_deref(),
        Some("2026.11.16")
    );
}

#[test]
fn a_dated_build_never_outranks_a_release() {
    // Builds before v11 were named for their day, `2026.11.16-1`, and that year outranks any
    // release number. On 2026-11-23 a version-less update took one over 11.0.2 (V11-37). The
    // dated scheme is retired: a release wins over any dated build, and dated builds still rank
    // among themselves when nothing else is installed.
    assert_eq!(
        newest(["11.0.1", "2026.11.16-1", "11.0.2", "2026.11.16"]).as_deref(),
        Some("11.0.2")
    );
    assert_eq!(
        newest(["2026.11.9", "2026.11.16"]).as_deref(),
        Some("2026.11.16")
    );
}

#[test]
fn a_set_of_versions_with_nothing_in_it_has_no_newest() {
    assert_eq!(newest(std::iter::empty::<&str>()), None);
}
