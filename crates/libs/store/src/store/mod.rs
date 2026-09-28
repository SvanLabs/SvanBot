//! The main SQLite database, one seam per domain.
//!
//! [`Store`] opens the database and owns the connection pool; each domain submodule
//! (`hands`, `decisions`, `replays`, `audits`, `calibration`, `kv`, `events`) implements its
//! own `impl Store` block, so a domain's SQL lives with its row types instead of in one
//! 900-line file. Public paths are unchanged: every row type is re-exported here.

use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use std::path::Path;

mod audits;
mod calibration;
mod decisions;
mod events;
mod hands;
mod incidents;
mod kv;
mod provenance;
mod replays;
mod scans;
mod slow;
mod timeline;

pub use audits::{AUDIT_RESULT_DAYS, AuditJob, AuditResult, AuditSummary};
pub use calibration::CalibrationRow;
pub use decisions::{PostflopBet, PostflopDecision, PreflopRaise, QuizSpot};
pub use hands::{HandRow, PlayerHand, ResultRow};
pub use incidents::Incident;
pub use provenance::{ArmHand, CONTROL_ARM, HandTag, TREATMENT_ARM, ordinary_hand};
pub use replays::ReplayRow;
pub use scans::{SCAN_SNAPSHOT_DAYS, SnapshotRow, snapshot_digest};
pub use timeline::{TimelineEvent, TimelineHand, TimelineHour};

/// The main database (`artifacts/svanbot10.db`): hands, decisions, replay records, calibration, kv
/// and events, WAL with synchronous FULL. Writes go through one connection behind a mutex; reads use a
/// small pool of read-only connections, so dashboard and learner queries never wait for (or block) a
/// bot's write. WAL readers see every committed write.
pub struct Store {
    conn: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: std::sync::atomic::AtomicUsize,
    /// Packs and unpacks the compressed columns (decision details, replay and audit records; 0229).
    codec: crate::packed::Codec,
    path: std::path::PathBuf,
}

/// Read-only connections per store.
const READERS: usize = 4;

