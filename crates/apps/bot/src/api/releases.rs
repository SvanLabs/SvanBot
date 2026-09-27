//! Dashboard releases: which commit is installed, what an update would bring, and operator-approved
//! one-click updates with a progress bar (0236). The changelog is generated live from
//! `git log <installed>..<update branch>`, grouped by our commit-prefix convention — no committed
//! CHANGELOG.md to drift, no new dependency.
//!
//! One click runs `scripts/update.sh` detached, in its own process group (the fleet head exits
//! during the hot swap; the update must outlive it): fetch the update branch from GitHub,
//! fast-forward, `scripts/release.sh` (build, test, install), and the fleet hot-swaps between
//! turns. Progress comes from `artifacts/release-progress.json` (`scripts/progress.py`) weighted by
//! the last run's stage times; the swap is confirmed by each process reporting the new build.

use super::*;

/// Group a commit subject by its leading prefix (`Deepen sv10-store: …` → Refactors), including
/// conventional-commit prefixes (`feat:`, `perf:`, `test:`, `chore:`, `refactor(x):`).
pub fn changelog_group(subject: &str) -> &'static str {
    let token: String = subject.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_ascii_lowercase();
    match token.as_str() {
        "deepen" | "collapse" | "split" | "unify" | "move" | "refactor" => "Refactors",
        "fix" => "Fixes",
        "add" | "feat" => "Features",
        "perf" => "Performance",
        "test" | "tests" => "Tests",
        "remove" | "chore" => "Cleanup",
        "merge" => "Merges",
        "map" => "Planning",
        "update" | "bump" | "release" => "Updates",
        "doc" | "docs" => "Docs",
        _ => "Other",
    }
}

/// Parse one `artifacts/releases.log` line (`<date> <commit> <subject>`) into (commit, subject).
pub fn parse_release_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    parts.next()?;
    let commit = parts.next()?.to_string();
    let subject: String = parts.collect::<Vec<_>>().join(" ");
    if commit.is_empty() || subject.is_empty() { None } else { Some((commit, subject)) }
}

/// Installed commit and when, from the newest `releases.log` entry.
fn installed(artifacts: &std::path::Path) -> Option<(String, String, String)> {
    let text = std::fs::read_to_string(artifacts.join("releases.log")).ok()?;
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    let (commit, subject) = parse_release_line(line)?;
    let at = line.split_whitespace().next().unwrap_or("").to_string();
    Some((commit, at, subject))
}

/// Run git in the repo root; `None` when git fails or is missing.
fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// (short hash, subject) of local `main` HEAD.
fn head_commit(root: &std::path::Path) -> Option<(String, String)> {
    let line = git(root, &["log", "-1", "--format=%h %s"])?;
    let line = line.trim_end();
    let mut parts = line.splitn(2, ' ');
    Some((parts.next()?.to_string(), parts.next().unwrap_or("").to_string()))
}

/// (full hash, subject) of every commit in `base..target`, newest first.
fn commit_range_to(root: &std::path::Path, base: &str, target: &str) -> Vec<(String, String)> {
    let range = format!("{base}..{target}");
    let Some(out) = git(root, &["log", "--format=%H %s", &range]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, ' ');
            Some((parts.next()?.to_string(), parts.next().unwrap_or("").to_string()))
        })
        .collect()
}

/// The branch and remote updates come from (`SVANBOT_UPDATE_BRANCH`, `SVANBOT_UPDATE_REMOTE`).
fn update_source() -> (String, String) {
    let var = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
    (var("SVANBOT_UPDATE_REMOTE", "origin"), var("SVANBOT_UPDATE_BRANCH", "main"))
}

/// The last `scripts/update.sh --check`: what the update branch holds that this checkout lacks.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct UpdateCheck {
    /// `remote/branch` checked.
    pub source: String,
    /// Short commit at the tip of the update branch, when the fetch worked.
    pub commit: Option<String>,
    /// Commits on the update branch not yet in this checkout.
    pub behind: u64,
    /// When the check ran (seconds since the epoch).
    pub checked_at: f64,
    /// Why the check failed (network, credentials), if it did.
    pub error: Option<String>,
}

/// Parse `update.sh --check` output: `<behind> <commit>`.
pub fn parse_check(stdout: &str) -> Option<(u64, String)> {
    let mut parts = stdout.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let commit = parts.next()?.to_string();
    Some((behind, commit))
}

