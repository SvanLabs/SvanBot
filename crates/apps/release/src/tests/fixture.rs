//! A throwaway release root: a git repository with one commit and, on demand, a snapshot.

use crate::layout::Root;
use crate::manifest::hash_file;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Fixture {
    pub dir: PathBuf,
    pub root: Root,
    pub commit: String,
}

pub fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

impl Fixture {
    pub fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("sv10-release-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README"), "x").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);
        let out = Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "--short=7", "HEAD"]).output().unwrap();
        let commit = String::from_utf8(out.stdout).unwrap().trim().to_string();
        let home = std::env::temp_dir().join(format!("sv10-release-home-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let root = Root::open(&dir, Some(&home)).expect("the fixture is a valid root");
        Fixture { dir, root, commit }
    }

    /// A fake executable that answers `--version` with `line`.
    pub fn program(path: &Path, line: &str) {
        std::fs::write(path, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A complete set (the three programs, a marker, a dashboard file) under `base`.
    pub fn set(&self, base: &Path, build: Option<&str>) {
        let bin = base.join("target/release");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(base.join("web/dist")).unwrap();
        for name in crate::binaries::REQUIRED {
            let line = match build {
                Some(b) => format!("{name} 10.0.1 {b}"),
                None => format!("{name} 10.0.1"),
            };
            Fixture::program(&bin.join(name), &line);
        }
        std::fs::write(bin.join(".sv10-installed-commit"), format!("{}\n", self.commit)).unwrap();
        std::fs::write(base.join("web/dist/index.html"), "<html></html>").unwrap();
    }

    /// `SHA256SUMS` for the two trees of `base`, in `sha256sum`'s format.
    pub fn manifest(base: &Path) -> String {
        let paths = crate::manifest::actual_paths(base).unwrap();
        paths.iter().map(|p| format!("{}  {p}\n", hash_file(&base.join(p)).unwrap())).collect()
    }

    /// A verified-looking snapshot of the commit under `artifacts/release-snapshots/`.
    pub fn snapshot(&self) -> PathBuf {
        let snap = self.dir.join("artifacts/release-snapshots").join(&self.commit);
        self.set(&snap, Some(&self.commit));
        std::fs::write(snap.join("SHA256SUMS"), Fixture::manifest(&snap)).unwrap();
        snap
    }
}
