//! Compressed cold columns (0229).
//!
//! The JSON that fills most of both databases is written once and read rarely: server hand exports
//! and their summaries (`history.db`), decision explanations, replay records and queued audits (the
//! main database). Those columns hold an `sv10-pack` frame (a BLOB) instead of text: raw DEFLATE
//! against a preset dictionary trained on the column's own rows, with the uncompressed length and
//! CRC-32 in the frame, so a damaged value fails loudly instead of reading as different JSON.
//!
//! Every reader goes through [`Codec::text`], which accepts both forms: rows written before the
//! migration stay readable, and the background compaction ([`compact_batch`]) packs them in small
//! resumable batches. Digests (`integrity::hand_digest`) are over the text, never the frame.
//!
//! Dictionaries live in the database they serve (`pack_dicts`), are never changed once written,
//! and are named in each frame by their id, so a copy of a database (backup, archive, snapshot)
//! decodes on its own. A build before this module reads only text: [`DATA_FORMAT`] is recorded next
//! to the database ([`mark_data_format`]) and `scripts/rollback.sh` refuses to install a build whose
//! source reads a lower format (`archive unpack` converts back, with the fleet stopped).

use anyhow::{Result, bail};
use parking_lot::RwLock;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sv10_pack::Dictionary;

/// Data format this build reads and writes: 1 = every column is text; 2 = the cold JSON columns may
/// hold `sv10-pack` frames. `scripts/rollback.sh` reads this line from a build's source.
pub const DATA_FORMAT: u32 = 2;

/// File next to the databases recording the highest [`DATA_FORMAT`] ever written there.
pub const DATA_FORMAT_FILE: &str = "data-format";

/// Largest decompressed value accepted (the largest stored JSON is well under 1 MB; the limit only
/// stops a damaged frame from asking for gigabytes).
pub const MAX_TEXT: usize = 64 << 20;

/// Compression level of stored values (zlib's default: its ratio, a third of level 9's time).
pub const LEVEL: u8 = 6;

/// Rows a dictionary is trained from, spread over the newest of the column.
const TRAIN_ROWS: i64 = 600;
/// Rows a column needs before a dictionary is trained (fewer and it would fit only those rows).
const TRAIN_MIN_ROWS: i64 = 200;
/// How often a writer looks for dictionaries another process trained.
const REFRESH: Duration = Duration::from_secs(300);

/// A compressed column: where it lives and which dictionary family packs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    /// Table name.
    pub table: &'static str,
    /// Column name.
    pub column: &'static str,
    /// Dictionary family (columns of the same shape share one).
    pub family: &'static str,
    /// Extra `SET` assignments evaluated against the row's text before it is packed (they read
    /// the old value, as every SQLite `UPDATE` does), filling the real columns that replaced SQL
    /// reads inside the JSON.
    pub fill: &'static str,
}

impl Column {
    /// `table.column`, for reports.
    pub fn label(&self) -> String {
        format!("{}.{}", self.table, self.column)
    }
}

/// `history.db` raw server exports.
pub const RAW_JSON: Column = Column {
    table: "raw",
    column: "json",
    family: "raw.json",
    fill: "profit = CASE WHEN typeof(json) = 'text' AND json_valid(json) THEN json_extract(json, '$.profit') ELSE profit END, ",
};
/// `history.db` derived summaries of the raw exports.
pub const RAW_SUMMARY: Column = Column { table: "raw", column: "summary", family: "summary", fill: "" };
/// `history.db` summaries from other sources.
pub const CORPUS_SUMMARY: Column = Column { table: "corpus", column: "summary", family: "summary", fill: "" };
/// Main database decision explanations.
pub const DECISION_DETAIL: Column = Column {
    table: "decisions",
    column: "detail",
    family: "decisions.detail",
    fill:
        "opponents = CASE WHEN typeof(detail) = 'text' AND json_valid(detail) THEN json_extract(detail, '$.opponents') ELSE opponents END,
           version = CASE WHEN typeof(detail) = 'text' AND json_valid(detail) THEN json_extract(detail, '$.version') ELSE version END,
           candidates = CASE WHEN typeof(detail) = 'text' AND json_valid(detail)
                             THEN json_array_length(json_extract(detail, '$.candidates')) ELSE candidates END, ",
};
/// Main database replay records.
pub const REPLAY_RECORD: Column = Column { table: "replays", column: "record", family: "record", fill: "" };
/// Main database queued audits (the same record shape as replays).
pub const AUDIT_RECORD: Column = Column { table: "audit_queue", column: "record", family: "record", fill: "" };

