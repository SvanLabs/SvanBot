//! Tests for the releases API: how a commit subject groups, what a commit range yields, the update
//! check and its stored result, the snapshots a rollback restores, and the progress bar's stage view.
//!
//! Split out of `releases.rs` (the 500-line rule: that file was at its 657-line baseline ceiling).

use super::*;

#[tokio::test]
async fn abandoned_update_progress_recovers_without_operator_intervention() {
    let s = Shared::for_test("abandoned-update", &["A"]);
    let progress = json!({"state":"running", "started":now_secs()-120.0, "updated":now_secs()-110.0,
        "stages":[{"name":"fetch","state":"running","started":now_secs()-120.0}]});
    std::fs::write(s.config.artifacts.join("release-progress.json"), progress.to_string()).unwrap();
    let Json(v) = release_progress(State(s)).await;
    assert_eq!(v["state"], "failed", "a dead updater must not keep the controls busy");
    assert_eq!(v["running"], false);
    assert!(v["eta"].is_null());
    assert_eq!(v["stages"][0]["state"], "failed");
}

#[test]
fn a_dead_update_owner_does_not_block_recovery_for_two_hours() {
    let s = Shared::for_test("dead-update-owner", &["A"]);
    std::fs::write(s.config.artifacts.join("release.lock"), json!({"started":now_secs(),"pid":u32::MAX}).to_string()).unwrap();
    assert!(!update_running(&s.config.artifacts), "the update owner is gone");
}

#[test]
fn update_ownership_distinguishes_live_reused_and_finished_processes() {
    let s = Shared::for_test("update-process-identity", &["A"]);
    let lock = s.config.artifacts.join("release.lock");
    record_owner(&lock, std::process::id());
    assert!(!lock.exists(), "recording must not recreate a removed marker");
    std::fs::write(&lock, "{}").unwrap();
    record_owner(&lock, std::process::id());
    assert!(update_running(&s.config.artifacts), "healthy owner remains active");
    let mut owner: Value = serde_json::from_str(&std::fs::read_to_string(&lock).unwrap()).unwrap();
    owner["process_start"] = json!("different process start");
    std::fs::write(&lock, owner.to_string()).unwrap();
    assert!(!update_running(&s.config.artifacts), "a reused PID is not the original owner");
}

#[test]
fn orphan_recovery_keeps_startup_grace_and_existing_terminal_messages() {
    let running = json!({"state":"running", "started":100.0, "updated":100.0});
    assert_eq!(liveness::reconcile(&running, false, 103.0), running);
    let failed = json!({"state":"failed", "message":"specific failure"});
    assert_eq!(liveness::reconcile(&failed, false, 300.0), failed);
}

#[tokio::test]
async fn manual_release_operation_remains_active_without_dashboard_lock() {
    let s = Shared::for_test("manual-release-operation", &["A"]);
    let operation = std::fs::File::create(s.config.artifacts.join("release-operation.lock")).unwrap();
    operation.lock().unwrap();
    assert!(
        !std::process::Command::new("flock")
            .arg("-n")
            .arg(s.config.artifacts.join("release-operation.lock"))
            .arg("true")
            .status()
            .unwrap()
            .success(),
        "the observer uses the same kernel lock as the shell release pipeline"
    );
    let progress = json!({"state":"running", "started":now_secs()-120.0, "updated":now_secs()-110.0,
        "stages":[{"name":"build","state":"running","started":now_secs()-120.0}]});
    std::fs::write(s.config.artifacts.join("release-progress.json"), progress.to_string()).unwrap();
    let Json(v) = release_progress(State(s)).await;
    assert_eq!(v["state"], "running");
    assert_eq!(v["running"], true, "a hand-run release owns the operation lock");
    drop(operation);
}

#[test]
fn a_single_process_fleet_lists_no_workers() {
    let shared = Shared::for_test("release-workers", &["A", "B"]);
    let old = json!({"hb": 1.0, "state": {}, "commit": "0000000"}).to_string();
    shared.store.put_kv(&crate::live::heartbeat_key("A"), &old).unwrap();
    assert!(worker_commits(&shared).is_empty(), "stale split-fleet heartbeats are not workers");
}

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

#[test]
fn a_failed_check_keeps_the_last_successful_fetch_time() {
    let dir = std::env::temp_dir().join(format!("sv10-update-fetched-{}", std::process::id()));
    let (root, artifacts) = (dir.join("root"), dir.join("artifacts"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::create_dir_all(&artifacts).unwrap();
    let script = root.join("scripts/update.sh");
    // The check the panel runs is a fetch; a stub stands in for it here.
    std::fs::write(&script, "echo '2 4f8372b'\n").unwrap();
    let ok = run_update_check(&root, &artifacts);
    assert_eq!((ok.behind, ok.commit.as_deref(), ok.error.as_deref()), (2, Some("4f8372b"), None));
    assert_eq!(ok.fetched_at, Some(ok.checked_at), "a check that fetched stamps the tip it saw");
    // One that cannot fetch keeps that stamp: the commit list the panel reads from the local refs is
    // still the last one GitHub confirmed, and must be dated as such rather than as today's (#320).
    std::fs::write(&script, "echo 'update: could not fetch origin/main (network or credentials)' >&2\nexit 1\n").unwrap();
    let failed = run_update_check(&root, &artifacts);
    assert!(failed.error.is_some(), "{failed:?}");
    assert_eq!(failed.fetched_at, Some(ok.checked_at));
    assert_eq!(last_check(&artifacts).and_then(|c| c.fetched_at), Some(ok.checked_at), "and it is what is stored");
    // A record written before the field existed dates its list by its own check when it succeeded,
    // and carries nothing when it did not.
    let legacy = |body: &str| {
        std::fs::write(artifacts.join("update-check.json"), body).unwrap();
        run_update_check(&root, &artifacts).fetched_at
    };
    assert_eq!(legacy(r#"{"source":"origin/main","commit":"4f8372b","behind":2,"checked_at":900.0,"error":null}"#), Some(900.0));
    assert_eq!(legacy(r#"{"source":"origin/main","commit":null,"behind":0,"checked_at":900.0,"error":"fetch failed"}"#), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_finished_runs_elapsed_is_its_duration_not_its_age() {
    let now = 100_000.0;
    // Started 3000 s ago and took 120 s: the last save (`updated`) is the end of the run.
    let finished = |state: &str| {
        json!({"state": state, "started": now - 3_000.0, "updated": now - 2_880.0,
               "stages": [{"name": "fetch", "state": "done", "seconds": 3.0}]})
    };
    for state in ["installed", "failed"] {
        let v = progress_view(&finished(state), &Value::Null, now);
        assert_eq!(v["elapsed"].as_f64(), Some(120.0), "{state} reports how long it took, not how long ago it ended: {v}");
        // And when it ended, so the panel can date the card instead of contradicting the row below it
        // that is about the *next* update (issue #320).
        assert_eq!(v["finished_at"].as_f64(), Some(now - 2_880.0), "{state} carries the end of the run: {v}");
    }
    // A running run has no recorded end: it still counts from `started` to now, and dates nothing.
    let running = json!({"state": "running", "started": now - 150.0, "updated": now - 140.0, "stages": []});
    assert_eq!(progress_view(&running, &Value::Null, now)["elapsed"].as_f64(), Some(150.0));
    assert!(progress_view(&running, &Value::Null, now)["finished_at"].is_null());
}
