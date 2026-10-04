//! `scripts/update.sh`: fetch the update branch, fast-forward this checkout, run the release, then
//! wait for the fleet to serve the installed commit and roll back when it does not (#726).

use crate::layout::Root;
use crate::ui::Ui;
use crate::{cli, fleet, gitops, identity, journal};
use std::path::Path;
use std::process::Command;

struct Update {
    root: Root,
    ui: Ui,
    env: Vec<(String, String)>,
    remote: String,
    branch: String,
    started: bool,
}

/// The exit status of `scripts/update.sh <args>` run in the checkout at the current directory.
pub fn run(args: &[String]) -> i32 {
    let env: Vec<(String, String)> = std::env::vars().collect();
    let get = |k: &str, d: &str| env.iter().find(|(key, _)| key == k).map_or(d.to_string(), |(_, v)| v.clone());
    let root = match cli::root_from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("update: {e}");
            return 1;
        }
    };
    let (remote, branch) = (get("SVANBOT_UPDATE_REMOTE", "origin"), get("SVANBOT_UPDATE_BRANCH", "main"));
    let mut up = Update { ui: Ui::default(), remote, branch, root, env, started: false };
    if let Err(e) = std::fs::create_dir_all(up.root.join("artifacts")) {
        eprintln!("update: cannot create artifacts/: {e}");
        return 1;
    }
    let status = match args.first().map(String::as_str) {
        Some("--rollback") => up.rollback_to(args.get(1).map_or("", String::as_str)),
        Some("--check") => up.check(),
        _ => up.update(),
    };
    if up.started {
        if status != 0 {
            up.progress(&["fail-running", "", &format!("update stopped unexpectedly (exit {status}); see the release log")]);
        }
        let _ = std::fs::remove_file(up.root.join("artifacts/release.lock"));
    }
    status
}

impl Update {
    fn path(&self) -> &Path {
        self.root.path()
    }