/// The main database's compressed columns.
pub const MAIN_COLUMNS: [Column; 3] = [DECISION_DETAIL, REPLAY_RECORD, AUDIT_RECORD];
/// `history.db`'s compressed columns.
pub const HISTORY_COLUMNS: [Column; 3] = [RAW_JSON, RAW_SUMMARY, CORPUS_SUMMARY];

/// The dictionary table (created by [`Codec::open`]).
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS pack_dicts (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 255), family TEXT NOT NULL, created TEXT NOT NULL, bytes BLOB NOT NULL)";

#[derive(Default)]
struct Dicts {
    by_id: HashMap<u8, Arc<Dictionary>>,
    newest: HashMap<String, u8>,
    loaded_at: Option<Instant>,
    /// Dictionary ids a reload has already failed to provide, so a read of many such rows opens
    /// one connection rather than one per row (0253). Cleared by every reload, because a
    /// dictionary can only arrive through one.
    missing: HashSet<u8>,
}

/// Packs and unpacks one database's compressed columns with that database's dictionaries.
pub struct Codec {
    path: PathBuf,
    dicts: RwLock<Dicts>,
}

impl Codec {
    /// Create the dictionary table if needed and load every dictionary.
    pub fn open(conn: &Connection, path: &Path) -> Result<Codec> {
        conn.execute_batch(SCHEMA)?;
        let codec = Codec { path: path.to_path_buf(), dicts: RwLock::new(Dicts::default()) };
        codec.reload(conn)?;
        Ok(codec)
    }

