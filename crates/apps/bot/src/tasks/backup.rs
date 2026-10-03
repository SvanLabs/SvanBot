//! Hourly/daily backups, rotation, seals and mirrors (0256).

use super::INTEGRITY_STATUS_KEY;
use crate::MODELS_KEY;
use crate::live::Shared;
use anyhow::Result;

/// Free-space guard: refuse to back up when the disk cannot hold three more copies.
fn backup_space_available(shared: &Shared) -> bool {
    // Never let backups fill the disk: require free space for three more copies.
    let db_size = std::fs::metadata(shared.config.artifacts.join("svanbot10.db")).map(|m| m.len()).unwrap_or(0);
    if let Some(free) = free_bytes(&shared.config.artifacts)
        && free < db_size.saturating_mul(3).max(512 * 1024 * 1024)
    {
        tracing::warn!("skipping database backup: only {} MB free", free / 1_048_576);
        return false;
    }
    true
}

/// Integrity of the live database, before any rotation: a failed structural check exits for
/// restore (the supervisor's restart runs the startup check); digest mismatches are logged but
/// never block the backup. Returns (hands digest-checked, mismatches).
fn verify_live_database(shared: &Shared) -> (i64, i64) {
    // Never rotate good backups out behind a copy of a damaged database: stop instead, and let the
    // supervisor's restart run the startup check, which restores the newest verified backup.
    if let Err(problem) = shared.store.quick_check() {
        tracing::error!("live database failed its integrity check ({problem}); exiting for restore");
        shared.log("fleet", "error", format!("database integrity check failed: {problem}; restarting to restore"));
        std::process::exit(70);
    }
    let digests = shared.store.verify_hand_digests();
    let (digest_checked, digest_bad) = match &digests {
        Ok((n, bad)) => (*n as i64, bad.len() as i64),
        Err(_) => (-1, -1),
    };
    match digests {
        Ok((_, bad)) if !bad.is_empty() => {
            tracing::error!("{} stored hands no longer match their digest: {:?}", bad.len(), &bad[..bad.len().min(5)]);
            shared.log("fleet", "error", format!("{} stored hands failed their content digest", bad.len()));
        }
        Err(e) => tracing::warn!("hand digest verification failed to run: {e}"),
        _ => {}
    }
    (digest_checked, digest_bad)
}

/// Drop rows the backup must not carry forward: queued audits past a day, old audit results,
/// expired replay records and scan snapshots no longer being read.
fn prune_backup_sources(shared: &Shared) {
    // Queued audits are dropped after a day (the analyst was not running); results are kept
    // `AUDIT_RESULT_DAYS`, which is also the longest window the findings scan measures a rare class
    // over (0345) — shortening one without the other deletes that scan's evidence.
    match shared.store.prune_audits(1, sv10_store::store::AUDIT_RESULT_DAYS) {
        Ok(n) if n > 0 => tracing::info!("pruned {n} old decision audit rows"),
        Err(e) => tracing::warn!("pruning decision audits failed: {e}"),
        _ => {}
    }
    match shared.store.prune_replays(crate::replay::KEEP_DAYS) {
        Ok(n) if n > 0 => tracing::info!("pruned {n} replay records older than {} days", crate::replay::KEEP_DAYS),
        Err(e) => tracing::warn!("pruning replay records failed: {e}"),
        _ => {}
    }
    // The scan's own inputs and outputs (0356) ride this backup: nothing else prunes them, and the
    // retention is the store's own, asserted to cover the window it summarizes.
    match shared.store.prune_scan_snapshots(sv10_store::store::SCAN_SNAPSHOT_DAYS) {
        Ok(n) if n > 0 => tracing::info!("pruned {n} scan snapshots no scan has read for {} days", sv10_store::store::SCAN_SNAPSHOT_DAYS),
        Err(e) => tracing::warn!("pruning scan snapshots failed: {e}"),
        _ => {}
    }
}