fn check_path(artifacts: &std::path::Path) -> std::path::PathBuf {
    artifacts.join("update-check.json")
}

/// The stored result of the last check, if any.
pub fn last_check(artifacts: &std::path::Path) -> Option<UpdateCheck> {
    std::fs::read_to_string(check_path(artifacts)).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Fetch the update branch (`scripts/update.sh --check`, blocking; its fetch times out) and store
/// what it holds. Never retried in a loop: the caller runs it on a timer or on a click (LESSONS 30).
pub fn run_update_check(root: &std::path::Path, artifacts: &std::path::Path) -> UpdateCheck {
    let (remote, branch) = update_source();
    let mut check = UpdateCheck { source: format!("{remote}/{branch}"), checked_at: now_secs(), ..UpdateCheck::default() };
    let out = std::process::Command::new("bash")
        .arg("scripts/update.sh")
        .arg("--check")
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => match parse_check(&String::from_utf8_lossy(&o.stdout)) {
            Some((behind, commit)) => {
                check.behind = behind;
                check.commit = Some(commit);
            }
            None => check.error = Some("unexpected check output".into()),
        },
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            check.error =
                Some(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("fetch failed").trim().chars().take(200).collect());
        }
        Err(e) => check.error = Some(format!("could not run scripts/update.sh: {e}")),
    }
    let tmp = check_path(artifacts).with_extension("tmp");
    if let Ok(j) = serde_json::to_string(&check)
        && std::fs::write(&tmp, j).is_ok()
    {
        let _ = std::fs::rename(&tmp, check_path(artifacts));
    }
    check
}

/// Whether a release build is currently running (lock file, fresh enough to trust).
fn update_running(artifacts: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(artifacts.join("release.lock")) else { return false };
    meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() < 2 * 3600)
}

pub(super) async fn releases(State(s): State<Arc<Shared>>) -> Response {
    let result = off_runtime({
        let s = s.clone();
        move || {
            let root = &s.config.root;
            let installed = installed(&s.config.artifacts);
            let head = head_commit(root);
            let dirty = source_dirty(root);
            // What an update would install: up to the fetched update branch when it is known
            // locally (the checker fetched it), else up to this checkout's HEAD.
            let (remote, branch) = update_source();
            let tip = format!("{remote}/{branch}");
            let target = if git(root, &["rev-parse", "--verify", "--quiet", &tip]).is_some() { tip } else { "HEAD".to_string() };
            let (behind, changelog) = match &installed {
                Some((base, _, _)) => {
                    let range = commit_range_to(root, base, &target);
                    let log = range
                        .iter()
                        .map(|(c, subject)| {
                            json!({"commit": c.chars().take(7).collect::<String>(), "subject": subject, "group": changelog_group(subject)})
                        })
                        .collect();
                    (range.len(), log)
                }
                _ => (0, Vec::new()),
            };
            (installed, head, behind, dirty, changelog, last_check(&s.config.artifacts))
        }
    })
    .await;
    let (installed, head, behind, dirty, changelog, check) = match result {
        Ok(r) => r,
        Err(r) => return r.into_response(),
    };
    let (installed_commit, installed_at, installed_subject) =
        installed.map(|(c, a, s)| (json!(c), json!(a), json!(s))).unwrap_or((Value::Null, Value::Null, Value::Null));
    let (head_commit, head_subject) = head.map(|(c, s)| (json!(c), json!(s))).unwrap_or((Value::Null, Value::Null));
    Json(json!({
        "installed": {"commit": installed_commit, "at": installed_at, "subject": installed_subject},
        "head": {"commit": head_commit, "subject": head_subject},
        "behind": behind,
        "dirty": dirty,
        "update_available": behind > 0 || check.as_ref().is_some_and(|c| c.behind > 0),
        "build": {"commit": crate::BUILD_COMMIT, "version": crate::VERSION},
        "changelog": changelog,
        "remote": check,
    }))
    .into_response()
}

/// Fetch the update branch now and report (`POST /api/releases/check`).
pub(super) async fn check_updates(State(s): State<Arc<Shared>>) -> Response {
    let (root, artifacts) = (s.config.root.clone(), s.config.artifacts.clone());
    match tokio::task::spawn_blocking(move || run_update_check(&root, &artifacts)).await {
        Ok(c) => Json(json!(c)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("update check failed: {e}")}))).into_response(),
    }
}

