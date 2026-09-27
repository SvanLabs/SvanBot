//! The findings scan's own inputs and outputs, one row per pass that changed (0356).
//!
//! 0352's decision: a finding that cannot be re-derived is a number from an unknown era. The scan
//! corrects against `calibration.v1`, a single kv slot the learner overwrites in place every cycle,
//! so the table a claim was computed against is gone by the time the ticket quoting it is read —
//! 0346 could not check its own ticket text inside a day. Each pass therefore records what it read
//! and what it emitted, as one payload: the calibration summary, the installed correction table
//! verbatim, the comparable-class counts, every finding with its value, and the constants that
//! define the arithmetic (so a row from a superseded input era is identifiable rather than silently
//! comparable).
//!
//! **Written only when the payload changes.** [`Store::record_scan_snapshot`] compares the payload's
//! digest with the newest row's and writes a row only on a difference; an unchanged pass stamps that
//! row's `seen` instead. A table with no row in it therefore means the scan has not run, which is a
//! different fact from a scan that ran and saw nothing — writing a row unconditionally would erase
//! the distinction. `seen` is what tells those apart: it moves on every pass, so a stale one is a
//! scan that stopped rather than a payload that stopped changing.
//!
//! The payload is TEXT, not an `sv10-pack` frame: it is written once per change and read by hand or
//! by a tool, never selected *into* by SQL, so the packed columns' dictionary machinery (0229) buys
//! nothing for it. Read it with [`Store::scan_snapshots`] — never with `json_extract`.
use super::*;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

/// How long a scan snapshot is kept: [`Store::prune_scan_snapshots`] drops rows no scan has read for
/// this many days, and nothing else deletes them.
///
/// It is coupled to [`AUDIT_RESULT_DAYS`] by the assert below, because that is the horizon it
/// depends on: the comparable-class counts a payload records are measured over windows up to
/// [`AUDIT_RESULT_DAYS`] long, and a snapshot store that expired before the verdicts it counted
/// would leave the live store holding evidence no recorded scan view covers — the comparison this
/// table exists for, failing exactly where the evidence still exists. It is also the floor the
/// assert below holds this constant to, so thirty days is what the coupling allows, not a taste.
///
/// Thirty days bounds the bytes at the same time, and the bound is not small (0352 guessed a few
/// hundred bytes a day; measured 2026-09-27 against the live store, one payload is ~30 KB — an
/// 11.7 KB installed table, a 3.4 KB calibration summary, 32 findings at 11.6 KB — and the scan runs
/// every half hour with inputs that move nearly every pass: 946 calibration rows and 126 verdicts an
/// hour, so ~1.5 MB a day and ~44 MB resident in steady state, carried into every hourly copy).
/// That is the price of the row and it is paid on purpose; a year of it would be half a gigabyte.
pub const SCAN_SNAPSHOT_DAYS: i64 = 30;

// The snapshot store must outlive the deepest window it summarizes. A shorter one prunes the record
// of an era while the verdicts that era counted are still measurable, and the failure is silent: the
// rows that would have shown the change are simply not there (0345's rule, reversed — there the
// window had to fit inside the retention, here the retention has to cover the window).
const _: () = assert!(
    SCAN_SNAPSHOT_DAYS >= AUDIT_RESULT_DAYS,
    "SCAN_SNAPSHOT_DAYS must cover the store's AUDIT_RESULT_DAYS retention: snapshots of eras whose verdicts are still live would be pruned under it"
);

/// SHA-256 of a snapshot payload, hex: what [`Store::record_scan_snapshot`] dedups on, and what a
/// reader can check a stored payload against.
pub fn snapshot_digest(payload: &str) -> String {
    sv10_digest::hex(sv10_digest::Sha256::digest(payload))
}

/// One recorded scan snapshot: what the findings scan read and emitted, and when it was read.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotRow {
    /// Row id, ascending with the era the payload describes.
    pub id: i64,
    /// When this payload was first recorded (RFC 3339): the era it opens.
    pub ts: String,
    /// The last scan that read this same payload (RFC 3339). A stale one is a scan that stopped, not
    /// a payload that stopped changing.
    pub seen: String,
    /// [`snapshot_digest`] of `payload`.
    pub digest: String,
    /// The scan's inputs and outputs, JSON.
    pub payload: String,
}

