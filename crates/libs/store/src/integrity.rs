//! Corruption defence for our SQLite databases: structural checks, backups verified by check and
//! SHA-256 sidecar, quarantine of damaged files, automatic restore from the newest verified backup,
//! and per-hand content digests for damage a structural check cannot see.

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::io::Read;
use std::path::{Path, PathBuf};
use sv10_digest::Sha256;

/// `PRAGMA quick_check` on a read-only connection: Ok, or the first problems SQLite reports.
pub fn quick_check(path: &Path) -> std::result::Result<(), String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open failed: {e}"))?;
    check_connection(&conn)
}

/// `PRAGMA quick_check(5)` on an open connection: Ok, or the problems joined.
pub fn check_connection(conn: &Connection) -> std::result::Result<(), String> {
    let rows: Vec<String> = conn
        .prepare("PRAGMA quick_check(5)")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| format!("check failed: {e}"))?;
    if rows.len() == 1 && rows[0] == "ok" { Ok(()) } else { Err(rows.join("; ")) }
}

/// Hex SHA-256 of a file, streamed in 1 MB chunks.
pub fn file_sha256(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(sv10_digest::hex(h.finalize()))
}

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".sha256");
    PathBuf::from(s)
}

/// Check a freshly written backup and record its hash; a backup that fails is deleted.
pub fn seal_backup(path: &Path) -> Result<()> {
    if let Err(e) = quick_check(path) {
        // What became of the file is said, not assumed (issue #326): a backup that could not be
        // removed is still the newest `.db` in the directory, which is the first one a restore reads.
        let gone = match sv10_rt::remove_stale_file(path) {
            Ok(()) => "was removed".to_string(),
            Err(err) => format!("is still there and could not be removed ({err})"),
        };
        anyhow::bail!("backup {} failed its check and {gone}: {e}", path.display());
    }
    // `VACUUM INTO` and `fs::copy` leave the file in the page cache: sync it first, so a disk that
    // cannot store it fails here instead of reading back as zeros later (0308).
    sv10_rt::sync_file(path)?;
    let tmp = sidecar(path).with_extension("sha256.tmp");
    std::fs::write(&tmp, file_sha256(path)?)?;
    sv10_rt::sync_file(&tmp)?;
    std::fs::rename(&tmp, sidecar(path))?;
    sv10_rt::sync_dir(path)?;
    Ok(())
}

/// A backup is trusted only if its sidecar hash matches (bit rot, partial copies) and it passes
/// the structural check.
pub fn verify_backup(path: &Path) -> bool {
    let Ok(expected) = std::fs::read_to_string(sidecar(path)) else { return false };
    file_sha256(path).map(|h| h == expected.trim()).unwrap_or(false) && quick_check(path).is_ok()
}

/// Outcome of [`ensure_healthy`].
#[derive(Debug, PartialEq)]
pub enum Health {
    /// The database passed its check.
    Healthy,
    /// No database file yet (first start).
    Missing,
    /// Damaged file moved aside and replaced by a verified backup.
    Restored {
        /// The backup copied into place.
        from: PathBuf,
        /// Where the damaged file was moved.
        quarantined: PathBuf,
    },
    /// Damaged file moved aside; no verified backup existed, so the database starts empty.
    Quarantined {
        /// Where the damaged file was moved.
        quarantined: PathBuf,
    },
}

/// Run before opening `db` for writing. Backups are files in `backups` whose name contains the
/// database's file stem and ends in `.db`; the newest verified one wins.
pub fn ensure_healthy(db: &Path, backups: &Path) -> Result<Health> {
    if !db.exists() {
        return Ok(Health::Missing);
    }
    let Err(problem) = quick_check(db) else { return Ok(Health::Healthy) };
    tracing::error!("database {} is damaged: {problem}", db.display());
    let quarantined = quarantine(db)?;
    let stem = db.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(backups)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "db") && p.file_name().is_some_and(|n| n.to_string_lossy().contains(&stem)))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    for (_, backup) in candidates {
        if !verify_backup(&backup) {
            tracing::warn!("skipping unverified backup {}", backup.display());
            continue;
        }
        let tmp = db.with_extension("db.restoring");
        // Durable before it is named: a crash mid-restore must not leave a partial database in place.
        sv10_rt::copy_durable(&backup, &tmp)?;
        std::fs::rename(&tmp, db)?;
        sv10_rt::sync_dir(db)?;
        tracing::error!("restored {} from verified backup {}", db.display(), backup.display());
        return Ok(Health::Restored { from: backup, quarantined });
    }
    tracing::error!("no verified backup for {}; starting it empty", db.display());
    Ok(Health::Quarantined { quarantined })
}

/// Move a database and its WAL/SHM files into `quarantine/` next to it.
fn quarantine(db: &Path) -> Result<PathBuf> {
    let dir = db.parent().unwrap_or(Path::new(".")).join("quarantine");
    std::fs::create_dir_all(&dir)?;
    let name = db.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "db".into());
    let target = dir.join(format!("{}-{name}", chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f")));
    std::fs::rename(db, &target)?;
    for suffix in ["-wal", "-shm"] {
        let mut from = db.as_os_str().to_owned();
        from.push(suffix);
        let mut to = target.as_os_str().to_owned();
        to.push(suffix);
        move_if_present(Path::new(&from), Path::new(&to))?;
    }
    Ok(target)
}

/// Rename `from` to `to`; a missing `from` is fine (no WAL or SHM), any other failure is not: a
/// WAL left behind next to a restored backup would be replayed into it.
fn move_if_present(from: &Path, to: &Path) -> std::io::Result<()> {
    // Checked up front: `rename` reports a missing destination directory as NotFound too.
    if std::fs::symlink_metadata(from).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(());
    }
    std::fs::rename(from, to)
}

/// Digest of a stored hand's immutable content (the net is filled later, so it is excluded).
pub fn hand_digest(bot: &str, hand_id: &str, ended_at: &str, hole: &str, board: &str, summary: &str) -> String {
    let mut h = Sha256::new();
    for part in [bot, hand_id, ended_at, hole, board, summary] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    sv10_digest::hex(&h.finalize()[..16])
}

#[cfg(test)]
mod tests {

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
}
