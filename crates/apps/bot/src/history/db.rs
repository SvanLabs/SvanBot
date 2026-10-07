//! History database access: stored server exports and corpus rows (0260).

use crate::live::Shared;
use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use sv10_core::model::HandSummary;

pub const STATUS_KEY: &str = "history.status";
#[cfg(test)]
mod compaction_tests;
/// (raw row id, bot, raw export json, full corpus summary when a richer source holds the hand).
pub(super) type TableHand = (i64, String, String, Option<String>);

/// A hand from a source other than our bots' server exports.
pub struct CorpusRow {
    pub hand_id: String,
    pub bot: String,
    pub table_id: String,
    pub started_at: String,
    pub summary: String,
}

pub struct HistoryDb {
    // Visible inside `history` for the sabotage test that drops a table to prove a failed page
    // never advances the frontier; production code goes through the methods below.
    pub(super) conn: Mutex<Connection>,
    /// Candidate scans run beside importer writes under WAL.
    reader: Mutex<Connection>,
    /// Packs and unpacks the export JSON and the summaries (0229).
    codec: sv10_store::packed::Codec,
}

impl HistoryDb {
    pub fn open(path: &Path) -> Result<HistoryDb> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS raw (
                id INTEGER PRIMARY KEY AUTOINCREMENT, hand_id TEXT NOT NULL UNIQUE, bot TEXT NOT NULL,
                table_id TEXT, hand_number INTEGER, started_at TEXT, json TEXT NOT NULL, summary TEXT);
             CREATE INDEX IF NOT EXISTS raw_table ON raw(table_id, hand_number);
             CREATE INDEX IF NOT EXISTS raw_time ON raw(started_at);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS corpus (
                id INTEGER PRIMARY KEY AUTOINCREMENT, hand_id TEXT NOT NULL UNIQUE, source TEXT NOT NULL,
                bot TEXT NOT NULL, table_id TEXT, started_at TEXT, summary TEXT NOT NULL, digest TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS corpus_source ON corpus(source, id);",
        )?;
        // Other processes (ingest, neural training, review) open it too: wait for a writer, never fail.
        conn.busy_timeout(Duration::from_secs(10))?;
        // 0229: the server's profit gets a real column (the export JSON is stored packed).
        sv10_store::packed::ensure_column(&conn, "raw", "profit", "INTEGER")?;
        let codec = sv10_store::packed::Codec::open(&conn, path)?;
        if let Some(dir) = path.parent() {
            sv10_store::packed::mark_data_format(dir, sv10_store::packed::DATA_FORMAT, false)?;
        }
        let reader =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        reader.busy_timeout(Duration::from_secs(10))?;
        Ok(HistoryDb { conn: Mutex::new(conn), reader: Mutex::new(reader), codec })
    }

    /// Compact one batch of the compressed columns (0229), like `Store::compact`: train a column's
    /// dictionary once it has enough rows, then pack up to `limit` rows after `cursor[i]`.
    pub fn compact(&self, cursor: &mut [Option<i64>; 3], limit: usize) -> Result<usize> {
        let mut packed = 0;
        for (column, at) in sv10_store::packed::HISTORY_COLUMNS.iter().zip(cursor.iter_mut()) {
            let Some(after) = *at else { continue };
            if !self.codec.has_dictionary(*column) {
                self.codec.train(&self.conn.lock(), *column)?;
            }
            let rows = sv10_store::packed::compact_candidates(&self.reader.lock(), &self.codec, *column, after, limit)?;
            if rows.is_empty() {
                *at = None;
                continue;
            }
            let (n, last) = sv10_store::packed::compact_rows(&self.conn.lock(), &self.codec, *column, &rows)?;
            packed += n;
            *at = last;
        }
        Ok(packed)
    }

    /// Rows of each compressed column still stored as text.
    pub fn text_rows(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.lock();
        sv10_store::packed::HISTORY_COLUMNS.iter().map(|c| Ok((c.label(), sv10_store::packed::text_rows(&conn, *c)?))).collect()
    }

    /// Convert every packed value back to text (`archive unpack`); returns rows converted.
    pub fn unpack_all(&self) -> Result<usize> {
        let conn = self.conn.lock();
        sv10_store::packed::HISTORY_COLUMNS.iter().map(|c| sv10_store::packed::unpack_column(&conn, &self.codec, *c)).sum()
    }

    /// Rewrite the file without its free pages (after the compaction freed most of them): returns
    /// the bytes before and after. Holds the connection for the rewrite (tens of seconds at 1 GB);
    /// the importer waits, and other processes wait up to their busy timeout.
    pub fn vacuum(&self, path: &Path) -> Result<(u64, u64)> {
        let size = || std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let before = size();
        // A VACUUM writes a whole second copy, and in WAL mode about as much again to the log, on
        // the disk the live store writes to. Started without room it fails part-way, fills the disk
        // for everything else while it runs, and was tried again ten minutes later (#897).
        if let Some(free) = sv10_rt::free_bytes(path) {
            anyhow::ensure!(
                free >= before.saturating_mul(5) / 2,
                "not compacting {}: {} MB free, and a rewrite of its {} MB needs about {} MB",
                path.display(),
                free >> 20,
                before >> 20,
                (before.saturating_mul(5) / 2) >> 20
            );
        }
        let conn = self.conn.lock();
        conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok((before, size()))
    }

    /// Bytes a VACUUM would return: free pages plus the space unused inside pages (`dbstat`), which rows
    /// that shrank in place leave behind and the free-page count does not see.
    pub fn reclaimable_bytes(&self) -> Result<u64> {
        let conn = self.conn.lock();
        let unused: i64 = conn.query_row("SELECT COALESCE(SUM(unused), 0) FROM dbstat", [], |r| r.get(0))?;
        drop(conn);
        Ok(self.free_bytes()? + unused.max(0) as u64)
    }

    /// Test helper: shrink every stored export in place, as packing does.
    #[cfg(test)]
    pub fn shrink_rows_for_test(&self) -> Result<()> {
        self.conn.lock().execute("UPDATE raw SET json = '{}'", [])?;
        Ok(())
    }

    /// Bytes in free pages (what a VACUUM would return to the disk).
    pub fn free_bytes(&self) -> Result<u64> {
        let conn = self.conn.lock();
        let (free, page): (i64, i64) = conn.query_row(
            "SELECT (SELECT freelist_count FROM pragma_freelist_count), (SELECT page_size FROM pragma_page_size)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((free * page) as u64)
    }

    /// Add hands from another source in one transaction together with its resume watermark, so
    /// a killed import never leaves a half-written batch. Returns (new, already present).
    pub fn insert_corpus(&self, source: &str, rows: &[CorpusRow], watermark: (&str, &str)) -> Result<(usize, usize)> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let (mut new, mut known) = (0, 0);
        for r in rows {
            // The digest is over the text; the stored summary is packed (0229).
            let digest = sv10_store::integrity::hand_digest(&r.bot, &r.hand_id, &r.started_at, source, "", &r.summary);
            let summary = self.codec.pack(&tx, sv10_store::packed::CORPUS_SUMMARY, &r.summary);
            let n = tx.execute(
                "INSERT OR IGNORE INTO corpus(hand_id, source, bot, table_id, started_at, summary, digest) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![r.hand_id, source, r.bot, r.table_id, r.started_at, summary, digest],
            )?;
            if n > 0 { new += 1 } else { known += 1 }
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![watermark.0, watermark.1],
        )?;
        tx.commit()?;
        Ok((new, known))
    }

    pub fn meta_value(&self, key: &str) -> Option<String> {
        self.meta(key)
    }

    /// Corpus rows whose stored content no longer matches its digest.
    pub fn verify_corpus(&self) -> Result<(usize, Vec<String>)> {
        let conn = self.conn.lock();
        let mut st = conn.prepare("SELECT hand_id, source, bot, COALESCE(started_at, ''), summary, digest FROM corpus")?;
        let mut checked = 0;
        let mut bad = Vec::new();
        let rows = st
            .query_map([], |r| Ok([r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, self.codec.text(r.get_ref(4)?)?, r.get(5)?]))?;
        for row in rows {
            let [id, source, bot, started, summary, digest] = row?;
            checked += 1;
            if sv10_store::integrity::hand_digest(&bot, &id, &started, &source, "", &summary) != digest {
                bad.push(id);
            }
        }
        Ok((checked, bad))
    }

    pub(crate) fn meta(&self, key: &str) -> Option<String> {
        self.conn.lock().query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) {
        if let Err(e) = self.conn.lock().execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        ) {
            // A lost frontier write only repeats pages on the next pass (inserts are idempotent).
            tracing::warn!("history meta {key} not saved: {e}");
        }
    }

    pub(crate) fn clear_meta(&self, key: &str) {
        if let Err(e) = self.conn.lock().execute("DELETE FROM meta WHERE key = ?1", [key]) {
            tracing::warn!("history meta {key} not cleared: {e}");
        }
    }

    /// Insert a page of hands; returns (new, already known).
    pub(crate) fn insert_page(&self, bot: &str, hands: &[Value]) -> Result<(usize, usize)> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let (mut new, mut known) = (0, 0);
        for h in hands {
            let Some(id) = h["hand_id"].as_str() else { continue };
            let json = self.codec.pack(&tx, sv10_store::packed::RAW_JSON, &h.to_string());
            let n = tx.execute(
                "INSERT OR IGNORE INTO raw(hand_id, bot, table_id, hand_number, started_at, json, profit) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, bot, h["table_id"].as_str(), h["hand_number"].as_i64(), h["started_at"].as_str(), json, h["profit"].as_i64()],
            )?;
            if n > 0 { new += 1 } else { known += 1 }
        }
        tx.commit()?;
        Ok((new, known))
    }

    /// The server's recorded profit for one of a bot's hands.
    pub fn profit(&self, bot: &str, hand_id: &str) -> Option<i64> {
        self.conn
            .lock()
            .query_row(
                "SELECT COALESCE(profit, CASE WHEN typeof(json) = 'text' AND json_valid(json) THEN json_extract(json, '$.profit') END)
                 FROM raw WHERE bot = ?1 AND hand_id = ?2",
                params![bot, hand_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    /// The server's export JSON of one hand (`review export`).
    pub fn export_json(&self, hand_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn.query_row("SELECT json FROM raw WHERE hand_id = ?1", [hand_id], |r| self.codec.text(r.get_ref(0)?)).optional()?)
    }

    /// (bot, hand_id, started_at, profit) of every export row starting at or after `since` (RFC
    /// 3339 text, compared as SQLite text like the fleet check did). The profit column is filled at
    /// insert; a row still in text form reads the export JSON (0229).
    pub fn raw_after(&self, since: &str) -> Result<Vec<(String, String, String, i64)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare(
            "SELECT bot, hand_id, COALESCE(started_at, ''),
                    COALESCE(profit, CASE WHEN typeof(json) = 'text' AND json_valid(json) THEN json_extract(json, '$.profit') END, 0)
             FROM raw WHERE COALESCE(started_at, '') >= ?1",
        )?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Corpus rows per source.
    pub fn corpus_counts(&self) -> Vec<(String, i64)> {
        let conn = self.conn.lock();
        let Ok(mut st) = conn.prepare("SELECT source, COUNT(*) FROM corpus GROUP BY source ORDER BY source") else { return vec![] };
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }

    pub fn count(&self) -> i64 {
        self.conn.lock().query_row("SELECT COUNT(*) FROM raw", [], |r| r.get(0)).unwrap_or(0)
    }

    pub(super) fn max_id(&self) -> i64 {
        self.conn.lock().query_row("SELECT COALESCE(MAX(id), 0) FROM raw", [], |r| r.get(0)).unwrap_or(0)
    }

    pub(super) fn tables_after(&self, id: i64, upto: i64) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare("SELECT DISTINCT COALESCE(table_id, '') FROM raw WHERE id > ?1 AND id <= ?2")?;
        let rows = st.query_map(params![id, upto], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(rows)
    }

    /// Every stored hand of one table, oldest first: (id, bot, raw json, full corpus summary if a
    /// richer source holds the same hand).
    pub(super) fn table_hands(&self, table: &str) -> Result<Vec<TableHand>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare(
            "SELECT r.id, r.bot, r.json, c.summary FROM raw r LEFT JOIN corpus c ON c.hand_id = r.hand_id
             WHERE COALESCE(r.table_id, '') = ?1 ORDER BY r.hand_number, r.started_at",
        )?;
        let rows = st
            .query_map([table], |r| Ok((r.get(0)?, r.get(1)?, self.codec.text(r.get_ref(2)?)?, self.codec.opt_text(r.get_ref(3)?)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Openpoker corpus hands not in the server history, after corpus row `id` and up to `upto`:
    /// (id, bot, started_at, summary). Bounded above by the row the caller stores as its watermark:
    /// a row committed after that was read would otherwise be counted now and again next pass.
    pub(super) fn corpus_only_after(&self, id: i64, upto: i64) -> Result<Vec<(i64, String, String, String)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare(
            "SELECT c.id, c.bot, COALESCE(c.started_at, ''), c.summary FROM corpus c
             WHERE c.id > ?1 AND c.id <= ?2 AND c.source LIKE 'openpoker-%'
               AND NOT EXISTS (SELECT 1 FROM raw WHERE raw.hand_id = c.hand_id) ORDER BY c.id",
        )?;
        let rows = st
            .query_map([id, upto], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, self.codec.text(r.get_ref(3)?)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(super) fn max_corpus_id(&self) -> i64 {
        self.conn.lock().query_row("SELECT COALESCE(MAX(id), 0) FROM corpus", [], |r| r.get(0)).unwrap_or(0)
    }

    pub(super) fn set_summaries(&self, rows: &[(i64, String)]) -> Result<()> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        for (id, s) in rows {
            tx.execute("UPDATE raw SET summary = ?1 WHERE id = ?2", params![self.codec.pack(&tx, sv10_store::packed::RAW_SUMMARY, s), id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every summary of one corpus source, in import order.
    pub fn source_summaries(&self, source: &str) -> Result<Vec<HandSummary>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare("SELECT summary FROM corpus WHERE source = ?1 ORDER BY id")?;
        let rows = st
            .query_map([source], |r| self.codec.text(r.get_ref(0)?))?
            .filter_map(|r| r.ok())
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect();
        Ok(rows)
    }

    /// The most recent `limit` replayed summaries, oldest first, for model training. With
    /// `with_corpus`, full-detail corpus summaries replace export-derived ones for the same hand and
    /// openpoker corpus hands missing from the export are added. Fitted models take the corpus only
    /// while it improves their held-out score (range fit, 2026-09-15: it did not).
    pub fn recent_summaries(&self, limit: usize, with_corpus: bool) -> Result<Vec<(String, HandSummary)>> {
        let conn = self.conn.lock();
        let sql = if with_corpus {
            "SELECT COALESCE(c.started_at, r.started_at) AS t, COALESCE(c.summary, r.summary) AS s
               FROM raw r LEFT JOIN corpus c ON c.hand_id = r.hand_id WHERE COALESCE(c.summary, r.summary) IS NOT NULL
             UNION ALL
             SELECT c.started_at, c.summary FROM corpus c
              WHERE c.source LIKE 'openpoker-%' AND NOT EXISTS (SELECT 1 FROM raw WHERE raw.hand_id = c.hand_id)
             ORDER BY t DESC LIMIT ?1"
        } else {
            "SELECT started_at AS t, summary AS s FROM raw WHERE summary IS NOT NULL ORDER BY t DESC LIMIT ?1"
        };
        let mut st = conn.prepare(sql)?;
        let mut rows: Vec<(String, HandSummary)> = st
            .query_map([limit as i64], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), self.codec.text(r.get_ref(1)?)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(t, s)| serde_json::from_str(&s).ok().map(|h| (t, h)))
            .collect();
        rows.reverse();
        Ok(rows)
    }
}

pub fn open(shared: &Shared) -> Result<HistoryDb> {
    HistoryDb::open(&shared.config.artifacts.join("history.db"))
}