/// Sealed hourly copy of the live database; `None` (after a warning) when the copy or seal fails.
fn write_hourly_backup(shared: &Shared, dir: &std::path::Path, now: &chrono::DateTime<chrono::Utc>) -> Option<std::path::PathBuf> {
    let hourly = dir.join(format!("svanbot10-{}.db", now.format("%Y%m%d%H")));
    if let Err(e) = shared.store.backup_to(&hourly).and_then(|_| sv10_store::integrity::seal_backup(&hourly)) {
        tracing::warn!("database backup failed: {e}");
        return None;
    }
    Some(hourly)
}

/// One daily copy per calendar day, taken from the hourly file.
fn ensure_daily_backup(dir: &std::path::Path, hourly: &std::path::Path, now: &chrono::DateTime<chrono::Utc>) {
    let daily = dir.join(format!("daily-svanbot10-{}.db", now.format("%Y%m%d")));
    // An interrupted copy or failed seal can leave the name occupied without a restorable
    // daily backup. Retry from this hour until the existing pair actually verifies.
    if !sv10_store::integrity::verify_backup(&daily) {
        let copied = std::fs::copy(hourly, &daily).map_err(anyhow::Error::from).and_then(|_| sv10_store::integrity::seal_backup(&daily));
        if let Err(e) = copied {
            tracing::warn!("daily backup copy failed: {e}");
        }
    }
}

/// Hourly copies kept on the SSD (`SVANBOT_HOURLY_BACKUPS`, default 3, or 2 when the hourlies are
/// mirrored): at ~630 MB each that is under 2 GB, and anything older is covered by the nightly
/// archive.
pub(super) fn hourly_backups_kept(mirrored: bool) -> usize {
    // Three on the SSD: about 1.7 GB, and anything older lives in the nightly archive. With the
    // opt-in mirror (a second disk, or the dedicated same-disk folder of #725), two.
    std::env::var("SVANBOT_HOURLY_BACKUPS").ok().and_then(|v| v.parse().ok()).unwrap_or(if mirrored { 2 } else { 3 }).clamp(2, 48)
}

/// Hourly copies kept on the second disk, or `None` for no mirror: the mirror is opt-in
/// (`SVANBOT_MIRROR_HOURLY_BACKUPS=24` turns it on) and off by default. With it off the SSD keeps
/// its few hourlies, and the nightly archive still puts daily, weekly and monthly copies on the
/// second disk.
pub(super) fn mirror_hourly_kept() -> Option<usize> {
    mirror_setting(std::env::var("SVANBOT_MIRROR_HOURLY_BACKUPS").ok().as_deref())
}

/// [`mirror_hourly_kept`] over the setting's text: unset, empty, `0` or unreadable is off.
pub(super) fn mirror_setting(value: Option<&str>) -> Option<usize> {
    value.and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n > 0).map(|n| n.clamp(2, 168))
}

/// Where hourly copies are mirrored (0229, #725): `<archive dir>/hourly`, a folder of its own. On
/// another disk it is a disk-loss copy; when no second device exists the same-disk folder is still
/// accepted — it survives deletion and rotation mistakes — and the step reports [`Mirror::SameDisk`]
/// rather than `ok`. `None` when the archive directory or `artifacts/` cannot be read.
pub(super) fn backup_mirror(shared: &Shared) -> Option<(std::path::PathBuf, bool)> {
    use std::os::unix::fs::MetadataExt;
    let dir = shared.config.archive_dir.join("hourly");
    // Both reads must land: an unreadable directory is no mirror, not a guess (the same-disk flag
    // is the second element).
    let (archive_dev, artifacts_dev) =
        (std::fs::metadata(&shared.config.archive_dir).ok()?.dev(), std::fs::metadata(&shared.config.artifacts).ok()?.dev());
    Some((dir, archive_dev == artifacts_dev))
}

/// How the mirror step ended for the hour just written (0292, #725). The status row the dashboard
/// reads carries this, so a mirror that stops working shows up there instead of only in a log line.
#[derive(Debug, Clone)]
pub(super) enum Mirror {
    /// The mirror is switched off (the default since 2026-09-27).
    Off,
    /// No mirror folder could be resolved: the archive directory or `artifacts/` is unreadable (0229).
    Absent,
    /// The pair is on the second disk.
    Copied,
    /// The pair is in `<archive>/hourly` on the same disk as the databases (#725): it guards against
    /// deletion and rotation mistakes, not the loss of the disk, so it is never reported as `ok`.
    SameDisk,
    /// Deliberately skipped: the mirror folder lacks room for three more copies and 1 GB.
    NoRoom,
    /// The mirror failed; the reason names the step that failed.
    Failed(String),
}

