//! Tests for the background loops: cadence, backup rotation, the second-disk mirror.
//!
//! Split out of `tasks.rs` (0320: the 500-line rule).

use sv10_core::model::ModelStore;
use sv10_store::store::Store;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sv10-tasks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn queued(id: &str, attempts: u32) -> (sv10_store::store::HandRow, u32) {
    let row = sv10_store::store::HandRow {
        bot: "A".into(),
        hand_id: id.into(),
        table_id: "t".into(),
        ended_at: "2026-09-20T13:19:29Z".into(),
        hero_seat: Some(1),
        hole: "AhKd".into(),
        board: String::new(),
        pot: 30,
        net: Some(-20),
        winners: "B".into(),
        summary: "{}".into(),
        showdown: false,
    };
    (row, attempts)
}

/// 0254: `jobs::blocking` is the only door to the blocking pool for a *loop*. The one-shot
/// call sites (startup, request handlers, the decision) keep `spawn_blocking` because they
/// surface the error to a caller.
#[test]
fn every_background_loop_goes_through_the_blocking_pool_helper() {
    // The loops live in tasks.rs and tasks/*.rs alike (0256): every file is scanned, or the
    // invariant would pass vacuously on the slimmed parent.
    for file in ["tasks.rs", "tasks/backup.rs", "tasks/calibration.rs", "tasks/hands.rs", "tasks/models.rs", "tasks/scan.rs"] {
        let source = match file {
            "tasks.rs" => include_str!("../tasks.rs"),
            "tasks/backup.rs" => include_str!("../tasks/backup.rs"),
            "tasks/calibration.rs" => include_str!("../tasks/calibration.rs"),
            "tasks/hands.rs" => include_str!("../tasks/hands.rs"),
            "tasks/models.rs" => include_str!("../tasks/models.rs"),
            _ => include_str!("../tasks/scan.rs"),
        };
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (n, line) in body.lines().enumerate() {
            assert!(!line.contains("spawn_blocking"), "{file}:{} reaches the blocking pool directly ({})", n + 1, line.trim());
        }
    }
}