    fn reload(&self, conn: &Connection) -> Result<()> {
        let mut st = conn.prepare("SELECT id, family, bytes FROM pack_dicts ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
        let mut fresh = Dicts { loaded_at: Some(Instant::now()), ..Dicts::default() };
        let known = self.dicts.read().by_id.clone();
        for row in rows {
            let (id, family, bytes) = row?;
            let id = u8::try_from(id)?;
            // An id of 0 is refused here rather than carried (#601): the schema's CHECK is not
            // re-applied to a row as it is read back, and the frame format reserves 0 for "no
            // dictionary", so packing with it panics in `sv10_pack` — on the decision path, for every
            // write to that column afterwards.
            if id == 0 {
                tracing::warn!("pack dictionary id 0 in {} is refused; {} packs without one", self.path.display(), family);
                continue;
            }
            let dict = known.get(&id).cloned().unwrap_or_else(|| Arc::new(Dictionary::new(&bytes)));
            fresh.by_id.insert(id, dict);
            fresh.newest.insert(family, id);
        }
        *self.dicts.write() = fresh;
        Ok(())
    }

    /// [`Codec::pack`] without a connection, from the dictionaries already loaded; `None` when they are
    /// due for a reload, and the caller then packs with [`Codec::pack`] under its own connection. Lets a
    /// writer compress before it takes the write lock, and without queueing on the read connection (0322).
    pub fn pack_cached(&self, column: Column, text: &str) -> Option<Vec<u8>> {
        let dicts = self.dicts.read();
        if dicts.loaded_at.is_none_or(|t| t.elapsed() > REFRESH) {
            return None;
        }
        Some(match dicts.newest.get(column.family).and_then(|id| Some((*id, dicts.by_id.get(id)?))) {
            Some((id, dict)) => sv10_pack::pack_prepared(text.as_bytes(), id, dict),
            None => sv10_pack::pack(text.as_bytes()),
        })
    }

    /// Pack `text` for `column` with the column's newest dictionary (none yet: a frame without one).
    /// `conn` is the writer, used to pick up dictionaries another process trained.
    pub fn pack(&self, conn: &Connection, column: Column, text: &str) -> Vec<u8> {
        let stale = self.dicts.read().loaded_at.is_none_or(|t| t.elapsed() > REFRESH);
        if stale && let Err(e) = self.reload(conn) {
            tracing::warn!("pack dictionaries not reloaded: {e}");
        }
        let dicts = self.dicts.read();
        match dicts.newest.get(column.family).and_then(|id| Some((*id, dicts.by_id.get(id)?))) {
            Some((id, dict)) => sv10_pack::pack_prepared(text.as_bytes(), id, dict),
            None => sv10_pack::pack(text.as_bytes()),
        }
    }

    /// The text of a stored value, text or frame (NULL is an error; see [`Codec::opt_text`]).
    pub fn text(&self, value: ValueRef<'_>) -> rusqlite::Result<String> {
        match value {
            ValueRef::Text(t) => String::from_utf8(t.to_vec()).map_err(|e| conversion(Box::new(e))),
            ValueRef::Blob(b) => self.unpack(b),
            ValueRef::Null => Err(rusqlite::Error::InvalidColumnType(0, "value".into(), rusqlite::types::Type::Null)),
            other => Err(rusqlite::Error::InvalidColumnType(0, "value".into(), other.data_type())),
        }
    }

    /// [`Codec::text`] with NULL as `None`.
    pub fn opt_text(&self, value: ValueRef<'_>) -> rusqlite::Result<Option<String>> {
        match value {
            ValueRef::Null => Ok(None),
            v => self.text(v).map(Some),
        }
    }

    fn unpack(&self, frame: &[u8]) -> rusqlite::Result<String> {
        let id = sv10_pack::dictionary_id(frame).ok_or_else(|| conversion(Box::new(sv10_pack::Error::NotAFrame)))?;
        if id != 0 && !self.dicts.read().by_id.contains_key(&id) && !self.dicts.read().missing.contains(&id) {
            // Trained by another process since this one loaded: read the table again. An id a reload
            // does not have is remembered, so a table full of frames naming it costs one connection
            // rather than one per row (0253).
            let conn = Connection::open_with_flags(
                &self.path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            self.reload(&conn).map_err(|e| conversion(e.into()))?;
            if !self.dicts.read().by_id.contains_key(&id) {
                self.dicts.write().missing.insert(id);
            }
        }
        let dicts = self.dicts.read();
        sv10_pack::text_prepared(frame, MAX_TEXT, |id| dicts.by_id.get(&id).map(|d| &**d)).map_err(|e| conversion(Box::new(e)))
    }

    /// Train a dictionary for `column`'s family from its newest rows when it has none and holds
    /// enough rows; returns the new dictionary's id.
    pub fn train(&self, conn: &Connection, column: Column) -> Result<Option<u8>> {
        self.reload(conn)?;
        if self.dicts.read().newest.contains_key(column.family) {
            return Ok(None);
        }
        let (table, col) = (column.table, column.column);
        let rows: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE {col} IS NOT NULL"), [], |r| r.get(0))?;
        if rows < TRAIN_MIN_ROWS {
            return Ok(None);
        }
        let mut st = conn.prepare(&format!("SELECT {col} FROM {table} WHERE {col} IS NOT NULL ORDER BY rowid DESC LIMIT ?1"))?;
        let samples = st.query_map([TRAIN_ROWS], |r| self.text(r.get_ref(0)?))?.collect::<rusqlite::Result<Vec<String>>>()?;
        let bytes = train_bytes(&samples);
        let next: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM pack_dicts", [], |r| r.get(0))?;
        if next > 255 {
            bail!("no dictionary ids left in {}", self.path.display());
        }
        conn.execute(
            "INSERT INTO pack_dicts (id, family, created, bytes) VALUES (?1, ?2, ?3, ?4)",
            params![next, column.family, chrono::Utc::now().to_rfc3339(), bytes],
        )?;
        self.reload(conn)?;
        Ok(Some(next as u8))
    }

    /// Whether `column`'s family has a dictionary.
    pub fn has_dictionary(&self, column: Column) -> bool {
        self.dicts.read().newest.contains_key(column.family)
    }
}

fn conversion(e: Box<dyn std::error::Error + Send + Sync>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, e)
}

/// A preset dictionary from sample rows (newest first): whole rows spread evenly over the sample,
/// oldest first so the newest sit nearest the data (shorter distances code cheaper), trimmed to the
/// 32 KB window. Measured on 20,000 exports: 0.146 of the text against 0.330 with no dictionary;
/// row fragments did worse (0.157 at 1 KB, 0.219 at 512 bytes).
pub fn train_bytes(samples: &[String]) -> Vec<u8> {
    const WINDOW: usize = 32 * 1024;
    let mut picked = Vec::new();
    let mut size = 0;
    if !samples.is_empty() {
        let avg = samples.iter().map(|s| s.len()).sum::<usize>() / samples.len();
        let want = (WINDOW / avg.max(1)).clamp(1, samples.len());
        for k in 0..want {
            let s = &samples[k * samples.len() / want];
            if size >= WINDOW {
                break;
            }
            size += s.len();
            picked.push(s.as_bytes());
        }
    }
    picked.reverse();
    let joined = picked.concat();
    joined[joined.len().saturating_sub(WINDOW)..].to_vec()
}

/// Pack up to `limit` of `column`'s rows after rowid `after` that are text, or packed without a
/// dictionary now that one exists, in one transaction. Returns (rows packed, last rowid seen); a
/// last rowid of `None` means the column is done.
pub fn compact_batch(conn: &Connection, codec: &Codec, column: Column, after: i64, limit: usize) -> Result<(usize, Option<i64>)> {
    let rows = compact_candidates(conn, codec, column, after, limit)?;
    compact_rows(conn, codec, column, &rows)
}

/// The rows [`compact_batch`] would pack, read on any connection. On a column already packed this
/// scans to the end of the table and finds nothing — 8 s on a restored 760 MB store, which under the
/// write lock stalled every hand insert — so a caller with a reader runs it there.
pub fn compact_candidates(conn: &Connection, codec: &Codec, column: Column, after: i64, limit: usize) -> Result<Vec<(i64, String)>> {
    let (table, col) = (column.table, column.column);
    let repack =
        if codec.has_dictionary(column) { format!(" OR (typeof({col}) = 'blob' AND substr({col}, 4, 1) = x'00')") } else { String::new() };
    let mut st = conn.prepare(&format!(
        "SELECT rowid, {col} FROM {table} WHERE rowid > ?1 AND (typeof({col}) = 'text'{repack}) ORDER BY rowid LIMIT ?2"
    ))?;
    Ok(st.query_map(params![after, limit as i64], |r| Ok((r.get(0)?, codec.text(r.get_ref(1)?)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// Pack `rows` from [`compact_candidates`] on the write connection; returns rows written and the
/// cursor to continue after (`None`: nothing left).
pub fn compact_rows(conn: &Connection, codec: &Codec, column: Column, rows: &[(i64, String)]) -> Result<(usize, Option<i64>)> {
    let (table, col, fill) = (column.table, column.column, column.fill);
    let Some(last) = rows.last().map(|r| r.0) else { return Ok((0, None)) };
    let tx = conn.unchecked_transaction()?;
    let mut written = 0;
    {
        // Read under the transaction: SQLite cannot promote a snapshot invalidated by another
        // writer, so a value checked here cannot be overwritten from a stale candidate.
        let mut current =
            tx.prepare(&format!("SELECT {col} FROM {table} WHERE rowid = ?1 AND (typeof({col}) = 'text' OR substr({col}, 4, 1) = x'00')"))?;
        let mut up = tx.prepare(&format!(
            "UPDATE {table} SET {fill}{col} = ?1 WHERE rowid = ?2 AND (typeof({col}) = 'text' OR substr({col}, 4, 1) = x'00')"
        ))?;
        for (rowid, text) in rows {
            let now = current.query_row([rowid], |r| codec.text(r.get_ref(0)?)).optional()?;
            if now.as_deref() == Some(text.as_str()) {
                written += up.execute(params![codec.pack(&tx, column, text), rowid])?;
            }
        }
    }
    tx.commit()?;
    Ok((written, Some(last)))
}

/// Rows of `column` still stored as text.
pub fn text_rows(conn: &Connection, column: Column) -> Result<i64> {
    let (table, col) = (column.table, column.column);
    Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE typeof({col}) = 'text'"), [], |r| r.get(0))?)
}

/// Convert every packed value of `column` back to text (`archive unpack`, before installing a
/// build that reads only text). Returns rows converted.
pub fn unpack_column(conn: &Connection, codec: &Codec, column: Column) -> Result<usize> {
    let (table, col) = (column.table, column.column);
    let mut done = 0;
    loop {
        let rows: Vec<(i64, String)> = {
            let mut st =
                conn.prepare(&format!("SELECT rowid, {col} FROM {table} WHERE typeof({col}) = 'blob' ORDER BY rowid LIMIT 2000"))?;
            st.query_map([], |r| Ok((r.get(0)?, codec.text(r.get_ref(1)?)?)))?.collect::<rusqlite::Result<_>>()?
        };
        if rows.is_empty() {
            return Ok(done);
        }
        let tx = conn.unchecked_transaction()?;
        {
            let mut up = tx.prepare(&format!("UPDATE {table} SET {col} = ?1 WHERE rowid = ?2"))?;
            for (rowid, text) in &rows {
                up.execute(params![text, rowid])?;
            }
        }
        tx.commit()?;
        done += rows.len();
    }
}

/// Record that the databases in `dir` may hold data format `format` (never lowers it unless
/// `lower`; `archive unpack` lowers it after converting everything back to text).
pub fn mark_data_format(dir: &Path, format: u32, lower: bool) -> Result<()> {
    let path = dir.join(DATA_FORMAT_FILE);
    let current = data_format(dir);
    if current == format || (current > format && !lower) {
        return Ok(());
    }
    let tmp = dir.join(format!(".{DATA_FORMAT_FILE}.{}", std::process::id()));
    std::fs::write(&tmp, format!("{format}\n"))?;
    std::fs::rename(&tmp, &path)?;
    // Durable, like every other atomic write here: a lost rename could leave the marker at an older
    // format, and `scripts/rollback.sh` trusts it when it decides whether a build can read the
    // databases (0255). `sv10_rt::sync_dir` is the same two lines for the app's own atomic writes;
    // this crate deliberately does not depend on the runtime helpers.
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// The data format recorded in `dir` (1 when nothing is recorded: text only).
pub fn data_format(dir: &Path) -> u32 {
    std::fs::read_to_string(dir.join(DATA_FORMAT_FILE)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1)
}

/// Add `column` to `table` when missing (the schema bumps behind the compressed columns).
pub fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let exists = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
        .query_row([column], |_| Ok(()))
        .optional()?
        .is_some();
    if !exists {
        add_column(conn, table, column, decl)?;
    }
    Ok(())
}

/// `ALTER TABLE .. ADD COLUMN`, where a column another process added since our check is success:
/// the bot, learner and analyst open the store together after a hot swap.
pub fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    match conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};")) {
        Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
        other => Ok(other?),
    }
}

#[cfg(test)]
mod tests;