impl Mirror {
    fn json(&self, dir: Option<&std::path::Path>, at: i64) -> serde_json::Value {
        let (state, error) = match self {
            Mirror::Off => ("off", None),
            Mirror::Absent => ("absent", None),
            Mirror::Copied => ("ok", None),
            Mirror::SameDisk => ("same_disk", None),
            Mirror::NoRoom => ("no_room", None),
            Mirror::Failed(e) => ("failed", Some(e.as_str())),
        };
        serde_json::json!({"state": state, "at": at, "dir": dir.map(|d| d.display().to_string()), "error": error})
    }
}

/// `path` plus its seal's extension, as `sv10_store::integrity` names sidecars.
fn sidecar(path: &std::path::Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".sha256");
    std::path::PathBuf::from(s)
}

fn discard(paths: &[&std::path::Path]) {
    for p in paths {
        // Best-effort: the caller is already returning an error, and the next run's
        // `clear_stale_temporaries` collects whatever is left here and warns about it (0292).
        let _ = std::fs::remove_file(p);
    }
}

/// Move a sealed hourly backup (and its sidecar) into the mirror folder (the second disk, or the
/// dedicated same-disk folder), then keep the newest `keep` there. Skipped when the mirror lacks
/// room for three more copies and 1 GB.
///
/// The pair used to be copied to a temporary name, read back against its seal and renamed. The
/// second disk returns `EUCLEAN` (os error 117, "structure needs cleaning") to that dance — 22
/// times over 2026-09-26/27 — and the operator's call (2026-09-28) is to stop guarding the hour
/// with it: the two files go straight to the names a restore reads, in one step, and a failure
/// names the step and leaves the SSD pair where it is (#374, 0292).
pub(super) fn mirror_backup(hourly: &std::path::Path, mirror: &std::path::Path, keep: usize) -> Result<bool> {
    std::fs::create_dir_all(mirror)?;
    let size = std::fs::metadata(hourly)?.len();
    if sv10_rt::free_bytes(mirror).is_some_and(|free| free < size.saturating_mul(3) + (1 << 30)) {
        tracing::warn!("hourly backup not mirrored: {} lacks room", mirror.display());
        return Ok(false);
    }
    let name = hourly.file_name().ok_or_else(|| anyhow::anyhow!("backup path has no name"))?;
    let (to, side_to) = (mirror.join(name), sidecar(&mirror.join(name)));
    let copied = sv10_rt::copy_durable(hourly, &to)
        .and_then(|_| sv10_rt::copy_durable(&sidecar(hourly), &side_to))
        .map_err(|e| anyhow::anyhow!("moving {} to {} failed: {e:#}", hourly.display(), to.display()));
    if let Err(e) = copied {
        // A file under the real name with half its bytes reads as this hour's backup, so it goes
        // before the error is returned: the SSD pair is untouched and the next attempt lands clean.
        discard(&[&to, &side_to]);
        return Err(e);
    }
    sv10_rt::sync_dir(&to)?;
    // The move's second half. A source that cannot be removed leaves a second copy, not a lost
    // backup, so it is a warning rather than a failed mirror.
    for p in [hourly, &sidecar(hourly)] {
        if let Err(e) = std::fs::remove_file(p) {
            tracing::warn!("mirrored pair copied but {} stays on the SSD: {e}", p.display());
        }
    }
    rotate_backups_keeping(mirror, keep, usize::MAX);
    Ok(true)
}

/// Daily copies kept on the SSD (`SVANBOT_DAILY_BACKUPS`, default 1): the nightly archive keeps
/// compressed daily, weekly and monthly copies on the HDD (`archive list`).
pub(super) fn daily_backups_kept() -> usize {
    // One on the SSD (2026-09-27): the nightly archive keeps the daily, weekly and monthly copies on the HDD.
    std::env::var("SVANBOT_DAILY_BACKUPS").ok().and_then(|v| v.parse().ok()).unwrap_or(1usize).clamp(1, 30)
}