/// Uncommitted or untracked build inputs, by the same definition `scripts/release.sh` refuses on.
fn source_dirty(root: &std::path::Path) -> bool {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            "crates",
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            ".cargo",
            "rust-toolchain",
            "rust-toolchain.toml",
            "web",
        ],
    )
    .is_some_and(|o| !o.trim().is_empty())
}

/// Default seconds per stage before a run has been timed (the i7-4770K, 2026-09). An update runs
/// fetch..install (a release by hand starts at snapshot); a rollback is the single stage `restore`.
/// `build` is lint, the test build and the release build side by side, then `test` runs the suite
/// (0302); runs recorded before that had separate `lint`, `test` and `build` stages.
const DEFAULT_STAGE_SECS: [(&str, f64); 7] =
    [("fetch", 5.0), ("snapshot", 15.0), ("build", 90.0), ("test", 10.0), ("dashboard", 40.0), ("install", 10.0), ("restore", 20.0)];

/// The progress bar's view of a run: every stage with its state and expected time (the last
/// successful run's, else a default), the share done weighted by those times, and the time left.
pub fn progress_view(progress: &Value, timings: &Value, now: f64) -> Value {
    let expected = |name: &str| {
        timings[name].as_f64().filter(|v| *v > 0.0).unwrap_or_else(|| DEFAULT_STAGE_SECS.iter().find(|s| s.0 == name).map_or(30.0, |s| s.1))
    };
    let recorded: Vec<&Value> = progress["stages"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    // A release by hand has no fetch stage: the plan starts at the first recorded stage.
    let first = recorded.first().and_then(|s| s["name"].as_str()).unwrap_or("fetch");
    let start = DEFAULT_STAGE_SECS.iter().position(|s| s.0 == first).unwrap_or(0);
    let plan: &[(&str, f64)] = if first == "restore" { &DEFAULT_STAGE_SECS[6..] } else { &DEFAULT_STAGE_SECS[start.min(5)..6] };
    let state = progress["state"].as_str().unwrap_or("idle");
    let mut stages = Vec::new();
    let (mut total, mut done, mut left) = (0.0, 0.0, 0.0);
    for (name, _) in plan {
        let exp = expected(name);
        total += exp;
        let rec = recorded.iter().rev().find(|s| s["name"] == *name);
        let st = rec.and_then(|r| r["state"].as_str()).unwrap_or(if state == "installed" { "done" } else { "pending" });
        let seconds = match (rec, st) {
            (Some(r), "running") => (now - r["started"].as_f64().unwrap_or(now)).max(0.0),
            (Some(r), _) => r["seconds"].as_f64().unwrap_or(0.0),
            _ => 0.0,
        };
        match st {
            "done" => done += exp,
            "running" => {
                done += exp * (seconds / exp).min(0.95);
                left += (exp - seconds).max(exp * 0.05);
            }
            "pending" => left += exp,
            _ => {}
        }
        stages.push(json!({"name": name, "state": st, "seconds": (seconds * 10.0).round() / 10.0, "expected": exp}));
    }
    let percent = match state {
        "installed" => 100.0,
        "idle" => 0.0,
        _ if total > 0.0 => (done / total * 100.0).clamp(0.0, 99.0),
        _ => 0.0,
    };
    let started = progress["started"].as_f64();
    json!({
        "state": state,
        "stages": stages,
        "percent": (percent * 10.0f64).round() / 10.0,
        "elapsed": started.map(|t| ((now - t).max(0.0) * 10.0).round() / 10.0),
        "eta": (state == "running").then(|| left.round()),
        "from": progress["from"],
        "commit": progress["commit"],
        "message": progress["message"],
    })
}

/// Saved release snapshots a rollback can restore (`artifacts/release-snapshots/<commit>`), newest
/// first, with the subject `releases.log` recorded for each and whether it is the installed one.
fn snapshots(artifacts: &std::path::Path) -> Vec<Value> {
    let installed = installed(artifacts).map(|(c, _, _)| c);
    let store_format = sv10_store::packed::data_format(artifacts);
    let root = artifacts.parent().unwrap_or(artifacts);
    let log = std::fs::read_to_string(artifacts.join("releases.log")).unwrap_or_default();
    let mut out: Vec<(std::time::SystemTime, Value)> = std::fs::read_dir(artifacts.join("release-snapshots"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let commit = e.file_name().to_str()?.to_string();
            if !valid_commit(&commit) {
                return None;
            }
            let at = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            let line = log.lines().rev().find(|l| parse_release_line(l).is_some_and(|(c, _)| c == commit));
            let subject = line.and_then(parse_release_line).map(|(_, s)| s);
            let installed_at = line.and_then(|l| l.split_whitespace().next()).map(String::from);
            // A build that cannot read the stored data is listed but refused (0229; rollback.sh checks too).
            let readable = build_data_format(root, &commit) >= store_format;
            Some((
                at,
                json!({"commit": commit, "subject": subject, "installed_at": installed_at, "current": installed.as_deref() == Some(commit.as_str()), "readable": readable}),
            ))
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// The data format a build reads: the `DATA_FORMAT` line of its store codec, 1 before the codec
/// existed (the same rule as `scripts/rollback.sh`).
fn build_data_format(root: &std::path::Path, commit: &str) -> u32 {
    let out =
        std::process::Command::new("git").arg("-C").arg(root).args(["show", &format!("{commit}:crates/libs/store/src/packed.rs")]).output();
    let source = out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    source.lines().find_map(|l| l.strip_prefix("pub const DATA_FORMAT: u32 = ")?.strip_suffix(';')?.parse().ok()).unwrap_or(1)
}

/// A short or full lowercase hex commit id (what snapshot directories are named by).
fn valid_commit(c: &str) -> bool {
    (7..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Saved builds a rollback can restore (`GET /api/releases/snapshots`, 0239).
pub(super) async fn release_snapshots(State(s): State<Arc<Shared>>) -> Json<Value> {
    Json(json!({"snapshots": snapshots(&s.config.artifacts), "running": update_running(&s.config.artifacts)}))
}

/// Restore a saved build (`POST /api/releases/rollback {commit}`, 0239): `scripts/update.sh --rollback`
/// detached, with the same lock and progress as an update; the fleet hot-swaps to it.
pub(super) async fn trigger_rollback(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let commit = body["commit"].as_str().unwrap_or("").to_string();
    let saved = snapshots(&s.config.artifacts);
    let Some(build) = saved.iter().find(|v| v["commit"] == commit.as_str() && v["current"] != true) else {
        return (StatusCode::BAD_REQUEST, Json(json!({"detail": "no saved build with that commit to roll back to"}))).into_response();
    };
    if build["readable"] == false {
        return (
            StatusCode::CONFLICT,
            Json(json!({"detail": "that build cannot read the compressed databases: stop the fleet, run ./target/release/archive unpack, then roll back"})),
        )
            .into_response();
    }
    spawn_update(&s, &["--rollback", &commit], &format!("rollback to {commit} started from the dashboard (play continues)"))
}

/// Update progress (`GET /api/releases/progress`): the stage view, whether each process runs the new
/// build yet, the bots still playing (updates never stop play) and the log tail.
/// The build each worker process reports, for the swap line. Only a split fleet's head has workers:
/// a single process reads no heartbeats, or keys left from an earlier split fleet show every bot as
/// a worker still on an old build ("workers 0/5", 2026-09-27).
fn worker_commits(s: &Shared) -> Vec<Value> {
    if !s.config.head {
        return Vec::new();
    }
    s.config
        .bots
        .iter()
        .filter_map(|b| {
            let v: Value = serde_json::from_str(&s.store.get_kv(&crate::live::heartbeat_key(&b.name)).ok()??).ok()?;
            Some(json!({"bot": b.name, "commit": crate::live::heartbeat_commit(&v)}))
        })
        .collect()
}

pub(super) async fn release_progress(State(s): State<Arc<Shared>>) -> Json<Value> {
    let artifacts = s.config.artifacts.clone();
    let read = |name: &str| std::fs::read_to_string(artifacts.join(name)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let progress = read("release-progress.json").unwrap_or(Value::Null);
    let timings = read("release-timings.json").unwrap_or(Value::Null);
    let mut view = progress_view(&progress, &timings, now_secs());
    let target = progress["commit"].as_str().map(String::from);
    let kv_commit = |key: &str| {
        s.store
            .get_kv(key)
            .ok()
            .flatten()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["commit"].as_str().map(String::from))
    };
    let workers = worker_commits(&s);
    let playing = s.bots.iter().filter(|b| b.read().mode == "playing").count();
    let short = |c: &str| c.chars().take(7).collect::<String>();
    view["swap"] = json!({
        "target": target,
        "fleet": crate::BUILD_COMMIT,
        "fleet_done": target.as_deref().is_some_and(|t| short(t) == short(crate::BUILD_COMMIT)),
        "learner": kv_commit(crate::LEARNER_STATUS_KEY),
        "analyst": kv_commit(crate::ANALYST_STATUS_KEY),
        "workers": workers,
    });
    view["bots_playing"] = json!(playing);
    view["bots_total"] = json!(s.bots.len());
    view["running"] = json!(update_running(&artifacts));
    view["log"] = json!(tail_lines(&artifacts.join("release.log"), 16 * 1024).into_iter().rev().take(40).rev().collect::<Vec<_>>());
    Json(view)
}

pub(super) async fn release_log(State(s): State<Arc<Shared>>) -> Json<Value> {
    Json(json!({
        "running": update_running(&s.config.artifacts),
        "log": tail_lines(&s.config.artifacts.join("release.log"), 64 * 1024),
    }))
}

/// Start `scripts/update.sh` detached (fetch, fast-forward, release; output to
/// `artifacts/release.log`, progress to `artifacts/release-progress.json`); the fleet hot-swaps
/// itself when the build installs. Refuses uncommitted build inputs or a concurrent run.
pub(super) async fn trigger_update(State(s): State<Arc<Shared>>) -> Response {
    if update_running(&s.config.artifacts) {
        return (StatusCode::CONFLICT, Json(json!({"detail": "an update is already running"}))).into_response();
    }
    let dirty = tokio::task::spawn_blocking({
        let s = s.clone();
        move || source_dirty(&s.config.root)
    })
    .await
    .unwrap_or(false);
    if dirty {
        return (
            StatusCode::CONFLICT,
            Json(json!({"detail": "the checkout has uncommitted build inputs; commit or discard them first (or run scripts/release.sh by hand)"})),
        )
            .into_response();
    }
    spawn_update(&s, &[], "update started from the dashboard: fetch, build, test, install, hot swap (play continues)")
}

/// Take the update lock and start `scripts/update.sh ARGS` detached in its own process group: the
/// run outlives this request and the fleet head (which exits during the hot swap). update.sh removes
/// the lock however it ends; a killed machine can only leave a lock that ages out after 2 hours.
fn spawn_update(s: &Arc<Shared>, args: &[&str], started: &str) -> Response {
    if update_running(&s.config.artifacts) {
        return (StatusCode::CONFLICT, Json(json!({"detail": "an update or rollback is already running"}))).into_response();
    }
    let lock = s.config.artifacts.join("release.lock");
    if std::fs::write(&lock, serde_json::json!({"started": now_secs()}).to_string()).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": "could not take the update lock"}))).into_response();
    }
    let log = s.config.artifacts.join("release.log");
    let log_file = std::fs::OpenOptions::new().create(true).append(true).open(&log);
    let mut cmd = std::process::Command::new("bash");
    cmd.arg("scripts/update.sh").args(args).current_dir(&s.config.root).stdin(std::process::Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    match log_file {
        Ok(f) => {
            cmd.stdout(f.try_clone().unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()));
            cmd.stderr(f);
        }
        Err(_) => {
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(_) => {
            s.log("fleet", "info", started);
            Json(json!({"started": true})).into_response()
        }
        Err(e) => {
            let _ = std::fs::remove_file(&lock);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not start scripts/update.sh: {e}")}))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_single_process_fleet_lists_no_workers() {
        let shared = Shared::for_test("release-workers", &["A", "B"]);
        let old = json!({"hb": 1.0, "state": {}, "commit": "0000000"}).to_string();
        shared.store.put_kv(&crate::live::heartbeat_key("A"), &old).unwrap();
        assert!(worker_commits(&shared).is_empty(), "stale split-fleet heartbeats are not workers");
    }
    use super::*;

    #[test]
    fn subjects_group_by_their_leading_prefix() {
        assert_eq!(changelog_group("Deepen sv10-store: per-domain seams"), "Refactors");
        assert_eq!(changelog_group("Map: big-spot review (0120)"), "Planning");
        assert_eq!(changelog_group("Fix two fuzzer-caught panics"), "Fixes");
        assert_eq!(changelog_group("Collapse re-export fans"), "Refactors");
        assert_eq!(changelog_group("Add analyst status page"), "Features");
        assert_eq!(changelog_group("docs: refresh operations"), "Docs");
        assert_eq!(changelog_group("9d42002 Showdown slams tab"), "Other");
        // Conventional-commit prefixes, which most recent commits use.
        assert_eq!(changelog_group("feat: per-street fold calibration (0156)"), "Features");
        assert_eq!(changelog_group("perf: spend the turn window on 10x samples (0148)"), "Performance");
        assert_eq!(changelog_group("test: pin the season scope"), "Tests");
        assert_eq!(changelog_group("chore: stay on svanbot10 10.0.0"), "Cleanup");
        assert_eq!(changelog_group("refactor(bot): split handler"), "Refactors");
        assert_eq!(changelog_group("Merge PR #1: season-scope the dashboard"), "Merges");
    }

    #[test]
    fn update_checks_parse_and_default() {
        assert_eq!(parse_check("3 4f8372b\n"), Some((3, "4f8372b".to_string())));
        assert_eq!(parse_check("0 ef4098e"), Some((0, "ef4098e".to_string())));
        assert_eq!(parse_check(""), None);
        assert_eq!(parse_check("up to date"), None);
        let dir = std::env::temp_dir().join(format!("sv10-update-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(last_check(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_is_weighted_by_the_last_runs_stage_times() {
        let now = 10_000.0;
        let timings = json!({"fetch": 4.0, "snapshot": 6.0, "build": 280.0, "test": 90.0, "dashboard": 15.0, "install": 5.0});
        // Mid-build, 140 s into a 280 s stage: fetch and snapshot done (10 s of 400 s) plus half the build.
        let running = json!({"state": "running", "started": now - 150.0, "stages": [
            {"name": "fetch", "state": "done", "started": now - 150.0, "seconds": 3.0},
            {"name": "snapshot", "state": "done", "started": now - 147.0, "seconds": 7.0},
            {"name": "build", "state": "running", "started": now - 140.0, "seconds": null}]});
        let v = progress_view(&running, &timings, now);
        assert_eq!(v["state"], "running");
        assert!((v["percent"].as_f64().unwrap() - 37.5).abs() < 0.1, "{v}");
        assert_eq!(v["eta"].as_f64(), Some(250.0), "140 s of build left, then 90 + 15 + 5");
        assert_eq!(v["elapsed"].as_f64(), Some(150.0));
        let names: Vec<&str> = v["stages"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["fetch", "snapshot", "build", "test", "dashboard", "install"]);
        assert_eq!(v["stages"][2]["state"], "running");
        assert_eq!(v["stages"][3]["state"], "pending");
        // A stage overrunning its estimate never shows done or negative time left.
        let late = progress_view(&running, &json!({"build": 60.0}), now);
        assert!(late["percent"].as_f64().unwrap() < 100.0 && late["eta"].as_f64().unwrap() > 0.0, "{late}");
        // Failed: the failed stage is named, no ETA; installed: 100 %.
        let failed = json!({"state": "failed", "started": now - 100.0, "message": "release failed", "stages": [
            {"name": "fetch", "state": "done", "seconds": 3.0}, {"name": "test", "state": "failed", "seconds": 20.0}]});
        let f = progress_view(&failed, &timings, now);
        assert_eq!((f["state"].as_str(), f["eta"].is_null()), (Some("failed"), true));
        assert!(f["stages"].as_array().unwrap().iter().any(|s| s["name"] == "test" && s["state"] == "failed"));
        let done = progress_view(
            &json!({"state": "installed", "commit": "abc1234", "stages": [{"name": "snapshot", "state": "done", "seconds": 5.0}]}),
            &timings,
            now,
        );
        assert_eq!(done["percent"], 100.0);
        // A release run by hand has no fetch stage; no run at all is idle at 0 %.
        let names: Vec<&str> = done["stages"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names[0], "snapshot");
        let idle = progress_view(&Value::Null, &Value::Null, now);
        assert_eq!((idle["state"].as_str(), idle["percent"].as_f64()), (Some("idle"), Some(0.0)));
    }

    #[test]
    fn a_rollback_is_one_restore_stage_and_lists_saved_builds_only() {
        let now = 5_000.0;
        let v = progress_view(
            &json!({"state": "running", "started": now - 8.0, "stages": [{"name": "restore", "state": "running", "started": now - 8.0}]}),
            &json!({"restore": 16.0}),
            now,
        );
        let names: Vec<&str> = v["stages"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["restore"]);
        assert!((v["percent"].as_f64().unwrap() - 50.0).abs() < 0.1, "{v}");
        // An update's plan never shows the restore stage.
        let u = progress_view(
            &json!({"state": "running", "started": now, "stages": [{"name": "fetch", "state": "running", "started": now}]}),
            &Value::Null,
            now,
        );
        assert!(u["stages"].as_array().unwrap().iter().all(|s| s["name"] != "restore"));

        assert!(valid_commit("4f8372b") && valid_commit(&"a".repeat(40)));
        assert!(!valid_commit("4F8372B") && !valid_commit("abc") && !valid_commit("4f8372b; rm -rf /") && !valid_commit("../../x"));
        let dir = std::env::temp_dir().join(format!("sv10-snapshots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for c in ["aaaaaaa", "bbbbbbb", "not-a-commit"] {
            std::fs::create_dir_all(dir.join("release-snapshots").join(c)).unwrap();
        }
        std::fs::write(
            dir.join("releases.log"),
            "2026-09-25T00:00:08+02:00 aaaaaaa style: older\n2026-09-26T01:30:28+02:00 bbbbbbb feat: newer\n2026-09-26T03:00:00+02:00 ccccccc feat: installed now\n",
        )
        .unwrap();
        let list = snapshots(&dir);
        let commits: Vec<&str> = list.iter().map(|v| v["commit"].as_str().unwrap()).collect();
        assert_eq!(commits.len(), 2, "{list:?}");
        assert!(commits.contains(&"aaaaaaa") && commits.contains(&"bbbbbbb"));
        let a = list.iter().find(|v| v["commit"] == "aaaaaaa").unwrap();
        assert_eq!((a["subject"].as_str(), a["current"].as_bool()), (Some("style: older"), Some(false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_lines_parse_commit_and_subject() {
        let (c, subject) = parse_release_line("2026-09-17T08:19:34+02:00 0a1bdba Fix two fuzzer-caught panics").unwrap();
        assert_eq!((c.as_str(), subject.as_str()), ("0a1bdba", "Fix two fuzzer-caught panics"));
        assert!(parse_release_line("").is_none());
        assert!(parse_release_line("2026-09-17T08:19:34+02:00").is_none());
    }

    #[test]
    fn range_and_status_read_a_real_repo() {
        let dir = std::env::temp_dir().join(format!("sv10-releases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f"), "1").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "Fix first bug"]);
        let base = git(&dir, &["rev-parse", "--short", "HEAD"]).unwrap();
        std::fs::write(dir.join("f"), "2").unwrap();
        run(&["commit", "-qam", "Deepen widgets: movable panels"]);
        let range = commit_range_to(&dir, base.trim(), "HEAD");
        assert_eq!(range.len(), 1);
        assert_eq!(changelog_group(&range[0].1), "Refactors");
        assert!(commit_range_to(&dir, range[0].0.trim(), "HEAD").is_empty());
        assert!(git(&dir, &["status", "--porcelain"]).unwrap().trim().is_empty());
        // The data format a build reads comes from its codec source; before the codec it is 1.
        assert_eq!(build_data_format(&dir, base.trim()), 1);
        std::fs::create_dir_all(dir.join("crates/libs/store/src")).unwrap();
        std::fs::write(dir.join("crates/libs/store/src/packed.rs"), "//! codec\npub const DATA_FORMAT: u32 = 2;\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "feat: packed columns"]);
        let packed = git(&dir, &["rev-parse", "--short", "HEAD"]).unwrap();
        assert_eq!(build_data_format(&dir, packed.trim()), 2);
        assert_eq!(build_data_format(&dir, "0000000"), 1, "an unknown commit reads as the oldest format");
        std::fs::write(dir.join("dirty"), "x").unwrap();
        assert!(!git(&dir, &["status", "--porcelain"]).unwrap().trim().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
