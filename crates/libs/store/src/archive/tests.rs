use super::*;
use chrono::TimeZone;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sv10-archive-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A manifest is sealed against rot, not against a name that says `../…` (#603).
///
/// The seal is a sidecar in the same directory, so an archive someone else produced can name any
/// path it likes: without confinement the restore writes there and reports it as a restored file.
#[test]
fn a_manifest_entry_that_leaves_the_archive_is_refused() {
    let dir = tmp("escape");
    let archive = dir.join("weekly/2026-W40");
    std::fs::create_dir_all(&archive).unwrap();
    let payload = b"an entry the manifest points at from outside";
    std::fs::write(archive.join("payload.bin"), payload).unwrap();
    let entry = Entry {
        name: "../written-by-restore.txt".into(),
        role: Role::RepoBundle,
        bytes: payload.len() as u64,
        sha256: crate::integrity::file_sha256(&archive.join("payload.bin")).unwrap(),
        raw_bytes: payload.len() as u64,
        raw_sha256: String::new(),
        rows: BTreeMap::new(),
        watermarks: BTreeMap::new(),
    };
    let manifest = Manifest {
        format: FORMAT,
        kind: Kind::Weekly,
        name: "weekly/2026-W40".into(),
        created_at: "2026-09-30T00:00:00Z".into(),
        app_version: "test".into(),
        git_commit: None,
        base: None,
        restore: "copy the files into artifacts/".into(),
        files: vec![entry],
    };
    std::fs::write(archive.join(MANIFEST), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    std::fs::write(archive.join(format!("{MANIFEST}.sha256")), crate::integrity::file_sha256(&archive.join(MANIFEST)).unwrap()).unwrap();

    // The seal is intact, so this is not a corruption case: the name itself has to be refused.
    assert!(load_manifest(&archive).is_ok());
    let problem = verify(&archive, false).join("; ");
    assert!(problem.contains("leaves the archive"), "verify says nothing about an escaping name: {problem}");

    let out = dir.join("out");
    let escaped = dir.join("written-by-restore.txt");
    restore(&dir, "weekly/2026-W40", &out).expect_err("a restore must refuse an entry that leaves the archive");
    assert!(!escaped.exists(), "the restore wrote outside its destination");
}

/// 0249: `SELECT *` maps by position. A column added or reordered between the full backup and
/// the differential would fill the wrong columns with no error; naming both sides fixes that,
/// and a column the full does not have is refused rather than skipped.
#[test]
fn a_differential_merges_by_column_name_and_refuses_an_unknown_column() {
    let dir = tmp("delta-columns");
    let full = dir.join("full.db");
    let delta = dir.join("delta.db");
    {
        let c = Connection::open(&full).unwrap();
        c.execute_batch("CREATE TABLE hands (id INTEGER PRIMARY KEY, net INTEGER, digest TEXT);").unwrap();
        // The full carries a column the differential predates: it must keep its value.
        c.execute("INSERT INTO hands (id, net, digest) VALUES (1, 100, 'a')", []).unwrap();
        c.execute("INSERT INTO hands (id, net, digest) VALUES (2, 200, 'b')", []).unwrap();
    }
    {
        let c = Connection::open(&delta).unwrap();
        c.execute_batch("CREATE TABLE hands (id INTEGER PRIMARY KEY, net INTEGER, digest TEXT);").unwrap();
        // Rows above the watermark, plus one the full already has: a repeated restore must not
        // double a row or overwrite it.
        c.execute("INSERT INTO hands (id, net, digest) VALUES (2, 999, 'B')", []).unwrap();
        c.execute("INSERT INTO hands (id, net, digest) VALUES (3, 300, 'c')", []).unwrap();
    }
    // `hands` has an id key, so the delta *adds* rows rather than replacing the table.
    let marks = BTreeMap::from([("hands".to_string(), 1i64)]);
    apply_delta(&full, &delta, &marks).unwrap();
    let c = Connection::open(&full).unwrap();
    let rows: Vec<(i64, i64, String)> = c
        .prepare("SELECT id, net, digest FROM hands ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        rows,
        [(1, 100, "a".into()), (2, 200, "b".into()), (3, 300, "c".into())],
        "the new row is added; a repeated one is ignored, so a restore is idempotent"
    );

    // A column the restored full does not have is an error, not a silent skip.
    let drift = dir.join("drift.db");
    {
        let c = Connection::open(&drift).unwrap();
        c.execute_batch("CREATE TABLE hands (id INTEGER PRIMARY KEY, net INTEGER, ev_net REAL);").unwrap();
        c.execute("INSERT INTO hands VALUES (2, 1.5, 1.5)", []).unwrap();
    }
    let err = apply_delta(&full, &drift, &marks).unwrap_err().to_string();
    assert!(err.contains("ev_net"), "the missing column is named: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 0248: table names reach these statements out of an archive — an external file — and were
/// interpolated into `"..."` without escaping, so a name holding a quote (a legal SQLite
/// identifier) broke the statement, and a crafted name would have closed it. Doubling the
/// quote is SQLite's own rule, so such a name merges like any other.
#[test]
fn a_quoted_table_name_from_an_archive_merges_instead_of_breaking_the_statement() {
    assert_eq!(ident("hands"), "\"hands\"");
    assert_eq!(ident("a\"b"), "\"a\"\"b\"");
    assert_eq!(ident("x\"; DROP TABLE kv; --"), "\"x\"\"; DROP TABLE kv; --\"");
    let dir = tmp("ident");
    let full = dir.join("full.db");
    let delta = dir.join("delta.db");
    for (path, rows) in [(&full, 0), (&delta, 1)] {
        let c = Connection::open(path).unwrap();
        c.execute_batch("CREATE TABLE kv (k TEXT); INSERT INTO kv VALUES ('keep');").unwrap();
        // The legal identifier `a"b`; the old `"a"b"` interpolation is a syntax error, and a
        // longer crafted name would be executable SQL rather than a table name.
        c.execute("CREATE TABLE \"a\"\"b\" (x)", []).unwrap();
        for i in 0..rows {
            c.execute("INSERT INTO \"a\"\"b\" (x) VALUES (?1)", [i]).unwrap();
        }
    }
    apply_delta(&full, &delta, &BTreeMap::new()).unwrap();
    let c = Connection::open(&full).unwrap();
    let kept: String = c.query_row("SELECT k FROM kv", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, "keep", "the merge touched only its own table");
    let n: i64 = c.query_row("SELECT COUNT(*) FROM \"a\"\"b\"", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1, "and its row was merged like any other");
    let _ = std::fs::remove_dir_all(&dir);
}

fn zstd_available() -> bool {
    Command::new("zstd").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn history(path: &Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS raw (id INTEGER PRIMARY KEY AUTOINCREMENT, hand_id TEXT NOT NULL UNIQUE, json TEXT NOT NULL, summary TEXT);
         CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .unwrap();
    c
}

fn add_hands(c: &Connection, from: usize, to: usize) {
    for i in from..to {
        c.execute("INSERT INTO raw(hand_id, json) VALUES (?1, ?2)", [format!("h{i}"), format!("{{\"n\":{i}}}")]).unwrap();
    }
}

fn at(d: u32, h: u32) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 9, d, h, 0, 0).unwrap()
}

#[test]
fn weekly_daily_monthly_restore_and_prune() {
    if !zstd_available() {
        eprintln!("zstd not installed; skipping");
        return;
    }
    let d = tmp("cycle");
    let root = d.join("archive");
    let src = Sources { live: d.join("live.db"), history: d.join("history.db"), repo: None, app_version: "test".into() };
    let live = Connection::open(&src.live).unwrap();
    live.execute_batch("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT); INSERT INTO kv VALUES ('params', 'a');").unwrap();
    let h = history(&src.history);
    add_hands(&h, 0, 50);
    h.execute("INSERT INTO meta VALUES ('cursor', '1')", []).unwrap();

    // Monday 14 Sep: the week's full, and the month's archive from it.
    let (made, _) = run(&root, &src, at(14, 3), Retention::default()).unwrap();
    assert_eq!(made, vec!["weekly/2026-W38".to_string(), "monthly/2026-09".to_string()]);
    let week = load_manifest(&root.join("weekly/2026-W38")).unwrap();
    let full = week.files.iter().find(|e| e.role == Role::HistoryFull).unwrap();
    assert_eq!(full.watermarks.get("raw"), Some(&50));

    // Next day: more hands, an updated cursor and live row; a daily differential.
    add_hands(&h, 50, 80);
    h.execute("UPDATE meta SET value = '2' WHERE key = 'cursor'", []).unwrap();
    live.execute("UPDATE kv SET v = 'b'", []).unwrap();
    let (made, _) = run(&root, &src, at(15, 3), Retention::default()).unwrap();
    assert_eq!(made, vec!["daily/2026-09-15".to_string()]);
    let daily = load_manifest(&root.join("daily/2026-09-15")).unwrap();
    let delta = daily.files.iter().find(|e| e.role == Role::HistoryDelta).unwrap();
    assert_eq!(delta.rows.get("raw"), Some(&80));
    // Idempotent: nothing more is due the same day.
    assert!(run(&root, &src, at(15, 9), Retention::default()).unwrap().0.is_empty());

    // Restoring the daily rebuilds 80 hands, the new cursor and the new live row.
    let out = d.join("restored");
    restore(&root, "daily/2026-09-15", &out).unwrap();
    let r = Connection::open(out.join("history.db")).unwrap();
    assert_eq!(r.query_row("SELECT COUNT(*), MAX(id) FROM raw", [], |x| Ok((x.get::<_, i64>(0)?, x.get::<_, i64>(1)?))).unwrap(), (80, 80));
    assert_eq!(r.query_row("SELECT value FROM meta", [], |x| x.get::<_, String>(0)).unwrap(), "2");
    let l = Connection::open(out.join("svanbot10.db")).unwrap();
    assert_eq!(l.query_row("SELECT v FROM kv", [], |x| x.get::<_, String>(0)).unwrap(), "b");

    // The monthly restores the weekly content exactly.
    let out = d.join("restored-monthly");
    restore(&root, "monthly/2026-09", &out).unwrap();
    let r = Connection::open(out.join("history.db")).unwrap();
    assert_eq!(r.query_row("SELECT COUNT(*) FROM raw", [], |x| x.get::<_, i64>(0)).unwrap(), 50);
    assert!(verify(&root.join("monthly/2026-09"), true).is_empty());

    // Bit rot is caught by verify and refused by restore.
    let file = root.join("daily/2026-09-15/history-delta.db.zst");
    let mut bytes = std::fs::read(&file).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    assert!(!verify(&root.join("daily/2026-09-15"), false).is_empty());
    assert!(restore(&root, "daily/2026-09-15", &d.join("bad")).is_err());

    // Pruning keeps a weekly that a kept daily depends on.
    let removed = prune(&root, Retention { daily: 1, weekly: 0, monthly: 0 }).unwrap();
    assert_eq!(removed, vec!["monthly/2026-09".to_string()]);
    assert!(root.join("weekly/2026-W38").exists());
    let removed = prune(&root, Retention { daily: 0, weekly: 0, monthly: 0 }).unwrap();
    assert_eq!(removed, vec!["daily/2026-09-15".to_string(), "weekly/2026-W38".to_string()]);
}

#[test]
fn a_new_week_starts_a_new_full_and_stale_staging_is_cleared() {
    if !zstd_available() {
        return;
    }
    let d = tmp("week");
    let root = d.join("archive");
    let src = Sources { live: d.join("live.db"), history: d.join("history.db"), repo: None, app_version: "test".into() };
    Connection::open(&src.live).unwrap().execute_batch("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT);").unwrap();
    add_hands(&history(&src.history), 0, 5);
    std::fs::create_dir_all(root.join("daily/.tmp-2026-09-19")).unwrap();
    run(&root, &src, at(19, 3), Retention::default()).unwrap();
    assert!(!root.join("daily/.tmp-2026-09-19").exists());
    let (made, _) = run(&root, &src, at(21, 3), Retention::default()).unwrap();
    assert_eq!(made, vec!["weekly/2026-W39".to_string()]);
    assert_eq!(list(&root, Kind::Weekly), vec!["weekly/2026-W38".to_string(), "weekly/2026-W39".to_string()]);
}

/// Issue #326: the removal of a stale staging directory had its result dropped, so one that would not
/// go was written into instead — `create_dir_all` succeeds on a directory that is still there, and the
/// weekly would be staged and verified on top of whatever the interrupted run left in it.
#[cfg(unix)]
#[test]
fn a_staging_directory_that_will_not_go_stops_the_run_before_it_is_written_into() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("stubborn-stage");
    let root = d.join("archive");
    let src = Sources { live: d.join("live.db"), history: d.join("history.db"), repo: None, app_version: "test".into() };
    let stale = root.join("weekly/.tmp-2026-W38");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("staged-before-the-crash.db"), b"a run that never finished").unwrap();
    // Read and traverse, but no write: `remove_dir_all` cannot unlink what is inside it.
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o500)).unwrap();
    let err = run(&root, &src, at(19, 3), Retention::default()).unwrap_err().to_string();
    assert!(err.contains(".tmp-2026-W38"), "the error has to name the staging directory it could not clear: {err}");
    assert!(stale.join("staged-before-the-crash.db").exists(), "nothing may be staged into a directory that outlived its removal");
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}