    fn var(&self, key: &str) -> Option<&str> {
        self.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn upstream(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }

    fn progress(&self, args: &[&str]) {
        let _ = Command::new("python3").arg("scripts/progress.py").args(args).current_dir(self.path()).output();
    }

    /// Run from the checkout (not `-C`), as the script does, so what it asks git is what git is asked.
    fn git_cmd(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command.args(args).current_dir(self.path()).env("GIT_TERMINAL_PROMPT", "0");
        command
    }

    fn git(&self, args: &[&str]) -> Option<String> {
        let out = self.git_cmd(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Output, or the exit status git failed with (the script runs under `set -e`, so that is the run's status).
    fn git_code(&self, args: &[&str]) -> Result<String, i32> {
        let out = self.git_cmd(args).output().map_err(|_| 1)?;
        if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()) } else { Err(out.status.code().unwrap_or(1)) }
    }

    fn git_ok(&self, args: &[&str]) -> bool {
        self.git_cmd(args).output().is_ok_and(|o| o.status.success())
    }

    /// A git command whose own output belongs in the log; whether it succeeded.
    fn git_logged(&self, args: &[&str]) -> bool {
        self.ui.run(&mut self.git_cmd(args)) == Some(0)
    }

    fn short(&self, rev: &str) -> String {
        self.git(&["rev-parse", "--short", rev]).unwrap_or_default()
    }

    fn begin(&mut self) -> bool {
        if let Err(e) = self.ui.start_log(&self.root.join("artifacts/release.log")) {
            eprintln!("update: cannot write artifacts/release.log: {e}");
            return false;
        }
        self.started = true;
        self.progress(&["start"]);
        true
    }

    /// `git fetch` of `branch` from the remote, bounded by `SVANBOT_UPDATE_FETCH_TIMEOUT` seconds.
    fn fetch_branch(&self, branch: &str) -> bool {
        let timeout = self.var("SVANBOT_UPDATE_FETCH_TIMEOUT").filter(|v| !v.is_empty()).unwrap_or("120");
        Command::new("timeout")
            .args([timeout, "git", "fetch", "--quiet", &self.remote, branch])
            .current_dir(self.path())
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn fetch(&self) -> bool {
        let ok = self.fetch_branch(&self.branch);
        if !ok {
            self.ui.complain(&format!("update: could not fetch {} (network or credentials); nothing changed", self.upstream()));
        }
        ok
    }

    /// `scripts/rollback.sh <flags>` in this process: its lines on the stream, its refusal as the script prints it.
    fn rollback_flags(&self, flags: &[&str]) -> bool {
        let args: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
        let mut out = String::new();
        let result = cli::rollback(&self.root, &args, &self.env, &mut out);
        out.lines().for_each(|l| self.ui.say(l));
        if let Err(e) = &result {
            self.ui.complain(&format!("rollback: {e}"));
        }
        result.is_ok()
    }

    fn rollback_to(&mut self, commit: &str) -> i32 {
        if !(7..=40).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            eprintln!("update: usage: scripts/update.sh --rollback <commit>");
            return 2;
        }
        if !self.begin() {
            return 1;
        }
        self.ui.say(&format!("== restoring the saved build {commit}"));
        self.progress(&["stage", "restore"]);
        if !self.rollback_flags(&[commit]) {
            self.progress(&["fail", "restore", &format!("rollback to {commit} failed; the installed build is unchanged")]);
            return 1;
        }
        self.progress(&["installed", commit]);
        0
    }

    fn check(&mut self) -> i32 {
        if !self.fetch() {
            return 1;
        }
        let upstream = self.upstream();
        let current = self.git(&["branch", "--show-current"]).filter(|c| !c.is_empty()).unwrap_or_else(|| "detached".into());
        let count = |range: String| self.git(&["rev-list", "--count", &range]).unwrap_or_default();
        println!("{} {} {current} {}", count(format!("HEAD..{upstream}")), self.short(&upstream), count(format!("{upstream}..HEAD")));
        0
    }

    fn update(&mut self) -> i32 {
        if !self.begin() {
            return 1;
        }
        let upstream = self.upstream();
        self.ui.say(&format!("== fetching {upstream}"));
        self.progress(&["stage", "fetch"]);
        if !self.fetch() {
            self.progress(&["fail", "fetch", &format!("could not fetch {upstream}")]);
            return 1;
        }
        let (before, target) = match (self.git_code(&["rev-parse", "HEAD"]), self.git_code(&["rev-parse", &upstream])) {
            (Ok(before), Ok(target)) => (before, target),
            (Err(code), _) | (_, Err(code)) => return code,
        };
        let dirty = journal::sweep_scratch(&self.root).map(|()| gitops::dirty_inputs(self.path()));
        match dirty {
            Ok(Ok(d)) if d.is_empty() => {}
            other => {
                match other {
                    Ok(Ok(d)) => {
                        d.lines().for_each(|l| self.ui.say(l));
                        self.ui.complain("rollback: uncommitted or untracked Rust/web build inputs");
                    }
                    Ok(Err(e)) | Err(e) => self.ui.complain(&format!("rollback: {e}")),
                }
                self.progress(&["fail", "fetch", "uncommitted build inputs in the checkout; commit or discard them first"]);
                return 1;
            }
        }
        if before != target {
            if self.git_ok(&["merge-base", "--is-ancestor", &before, &target]) {
                if !self.git_logged(&["merge", "--ff-only", "--quiet", &target]) {
                    self.progress(&["fail", "fetch", "fast-forward refused; local edits are preserved, see the release log"]);
                    return 1;
                }
                let commits = self.git(&["rev-list", "--count", &format!("{before}..{target}")]).unwrap_or_default();
                self.ui.say(&format!("fast-forwarded {} -> {} ({commits} commits)", self.short(&before), self.short(&target)));
            } else if self.git_ok(&["merge-base", &before, &target]) {
                return self.ahead_updates(&before, &target);
            } else if let Some(status) = self.adopt_upstream(&before, &target) {
                return status;
            }
        }
        // Read before the install moves anything: after a hot swap the pid files briefly name a
        // process that has already exited, so the same question asked afterwards reads the fleet as down.
        let fleet_running = fleet::installed_bot_running(self.path());
        let previous = identity::installed_commit(self.path()).ok().flatten();
        if !self.release() {
            if before != target && self.git_logged(&["reset", "--quiet", "--keep", &before]) {
                self.ui.say(&format!("update: release failed; checkout restored to {}", self.short(&before)));
            }
            self.progress(&["fail", "", "release failed; the fleet keeps playing the installed build"]);
            return 1;
        }
        if fleet_running && !self.health_gate(previous.as_deref(), &before, &target) {
            return 1;
        }
        self.progress(&["installed", &self.short("HEAD")]);
        0
    }

    /// `SV10_UPDATE_RUN=1 ${SV10_RELEASE_SCRIPT:-scripts/release.sh}`; release.sh reports its own stages.
    fn release(&self) -> bool {
        let script = self.var("SV10_RELEASE_SCRIPT").filter(|s| !s.is_empty()).unwrap_or("scripts/release.sh");
        let script = if script.contains('/') { self.path().join(script) } else { script.into() };
        self.ui.run(Command::new(script).env("SV10_UPDATE_RUN", "1").current_dir(self.path())) == Some(0)
    }

    /// Installing the files is not the fleet running them: wait, bounded, for `/api/health` to report
    /// the installed commit, and restore the previous verified build when it never does. `false` when
    /// the run must end with a failure.
    fn health_gate(&self, previous: Option<&str>, before: &str, target: &str) -> bool {
        let Some(installed) = identity::installed_commit(self.path()).ok().flatten() else {
            self.ui.complain("update: warning: the release installed no commit identity; the fleet cannot be verified");
            return true;
        };
        if self.rollback_flags(&["--await-health", &installed]) {
            return true;
        }
        if !fleet::installed_bot_running(self.path()) && !fleet::supervisors_running(self.path()) {
            // The fleet stopped while the release ran: nothing is left to verify and nothing is broken.
            self.ui.complain(&format!("update: installed {installed}, but the fleet stopped during the release; the install is unverified — start the fleet and check /api/health"));
            return true;
        }
        // Name only a previous commit that can actually be restored: a markerless or adopted install has
        // a commit in releases.log but no verified snapshot.
        let target_commit =
            previous.filter(|p| cli::rollback(&self.root, &["--verify".into(), p.to_string()], &self.env, &mut String::new()).is_ok());
        let rolled_back = target_commit.is_some_and(|c| self.rollback_flags(&[c]));
        match (target_commit, rolled_back) {
            (Some(c), true) => {
                self.ui.complain(&format!("update: rolled back to the verified build {c}; the fleet keeps playing it"));
                self.progress(&[
                    "fail",
                    "install",
                    &format!("the new build did not answer /api/health with its commit; rolled back to {c}"),
                ]);
            }
            (Some(c), false) => {
                self.ui.complain(&format!("update: the automatic rollback to {c} failed; the fleet may still be on {installed}"));
                self.progress(&[
                    "fail",
                    "install",
                    &format!("the new build did not answer /api/health and the rollback failed; run scripts/rollback.sh {c} by hand"),
                ]);
            }
            (None, _) => {
                self.ui.complain(&format!("update: the new build did not answer /api/health and no verified rollback target exists; the fleet may still be on {installed}"));
                self.progress(&["fail", "install", "the new build did not answer /api/health and no verified rollback target exists; fix the build, release again, or restore a snapshot by hand"]);
            }
        }
        if rolled_back && before != target && self.git_logged(&["reset", "--quiet", "--keep", before]) {
            self.ui
                .complain(&format!("update: checkout restored to {}, so the source matches the build the fleet runs", self.short(before)));
        }
        false
    }

    /// The checkout is ahead of the update branch (#394). Commits on some remote branch mean it is simply
    /// further along another line, and nothing needs installing; commits on no remote are unfinished work.
    fn ahead_updates(&self, before: &str, target: &str) -> i32 {
        let range = format!("{target}..{before}");
        let count = |not_remotes: bool| {
            let mut args = vec!["rev-list", "--count", range.as_str()];
            args.extend(not_remotes.then_some(["--not", "--remotes"]).into_iter().flatten());
            self.git(&args).unwrap_or_default()
        };
        let ahead = count(false);
        let mut unshared = count(true);
        let current = self.git(&["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
        if unshared != "0" && !current.is_empty() && current != self.branch {
            self.fetch_branch(&current);
            unshared = count(true);
        }
        let upstream = self.upstream();
        if unshared != "0" {
            self.ui.complain(&format!("update: this checkout has commits that {upstream} does not contain; update by hand"));
            self.progress(&["fail", "fetch", &format!("local commits not on {upstream}")]);
            return 1;
        }
        self.ui.say(&format!(
            "update: this checkout is {ahead} commit(s) ahead of {upstream}; nothing to install (set SVANBOT_UPDATE_BRANCH={current} in .env to follow it here instead)"
        ));
        self.progress(&["current", &format!("this checkout is {ahead} commit(s) ahead of {upstream}; nothing to install (follow it with SVANBOT_UPDATE_BRANCH={current})")]);
        0
    }

    /// Move a checkout that shares no history with the update branch onto it, only when the operator said
    /// so ahead of time (`SVANBOT_ADOPT_UPSTREAM=1`). The old head keeps a ref of its own; a temporary
    /// `git replace --graft` makes it an ancestor so the ordinary `--ff-only` path (and its guards) does the
    /// move, then the graft is deleted. `None` when the checkout is on the branch and the update goes on.
    fn adopt_upstream(&self, before: &str, target: &str) -> Option<i32> {
        let upstream = self.upstream();
        if self.var("SVANBOT_ADOPT_UPSTREAM") != Some("1") {
            self.ui
                .complain(&format!("update: this checkout shares no history with {upstream} — a different repository, not a diverged one"));
            self.ui
                .complain(&format!("update: set SVANBOT_ADOPT_UPSTREAM=1 to move this checkout onto {upstream}; see docs/OPERATIONS.md"));
            self.progress(&[
                "fail",
                "fetch",
                &format!("unrelated history: set SVANBOT_ADOPT_UPSTREAM=1 to move this checkout onto {upstream}"),
            ]);
            return Some(1);
        }
        let stamp = Command::new("date")
            .args(["-u", "+%Y%m%dT%H%M%SZ"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let kept = format!("refs/adopt/before-upstream-{stamp}");
        self.git_ok(&["update-ref", &kept, before]);
        let root_commit =
            self.git(&["rev-list", "--max-parents=0", target]).and_then(|r| r.lines().last().map(str::to_string)).unwrap_or_default();
        let dropped = self.git(&["diff", "--name-only", "--diff-filter=D", before, target]).unwrap_or_default();
        self.ui.say(&format!("== adopting {upstream}: this checkout's history is unrelated to it"));
        self.ui.say(&format!("kept this checkout's head {before} as {kept}"));
        self.ui.say(&format!(
            "tracked files here that {upstream} does not have: {} (all recoverable from {kept})",
            dropped.lines().filter(|l| !l.is_empty()).count()
        ));
        dropped.split('\n').take(20).for_each(|l| self.ui.say(&format!("  - {l}")));
        if !self.git_logged(&["replace", "--graft", &root_commit, before]) {
            self.ui.complain(&format!("update: could not graft {root_commit} onto {before}; this checkout is unchanged"));
            self.progress(&["fail", "fetch", "adoption failed: could not relate the two histories"]);
            return Some(1);
        }
        if !self.git_logged(&["merge", "--ff-only", "--quiet", target]) {
            self.git_ok(&["replace", "-d", &root_commit]);
            self.ui.complain(&format!("update: adopting {upstream} failed; this checkout is unchanged"));
            self.ui.complain("update: commit, stash or discard the local changes git named above, then update again");
            self.progress(&["fail", "fetch", "adoption refused: local changes would be overwritten"]);
            return Some(1);
        }
        if !self.git_ok(&["replace", "-d", &root_commit]) {
            self.ui.complain(&format!("update: warning: left a replace ref on {root_commit}; history will read as joined"));
        }
        self.ui.say(&format!("adopted {upstream} at {}", self.short(target)));
        None
    }
}
