//! Snapshot creation and pruning, install, and preserving an unidentified build.

use super::fixture::{Fixture, rollback_retrying as rollback};
use crate::error::ReleaseError;
use crate::manifest;
use std::path::Path;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn run(f: &Fixture, a: &[&str], e: &[(&str, &str)]) -> Result<String, ReleaseError> {
    let mut out = String::new();
    rollback(&f.root, &args(a), &env(e), &mut out).map(|()| out)
}

#[test]
fn a_snapshot_copies_the_installed_set_with_a_manifest_and_is_kept_when_repeated() {
    let f = Fixture::new("mksnap");
    f.set(&f.dir, Some(&f.commit));
    // Hidden files and non-executables are not part of a snapshot's executables.
    std::fs::write(f.dir.join("target/release/.hidden"), "x").unwrap();
    std::fs::write(f.dir.join("target/release/libfoo.d"), "x").unwrap();
    let out = run(&f, &["--snapshot", &f.commit], &[]).unwrap();
    assert_eq!(out, format!("Verified release snapshot {c}\nCreated release snapshot {c}\n", c = f.commit));
    let snap = f.dir.join("artifacts/release-snapshots").join(&f.commit);
    let mut names: Vec<_> =
        std::fs::read_dir(snap.join("target/release")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, [".sv10-installed-commit", "analyst", "learner", "sv10-bot"]);
    let sums = std::fs::read_to_string(snap.join("SHA256SUMS")).unwrap();
    assert!(sums.lines().all(|l| l.as_bytes()[64..66] == *b"  "), "sha256sum format");
    assert_eq!(sums.lines().count(), 5);
    assert!(manifest::verify_tree(&snap, &sums).is_ok());
    let again = run(&f, &["--snapshot", &f.commit], &[]).unwrap();
    assert_eq!(again, format!("Verified release snapshot {c}\nRelease snapshot {c} already exists; keeping it unchanged\n", c = f.commit));
    // A different build is not this commit's snapshot.
    f.set(&f.dir, Some("abcdef0"));
    assert!(matches!(run(&f, &["--snapshot", &f.commit], &[]), Err(ReleaseError::BinaryBuildMismatch { .. })));
}

#[test]
fn old_snapshots_are_pruned_to_the_keep_count_and_the_new_one_survives() {
    let f = Fixture::new("prune");
    f.set(&f.dir, Some(&f.commit));
    let parent = f.dir.join("artifacts/release-snapshots");
    std::fs::create_dir_all(&parent).unwrap();
    for (i, name) in ["aaaaaaa", "bbbbbbb", "ccccccc"].iter().enumerate() {
        std::fs::create_dir_all(parent.join(name)).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
        std::fs::File::open(parent.join(name)).unwrap().set_modified(t).unwrap();
    }
    std::fs::create_dir_all(parent.join(".hidden")).unwrap();
    let out = run(&f, &["--snapshot", &f.commit], &[("SV10_KEEP_SNAPSHOTS", "2")]).unwrap();
    assert!(out.ends_with("Pruned release snapshot bbbbbbb\nPruned release snapshot aaaaaaa\n"), "{out}");
    assert!(parent.join(&f.commit).is_dir() && parent.join(".hidden").is_dir());
    for bad in ["1", "x", "-3", ""] {
        std::fs::remove_dir_all(parent.join(&f.commit)).unwrap(); // an existing snapshot returns before pruning
        let err = run(&f, &["--snapshot", &f.commit], &[("SV10_KEEP_SNAPSHOTS", bad)]).unwrap_err();
        assert_eq!(err.to_string(), "SV10_KEEP_SNAPSHOTS must be a whole number of at least 2", "{bad:?}");
    }
}