#[test]
fn backup_rotation_keeps_the_newest_copies_and_their_seals() {
    let dir = std::env::temp_dir().join(format!("sv10-rotate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for h in 10..20 {
        for name in [format!("svanbot10-20260924{h}.db"), format!("svanbot10-20260924{h}.db.sha256")] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
    }
    for d in 10..13 {
        std::fs::write(dir.join(format!("daily-svanbot10-202609{d}.db")), "x").unwrap();
    }
    super::backup::rotate_backups_keeping(&dir, 6, 2);
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let hourly: Vec<&String> = left.iter().filter(|n| n.starts_with("svanbot10-") && n.ends_with(".db")).collect();
    assert_eq!(hourly.first().map(|s| s.as_str()), Some("svanbot10-2026092414.db"));
    assert_eq!(hourly.len(), 6);
    assert_eq!(left.iter().filter(|n| n.ends_with(".sha256")).count(), 6, "seals go with their copies");
    assert_eq!(left.iter().filter(|n| n.starts_with("daily-")).count(), 2);
    assert!((2..=48).contains(&super::backup::hourly_backups_kept(false)));
    // The HDD mirror is opt-in since 2026-09-27: unset, empty, 0 or junk is off; a count turns it on.
    for off in [None, Some(""), Some("0"), Some("many")] {
        assert_eq!(super::backup::mirror_setting(off), None, "{off:?}");
    }
    assert_eq!(super::backup::mirror_setting(Some("24")), Some(24));
    assert_eq!(super::backup::mirror_setting(Some("1")), Some(2), "at least two copies when on");
    assert!(super::backup::hourly_backups_kept(true) <= super::backup::hourly_backups_kept(false));
    assert!((1..=30).contains(&super::backup::daily_backups_kept()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #326 made a copy that will not rotate out say so; #347 is the log capture that lets a test read it.
#[test]
fn a_backup_that_cannot_be_rotated_out_is_reported_and_a_clean_rotation_is_silent() {
    let stuck = dir("rotate-stuck");
    // The oldest "copy" is a directory, which `remove_file` refuses as it would a busy one.
    std::fs::create_dir(stuck.join("svanbot10-2026092410.db")).unwrap();
    for h in 11..14 {
        std::fs::write(stuck.join(format!("svanbot10-20260924{h}.db")), "x").unwrap();
    }
    let ((), log) = crate::testlog::capture(|| super::backup::rotate_backups_keeping(&stuck, 3, 1));
    assert!(log.contains("could not rotate out") && log.contains("svanbot10-2026092410.db"), "{log}");

    let clean = dir("rotate-clean");
    for h in 10..14 {
        std::fs::write(clean.join(format!("svanbot10-20260924{h}.db")), "x").unwrap();
    }
    let ((), log) = crate::testlog::capture(|| super::backup::rotate_backups_keeping(&clean, 3, 1));
    assert_eq!(log, "", "a rotation that worked has nothing to report");
    let _ = (std::fs::remove_dir_all(&stuck), std::fs::remove_dir_all(&clean));
}

#[test]
fn hourly_backups_are_moved_to_the_second_disk_and_keep_the_newest() {
    let d = dir("mirror");
    let (ssd, hdd) = (d.join("backups"), d.join("hdd").join("hourly"));
    std::fs::create_dir_all(&ssd).unwrap();
    for h in 10..13 {
        let copy = ssd.join(format!("svanbot10-20260926{h}.db"));
        let c = rusqlite::Connection::open(&copy).unwrap();
        c.execute_batch(&format!("CREATE TABLE t (x); INSERT INTO t VALUES ({h});")).unwrap();
        drop(c);
        sv10_store::integrity::seal_backup(&copy).unwrap();
        assert!(super::backup::mirror_backup(&copy, &hdd, 2).unwrap());
    }
    let mut left: Vec<String> = std::fs::read_dir(&hdd).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(
        left,
        ["svanbot10-2026092611.db", "svanbot10-2026092611.db.sha256", "svanbot10-2026092612.db", "svanbot10-2026092612.db.sha256"]
    );
    assert!(sv10_store::integrity::verify_backup(&hdd.join("svanbot10-2026092612.db")));
    // A move, not a copy (#374): the hour leaves the SSD, so there is exactly one pair and it is
    // the one a restore reads.
    assert!(!ssd.join("svanbot10-2026092612.db").exists(), "the moved hour is off the SSD");
    assert!(!ssd.join("svanbot10-2026092612.db.sha256").exists(), "and its seal went with it");
}

#[test]
fn a_failed_move_leaves_the_ssd_pair_and_names_the_step() {
    // 2026-09-27: /backup-disk reported `Structure needs cleaning` (EUCLEAN) to the copy-verify-
    // rename the mirror used to do (0292, 22 times over two days). The operator's call is a plain
    // move (#374), so what a failure has to preserve is the SSD pair — and it has to say which step
    // failed.
    let d = dir("mirror-fail");
    let (ssd, hdd) = (d.join("backups"), d.join("hdd").join("hourly"));
    std::fs::create_dir_all(&ssd).unwrap();
    std::fs::create_dir_all(&hdd).unwrap();
    let name = "svanbot10-2026092614.db";
    let good = ssd.join(name);
    let c = rusqlite::Connection::open(&good).unwrap();
    c.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);").unwrap();
    drop(c);
    sv10_store::integrity::seal_backup(&good).unwrap();
    // A second disk that refuses the write, which is what a failing one does.
    use std::os::unix::fs::PermissionsExt;
    let chmod = |mode: u32| {
        let mut perms = std::fs::metadata(&hdd).unwrap().permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(&hdd, perms).unwrap();
    };
    chmod(0o555);
    let err = super::backup::mirror_backup(&good, &hdd, 2).unwrap_err().to_string();
    assert!(err.contains("moving") && err.contains(name), "the failure names the step that failed: {err}");
    assert!(sv10_store::integrity::verify_backup(&good), "the SSD pair is untouched");
    chmod(0o755);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_storage_row_says_why_the_second_disk_has_no_copy() {
    // The dashboard's storage row is where the operator looks; before this a failed mirror was
    // only a log line the dashboard never shows (0292).
    let shared = crate::live::Shared::for_test("backup-status", &["A"]);
    let d = dir("mirror-status");
    let now = chrono::Utc::now();
    let hourly = d.join("svanbot10-2026092614.db");
    super::backup::write_backup_status(
        &shared,
        &now,
        &hourly,
        3,
        0,
        &super::backup::Mirror::Failed("the copy's structural check failed: database disk image is malformed".into()),
        Some(&d.join("hourly")),
    );
    let row: serde_json::Value = serde_json::from_str(&shared.store.get_kv(super::INTEGRITY_STATUS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(row["last_backup"], "svanbot10-2026092614.db");
    assert_eq!(row["mirror"]["state"], "failed");
    assert_eq!(row["mirror"]["at"], now.timestamp());
    assert!(row["mirror"]["dir"].as_str().unwrap().ends_with("hourly"), "the mirror's directory is named");
    assert!(
        row["mirror"]["error"].as_str().unwrap().contains("structural check failed"),
        "the row carries the reason the operator has to act on"
    );
    assert!(shared.log.lock().iter().any(|l| l.level == "error" && l.message.contains("backup not mirrored")));
    super::backup::write_backup_status(&shared, &now, &hourly, 3, 0, &super::backup::Mirror::Copied, None);
    let row: serde_json::Value = serde_json::from_str(&shared.store.get_kv(super::INTEGRITY_STATUS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(row["mirror"]["state"], "ok");
    assert!(row["mirror"]["error"].is_null());
}

/// #725: with no second device the mirror is accepted in its own folder under the archive root, and
/// `backup_mirror` says it is the same disk so the caller can never report it as `ok`.
#[test]
fn a_mirror_folder_on_the_databases_disk_is_accepted_and_flagged() {
    let shared = crate::live::Shared::for_test("mirror-same-disk", &["A"]);
    // `for_test`'s archive dir is inside `artifacts/` — same disk — and must exist to be read.
    std::fs::create_dir_all(&shared.config.archive_dir).unwrap();
    let (dir, same_disk) = super::backup::backup_mirror(&shared).expect("the dedicated hourly folder resolves on one disk");
    assert_eq!(dir, shared.config.archive_dir.join("hourly"), "the archive root's own hourly folder");
    assert!(same_disk, "one disk here: the state has to say so");
    // An unreadable archive directory is still no mirror, not a guess.
    std::fs::remove_dir_all(&shared.config.archive_dir).unwrap();
    assert!(super::backup::backup_mirror(&shared).is_none());
}

/// #725: the same-disk mirror is a real copy of the hour and a real warning. The row says
/// `same_disk` (never `ok`), and the log line names what it protects against and what it does not.
#[test]
fn the_same_disk_mirror_reports_its_own_state_and_log_line() {
    let shared = crate::live::Shared::for_test("mirror-same-disk-row", &["A"]);
    let d = dir("mirror-same-disk-row");
    let now = chrono::Utc::now();
    let hourly = d.join("svanbot10-2026092614.db");
    let mirror = d.join("hourly");
    let ((), log) = crate::testlog::capture(|| {
        super::backup::write_backup_status(&shared, &now, &hourly, 3, 0, &super::backup::Mirror::SameDisk, Some(&mirror));
    });
    let row: serde_json::Value = serde_json::from_str(&shared.store.get_kv(super::INTEGRITY_STATUS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(row["mirror"]["state"], "same_disk");
    assert!(row["mirror"]["error"].is_null());
    assert_eq!(row["mirror"]["at"], now.timestamp());
    assert!(row["mirror"]["dir"].as_str().unwrap().ends_with("hourly"), "the mirror's directory is named");
    assert!(log.contains("same disk") && log.contains("not a disk-loss copy"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_worker_reloads_the_heads_models_only_when_they_change() {
    let shared = crate::live::Shared::for_test("worker-models", &["A"]);
    let mut last = None;
    assert!(!super::refresh_models_from_store(&shared, &mut last), "nothing saved yet");
    shared.models.write().response_ratios = std::sync::Arc::new([("villain".to_string(), [2.0, 1.0, 1.0])].into_iter().collect());
    shared.models.write().fold_offsets = std::sync::Arc::new([("villain".to_string(), -0.6)].into_iter().collect());
    let mut head = ModelStore { schema: sv10_core::model::MODEL_SCHEMA, ..Default::default() };
    head.players.entry("villain".into()).or_default().hands = 42.0;
    shared.store.put_kv(crate::MODELS_KEY, &serde_json::to_string(&head).unwrap()).unwrap();
    assert!(super::refresh_models_from_store(&shared, &mut last));
    assert_eq!(shared.models.read().players["villain"].hands, 42.0, "the head's canonical models");
    assert_eq!(shared.models.read().profile("villain").response_ratio, [2.0, 1.0, 1.0], "the installed correction survives");
    assert_eq!(shared.models.read().profile("villain").fold_logit_offset, -0.6, "and so does the fold calibration");
    assert!(!super::refresh_models_from_store(&shared, &mut last), "unchanged checkpoint: no reload");
}

#[test]
fn a_worker_refresh_keeps_hands_recovered_after_the_head_checkpoint() {
    let shared = crate::live::Shared::for_test("worker-recovered-models", &["A"]);
    let (mut hand, _) = queued("fresh-hand", 0);
    hand.summary = serde_json::json!({
        "players": [[0, "villain"], [1, "A"]], "button": 0, "bb": 20,
        "history": [], "board": [], "shown": []
    })
    .to_string();
    let rowid = shared.store.insert_hand(&hand).unwrap();
    let head = ModelStore { schema: sv10_core::model::MODEL_SCHEMA, watermark: Some(0), ..Default::default() };
    shared.store.put_kv(crate::MODELS_KEY, &serde_json::to_string(&head).unwrap()).unwrap();
    let mut recovered = head.clone();
    super::recover_models(&shared.store, &mut recovered).unwrap();
    assert_eq!(recovered.watermark, Some(rowid));
    assert_eq!(recovered.hero_seen[&format!("{}A", sv10_core::model::HERO_SEEN_ONE)].hands, 1.0);
    *shared.models.write() = recovered;

    let mut last = None;
    assert!(super::refresh_models_from_store(&shared, &mut last));
    assert_eq!(shared.models.read().watermark, Some(rowid), "first refresh must not discard startup recovery");
    assert_eq!(shared.models.read().hero_seen[&format!("{}A", sv10_core::model::HERO_SEEN_ONE)].hands, 1.0);

    let mut newer_head = head;
    newer_head.players.entry("head-only".into()).or_default().hands = 5.0;
    shared.store.put_kv(crate::MODELS_KEY, &serde_json::to_string(&newer_head).unwrap()).unwrap();
    assert!(super::refresh_models_from_store(&shared, &mut last));
    assert_eq!(shared.models.read().players["head-only"].hands, 5.0, "a newer head checkpoint still installs");
    assert_eq!(shared.models.read().watermark, Some(rowid), "its missed hands are replayed before installation");
}

#[test]
fn a_worker_applies_self_calibration_without_writing_the_table() {
    let shared = crate::live::Shared::for_test("worker-cal", &["A"]);
    for i in 0..400 {
        shared.store.insert_calibration("A", &format!("h{i}"), "river:call", 80.0, if i % 2 == 0 { 40.0 } else { 60.0 }, 100.0).unwrap();
    }
    super::update_calibration_with(&shared, false);
    assert!(shared.params.read().ev_bias.get("river:call").is_some_and(|b| *b < 0.0), "worker plays with the correction");
    assert!(shared.store.get_kv(crate::CALIBRATION_KEY).unwrap().is_none(), "only the head writes the table");
    super::update_calibration_with(&shared, true);
    assert!(shared.store.get_kv(crate::CALIBRATION_KEY).unwrap().is_some());
}

#[test]
fn the_retry_queue_gives_up_on_a_hand_and_stays_bounded() {
    // A hand that failed every retry for MAX_HAND_RETRIES rounds is dropped, not retried forever.
    let (keep, dropped) = super::bound_unstored(vec![queued("old", super::MAX_HAND_RETRIES), queued("new", 1)]);
    assert_eq!(keep.iter().map(|(r, _)| r.hand_id.as_str()).collect::<Vec<_>>(), ["new"]);
    assert_eq!(dropped, 1);
    // The queue never holds more than MAX_UNSTORED_HANDS; the oldest go first.
    let many: Vec<_> = (0..super::MAX_UNSTORED_HANDS + 3).map(|i| queued(&format!("h{i}"), 0)).collect();
    let (keep, dropped) = super::bound_unstored(many);
    assert_eq!((keep.len(), dropped), (super::MAX_UNSTORED_HANDS, 3));
    assert_eq!(keep[0].0.hand_id, "h3");
}

#[test]
fn a_hand_queued_while_the_retry_round_runs_is_not_dropped() {
    // 0247: the old code took the queue, worked unlocked and assigned the result back, so a
    // hand the frame loop pushed in between existed nowhere. The drain and the write share one
    // lock now, and this is the invariant that proves it.
    let shared = crate::live::Shared::for_test("unstored-race", &["A"]);
    shared.unstored_hands.lock().push(queued("during", 0));
    let (left, dropped) = super::hands::requeue(&shared, vec![queued("retried", 1)]);
    assert_eq!((left, dropped), (2, 0));
    let ids: Vec<String> = shared.unstored_hands.lock().iter().map(|(r, _)| r.hand_id.clone()).collect();
    assert_eq!(ids, ["retried", "during"], "retried hands stay first, the late one is not lost");
    // An exhausted hand is still given up, and the late one still survives the bounding.
    let shared = crate::live::Shared::for_test("unstored-race-full", &["A"]);
    let many: Vec<_> = (0..super::MAX_UNSTORED_HANDS).map(|i| queued(&format!("old{i}"), 0)).collect();
    shared.unstored_hands.lock().push(queued("during", 0));
    let (left, dropped) = super::hands::requeue(&shared, many);
    assert_eq!((left, dropped), (super::MAX_UNSTORED_HANDS, 1));
    let ids: Vec<String> = shared.unstored_hands.lock().iter().map(|(r, _)| r.hand_id.clone()).collect();
    assert_eq!(
        (ids.first().map(String::as_str), ids.last().map(String::as_str)),
        (Some("old1"), Some("during")),
        "the oldest go first, the late hand stays"
    );
}

#[test]
fn a_queued_hand_is_stored_and_advances_the_watermark_without_replay() {
    let shared = crate::live::Shared::for_test("unstored", &["A"]);
    shared.models.write().watermark = Some(0);
    let row = sv10_store::store::HandRow {
        bot: "A".into(),
        hand_id: "h-locked".into(),
        table_id: "t".into(),
        ended_at: "2026-09-20T13:19:29Z".into(),
        hero_seat: Some(1),
        hole: "AhKd".into(),
        board: String::new(),
        pot: 30,
        net: Some(-20),
        winners: "B".into(),
        summary: "{}".into(),
        showdown: false,
    };
    shared.unstored_hands.lock().push((row, 0));
    assert_eq!(super::retry_unstored_hands(&shared), (1, 0));
    assert!(shared.unstored_hands.lock().is_empty());
    assert!(shared.store.hand("A", "h-locked").unwrap().is_some());
    let rowid = shared.store.max_hand_rowid().unwrap();
    assert_eq!(shared.models.read().watermark, Some(rowid), "a restart must not observe it twice");
    assert_eq!(super::retry_unstored_hands(&shared), (0, 0), "nothing left to retry");
}

#[test]
fn identical_kv_values_are_not_rewritten() {
    let d = dir("kv");
    let store = Store::open(&d.join("svanbot10.db")).unwrap();
    store.put_kv("k", "v1").unwrap();
    let changes = |s: &Store| s.total_changes();
    let before = changes(&store);
    store.put_kv("k", "v1").unwrap();
    assert_eq!(changes(&store), before, "identical value rewrote the row");
    store.put_kv("k", "v2").unwrap();
    assert_eq!(changes(&store), before + 1);
    assert_eq!(store.get_kv("k").unwrap().as_deref(), Some("v2"));
}

#[test]
fn calibration_applies_only_the_part_of_a_residual_the_evidence_supports() {
    // (category, n, mean predicted, mean realized, residual variance).
    let rows = vec![
        ("thin".into(), 59, 10.0, 12.0, 0.01),                  // too few hands: reported, not applied
        ("noisy".into(), 200, 10.0, 10.1, 4.0),                 // gap inside 1.96 se: not applied
        ("solid".into(), 200, 10.0, 12.0, 0.04),                // 95%-supported part, shrunk by n
        ("negative".into(), 200, 12.0, 10.0, 0.04),             // overprediction lowers the candidate
        ("huge".into(), 10_000, 0.0, 20.0, 0.04),               // making an action better: capped at 3 bb
        ("overoptimistic".into(), 1_000, 80.0, 56.0, 64_000.0), // river:call shaped, well past 3 bb
        ("hopeless".into(), 10_000, 60.0, 20.0, 0.04),          // making an action worse: capped at 15 bb
    ];
    let pot_rows = vec![
        ("solid".into(), 200, 0.5, 0.01),
        ("overoptimistic".into(), 1_000, -0.07, 0.25), // river:call shaped: -7% of pot, se 0.0158
        ("hopeless".into(), 50, -0.9, 0.01),           // too few for a per-pot bound
    ];
    // Upward corrections need margin support: "solid" and "huge" have it in full.
    let margin_rows = vec![("solid".into(), 10_000, 5.0, 0.01), ("huge".into(), 10_000, 5.0, 0.01)];
    let (bias, caps, table) = super::calibration::calibration_update(rows, pot_rows, margin_rows);
    // A penalty is bounded by its supported per-pot residual; upward corrections get no cap.
    assert!((caps["overoptimistic"] - (0.07 + 1.96 * (0.25f64 / 1_000.0).sqrt())).abs() < 1e-9);
    assert!(!caps.contains_key("solid") && !caps.contains_key("hopeless"), "{caps:?}");
    assert!(!bias.contains_key("thin") && !bias.contains_key("noisy"));
    let supported = |resid: f64, var: f64, n: f64| ((resid.abs() - 1.96 * (var / n).sqrt()).max(0.0) * resid.signum()) * n / (n + 250.0);
    assert!((bias["solid"] - supported(2.0, 0.04, 200.0)).abs() < 1e-9);
    assert!((bias["negative"] - supported(-2.0, 0.04, 200.0)).abs() < 1e-9);
    assert_eq!(bias["huge"], 3.0);
    // The old flat 3 bb cap bound here; the supported part is now applied in full.
    let over = supported(-24.0, 64_000.0, 1_000.0);
    assert!((bias["overoptimistic"] - over).abs() < 1e-9, "{over}");
    assert!(bias["overoptimistic"] < -6.0 && bias["overoptimistic"] > -7.0);
    assert_eq!(bias["hopeless"], -15.0);
    // Every category is reported, and the pot-relative column stays a candidate, never applied.
    assert_eq!(table.len(), 7);
    assert_eq!(table["solid"]["pot_bias_candidate"].as_f64().unwrap(), 0.5 * 200.0 / 450.0);
    assert!(!bias.values().any(|v| v.is_nan()));
    // 0330: the table names the rule that set each size.
    let by = |c: &str| table[c]["bound_by"].as_str().unwrap().to_string();
    assert_eq!(
        [by("thin"), by("noisy"), by("solid"), by("huge"), by("hopeless"), by("overoptimistic")],
        ["too few samples", "inside its 95% band", "evidence, in full", "upward cap", "downward cap", "evidence, in full"]
    );
}

#[test]
fn an_upward_correction_is_bounded_by_its_margin_evidence() {
    // Live 2026-09-26 shapes: flop:bet:big +4.3 bb overall but +0.5 at the margin; preflop:call
    // +4.2 overall and +3.4 at the margin; river:call a penalty, untouched by the bound.
    let rows: Vec<sv10_store::store::CalibrationRow> = vec![
        ("flop:bet:big".into(), 46_704, 4.81, 9.10, 50.0),
        ("preflop:call".into(), 65_980, 0.69, 4.85, 50.0),
        ("river:call".into(), 1_554, 78.6, 58.0, 44_000.0),
        ("no_margin".into(), 5_000, 3.0, 9.0, 1.0),
    ];
    let margin_rows = vec![("flop:bet:big".into(), 9_800, 0.5, 900.0), ("preflop:call".into(), 43_000, 3.3, 3_300.0)];
    let (bias, _, table) = super::calibration::calibration_update(rows, Vec::new(), margin_rows);
    assert!(!bias.contains_key("flop:bet:big"), "margin residual inside its 95% band: no upward correction");
    let pc = bias["preflop:call"];
    assert!(pc > 2.5 && pc < 3.0, "the margin supports most of it: {pc}");
    assert!(bias["river:call"] < -6.0, "penalties are not bounded by the margin");
    assert!(!bias.contains_key("no_margin"), "no margin evidence, no upward correction");
    assert_eq!(table["flop:bet:big"]["margin_n"], 9_800);
    assert!(table["flop:bet:big"]["up_bound_bb"].as_f64().unwrap() == 0.0);
    // 0330: the +3 bb cap is not what holds these back; the margin is.
    assert_eq!(table["flop:bet:big"]["bound_by"], "margin evidence");
    assert_eq!(table["preflop:call"]["bound_by"], "margin evidence");
    assert_eq!(table["no_margin"]["bound_by"], "margin evidence");
}

// `fold_new_hands` and the watermarks it advances are exercised in `tasks/models.rs`, next to
// the code (0321).

#[test]
fn only_the_operator_stop_retires_a_bot() {
    // #744: the old `_ => break` retired a bot on any non-panic end; only `desired=stop` may.
    assert_eq!(super::after_session(&Ok(()), "stop"), super::AfterSession::Retire);
    assert_eq!(super::after_session(&Ok(()), "run"), super::AfterSession::Restart, "a return while running restarts");
    assert_eq!(super::after_session(&Ok(()), "pause"), super::AfterSession::Restart);
}

#[tokio::test]
async fn a_panicked_or_cancelled_session_restarts() {
    let panicked = tokio::spawn(async { panic!("session panicked") }).await;
    assert_eq!(super::after_session(&panicked, "run"), super::AfterSession::Restart);
    let cancelled = tokio::spawn(async { std::future::pending::<()>().await });
    cancelled.abort();
    let cancelled = cancelled.await;
    assert_eq!(super::after_session(&cancelled, "run"), super::AfterSession::Restart);
}
