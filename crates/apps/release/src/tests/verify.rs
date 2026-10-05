use super::fixture::{Fixture, git};
use crate::binaries::{Identity, require_binaries};
use crate::cli::rollback;
use crate::error::ReleaseError;
use crate::layout::Root;
use crate::{identity, manifest, snapshot, space, store_format};
use std::os::unix::fs::symlink;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_root_must_be_a_git_checkout_that_is_not_slash_or_home() {
    let f = Fixture::new("root");
    assert_eq!(Root::open(std::path::Path::new(""), None).unwrap_err(), ReleaseError::RootEmpty);
    let missing = f.dir.join("nope");
    assert!(matches!(Root::open(&missing, None), Err(ReleaseError::RootMissing(_))));
    assert!(matches!(Root::open(std::path::Path::new("/"), Some(&f.dir)), Err(ReleaseError::RootUnsafe(_))));
    assert!(matches!(Root::open(&f.dir, Some(&f.dir)), Err(ReleaseError::RootUnsafe(_))), "the home directory is not a release root");
    let plain = std::env::temp_dir().join(format!("sv10-release-plain-{}", std::process::id()));
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(Root::open(&plain, Some(&f.dir)).unwrap_err(), ReleaseError::RootNotGit);
}

#[test]
fn a_managed_path_may_not_be_a_symlink_or_leave_the_root() {
    let f = Fixture::new("layout");
    assert!(f.root.validate_layout().is_ok(), "an empty root is a valid layout");
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    let elsewhere = std::env::temp_dir().join(format!("sv10-release-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&elsewhere).unwrap();
    symlink(&elsewhere, f.dir.join("target")).unwrap();
    assert_eq!(f.root.validate_layout().unwrap_err(), ReleaseError::PathSymlink("target".into()));
    assert_eq!(f.root.assert_managed_path("../x").unwrap_err(), ReleaseError::PathUnsafe("../x".into()));
    assert_eq!(f.root.assert_managed_path("/etc").unwrap_err(), ReleaseError::PathUnsafe("/etc".into()));
    assert_eq!(f.root.assert_managed_path("").unwrap_err(), ReleaseError::PathUnsafe(String::new()));
    assert_eq!(ReleaseError::PathSymlink("target".into()).to_string(), "managed path contains a symlink: target");
}

#[test]
fn the_installed_commit_is_the_marker_then_the_newest_resolvable_log_line() {
    let f = Fixture::new("installed");
    assert_eq!(identity::installed_commit(f.root.path()).unwrap(), None, "nothing installed, nothing logged");
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    std::fs::write(f.dir.join("artifacts/releases.log"), format!("t1 {} first\nt2 deadbee gone\nt3\n", f.commit)).unwrap();
    assert_eq!(identity::installed_commit(f.root.path()).unwrap().as_deref(), Some(f.commit.as_str()), "skips lines that do not resolve");
    std::fs::create_dir_all(f.dir.join("target/release")).unwrap();
    std::fs::write(f.dir.join("target/release/.sv10-installed-commit"), "  deadbee \n").unwrap();
    assert_eq!(
        identity::installed_commit(f.root.path()).unwrap_err(),
        ReleaseError::CommitUnresolved("deadbee".into()),
        "a marker that does not resolve is an error"
    );
    std::fs::write(f.dir.join("target/release/.sv10-installed-commit"), format!("{}\n", f.commit)).unwrap();
    assert_eq!(identity::installed_commit(f.root.path()).unwrap().as_deref(), Some(f.commit.as_str()));
    assert_eq!(identity::resolve_commit(f.root.path(), "").unwrap_err(), ReleaseError::CommitEmpty);
    assert_eq!(identity::resolve_commit(f.root.path(), "HEAD").unwrap(), f.commit);
}

#[test]
fn a_manifest_is_safe_only_with_real_hashes_and_paths_inside_the_two_trees() {
    let h = "a".repeat(64);
    assert!(manifest::parse_safe(&format!("{h}  target/release/sv10-bot\n{h} *web/dist/a.js\n")).is_ok());
    assert_eq!(manifest::parse_safe("nothex  target/release/x\n").unwrap_err(), ReleaseError::ManifestHashEntry);
    for bad in ["target/release/../x", "/etc/passwd", "src/main.rs", "web/dist/../../x", ""] {
        assert_eq!(manifest::parse_safe(&format!("{h}  {bad}\n")).unwrap_err(), ReleaseError::ManifestPathUnsafe(bad.into()), "{bad}");
    }
}

#[test]
fn a_snapshot_verifies_and_every_tamper_is_refused() {
    let f = Fixture::new("snap");
    let snap = f.snapshot();
    assert!(snapshot::verify(&f.root, &f.commit).is_ok());
    // A changed file.
    std::fs::write(snap.join("web/dist/index.html"), "changed").unwrap();
    assert_eq!(snapshot::verify(&f.root, &f.commit).unwrap_err(), ReleaseError::ManifestMismatch);
    std::fs::write(snap.join("web/dist/index.html"), "<html></html>").unwrap();
    assert!(snapshot::verify(&f.root, &f.commit).is_ok());
    // An extra file the manifest does not list.
    std::fs::write(snap.join("web/dist/extra.js"), "x").unwrap();
    assert_eq!(snapshot::verify(&f.root, &f.commit).unwrap_err(), ReleaseError::ManifestIncomplete);
    std::fs::remove_file(snap.join("web/dist/extra.js")).unwrap();
    // A symlink in the tree.
    symlink("/etc/hostname", snap.join("web/dist/link")).unwrap();
    assert!(matches!(snapshot::verify(&f.root, &f.commit), Err(ReleaseError::SpecialFile(p)) if p.ends_with("web/dist/link")));
    std::fs::remove_file(snap.join("web/dist/link")).unwrap();
    // A missing manifest, then a missing snapshot.
    std::fs::rename(snap.join("SHA256SUMS"), snap.join("SUMS.bak")).unwrap();
    assert_eq!(snapshot::verify(&f.root, &f.commit).unwrap_err(), ReleaseError::SnapshotManifestMissing(f.commit.clone()));
    assert_eq!(snapshot::verify(&f.root, "abcdef0").unwrap_err(), ReleaseError::SnapshotUnsafe("abcdef0".into()));
}

#[test]
fn the_binaries_must_exist_run_and_name_the_commit() {
    let f = Fixture::new("bins");
    let base = f.dir.join("set");
    f.set(&base, Some(&f.commit));
    let check = |expected: Option<&str>, mode| require_binaries(f.root.path(), &base.join("target/release"), expected, mode);
    assert!(check(Some(&f.commit), Identity::Strict).is_ok());
    assert!(check(None, Identity::Strict).is_ok(), "no expectation, no identity needed");
    assert_eq!(check(Some("HEAD"), Identity::AllowMarker), Ok(()), "any name that resolves to the commit is the expected build");
    Fixture::program(&base.join("target/release/learner"), "learner 10.0.1 abcdef0");
    assert_eq!(
        check(Some(&f.commit), Identity::AllowMarker).unwrap_err(),
        ReleaseError::BinaryBuildMismatch { name: "learner".into(), got: "abcdef0".into(), want: f.commit.clone() }
    );
    Fixture::program(&base.join("target/release/learner"), "learner 10.0.1 abc1234 extra");
    assert_eq!(check(Some(&f.commit), Identity::AllowMarker).unwrap_err(), ReleaseError::BinaryMalformed("learner".into()));
    Fixture::program(&base.join("target/release/learner"), "other 10.0.1");
    assert_eq!(check(None, Identity::AllowMarker).unwrap_err(), ReleaseError::BinaryMalformed("learner".into()));
    std::fs::remove_file(base.join("target/release/learner")).unwrap();
    assert_eq!(check(None, Identity::AllowMarker).unwrap_err(), ReleaseError::BinaryMissing("learner".into()));
}

#[test]
fn a_build_without_its_own_commit_is_judged_by_the_marker_unless_strict() {
    let f = Fixture::new("marker");
    let base = f.dir.join("set");
    f.set(&base, None);
    let check = |mode| require_binaries(f.root.path(), &base.join("target/release"), Some(&f.commit), mode);
    assert!(check(Identity::AllowMarker).is_ok(), "the marker names the commit");
    assert_eq!(check(Identity::Strict).unwrap_err(), ReleaseError::BinaryNoIdentity("sv10-bot".into()));
    std::fs::write(base.join("target/release/.sv10-installed-commit"), "abcdef0\n").unwrap();
    assert_eq!(check(Identity::AllowMarker).unwrap_err(), ReleaseError::BinaryNoMarker("sv10-bot".into()));
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(base.join("target/release/analyst"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(base.join("target/release/.sv10-installed-commit"), format!("{}\n", f.commit)).unwrap();
    assert_eq!(check(Identity::AllowMarker).unwrap_err(), ReleaseError::BinaryNotExecutable("analyst".into()));
}

#[test]
fn free_space_is_compared_in_megabytes_and_the_setting_must_be_a_whole_number() {
    let p = std::path::Path::new("/r");
    assert_eq!(space::check_bytes(p, 5000 * 1024 * 1024, 4096).unwrap(), "5000 MB free on /r (>= 4096 MB)");
    assert_eq!(
        space::check_bytes(p, 100 * 1024 * 1024, 4096).unwrap_err().to_string(),
        "only 100 MB free on /r and a release needs about 4096 MB (SV10_MIN_FREE_MB)"
    );
    assert_eq!(space::check(p, Some("lots")).unwrap_err(), ReleaseError::SpaceSetting);
    assert_eq!(space::check(p, Some("")).unwrap_err(), ReleaseError::SpaceSetting);
    assert!(space::check(&std::env::temp_dir(), Some("1")).is_ok(), "a real volume has one megabyte free");
}

#[test]
fn the_data_format_comes_from_the_codec_line_or_defaults_to_one() {
    let f = Fixture::new("format");
    assert_eq!(store_format::of_build(f.root.path(), &f.commit), 1, "no codec file in that commit");
    std::fs::create_dir_all(f.dir.join("crates/libs/store/src")).unwrap();
    std::fs::write(f.dir.join("crates/libs/store/src/packed.rs"), "// x\npub const DATA_FORMAT: u32 = 3;\n").unwrap();
    git(&f.dir, &["add", "."]);
    git(&f.dir, &["commit", "-q", "-m", "codec"]);
    assert_eq!(store_format::of_build(f.root.path(), "HEAD"), 3);
    assert_eq!(store_format::of_store(f.root.path()), 1, "no marker file");
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    std::fs::write(f.dir.join("artifacts/data-format"), " 2\n").unwrap();
    assert_eq!(store_format::of_store(f.root.path()), 2);
    std::fs::write(f.dir.join("artifacts/data-format"), "two").unwrap();
    assert_eq!(store_format::of_store(f.root.path()), 1);
}

#[test]
fn the_rollback_surface_prints_the_scripts_lines_and_usage() {
    let f = Fixture::new("cli");
    f.snapshot();
    let mut out = String::new();
    rollback(&f.root, &args(&["--verify", &f.commit]), &[], &mut out).unwrap();
    assert_eq!(out, format!("Verified release snapshot {}\n", f.commit));
    let mut out = String::new();
    assert_eq!(
        rollback(&f.root, &args(&["--verify"]), &[], &mut out).unwrap_err().to_string(),
        "usage: scripts/rollback.sh --verify <commit>"
    );
    assert_eq!(rollback(&f.root, &args(&["--installed-commit"]), &[], &mut out).unwrap_err(), ReleaseError::NoInstalledCommit);
    assert_eq!(rollback(&f.root, &args(&[]), &[], &mut out).unwrap_err().to_string(), "usage: scripts/rollback.sh <commit>");
    assert_eq!(
        rollback(&f.root, &args(&["--validate-layout", "x"]), &[], &mut out).unwrap_err().to_string(),
        "usage: scripts/rollback.sh --validate-layout"
    );
    let env = vec![("SV10_MIN_FREE_MB".to_string(), "x".to_string())];
    assert_eq!(rollback(&f.root, &args(&["--check-space"]), &env, &mut out).unwrap_err(), ReleaseError::SpaceSetting);
    let mut out = String::new();
    rollback(&f.root, &args(&["--data-format", "HEAD"]), &[], &mut out).unwrap();
    assert_eq!(out, "1 1\n");
}

/// `lock::busy` is how a run steps aside before it has touched anything (#879): true only while
/// another holder has the lock, and it neither takes the lock nor needs the file to exist.
#[test]
fn the_lock_reads_busy_only_while_it_is_held() {
    let f = Fixture::new("lock-busy");
    assert!(!crate::lock::busy(&f.root), "no lock file yet");
    let held = crate::lock::acquire(&f.root, None).unwrap();
    assert!(crate::lock::busy(&f.root));
    drop(held);
    assert!(!crate::lock::busy(&f.root));
    assert!(crate::lock::acquire(&f.root, None).is_ok(), "asking did not take it");
}