#[test]
fn an_install_stages_writes_identity_and_swaps_in_and_refuses_unsafe_sources() {
    let f = Fixture::new("install");
    let built = f.dir.join("built");
    f.set(&built, Some(&f.commit));
    std::fs::create_dir_all(f.dir.join("target")).unwrap();
    std::fs::create_dir_all(f.dir.join("web")).unwrap();
    let (bin, web) = (built.join("target/release"), built.join("web/dist"));
    let (bin, web) = (bin.to_str().unwrap(), web.to_str().unwrap());
    let out = run(&f, &["--install", bin, web, &f.commit], &[]).unwrap();
    assert_eq!(out, format!("Installed verified release set {}\n", f.commit));
    assert_eq!(std::fs::read_to_string(f.dir.join("target/release/.sv10-installed-commit")).unwrap().trim(), f.commit);
    assert!(f.dir.join("web/dist/index.html").is_file());
    // Installing over an existing set swaps and leaves no scratch behind.
    run(&f, &["--install", bin, web, &f.commit], &[]).unwrap();
    let hidden: Vec<_> =
        std::fs::read_dir(f.dir.join("target")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
    assert!(hidden.is_empty());
    // Sources: missing, outside the repository, a symlink.
    let missing = f.dir.join("nope");
    assert_eq!(
        run(&f, &["--install", missing.to_str().unwrap(), web, &f.commit], &[]).unwrap_err().to_string(),
        format!("install source is not a directory: {}", missing.display())
    );
    let outside = std::env::temp_dir();
    assert_eq!(
        run(&f, &["--install", outside.to_str().unwrap(), web, &f.commit], &[]).unwrap_err().to_string(),
        format!("install source escapes repository: {}", outside.display())
    );
    let link = f.dir.join("link");
    std::os::unix::fs::symlink(bin, &link).unwrap();
    assert_eq!(
        run(&f, &["--install", link.to_str().unwrap(), web, &f.commit], &[]).unwrap_err().to_string(),
        format!("install source is a symlink: {}", link.display())
    );
    // A strict install needs the binaries to name their own build.
    f.set(&built, None);
    assert_eq!(run(&f, &["--install", bin, web, &f.commit], &[]).unwrap_err(), ReleaseError::BinaryNoIdentity("sv10-bot".into()));
}

#[test]
fn an_unidentified_build_is_preserved_only_beside_a_verified_snapshot() {
    let f = Fixture::new("preserve");
    f.set(&f.dir, None);
    std::fs::remove_file(f.dir.join("target/release/.sv10-installed-commit")).unwrap();
    let refusal = "no verified rollback snapshot exists; refusing to release over an unidentified build";
    assert_eq!(run(&f, &["--preserve-unidentified"], &[]).unwrap_err().to_string(), refusal);
    f.snapshot();
    let out = run(&f, &["--preserve-unidentified"], &[]).unwrap();
    let dest = out.lines().nth(1).unwrap().strip_prefix("Preserved unidentified build at ").unwrap();
    assert_eq!(out.lines().next().unwrap(), format!("Rollback target stays verified snapshot {}", f.commit));
    assert!(Path::new(dest).join("SHA256SUMS").is_file());
    assert_eq!(std::fs::read_to_string(Path::new(dest).join("VERSIONS")).unwrap().lines().count(), 3);
    assert_eq!(std::fs::read_dir(f.dir.join("artifacts/unidentified-builds")).unwrap().count(), 1, "no staging left");
    std::fs::write(f.dir.join("target/release/.sv10-installed-commit"), "x\n").unwrap();
    assert_eq!(
        run(&f, &["--preserve-unidentified"], &[]).unwrap_err().to_string(),
        "installed release is identified; snapshot it with --snapshot instead"
    );
}

#[test]
fn a_manifest_refuses_whitespace_in_paths_and_an_empty_release() {
    let f = Fixture::new("mkmanifest");
    std::fs::create_dir_all(f.dir.join("set/target/release")).unwrap();
    std::fs::create_dir_all(f.dir.join("set/web/dist")).unwrap();
    assert_eq!(manifest::write(&f.dir.join("set")), Err(ReleaseError::ManifestEmpty));
    std::fs::write(f.dir.join("set/web/dist/a b.js"), "x").unwrap();
    assert_eq!(manifest::write(&f.dir.join("set")), Err(ReleaseError::ManifestWhitespace("web/dist/a b.js".into())));
}
