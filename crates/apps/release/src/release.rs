//! `scripts/release.sh`: snapshot what is installed, lint, test and build the checkout, build the
//! dashboard, and install the result. Running processes hot-swap to it by themselves.

use crate::layout::Root;
use crate::ui::Ui;
use crate::{cli, fleet, lock};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

/// Seconds a stage may take before it is named in the log (0334: nothing is worth more than two
/// minutes on a live running system).
const BUDGET_SECS: u64 = 120;

/// One of the parallel build jobs: where cargo builds it, whether it carries the commit id, what runs.
#[derive(Debug, PartialEq, Eq)]
pub struct Job {
    pub name: &'static str,
    pub target_dir: &'static str,
    /// Only the shipped build carries the commit id: cargo tracks `SVANBOT_COMMIT`, so exporting it to
    /// lint and the test build recompiled `sv10-bot` in both on every release (#329).
    pub with_commit: bool,
    pub argv: &'static [&'static str],
}

/// The jobs started together: lint and the test build use `target/dev` (the cache the gate keeps warm),
/// the shipped binaries build in `target/stage`, never in `target/release` (LESSONS 9).
pub fn jobs(skip_tests: bool) -> Vec<Job> {
    let mut jobs = Vec::new();
    if !skip_tests {
        jobs.push(Job { name: "lint", target_dir: "target/dev", with_commit: false, argv: &["scripts/check.sh", "lint"] });
        jobs.push(Job {
            name: "test-build",
            target_dir: "target/dev",
            with_commit: false,
            argv: &["cargo", "test", "--no-run", "--profile", "gate", "--workspace", "-q"],
        });
    }
    jobs.push(Job {
        name: "release-build",
        target_dir: "target/stage",
        with_commit: true,
        argv: &["cargo", "build", "--release", "--workspace", "--bins", "-q"],
    });
    jobs
}

/// The steps of a release in order, as the log and the progress bar name them.
pub const SEQUENCE: [&str; 8] = ["validate-layout", "check-space", "snapshot", "build", "test", "dashboard", "install", "record"];

struct Release {
    root: Root,
    ui: Ui,
    env: Vec<(String, String)>,
    own_run: bool,
    /// Another operation holds the lock: its progress card and marker are not this run's to touch.
    outranked: bool,
    web_stage: Option<PathBuf>,
}

/// The exit status of `scripts/release.sh`, run in the checkout at the current directory.
pub fn run() -> i32 {
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let mut path = std::env::var("PATH").unwrap_or_default();
    // Node lives in ~/.local/bin on the reference box, and the fleet's own PATH (which a dashboard-triggered
    // release inherits) does not include it (#696).
    for extra in [format!("{home}/.cargo/bin"), format!("{home}/.local/bin")] {
        if Path::new(&extra).is_dir() || extra.ends_with(".cargo/bin") {
            path = format!("{extra}:{path}");
        }
    }
    set(&mut env, "PATH", &path);
    let root = match cli::root_from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("rollback: {e}");
            return 1;
        }
    };
    let own_run = get(&env, "SV10_UPDATE_RUN") != Some("1");
    let mut release = Release { root, ui: Ui::default(), env, own_run, outranked: false, web_stage: None };
    if let Err(status) = release.resources() {
        return status;
    }
    let status = release.go();
    release.finish(status)
}

fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn set(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    env.retain(|(k, _)| k != key);
    env.push((key.into(), value.into()));
}

impl Release {
    fn path(&self) -> &Path {
        self.root.path()
    }

    fn progress(&self, args: &[&str]) {
        let _ = Command::new("python3").arg("scripts/progress.py").args(args).current_dir(self.path()).output();
    }

    /// `SVANBOT_BUILD_STAGES=2 source scripts/resources.sh`: the job budget shared between the two
    /// compiler stages, then the disk check.
    fn resources(&mut self) -> Result<(), i32> {
        if get(&self.env, "CARGO_BUILD_JOBS").is_none_or(str::is_empty) {
            let out =
                Command::new("python3").args(["scripts/host_resources.py", "--jobs"]).current_dir(self.path()).output().map_err(|_| 1)?;
            if !out.status.success() {
                return Err(out.status.code().unwrap_or(1));
            }
            let jobs: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().map_err(|_| 1)?;
            set(&mut self.env, "CARGO_BUILD_JOBS", &(jobs / 2).max(1).to_string());
        }
        let target = get(&self.env, "CARGO_TARGET_DIR").filter(|v| !v.is_empty()).unwrap_or("target/dev").to_string();
        let status = Command::new("python3")
            .args(["scripts/host_resources.py", "--check-disk", &target, "artifacts"])
            .current_dir(self.path())
            .status()
            .map_err(|_| 1)?;
        if status.success() { Ok(()) } else { Err(status.code().unwrap_or(1)) }
    }