/// Keep the newest hourly and daily copies, removing each rotated file's sidecar with it.
fn rotate_backups(dir: &std::path::Path, mirrored: bool) {
    rotate_backups_keeping(dir, hourly_backups_kept(mirrored), daily_backups_kept());
}

pub(super) fn rotate_backups_keeping(dir: &std::path::Path, hourly: usize, daily: usize) {
    for (prefix, keep) in [("svanbot10-", hourly), ("daily-svanbot10-", daily)] {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| {
                        p.extension().is_some_and(|x| x == "db")
                            && p.file_name().map(|n| n.to_string_lossy().starts_with(prefix)).unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        while files.len() > keep {
            let old = files.remove(0);
            // A copy that will not go is the disk filling up later; rotation is where that starts
            // to matter, so it says so rather than counting itself done (issue #326).
            if let Err(e) = sv10_rt::remove_stale_file(&old) {
                tracing::warn!("could not rotate out {} (the directory keeps it): {e}", old.display());
            }
            let mut side = old.into_os_string();
            side.push(".sha256");
            // Its seal is best-effort: an orphaned `.sha256` matches nothing this directory is read by.
            let _ = std::fs::remove_file(side);
        }
    }
}

/// Backups written before sealing existed get a sidecar once, if they pass the check.
fn seal_unsealed_backups(dir: &std::path::Path) {
    for old in std::fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()) {
        let mut side = old.clone().into_os_string();
        side.push(".sha256");
        if old.extension().is_some_and(|x| x == "db") && !std::path::Path::new(&side).exists() {
            match sv10_store::integrity::quick_check(&old) {
                Ok(()) => {
                    if let Err(e) = sv10_store::integrity::seal_backup(&old) {
                        tracing::warn!("backup {} passed its check but could not be sealed: {e}", old.display());
                    }
                }
                Err(e) => tracing::warn!("unsealed backup {} fails its check: {e}", old.display()),
            }
        }
    }
}

/// Dashboard status row: what was checked, what was written, and the newest sealed archive.
pub(super) fn write_backup_status(
    shared: &Shared,
    now: &chrono::DateTime<chrono::Utc>,
    hourly: &std::path::Path,
    digest_checked: i64,
    digest_bad: i64,
    mirror: &Mirror,
    mirror_dir: Option<&std::path::Path>,
) {
    let corpus = crate::history::open(shared).map(|db| db.corpus_counts()).unwrap_or_default();
    let archive_latest = [sv10_store::archive::Kind::Daily, sv10_store::archive::Kind::Weekly]
        .into_iter()
        .filter_map(|k| sv10_store::archive::list(&shared.config.archive_dir, k).pop())
        .filter_map(|n| sv10_store::archive::load_manifest(&shared.config.archive_dir.join(&n)).ok())
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(|m| m.name);
    let status = serde_json::json!({
        "checked_at": now.timestamp(), "database_check": "ok", "digests_checked": digest_checked, "digest_mismatches": digest_bad,
        "last_backup": hourly.file_name().map(|n| n.to_string_lossy().to_string()), "archive": shared.config.archive_dir.display().to_string(),
        "archive_latest": archive_latest, "corpus": corpus.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        // The copy of this hour that left the SSD backup folder, and why it is not there when it is
        // not (0292); `same_disk` says it did not leave the disk (#725).
        "mirror": mirror.json(mirror_dir, now.timestamp()),
        "tables": {"flop": sv10_core::tables::loaded(3).is_some(), "turn": sv10_core::tables::loaded(4).is_some()},
    });
    // The dashboard's storage row is this key: a write that does not land leaves the previous run's
    // numbers on the page as if they were this run's, with nothing else to say they are old (#326).
    if let Err(e) = shared.store.put_kv(INTEGRITY_STATUS_KEY, &status.to_string()) {
        tracing::warn!("the backup status could not be stored: {e}");
    }
    match mirror {
        // Loud and non-fatal: the failure is in the log and on the dashboard's storage row, but play
        // and the SSD backups carry on (0292).
        Mirror::Failed(why) => {
            tracing::error!("database backed up to {}, but not mirrored: {why}", hourly.display());
            // No "second disk" in this line: the mirror may have been the same-disk folder (#725).
            shared.log("fleet", "error", format!("backup not mirrored: {why}"));
        }
        // The log line carries the same honesty as the row: this copy is on the databases' disk and
        // does not survive losing it (#725).
        Mirror::SameDisk => tracing::info!(
            "database backed up to {}; the hour is also in {} on the same disk (a deletion guard, not a disk-loss copy)",
            hourly.display(),
            mirror_dir.map_or_else(|| "the mirror folder".to_string(), |d| d.display().to_string())
        ),
        _ => tracing::info!("database backed up to {}", hourly.display()),
    }
}

