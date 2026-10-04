//! `sv10-release release` end to end in a throwaway checkout, with stub `cargo` and `npm` and stub
//! helper scripts: first install, a second release that snapshots the first, and a failing build.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn write(path: &Path, text: &str, executable: bool) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    if executable {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Checkout {
    dir: PathBuf,
    path: String,
    home: PathBuf,
}

impl Checkout {
    fn new(tag: &str) -> Checkout {
        let base = std::env::temp_dir().join(format!("sv10-release-run-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("checkout");
        let (stubs, home) = (base.join("stubs"), base.join("home"));
        std::fs::create_dir_all(&home).unwrap();
        git(&base, &["init", "-q", "-b", "main", "checkout"]);
        // The helpers a release calls, as stubs; progress.py is the real one.
        write(&dir.join("scripts/host_resources.py"), "import sys\nprint(4) if '--jobs' in sys.argv else None\n", false);
        write(&dir.join("scripts/check.sh"), "#!/bin/sh\nexit 0\n", true);
        // The tests are hermetic: they must not see the release's own lock descriptor.
        write(&dir.join("scripts/test.py"), "import os\nraise SystemExit(1 if os.environ.get('SV10_RELEASE_LOCK_FD') else 0)\n", false);
        write(&dir.join("scripts/build-lock.sh"), "#!/bin/sh\nexit 0\n", true);
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/progress.py"), dir.join("scripts/progress.py")).unwrap();
        write(&dir.join(".gitignore"), "artifacts/\ntarget/\nweb/dist/\n", false);
        write(&dir.join("web/package.json"), "{}\n", false);
        // `cargo build --release` leaves the three programs, each naming the commit it was built for.
        write(
            &stubs.join("cargo"),
            "#!/bin/sh\ncase \"$*\" in\n*build*--release*)\n[ -z \"$STUB_BUILD_FAILS\" ] || { echo 'error: could not compile' >&2; exit 101; }\nmkdir -p \"$CARGO_TARGET_DIR/release\"\nfor n in sv10-bot learner analyst; do printf '#!/bin/sh\\necho \"%s 10.0.0 %s\"\\n' \"$n\" \"$SVANBOT_COMMIT\" > \"$CARGO_TARGET_DIR/release/$n\"; chmod +x \"$CARGO_TARGET_DIR/release/$n\"; done;;\nesac\nexit 0\n",
            true,
        );
        write(
            &stubs.join("npm"),
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = --outDir ] && out=$2; shift; done\nmkdir -p \"$out\" && echo '<html></html>' > \"$out/index.html\"\n",
            true,
        );
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        let path = format!("{}:/usr/bin:/bin", stubs.display());
        Checkout { dir, path, home }
    }

    fn release(&self, extra: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sv10-release"))
            .arg("release")
            .current_dir(&self.dir)
            .env_clear()
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .envs(extra.iter().copied())
            .output()
            .unwrap()
    }

    fn commit(&self, message: &str) -> String {
        write(&self.dir.join("crates/a.rs"), message, false);
        git(&self.dir, &["add", "."]);
        git(&self.dir, &["commit", "-q", "-m", message]);
        git(&self.dir, &["rev-parse", "--short", "HEAD"])
    }

    fn progress(&self) -> String {
        std::fs::read_to_string(self.dir.join("artifacts/release-progress.json")).unwrap_or_default()
    }
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn a_release_installs_snapshots_the_previous_build_and_a_failed_build_changes_nothing() {
    let c = Checkout::new("flow");
    let first = git(&c.dir, &["rev-parse", "--short", "HEAD"]);
    let out = c.release(&[]);
    assert!(out.status.success(), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains("== first install: no prior executable or dashboard set to snapshot"), "{said}");
    assert!(said.contains(&format!("Installed {first} in ")), "{said}");
    let bot = Command::new(c.dir.join("target/release/sv10-bot")).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&bot.stdout).trim(), format!("sv10-bot 10.0.0 {first}"));
    assert!(c.dir.join("web/dist/index.html").is_file());
    assert!(std::fs::read_to_string(c.dir.join("artifacts/releases.log")).unwrap().contains(&format!(" {first} first")));
    assert!(c.progress().contains("\"installed\""), "{}", c.progress());
    assert!(std::fs::read_to_string(c.dir.join("artifacts/release.log")).unwrap().contains("== free space"));
    assert!(!c.dir.join("artifacts/release.lock").exists());

    let second = c.commit("second");
    let out = c.release(&[]);
    assert!(out.status.success(), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains(&format!("== snapshotting installed release {first}")), "{said}");
    assert!(c.dir.join("artifacts/release-snapshots").join(&first).join("SHA256SUMS").is_file());
    assert!(said.find("== free space") < said.find("== snapshotting"), "free space is checked first: {said}");
    assert!(said.contains(&format!("Installed {second} in ")), "{said}");

    // A build that fails installs nothing and says the fleet is not there.
    let third = c.commit("third");
    let out = c.release(&[("STUB_BUILD_FAILS", "1")]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains("== release-build failed (log: target/stage/release-build.log)") && said.contains("could not compile"), "{said}");
    let bot = Command::new(c.dir.join("target/release/sv10-bot")).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&bot.stdout).contains(&second), "the installed build is untouched");
    assert!(c.progress().contains("release failed (exit 1); no fleet is running"), "{}", c.progress());
    let _ = third;

    // Uncommitted build inputs refuse the release; ALLOW_DIRTY is the emergency way past.
    write(&c.dir.join("crates/dirty.rs"), "x", false);
    let out = c.release(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).contains("Commit the listed build inputs first"), "{}", text(&out));
}