    /// The release's own refusals as the script prints them; whether `flags` succeeded.
    fn rollback(&self, flags: &[&str], quiet: bool) -> Option<String> {
        let args: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
        let mut out = String::new();
        let result = cli::rollback(&self.root, &args, &self.env, &mut out);
        if !quiet {
            out.lines().for_each(|l| self.ui.say(l));
        }
        match result {
            Ok(()) => Some(out),
            Err(e) => {
                if !quiet {
                    self.ui.complain(&format!("rollback: {e}"));
                }
                None
            }
        }
    }

    /// A stage and how long it took, named when over budget.
    fn timed(&self, name: &str, stage: impl FnOnce() -> bool) -> bool {
        let started = Instant::now();
        let ok = stage();
        let took = started.elapsed().as_secs();
        self.ui.say(&format!("   {name}: {took} s"));
        self.over_budget(name, took);
        ok
    }

    fn over_budget(&self, name: &str, took: u64) {
        if took > BUDGET_SECS {
            self.ui.complain(&format!("WARNING: {name} took {took} s, over the {BUDGET_SECS} s budget (0334)"));
        }
    }

    /// Every thread at idle CPU priority (0302): the fleet always goes first.
    fn idle(&self, argv: &[&str]) -> Command {
        let has_chrt = get(&self.env, "PATH").is_some_and(|p| p.split(':').any(|d| Path::new(d).join("chrt").is_file()));
        let mut command = Command::new("nice");
        command.args(["-n", "19"]);
        if has_chrt {
            command.args(["chrt", "-i", "0"]);
        }
        command.args(argv);
        command
    }

    fn command(&self, mut command: Command) -> Command {
        // The lock descriptor is this process's own: a child that inherited the name but not the file
        // (the tests, hermetic by design) would find a lock it does not hold and refuse.
        command.current_dir(self.path()).env_clear().envs(self.env.iter().map(|(k, v)| (k, v))).env_remove("SV10_RELEASE_LOCK_FD");
        command
    }