impl Store {
    /// Open or create the database, applying the schema and migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // Before the schema: a peer migrating at the same hot swap holds the write lock. The wait is
        // `slow::busy_handler` (0322) — the same 10 s `busy_timeout` gave, and it records how long
        // SQLite made this hold wait for another process's lock, which is what names the holder.
        conn.busy_handler(Some(slow::busy_handler))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS hands (
                bot TEXT NOT NULL, hand_id TEXT NOT NULL, table_id TEXT, ended_at TEXT NOT NULL,
                hero_seat INTEGER, hole TEXT, board TEXT, pot INTEGER, net INTEGER, winners TEXT,
                summary TEXT, PRIMARY KEY (bot, hand_id));
             CREATE INDEX IF NOT EXISTS hands_bot_time ON hands(bot, ended_at);
             CREATE TABLE IF NOT EXISTS decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT, bot TEXT NOT NULL, hand_id TEXT, ts TEXT NOT NULL,
                street TEXT, action TEXT, amount INTEGER, equity REAL, pot INTEGER, to_call INTEGER,
                latency_ms REAL, detail TEXT);
             CREATE INDEX IF NOT EXISTS decisions_hand ON decisions(bot, hand_id);
             CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS calibration (
                id INTEGER PRIMARY KEY AUTOINCREMENT, bot TEXT NOT NULL, hand_id TEXT, ts TEXT NOT NULL,
                category TEXT NOT NULL, predicted REAL NOT NULL, realized REAL NOT NULL);
             CREATE INDEX IF NOT EXISTS calibration_category ON calibration(category);
             CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, bot TEXT, level TEXT, message TEXT);
             CREATE TABLE IF NOT EXISTS replays (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, bot TEXT NOT NULL, hand_id TEXT,
                net_digest TEXT, record TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS replay_nets (digest TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS audit_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, bot TEXT NOT NULL, hand_id TEXT,
                net_digest TEXT, record TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS decision_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, bot TEXT NOT NULL, hand_id TEXT, street TEXT,
                live_action TEXT, deep_action TEXT, gap_bb REAL NOT NULL, pot_bb REAL, deep_ms REAL, samples INTEGER);
             CREATE INDEX IF NOT EXISTS decision_audit_ts ON decision_audit(ts);
             CREATE TABLE IF NOT EXISTS hand_provenance (
                bot TEXT NOT NULL, hand_id TEXT NOT NULL, target TEXT NOT NULL, arm TEXT NOT NULL,
                record TEXT NOT NULL, ts TEXT NOT NULL, PRIMARY KEY (bot, hand_id));
             CREATE INDEX IF NOT EXISTS hand_provenance_target ON hand_provenance(target, arm);
             CREATE TABLE IF NOT EXISTS hash_incidents (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, bot TEXT NOT NULL,
                table_id TEXT NOT NULL, table_seq INTEGER NOT NULL, verdict TEXT NOT NULL,
                summary TEXT NOT NULL, canonical TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS scan_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, seen TEXT NOT NULL,
                digest TEXT NOT NULL, payload TEXT NOT NULL);",
        )?;
        crate::packed::ensure_column(&conn, "hands", "showdown", "INTEGER")?;
        crate::packed::ensure_column(&conn, "hands", "digest", "TEXT")?;
        // 0213: the all-in luck-adjusted net of each hand, filled in the background.
        crate::packed::ensure_column(&conn, "hands", "ev_net", "REAL")?;
        crate::packed::ensure_column(&conn, "calibration", "scale", "REAL")?;
        // 0229: what SQL read inside `decisions.detail` gets real columns, filled at insert and by
        // the compaction (which packs the detail); rows before both read the JSON while it is text.
        crate::packed::ensure_column(&conn, "decisions", "opponents", "INTEGER")?;
        crate::packed::ensure_column(&conn, "decisions", "version", "TEXT")?;
        crate::packed::ensure_column(&conn, "decisions", "candidates", "INTEGER")?;
        // 0316: which replay version each deep re-solve graded, so a decision-loss measurement can be
        // read on records that carry the live inputs (v3) — and can never silently mix in the ones
        // that do not. Rows before this column read `None` ("not recorded").
        crate::packed::ensure_column(&conn, "decision_audit", "replay_version", "INTEGER")?;
        let codec = crate::packed::Codec::open(&conn, path)?;
        if let Some(dir) = path.parent() {
            crate::packed::mark_data_format(dir, crate::packed::DATA_FORMAT, false)?;
        }
        let readers = (0..READERS)
            .map(|_| {
                let r = Connection::open_with_flags(
                    path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                        | rusqlite::OpenFlags::SQLITE_OPEN_URI,
                )?;
                r.busy_timeout(std::time::Duration::from_secs(10))?;
                Ok(Mutex::new(r))
            })
            .collect::<Result<Vec<_>>>()?;
        let store = Store { conn: Mutex::new(conn), readers, next_reader: Default::default(), codec, path: path.to_path_buf() };
        store.backfill_calibration_scale()?;
        store.backfill_showdown()?;
        store.backfill_digests()?;
        Ok(store)
    }
    /// One-time migration: pot size (pot + to call, bb) for calibration rows recorded before the
    /// column existed, from the matching decision (same bot, hand, street and action family).
    fn backfill_calibration_scale(&self) -> Result<()> {
        let conn = self.write_lock();
        if !conn.prepare("SELECT 1 FROM calibration WHERE scale IS NULL LIMIT 1")?.exists([])? {
            return Ok(());
        }
        conn.execute_batch(
            "UPDATE calibration SET scale = (
                SELECT (d.pot + d.to_call) / 20.0 FROM decisions d
                WHERE d.bot = calibration.bot AND d.hand_id = calibration.hand_id
                  AND d.street = substr(calibration.category, 1, instr(calibration.category, ':') - 1)
                  AND d.action = CASE substr(calibration.category, instr(calibration.category, ':') + 1, 4)
                      WHEN 'call' THEN 'call' WHEN 'chec' THEN 'check' WHEN 'alli' THEN 'all_in' ELSE 'raise' END
                ORDER BY d.id LIMIT 1)
             WHERE scale IS NULL;
             UPDATE calibration SET scale = -1 WHERE scale IS NULL;",
        )?;
        Ok(())
    }
    /// A free read-only connection (the next one in turn when all are busy).
    fn read(&self) -> parking_lot::MutexGuard<'_, Connection> {
        if let Some(guard) = self.readers.iter().find_map(|r| r.try_lock()) {
            return guard;
        }
        let i = self.next_reader.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.readers.len();
        self.readers[i].lock()
    }
    /// Structural check of the open database (see `integrity`).
    pub fn quick_check(&self) -> std::result::Result<(), String> {
        crate::integrity::check_connection(&self.write_lock())
    }
    /// One-time migration: digest hands stored before the column existed.
    fn backfill_digests(&self) -> Result<()> {
        let conn = self.write_lock();
        let rows: Vec<(i64, [String; 6])> = conn
            .prepare("SELECT rowid, bot, hand_id, ended_at, COALESCE(hole, ''), COALESCE(board, ''), COALESCE(summary, '') FROM hands WHERE digest IS NULL")?
            .query_map([], |r| Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?])))?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.is_empty() {
            return Ok(());
        }
        let tx = conn.unchecked_transaction()?;
        for (rowid, [bot, id, ended, hole, board, summary]) in rows {
            let digest = crate::integrity::hand_digest(&bot, &id, &ended, &hole, &board, &summary);
            tx.execute("UPDATE hands SET digest = ?1 WHERE rowid = ?2", params![digest, rowid])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Rows changed through this connection since it opened (lets callers verify a no-op write).
    pub fn total_changes(&self) -> u64 {
        self.write_lock().total_changes()
    }
    #[cfg(test)]
    pub(crate) fn corrupt_summary_for_test(&self, hand_id: &str) {
        self.write_lock().execute("UPDATE hands SET summary = summary || ' ' WHERE hand_id = ?1", params![hand_id]).unwrap();
    }
    /// One-time migration: derive the showdown flag for hands stored before the column existed.
    fn backfill_showdown(&self) -> Result<()> {
        let conn = self.write_lock();
        let rows: Vec<(i64, Option<i64>, String)> = conn
            .prepare("SELECT rowid, hero_seat, COALESCE(summary, '') FROM hands WHERE showdown IS NULL")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.is_empty() {
            return Ok(());
        }
        let tx = conn.unchecked_transaction()?;
        for (rowid, seat, summary) in rows {
            let went = serde_json::from_str::<sv10_model::model::HandSummary>(&summary)
                .map(|h| seat.map(|s| h.shown.iter().any(|(x, _)| *x as i64 == s)).unwrap_or(false))
                .unwrap_or(false);
            tx.execute("UPDATE hands SET showdown = ?1 WHERE rowid = ?2", params![went as i64, rowid])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Consistent online copy of the whole database (`VACUUM INTO`), replacing any file at `path`.
    ///
    /// On its own read-only connection (0229): the copy is a consistent snapshot, and the bots'
    /// writes never wait behind it (it used to hold the writer's lock for the whole copy).
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(path);
        let conn = Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    /// Compact one batch of the main database's compressed columns (0229): train a column's
    /// dictionary once it has enough rows, then pack up to `limit` text rows after `cursor[i]`.
    /// Returns rows packed; a column whose cursor comes back `None` is done until new text arrives.
    pub fn compact(&self, cursor: &mut [Option<i64>; 3], limit: usize) -> Result<usize> {
        let mut packed = 0;
        for (column, at) in crate::packed::MAIN_COLUMNS.iter().zip(cursor.iter_mut()) {
            let Some(after) = *at else { continue };
            let conn = self.write_lock();
            if !self.codec.has_dictionary(*column) {
                self.codec.train(&conn, *column)?;
            }
            let (n, last) = crate::packed::compact_batch(&conn, &self.codec, *column, after, limit)?;
            packed += n;
            *at = last;
        }
        Ok(packed)
    }

    /// Rows of each main-database compressed column still stored as text.
    pub fn text_rows(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.read();
        crate::packed::MAIN_COLUMNS.iter().map(|c| Ok((c.label(), crate::packed::text_rows(&conn, *c)?))).collect()
    }

    /// Convert every packed value back to text (`archive unpack`); returns rows converted.
    pub fn unpack_all(&self) -> Result<usize> {
        let conn = self.write_lock();
        crate::packed::MAIN_COLUMNS.iter().map(|c| crate::packed::unpack_column(&conn, &self.codec, *c)).sum()
    }

    /// Bytes in free pages (what a VACUUM would return to the disk).
    pub fn free_bytes(&self) -> Result<u64> {
        let conn = self.read();
        let (free, page): (i64, i64) = conn.query_row(
            "SELECT (SELECT freelist_count FROM pragma_freelist_count), (SELECT page_size FROM pragma_page_size)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((free * page) as u64)
    }

    /// Rewrite the file without its free pages; returns the bytes before and after. It holds the
    /// writer for the whole rewrite, so only `archive compact --vacuum-main` runs it, with the fleet
    /// stopped (the fleet reuses free pages instead). `hands` has no INTEGER PRIMARY KEY, and SQLite
    /// documents that VACUUM may renumber such rowids, which the hand cursors (head tailer, EV fill,
    /// monitor) depend on: the bundled SQLite keeps them (a test pins it), and this checks it.
    pub fn vacuum(&self) -> Result<(u64, u64)> {
        let size = || std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        let before = size();
        let conn = self.write_lock();
        let rowids = |c: &Connection| -> Result<(i64, i64, i64)> {
            Ok(c.query_row("SELECT COUNT(*), COALESCE(MAX(rowid), 0), COALESCE(SUM(rowid), 0) FROM hands", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?)
        };
        let was = rowids(&conn)?;
        conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        let now = rowids(&conn)?;
        anyhow::ensure!(
            was == now,
            "VACUUM renumbered hands rowids ({was:?} -> {now:?}): reset the head tailer and EV fill cursors before starting the fleet"
        );
        Ok((before, size()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `SV10_MIGRATE_COPY=<copy of a live database> cargo test -- --ignored`: the scale backfill on real data.
    #[test]
    #[ignore]
    fn backfills_calibration_scale_on_a_live_copy() {
        let path = std::env::var("SV10_MIGRATE_COPY").expect("SV10_MIGRATE_COPY");
        let store = Store::open(Path::new(&path)).unwrap();
        let conn = store.conn.lock();
        let (total, known): (i64, i64) =
            conn.query_row("SELECT COUNT(*), SUM(scale > 0) FROM calibration", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        eprintln!("calibration rows {total}, with pot {known}");
        assert!(known as f64 >= total as f64 * 0.95);
    }

    fn detail(i: i64) -> String {
        serde_json::json!({"version": format!("v{}", i % 3), "opponents": 1 + i % 4, "street": "flop",
            "candidates": (0..(1 + i % 3)).map(|k| serde_json::json!({"action": k, "ev": i as f64 / 7.0})).collect::<Vec<_>>()})
        .to_string()
    }

    /// 0229: decisions written before the migration (text) and after (packed at insert) read the
    /// same; the compaction packs the old ones and fills the columns SQL selects on, and every
    /// query answers the same before and after.
    #[test]
    fn old_text_decisions_and_new_packed_ones_read_alike_through_the_compaction() {
        let dir = std::env::temp_dir().join(format!("sv10-store-compact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        assert_eq!(crate::packed::data_format(&dir), crate::packed::DATA_FORMAT);
        {
            // Rows as a build before 0229 wrote them: text detail, no columns.
            let conn = store.conn.lock();
            for i in 0..300 {
                conn.execute(
                    "INSERT INTO decisions (bot, hand_id, ts, street, action, amount, equity, pot, to_call, latency_ms, detail)
                     VALUES ('A', ?1, '2026-09-26T00:00:00Z', 'flop', 'raise', 40, 0.5, 100, 0, 3.0, ?2)",
                    params![format!("h{i}"), detail(i)],
                )
                .unwrap();
            }
        }
        for i in 300..320 {
            store.insert_decision("A", &format!("h{i}"), "flop", "raise", Some(40), Some(0.5), 100, 0, 3.0, &detail(i)).unwrap();
        }
        let snapshot = |store: &Store| {
            let ids: Vec<String> = (0..320).map(|i| format!("h{i}")).collect();
            let versions = store.decision_versions("A", &ids).unwrap();
            let opponents: Vec<Option<i64>> = store.postflop_decisions().unwrap().iter().map(|d| d.opponents).collect();
            let bets: Vec<String> = store.heads_up_postflop_bets(1000).unwrap().into_iter().map(|b| b.detail).collect();
            let hand = store.decisions_for_hand("A", "h7").unwrap();
            let quiz = store.random_quiz_spot(1000, 3, 0).unwrap().map(|q| q.detail);
            (versions, opponents, bets, hand, quiz)
        };
        let before = snapshot(&store);
        assert_eq!(before.0.len(), 320);
        assert_eq!(before.0["h301"], "v1");
        assert_eq!(before.1.iter().filter(|o| o.is_some()).count(), 320);
        assert_eq!(before.2.len(), 320);
        assert_eq!(before.2[0], detail(319), "newest first, packed at insert and read back");
        assert_eq!(before.3[0]["detail"]["version"], "v1");
        let q: serde_json::Value = serde_json::from_str(&before.4.clone().unwrap()).unwrap();
        assert_eq!(q["candidates"].as_array().unwrap().len(), 3, "only spots with three options");
        assert_eq!(store.text_rows().unwrap()[0], ("decisions.detail".to_string(), 300));

        let mut cursor = [Some(0); 3];
        let mut packed = 0;
        while cursor.iter().any(|c| c.is_some()) {
            packed += store.compact(&mut cursor, 64).unwrap();
        }
        // The 300 text rows, and the 20 packed at insert before a dictionary existed.
        assert_eq!(packed, 320);
        assert_eq!(store.text_rows().unwrap()[0], ("decisions.detail".to_string(), 0));
        let after = snapshot(&store);
        assert_eq!((&after.0, &after.1, &after.2, &after.3), (&before.0, &before.1, &before.2, &before.3));
        assert!(after.4.is_some());
        let filled: i64 = store
            .read()
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE opponents IS NOT NULL AND version IS NOT NULL AND candidates IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(filled, 320);

        // A backup taken on its own connection while the writer's lock is held is complete.
        let held = store.conn.lock();
        store.backup_to(&dir.join("backup.db")).unwrap();
        drop(held);
        let copy = Store::open(&dir.join("backup.db")).unwrap();
        assert_eq!(copy.heads_up_postflop_bets(1000).unwrap().len(), 320);

        // VACUUM keeps every hand's rowid (the hand cursors depend on it; SQLite only promises it for
        // INTEGER PRIMARY KEY tables, so this pins the bundled version's behaviour).
        {
            let conn = store.conn.lock();
            for i in 0..400 {
                conn.execute(
                    "INSERT INTO hands (bot, hand_id, ended_at, summary) VALUES ('A', ?1, '2026-09-26T00:00:00Z', ?2)",
                    params![format!("v{i}"), "x".repeat(900)],
                )
                .unwrap();
            }
            conn.execute("DELETE FROM hands WHERE CAST(substr(hand_id, 2) AS INTEGER) % 3 = 0", []).unwrap();
        }
        let ids = |s: &Store| -> Vec<(i64, String)> {
            s.read()
                .prepare("SELECT rowid, hand_id FROM hands ORDER BY rowid")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        let before_vacuum = ids(&store);
        let (was, now) = store.vacuum().unwrap();
        assert!(now < was, "{was} -> {now}");
        assert_eq!(ids(&store), before_vacuum);

        // `archive unpack` turns it all back into text an older build reads.
        assert_eq!(store.unpack_all().unwrap(), 320);
        let text: String = store.read().query_row("SELECT detail FROM decisions WHERE hand_id = 'h5'", [], |r| r.get(0)).unwrap();
        assert_eq!(text, detail(5));
    }
}
