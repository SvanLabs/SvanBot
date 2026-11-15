#[test]
fn side_files_move_when_present_and_a_missing_one_is_fine() {
    let dir = std::env::temp_dir().join(format!("sv10-move-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (from, to) = (dir.join("a.db-wal"), dir.join("b.db-wal"));
    assert!(move_if_present(&from, &to).is_ok(), "absent WAL");
    std::fs::write(&from, b"wal").unwrap();
    move_if_present(&from, &to).unwrap();
    assert!(!from.exists() && to.exists());
    // Any other failure surfaces: moving into a directory that does not exist.
    std::fs::write(&from, b"wal").unwrap();
    assert!(move_if_present(&from, &dir.join("missing").join("c")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

use super::*;
use crate::store::{HandRow, Store};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sv10-integrity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hand(id: &str) -> HandRow {
    HandRow {
        bot: "A".into(),
        hand_id: id.into(),
        table_id: "t".into(),
        ended_at: "2026-09-15T00:00:00Z".into(),
        hero_seat: Some(0),
        hole: "AsKd".into(),
        board: String::new(),
        pot: 30,
        net: Some(10),
        winners: String::new(),
        summary: "{}".into(),
        showdown: false,
    }
}

/// Overwrite the SQLite header so the file is no longer a database.
fn smash_header(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(&[0xAB; 100]).unwrap();
}

fn hand_ids(db: &Path) -> Vec<String> {
    Store::open(db).unwrap().recent_hands_light("A", 100).unwrap().into_iter().map(|h| h.hand_id).collect()
}

#[test]
fn healthy_and_missing_databases_are_left_alone() {
    let d = dir("healthy");
    let db = d.join("svanbot10.db");
    assert_eq!(ensure_healthy(&db, &d.join("backups")).unwrap(), Health::Missing);
    Store::open(&db).unwrap().insert_hand(&hand("h1")).unwrap();
    assert_eq!(ensure_healthy(&db, &d.join("backups")).unwrap(), Health::Healthy);
}

#[test]
fn damaged_database_is_restored_from_newest_verified_backup() {
    let d = dir("restore");
    let (db, backups) = (d.join("svanbot10.db"), d.join("backups"));
    let store = Store::open(&db).unwrap();
    store.insert_hand(&hand("old")).unwrap();
    let good = backups.join("svanbot10-2026091400.db");
    store.backup_to(&good).unwrap();
    seal_backup(&good).unwrap();
    store.insert_hand(&hand("new")).unwrap();
    // A newer backup whose bytes rotted after sealing must be skipped.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let rotten = backups.join("svanbot10-2026091401.db");
    store.backup_to(&rotten).unwrap();
    seal_backup(&rotten).unwrap();
    smash_header(&rotten);
    drop(store);
    smash_header(&db);

    match ensure_healthy(&db, &backups).unwrap() {
        Health::Restored { from, quarantined } => {
            assert_eq!(from, good);
            assert!(quarantined.exists());
        }
        other => panic!("expected a restore, got {other:?}"),
    }
    assert_eq!(hand_ids(&db), vec!["old".to_string()]);
}

#[test]
fn damaged_database_without_backup_is_quarantined_and_starts_empty() {
    let d = dir("quarantine");
    let db = d.join("history.db");
    Store::open(&db).unwrap().insert_hand(&hand("h1")).unwrap();
    smash_header(&db);
    assert!(matches!(ensure_healthy(&db, &d.join("backups")).unwrap(), Health::Quarantined { .. }));
    assert!(!db.exists());
    assert!(hand_ids(&db).is_empty());
}

/// A backup that fails its check is deleted, and the error has to say which of the two happened:
/// one that is still there is the newest `.db` in the directory, so a message claiming a removal
/// that did not happen sends the operator looking in the wrong place (issue #326).
#[cfg(unix)]
#[test]
fn a_backup_that_fails_its_check_says_whether_the_file_went() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("seal-message");
    let held = d.join("held");
    std::fs::create_dir_all(&held).unwrap();
    let path = held.join("svanbot10-2026091500.db");
    std::fs::write(&path, b"not a database").unwrap();
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o500)).unwrap();
    let err = seal_backup(&path).unwrap_err().to_string();
    assert!(path.exists() && err.contains("could not be removed"), "claimed a removal that did not happen: {err}");
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o700)).unwrap();
    let err = seal_backup(&path).unwrap_err().to_string();
    assert!(!path.exists() && err.contains("was removed"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// The backup copy is staged before the damaged file moves (#876): when the copy fails, the
/// database is still there for the next attempt instead of being read as a first start.
#[test]
fn a_backup_that_cannot_be_copied_leaves_the_damaged_database_in_place() {
    let d = dir("copy-fails");
    let (db, backups) = (d.join("svanbot10.db"), d.join("backups"));
    let store = Store::open(&db).unwrap();
    store.insert_hand(&hand("old")).unwrap();
    let good = backups.join("svanbot10-2026091400.db");
    store.backup_to(&good).unwrap();
    seal_backup(&good).unwrap();
    drop(store);
    smash_header(&db);
    // A directory where the staged copy goes makes the copy fail, as a full disk would.
    std::fs::create_dir_all(db.with_extension("db.restoring")).unwrap();

    let err = ensure_healthy(&db, &backups).unwrap_err().to_string();
    assert!(err.contains("left where it is"), "{err}");
    assert!(db.exists(), "the damaged database was moved away with nothing to replace it");
    assert!(!d.join("quarantine").exists());
    // With the obstacle gone the same call restores it.
    std::fs::remove_dir_all(db.with_extension("db.restoring")).unwrap();
    assert!(matches!(ensure_healthy(&db, &backups).unwrap(), Health::Restored { .. }));
    assert_eq!(hand_ids(&db), vec!["old".to_string()]);
}

/// A database a peer holds locked is not a damaged one (#876): the check reports that it could
/// not look, and the file stays where it is.
#[test]
fn a_locked_database_is_not_quarantined() {
    let d = dir("locked");
    let db = d.join("svanbot10.db");
    Store::open(&db).unwrap().insert_hand(&hand("h1")).unwrap();
    let holder = Connection::open(&db).unwrap();
    holder.execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; CREATE TABLE held (x);").unwrap();

    assert!(matches!(check_within(&db, std::time::Duration::from_millis(50)), Err(Unchecked::Busy(_))));
    drop(holder);
    assert_eq!(ensure_healthy(&db, &d.join("backups")).unwrap(), Health::Healthy);
    assert!(!d.join("quarantine").exists());
}

#[test]
fn edited_hand_content_fails_its_digest() {
    let d = dir("digest");
    let store = Store::open(&d.join("svanbot10.db")).unwrap();
    store.insert_hand(&hand("h1")).unwrap();
    store.insert_hand(&hand("h2")).unwrap();
    assert_eq!(store.verify_hand_digests().unwrap(), (2, vec![]));
    store.corrupt_summary_for_test("h2");
    assert_eq!(store.verify_hand_digests().unwrap(), (2, vec![("A".to_string(), "h2".to_string())]));
}

#[test]
fn live_check_finishes_while_the_writer_is_held() {
    // The check of every page ran on the writer, so the store's write lock was held for its whole
    // length and every other write waited (#918). A reader must do the check instead.
    let d = dir("reader-check");
    let store = std::sync::Arc::new(Store::open(&d.join("svanbot10.db")).unwrap());
    let held = store.write_lock();
    let (tx, rx) = std::sync::mpsc::channel();
    let checker = std::sync::Arc::clone(&store);
    std::thread::spawn(move || tx.send(checker.quick_check()).unwrap());
    let verdict = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("quick_check waited for the writer");
    assert!(verdict.is_ok(), "{verdict:?}");
    drop(held);
    std::fs::remove_dir_all(&d).unwrap();
}

/// A database the process cannot open is not a damaged one: `SQLITE_CANTOPEN` says nothing about the
/// contents, so the file stays where it is instead of being quarantined behind an older backup (1012).
#[test]
fn an_unreadable_database_is_not_quarantined() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("unreadable");
    let db = d.join("svanbot10.db");
    Store::open(&db).unwrap().insert_hand(&hand("h1")).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::File::open(&db).is_ok();
    if !readable {
        assert!(matches!(check_within(&db, std::time::Duration::from_millis(50)), Err(Unchecked::Unreadable(_))));
        assert!(ensure_healthy(&db, &d.join("backups")).is_err(), "no verdict, so the caller is told it could not check");
        assert!(db.exists() && !d.join("quarantine").exists(), "the file must stay where it is");
    }
    // Running as root, mode 000 does not stop the open: nothing to prove here.
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(ensure_healthy(&db, &d.join("backups")).unwrap(), Health::Healthy);
    std::fs::remove_dir_all(&d).unwrap();
}
