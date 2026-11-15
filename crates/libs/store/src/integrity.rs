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
    inspect(path).map_err(|e| match e {
        Unchecked::Damaged(why) | Unchecked::Busy(why) | Unchecked::Unreadable(why) => why,
    })
}

/// Why a database did not pass its check.
#[derive(Debug)]
pub enum Unchecked {
    /// SQLite looked and reported damage, or could not read the file as a database at all.
    Damaged(String),
    /// Another connection held the file for the whole wait: nothing was learned about its contents.
    Busy(String),
    /// SQLite could not open or read the file for a reason that says nothing about its contents (no
    /// permission, an I/O error, a path it cannot reach): no verdict either, and never a reason to
    /// move the file aside (1012).
    Unreadable(String),
}

/// Sort a SQLite error into a verdict. Only an error that says the *contents* are not a database
/// is damage; busy, locked and everything else (`SQLITE_CANTOPEN` for a file the bot user cannot
/// open, `SQLITE_IOERR`, ...) leaves the file where it is.
fn sort(what: &str, e: &rusqlite::Error) -> Unchecked {
    use rusqlite::ErrorCode::{DatabaseBusy, DatabaseCorrupt, DatabaseLocked, NotADatabase};
    let why = format!("{what} failed: {e}");
    match e.sqlite_error_code() {
        Some(DatabaseBusy | DatabaseLocked) => Unchecked::Busy(why),
        Some(NotADatabase | DatabaseCorrupt) => Unchecked::Damaged(why),
        _ => Unchecked::Unreadable(why),
    }
}

/// How long a check waits for a peer holding the file (recovering the WAL, or migrating at a
/// hot swap) before it gives up without a verdict.
const CHECK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn inspect(path: &Path) -> std::result::Result<(), Unchecked> {
    check_within(path, CHECK_WAIT)
}

fn check_within(path: &Path, wait: std::time::Duration) -> std::result::Result<(), Unchecked> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| sort("open", &e))?;
    conn.busy_timeout(wait).map_err(|e| sort("open", &e))?;
    let rows: Vec<String> = conn
        .prepare("PRAGMA quick_check(5)")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| sort("check", &e))?;
    if rows.len() == 1 && rows[0] == "ok" { Ok(()) } else { Err(Unchecked::Damaged(rows.join("; "))) }
}

/// `PRAGMA quick_check(5)` on an open connection: Ok, or the problems joined.
pub fn check_connection(conn: &Connection) -> std::result::Result<(), String> {
    let rows: Vec<String> = conn
        .prepare("PRAGMA quick_check(5)")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| format!("check failed: {e}"))?;
    if rows.len() == 1 && rows[0] == "ok" { Ok(()) } else { Err(rows.join("; ")) }
}

/// `PRAGMA quick_check(5)` on an open connection, with a busy or locked result kept apart from
/// damage: a check that waited out its busy timeout has learned nothing about the file (#918).
pub fn check_reader(conn: &Connection) -> std::result::Result<(), Unchecked> {
    let rows: Vec<String> = conn
        .prepare("PRAGMA quick_check(5)")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| sort("check", &e))?;
    if rows.len() == 1 && rows[0] == "ok" { Ok(()) } else { Err(Unchecked::Damaged(rows.join("; "))) }
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
    let problem = match inspect(db) {
        Ok(()) => return Ok(Health::Healthy),
        // A locked file is not a damaged one: moving it aside would take a healthy database out from
        // under the peers that hold it and put an older backup in its place.
        Err(Unchecked::Busy(why) | Unchecked::Unreadable(why)) => {
            anyhow::bail!("could not check {}: {why}; it is left where it is", db.display())
        }
        Err(Unchecked::Damaged(why)) => why,
    };
    tracing::error!("database {} is damaged: {problem}", db.display());
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
    // The replacement is staged before the damaged file moves. Quarantining first and copying second
    // meant a copy that failed (a full disk is when both happen) left no database at all, and the
    // next start read that as a first start and opened an empty one without looking at a backup.
    let tmp = db.with_extension("db.restoring");
    let (mut staged, mut copy_error) = (None, None);
    for (_, backup) in candidates {
        if !verify_backup(&backup) {
            tracing::warn!("skipping unverified backup {}", backup.display());
            continue;
        }
        // Durable before it is named: a crash mid-restore must not leave a partial database in place.
        match sv10_rt::copy_durable(&backup, &tmp) {
            Ok(_) => {
                staged = Some(backup);
                break;
            }
            Err(e) => {
                tracing::error!("could not copy verified backup {}: {e}", backup.display());
                copy_error = Some(e);
            }
        }
    }
    let Some(from) = staged else {
        if let Some(e) = copy_error {
            anyhow::bail!("{} is damaged and no verified backup could be copied into place ({e}); it is left where it is", db.display());
        }
        let quarantined = quarantine(db)?;
        tracing::error!("no verified backup for {}; starting it empty", db.display());
        return Ok(Health::Quarantined { quarantined });
    };
    let quarantined = quarantine(db)?;
    std::fs::rename(&tmp, db)?;
    sv10_rt::sync_dir(db)?;
    tracing::error!("restored {} from verified backup {}", db.display(), from.display());
    Ok(Health::Restored { from, quarantined })
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
mod tests;
