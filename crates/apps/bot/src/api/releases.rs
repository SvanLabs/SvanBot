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

pub(super) mod checkout;
use checkout::*;
mod liveness;
use liveness::{record_owner, update_running};

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
                // Base for "what would an update install": the installed commit when recorded,
                // else this checkout's own HEAD — a box with no install record still shows the
                // line ahead of it instead of a silent empty changelog (issue #757 follow-up).
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
                // No install on record (fresh box, wiped progress): measure from this checkout's
                // HEAD so pending commits still show with their subjects.
                _ => match &head {
                    Some((h, _)) => {
                        let range = commit_range_to(root, h, &target);
                        let log = range
                            .iter()
                            .map(|(c, subject)| {
                                json!({"commit": c.chars().take(7).collect::<String>(), "subject": subject, "group": changelog_group(subject)})
                            })
                            .collect();
                        (range.len(), log)
                    }
                    _ => (0, Vec::new()),
                },
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
        // `current` is a run that installed nothing and did nothing (#394): the fetch it recorded is
        // not a step towards an install, so the card must not read as one partly done.
        "idle" | "current" => 0.0,
        _ if total > 0.0 => (done / total * 100.0).clamp(0.0, 99.0),
        _ => 0.0,
    };
    let started = progress["started"].as_f64();
    // A finished run's elapsed is how long it took, not how long ago it ended (issue #320): `updated`
    // is the last save, which for `installed` and `failed` is the end of the run, and the panel reads
    // this field as a duration ("in 237m 27s" for a pipeline whose own stages total 1m 19s). A
    // `running` run has no end to freeze, so it counts from `started` to now as it always did, and a
    // record with no `updated` (written before the field existed) keeps that reading too.
    let ended = (state != "running").then(|| progress["updated"].as_f64()).flatten();
    json!({
        "state": state,
        "stages": stages,
        "percent": (percent * 10.0f64).round() / 10.0,
        "elapsed": started.map(|t| ((ended.unwrap_or(now) - t).max(0.0) * 10.0).round() / 10.0),
        // When a finished run ended, so the panel can date it instead of leaving a bare duration
        // that the next line of the panel contradicts (issue #320).
        "finished_at": ended,
        "eta": (state == "running").then(|| left.round()),
        "from": progress["from"],
        "commit": progress["commit"],
        "message": progress["message"],
    })
}

/// Saved builds a rollback can restore (`GET /api/releases/snapshots`, 0239).
pub(super) async fn release_snapshots(State(s): State<Arc<Shared>>) -> Result<Json<Value>, ApiError> {
    // One `git show` per saved build: off the workers that carry the table connections.
    off_runtime(move || Json(json!({"snapshots": snapshots(&s.config.artifacts), "running": update_running(&s.config.artifacts)}))).await
}

/// Restore a saved build (`POST /api/releases/rollback {commit}`, 0239): `scripts/update.sh --rollback`
/// detached, with the same lock and progress as an update; the fleet hot-swaps to it.
pub(super) async fn trigger_rollback(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let commit = body["commit"].as_str().unwrap_or("").to_string();
    let artifacts = s.config.artifacts.clone();
    let saved = match off_runtime(move || snapshots(&artifacts)).await {
        Ok(saved) => saved,
        Err(e) => return e.into_response(),
    };
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
    let running = update_running(&artifacts);
    let progress = liveness::reconcile(&read("release-progress.json").unwrap_or(Value::Null), running, now_secs());
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
    let now = now_secs();
    let playing = s
        .bots
        .iter()
        .filter(|b| {
            let b = b.read();
            if b.mode == "playing" {
                return true;
            }
            // Split fleet: the head never spawns bots, so an offline local slot reads
            // through its worker's heartbeat — the same substitution the raw bot views
            // apply, or the updater reports a playing fleet as 0 of N.
            if b.mode != "offline" || !s.config.head {
                return false;
            }
            crate::live::read_remote_bot(&s.store, &b.name, now).is_some_and(|(remote, _)| remote.mode == "playing")
        })
        .count();
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
    view["running"] = json!(running);
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
/// the lock however it ends; liveness checks recover a dead owner's marker without waiting two hours.
fn spawn_update(s: &Arc<Shared>, args: &[&str], started: &str) -> Response {
    if update_running(&s.config.artifacts) {
        return (StatusCode::CONFLICT, Json(json!({"detail": "an update or rollback is already running"}))).into_response();
    }
    let lock = s.config.artifacts.join("release.lock");
    if std::fs::write(&lock, serde_json::json!({"started": now_secs()}).to_string()).is_err() {
        // A write that fails can leave the file created and empty, and an empty marker reads as a
        // run in progress for two hours: the dashboard then refuses the update and the rollback.
        let _ = std::fs::remove_file(&lock);
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
        Ok(mut child) => {
            record_owner(&lock, child.id());
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            s.log("fleet", "info", started);
            Json(json!({"started": true})).into_response()
        }
        Err(e) => {
            // Best-effort, and the right answer here: the run never started, so nothing else holds
            // this lock, and one that survives ages out after 2 hours. Failing this response over its
            // removal would report a spawn failure as a lock failure (issue #326).
            let _ = std::fs::remove_file(&lock);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not start scripts/update.sh: {e}")}))).into_response()
        }
    }
}

#[cfg(test)]
mod tests;