/// Hourly consistent database backups: the newest hourlies and dailies on the SSD, and a day of
/// hourlies in the mirror folder when the mirror is on — on a second disk, or in its own same-disk
/// folder when there is none (#725).
pub fn backup_database(shared: &Shared) {
    let dir = shared.config.artifacts.join("backups");
    let now = chrono::Utc::now();
    if !backup_space_available(shared) {
        return;
    }
    let (digest_checked, digest_bad) = verify_live_database(shared);
    prune_backup_sources(shared);
    let Some(hourly) = write_hourly_backup(shared, &dir, &now) else {
        return;
    };
    ensure_daily_backup(&dir, &hourly, &now);
    // A mirror that fails is loud (log, dashboard status) and never fatal: the SSD copy, the daily
    // copies and the nightly archive still stand on their own (0292).
    let keep = mirror_hourly_kept();
    let mirror = keep.and(backup_mirror(shared));
    let mirror_step = match (keep, &mirror) {
        (None, _) => Mirror::Off,
        (Some(_), None) => Mirror::Absent,
        (Some(keep), Some((dir, same_disk))) => match mirror_backup(&hourly, dir, keep) {
            Ok(true) if *same_disk => Mirror::SameDisk,
            Ok(true) => Mirror::Copied,
            Ok(false) => Mirror::NoRoom,
            Err(e) => Mirror::Failed(format!("{e:#}")),
        },
    };
    let mirrored = matches!(mirror_step, Mirror::Copied | Mirror::SameDisk);
    rotate_backups(&dir, mirrored);
    seal_unsealed_backups(&dir);
    write_backup_status(shared, &now, &hourly, digest_checked, digest_bad, &mirror_step, mirror.as_ref().map(|(d, _)| d.as_path()));
}

/// Free bytes on the filesystem holding `path`.
pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
    sv10_rt::free_bytes(path)
}

pub fn save_models(shared: &Shared) {
    let json = {
        let m = shared.models.read();
        serde_json::to_string(&*m)
    };
    match json {
        Ok(j) => {
            if let Err(e) = shared.store.put_kv(MODELS_KEY, &j) {
                tracing::warn!("saving models failed: {e}");
            }
        }
        Err(e) => tracing::warn!("serializing models failed: {e}"),
    }
    // The state-hash tallies ride the same checkpoint (0301): they are the panel's only figure that
    // must outlive the process, and this is the write that already runs every five minutes, at
    // shutdown and before a hot swap.
    crate::live::save_state_hash_totals(&shared.store, &shared.bots);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsealed_daily_copy_is_replaced_on_the_next_hour() {
        let dir = std::env::temp_dir().join(format!("sv10-daily-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-26T14:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let hourly = dir.join("svanbot10-2026092614.db");
        let daily = dir.join("daily-svanbot10-20260926.db");
        let db = rusqlite::Connection::open(&hourly).unwrap();
        db.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (7);").unwrap();
        drop(db);
        sv10_store::integrity::seal_backup(&hourly).unwrap();

        // An interrupted copy leaves a path but no usable sealed backup.
        std::fs::write(&daily, b"partial copy").unwrap();
        ensure_daily_backup(&dir, &hourly, &now);
        assert!(sv10_store::integrity::verify_backup(&daily), "the next hour must repair the daily backup");
        let db = rusqlite::Connection::open(&daily).unwrap();
        assert_eq!(db.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
        drop(db);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