/// #772: an archive is data only. A weekly written from a repository carries the two databases and the
/// commit id, and no code bundle; one written before the rule (a `repo.bundle` entry) still restores its bundle.
#[test]
fn a_weekly_carries_data_and_the_commit_but_no_code_bundle() {
    let dir = tmp("data-only");
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let (live, history) = (dir.join("live.db"), dir.join("history.db"));
    for db in [&live, &history] {
        Connection::open(db).unwrap().execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1);").unwrap();
    }
    let src = Sources { live, history, repo: Some(repo.clone()), app_version: "test".into() };
    let root = dir.join("archive");
    let now = chrono::Utc.with_ymd_and_hms(2026, 10, 5, 4, 30, 0).unwrap();
    let manifest = make_weekly(&root, "weekly/2026-W41", &src, now).unwrap();
    assert!(manifest.files.iter().all(|f| f.role != Role::RepoBundle), "{:?}", manifest.files);
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(manifest.git_commit.as_deref().map(str::len), Some(40), "the commit to install is recorded");
    assert!(!root.join("weekly/2026-W41/repo.bundle").exists());
    assert!(verify(&root.join("weekly/2026-W41"), true).is_empty());
    let out = dir.join("out");
    restore(&root, "weekly/2026-W41", &out).unwrap();
    assert!(out.join("svanbot10.db").is_file() && out.join("history.db").is_file() && !out.join("repo.bundle").exists());
}