    fn start(&self, job: &Job, commit: &str, stage: &Path) -> std::io::Result<Child> {
        let log = std::fs::File::create(stage.join(format!("{}.log", job.name)))?;
        let mut command = self.command(self.idle(job.argv));
        command.env("CARGO_TARGET_DIR", job.target_dir);
        if job.with_commit {
            command.env("SVANBOT_COMMIT", commit);
        } else {
            command.env_remove("SVANBOT_COMMIT");
        }
        command.stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log)).spawn()
    }

    /// Everything from the layout check to the success line; the exit status.
    fn go(&mut self) -> i32 {
        let _ = std::fs::create_dir_all(self.root.join("artifacts"));
        // Asked before the progress card is touched: a run that lost the lock further down had
        // already restarted the card of the run that holds it, and then marked that run failed and
        // deleted its marker on the way out (#879).
        if lock::busy(&self.root) {
            self.outranked = true;
            self.ui.complain("Another release, snapshot, or rollback operation is active.");
            return 1;
        }
        if self.own_run {
            self.progress(&["start"]);
        }
        if !self.rollback(&["--validate-layout"], false).is_some() {
            return 1;
        }
        let held = match lock::acquire(&self.root, None) {
            Ok(held) => held,
            Err(_) => {
                self.ui.complain("Another release, snapshot, or rollback operation is active.");
                return 1;
            }
        };
        if let Some(fd) = held.fd() {
            // The steps below run in this process: they see the lock as inherited, like `rollback.sh` did.
            set(&mut self.env, "SV10_RELEASE_LOCK_FD", &fd.to_string());
        }
        if self.own_run && self.ui.start_log(&self.root.join("artifacts/release.log")).is_err() {
            return 1;
        }
        self.sequence()
    }

    fn sequence(&mut self) -> i32 {
        let started = Instant::now();
        let flag = |env: &[(String, String)], k: &str| get(env, k) == Some("1");
        let (allow_dirty, adopt, skip_tests) =
            (flag(&self.env, "ALLOW_DIRTY"), flag(&self.env, "RELEASE_ADOPT_UNIDENTIFIED"), flag(&self.env, "SKIP_TESTS"));
        if !allow_dirty && self.rollback(&["--validate-source-clean"], false).is_none() {
            self.ui.complain("Commit the listed build inputs first (or use ALLOW_DIRTY=1 only for an emergency).");
            return 1;
        }
        let Some(commit) = crate::gitops::git(self.path(), &["rev-parse", "--short=7", "HEAD"]) else { return 1 };
        set(&mut self.env, "SVANBOT_COMMIT", &commit);
        // Free space first: the snapshot copies the installed sets, the build stages a whole release, and a
        // root that ran out half-way leaves junk or fails late (LESSONS 22). Fail closed before anything is written.
        self.ui.say("== free space");
        if self.rollback(&["--check-space"], false).is_none() {
            return 1;
        }
        self.progress(&["stage", "snapshot"]);
        if !self.snapshot_installed(adopt) {
            return 1;
        }
        self.ui.say(&format!("== checking and building {commit} (lint, tests and release in parallel)"));
        self.progress(&["stage", "build"]);
        let stage = self.root.join("target/stage");
        if std::fs::create_dir_all(&stage).and(std::fs::create_dir_all(self.root.join("target/dev"))).is_err() {
            return 1;
        }
        if !self.build(&commit, &stage, skip_tests) {
            return 1;
        }
        if !skip_tests {
            self.ui.say("== testing");
            self.progress(&["stage", "test"]);
            let threads = get(&self.env, "RUST_TEST_THREADS").unwrap_or("2").to_string();
            let ok = self.timed("tests", || {
                let mut command = self.command(self.idle(&["python3", "scripts/test.py", "--profile", "gate", "-q"]));
                command.env("CARGO_TARGET_DIR", "target/dev").env("RUST_TEST_THREADS", threads).env_remove("SVANBOT_COMMIT");
                self.ui.run(&mut command) == Some(0)
            });
            if !ok {
                return 1;
            }
        }
        for name in crate::binaries::REQUIRED {
            let said = Command::new(stage.join("release").join(name))
                .arg("--version")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
            if !said.is_some_and(|s| s.lines().any(|l| l.starts_with(&format!("{name} ")))) {
                self.ui.complain(&format!("{name} --version failed; not installing"));
                return 1;
            }
        }
        self.ui.say("== dashboard");
        self.progress(&["stage", "dashboard"]);
        let Some(web_stage) = make_web_stage(self.path(), &commit) else { return 1 };
        self.web_stage = Some(web_stage.clone());
        let built = self.timed("dashboard", || {
            let mut command = self.command(Command::new("npm"));
            command.args(["run", "build", "--", "--outDir"]).arg(&web_stage).arg("--emptyOutDir").current_dir(self.root.join("web"));
            command.stdout(Stdio::null()).stderr(Stdio::inherit()).status().is_ok_and(|s| s.success())
        });
        if !built {
            return 1;
        }
        self.ui.say("== installing");
        self.progress(&["stage", "install"]);
        let release_dir = stage.join("release");
        let (release_dir, web_dir) = (release_dir.to_string_lossy().into_owned(), web_stage.to_string_lossy().into_owned());
        if !self.timed("install", || self.rollback(&["--install", &release_dir, &web_dir, &commit], false).is_some()) {
            return 1;
        }
        let took = started.elapsed().as_secs();
        // The new build is live from here: a failure in the bookkeeping below is not a failed release (#726).
        if let Some(dir) = self.web_stage.take()
            && std::fs::remove_dir_all(&dir).is_err()
        {
            self.ui.complain(&format!("warning: could not remove the dashboard staging {}", dir.display()));
        }
        self.record(&commit);
        self.ui.say(&format!(
            "Installed {commit} in {took} s. The fleet swaps within ~1 minute, the learner after its current step (watch artifacts/logs/svanbot10.log for 'hot swap')."
        ));
        self.over_budget("release", took);
        if self.own_run {
            self.progress(&["installed", &commit]);
        }
        0
    }

    /// Snapshot the installed release before lint or build can replace it. A partial or unidentified existing
    /// installation fails closed; only a true first install may proceed without a recovery point.
    fn snapshot_installed(&self, adopt_unidentified: bool) -> bool {
        if !self.root.join("target/release").is_dir() && !self.root.join("web/dist").is_dir() {
            self.ui.say("== first install: no prior executable or dashboard set to snapshot");
            return true;
        }
        let has_marker = self.root.join("target/release").join(crate::identity::MARKER).is_file();
        if !has_marker && adopt_unidentified {
            self.ui.say("== preserving the unidentified installed build (RELEASE_ADOPT_UNIDENTIFIED=1)");
            return self.rollback(&["--preserve-unidentified"], false).is_some();
        }
        let Some(installed) = self.rollback(&["--installed-commit"], true).map(|o| o.trim().to_string()) else {
            self.ui.complain("Existing installation has no verifiable commit identity; refusing to overwrite it.");
            return false;
        };
        if !has_marker {
            self.ui.say(&format!("== verifying one-time legacy installed identity {installed}"));
            if self.rollback(&["--adopt-legacy", &installed], false).is_none() {
                self.ui.complain("If the installed build was made outside release.sh (e.g. '<name> <version> dev'), rerun with");
                self.ui.complain("RELEASE_ADOPT_UNIDENTIFIED=1 to keep a verified copy aside and release over it.");
                return false;
            }
        }
        self.ui.say(&format!("== snapshotting installed release {installed}"));
        self.rollback(&["--snapshot", &installed], false).is_some()
    }

    /// Lint, the test build and the release build run at once (0302); each job writes its own log, and a
    /// failure prints its tail.
    fn build(&self, commit: &str, stage: &Path, skip_tests: bool) -> bool {
        let plan = jobs(skip_tests);
        let mut running: Vec<(&Job, std::thread::JoinHandle<(bool, u64)>)> = Vec::new();
        for job in &plan {
            if job.name == "release-build" {
                // Another cargo in `target/stage` holds cargo's build lock and the job then reports that wait
                // as build time: name the wait in the log the dashboard tails (#363).
                let mut probe = self.command(Command::new("scripts/build-lock.sh"));
                self.ui.run(probe.arg("target/stage/release"));
            }
            // Each job is timed by its own waiter, so a job that ends early is not charged for the ones before it.
            let (started, child) = (Instant::now(), self.start(job, commit, stage));
            running.push((
                job,
                std::thread::spawn(move || {
                    let ok = child.and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                    (ok, started.elapsed().as_secs())
                }),
            ));
        }
        let mut failed = false;
        for (job, waiter) in running {
            let (ok, secs) = waiter.join().unwrap_or((false, 0));
            let log = stage.join(format!("{}.log", job.name));
            if ok {
                self.ui.say(&format!("   {} ok ({secs} s)", job.name));
                self.over_budget(job.name, secs);
                if secs > BUDGET_SECS {
                    let kept = stage.join(format!("{}.over-budget.log", job.name));
                    let _ = std::fs::copy(&log, &kept);
                    self.ui.complain(&format!("   {}: over budget, log kept at target/stage/{}.over-budget.log", job.name, job.name));
                }
            } else {
                self.ui.say(&format!("== {} failed (log: target/stage/{}.log)", job.name, job.name));
                let text = std::fs::read_to_string(&log).unwrap_or_default();
                let lines: Vec<&str> = text.lines().collect();
                lines[lines.len().saturating_sub(30)..].iter().for_each(|l| self.ui.say(l));
                failed = true;
            }
        }
        !failed
    }

    /// `artifacts/releases.log`: when, which commit, and its subject.
    fn record(&self, commit: &str) {
        use std::io::Write;
        let subject = crate::gitops::git(self.path(), &["log", "-1", "--format=%s", commit]).unwrap_or_default();
        let subject: String = subject.chars().take(120).collect();
        let now = Command::new("date")
            .arg("+%FT%T%:z")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let logged = std::fs::create_dir_all(self.root.join("artifacts"))
            .and_then(|()| std::fs::OpenOptions::new().append(true).create(true).open(self.root.join("artifacts/releases.log")))
            .and_then(|mut f| writeln!(f, "{now} {commit} {subject}"));
        if logged.is_err() {
            self.ui.complain("warning: could not append to artifacts/releases.log");
        }
    }

    /// What the script's exit trap does.
    fn finish(&mut self, status: i32) -> i32 {
        if let Some(dir) = self.web_stage.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
        if self.own_run && !self.outranked {
            let _ = std::fs::remove_file(self.root.join("artifacts/release.lock"));
            if status != 0 {
                // Nothing plays when start.sh was waiting on this release (#790): do not claim a fleet that is not there.
                let state = if fleet::playing(self.path()) {
                    "the fleet keeps playing the installed build"
                } else {
                    "no fleet is running; scripts/start.sh starts what is installed"
                };
                self.progress(&["fail", "", &format!("release failed (exit {status}); {state}")]);
            }
        }
        status
    }
}

fn make_web_stage(root: &Path, commit: &str) -> Option<PathBuf> {
    crate::fsops::make_temp_dir(&root.join("target"), &format!(".web-stage-{commit}.")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_shipped_build_carries_the_commit_and_nothing_builds_into_target_release() {
        for skip in [false, true] {
            let plan = jobs(skip);
            assert!(plan.iter().all(|j| j.with_commit == (j.name == "release-build")), "{plan:?}");
            assert!(plan.iter().all(|j| j.target_dir != "target/release"), "{plan:?}");
            assert_eq!(plan.len(), if skip { 1 } else { 3 });
        }
        assert_eq!(jobs(false)[0].argv, ["scripts/check.sh", "lint"]);
    }

    #[test]
    fn free_space_is_checked_before_anything_is_snapshotted() {
        let at = |step: &str| SEQUENCE.iter().position(|s| *s == step).unwrap();
        assert!(at("check-space") < at("snapshot") && at("snapshot") < at("build") && at("build") < at("install"));
    }
}