impl Store {
    /// Record one scan's payload: a new row when it differs from the newest, otherwise a stamp of
    /// `seen` on that newest row. Returns whether a row was written, so a caller can say when the
    /// scan's own view changed.
    pub fn record_scan_snapshot(&self, payload: &str) -> Result<bool> {
        let digest = snapshot_digest(payload);
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.write_lock();
        let tx = conn.unchecked_transaction()?;
        let newest: Option<(i64, String)> = tx
            .query_row("SELECT id, digest FROM scan_snapshots ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let written = match newest {
            // The same payload: no row, and no page change when the stamp would not move either.
            Some((id, digest_of_row)) if digest_of_row == digest => {
                tx.execute("UPDATE scan_snapshots SET seen = ?2 WHERE id = ?1 AND seen IS NOT ?2", params![id, now])?;
                false
            }
            _ => {
                tx.execute(
                    "INSERT INTO scan_snapshots (ts, seen, digest, payload) VALUES (?1, ?1, ?2, ?3)",
                    params![now, digest, payload],
                )?;
                true
            }
        };
        tx.commit()?;
        Ok(written)
    }

    /// The newest `limit` snapshots, newest first.
    pub fn scan_snapshots(&self, limit: usize) -> Result<Vec<SnapshotRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT id, ts, seen, digest, payload FROM scan_snapshots ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(SnapshotRow { id: r.get(0)?, ts: r.get(1)?, seen: r.get(2)?, digest: r.get(3)?, payload: r.get(4)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Drop snapshots no scan has read for `days`; returns rows removed. Pruning is on `seen`, not on
    /// `ts`, so the era a scan is still reading is never the one dropped — a payload that has not
    /// moved for a year is still the current one.
    pub fn prune_scan_snapshots(&self, days: i64) -> Result<usize> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        Ok(self.write_lock().execute("DELETE FROM scan_snapshots WHERE seen < ?1", params![cutoff])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_snapshot_is_written_on_a_change_and_pruned_by_its_last_read() {
        let dir = std::env::temp_dir().join(format!("sv10-store-snapshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();

        assert!(store.record_scan_snapshot("{\"a\":1}").unwrap(), "the first pass writes its inputs and outputs");
        let (first_ts, first_seen) = {
            let rows = store.scan_snapshots(10).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].payload, "{\"a\":1}");
            assert_eq!(rows[0].digest, snapshot_digest("{\"a\":1}"), "the digest is the payload's, so a reader can check it");
            assert_eq!(rows[0].ts, rows[0].seen, "first recorded and first read at the same pass");
            (rows[0].ts.clone(), rows[0].seen.clone())
        };
        // The unchanged pass: no row, but not no read — `seen` moving is how a scan that ran and saw
        // nothing new is told apart from a scan that did not run.
        assert!(!store.record_scan_snapshot("{\"a\":1}").unwrap(), "the same payload is not a second row");
        let rows = store.scan_snapshots(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, first_ts, "the era opened at the first pass and does not move");
        assert!(rows[0].seen >= first_seen, "the second pass stamped its read");

        assert!(store.record_scan_snapshot("{\"a\":2}").unwrap(), "a changed payload is a new era");
        let rows = store.scan_snapshots(10).unwrap();
        assert_eq!(rows.len(), 2, "and the era before it is kept rather than overwritten");
        assert_eq!(rows[0].payload, "{\"a\":2}", "newest first");

        // The scan stops: nothing has read either row for longer than the retention.
        let stale = (chrono::Utc::now() - chrono::Duration::days(SCAN_SNAPSHOT_DAYS + 1)).to_rfc3339();
        store.write_lock().execute("UPDATE scan_snapshots SET seen = ?1 WHERE id = ?2", params![stale, rows[0].id]).unwrap();
        store.write_lock().execute("UPDATE scan_snapshots SET seen = ?1 WHERE id = ?2", params![stale, rows[1].id]).unwrap();
        assert_eq!(store.prune_scan_snapshots(SCAN_SNAPSHOT_DAYS).unwrap(), 2, "a snapshot nobody reads ages out");
        assert!(store.scan_snapshots(10).unwrap().is_empty(), "so an empty table means the scan has not run");

        // A payload that has not moved for a year is still the one the scan reads, and pruning on
        // `seen` (not on `ts`) is what keeps it: it is the current state, not a stale era.
        assert!(store.record_scan_snapshot("{\"a\":3}").unwrap());
        store.write_lock().execute("UPDATE scan_snapshots SET ts = '2020-01-01T00:00:00Z'", []).unwrap();
        assert_eq!(store.prune_scan_snapshots(SCAN_SNAPSHOT_DAYS).unwrap(), 0);
        assert_eq!(store.scan_snapshots(10).unwrap().len(), 1, "the current row survives however old its payload is");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
