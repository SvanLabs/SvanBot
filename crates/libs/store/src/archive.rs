//! Long-term archives on a second disk: daily differentials, weekly fulls and monthly
//! recompressed fulls, each a directory with a sealed `MANIFEST.json`.
//!
//! Layout under the archive root (`SVANBOT_ARCHIVE_DIR`; on the reference box `/backup-disk/svanbot10`):
//!
//! - `weekly/YYYY-Www/` — full `VACUUM INTO` copies of the live database and `history.db`
//!   (zstd). The code is not archived: the manifest records the commit.
//! - `daily/YYYY-MM-DD/` — a full copy of the live database (small, and its rows are replaced in
//!   place) and a **differential** of `history.db`: every row of an `id`-keyed table above the
//!   week's full watermark, plus full copies of the other tables. Restoring a daily needs only its
//!   weekly base, never a chain of dailies.
//! - `monthly/YYYY-MM/` — the month's first weekly full recompressed at zstd level 19 (long mode).
//!
//! Integrity: every file records its stored SHA-256 and the SHA-256 of its decompressed content,
//! checked end to end (the decompressed stream is hashed) before the archive directory is renamed
//! into place. `MANIFEST.json` has a `.sha256` sidecar. A half-written archive stays under
//! `.tmp-*` and is removed by the next run. Compression uses the `zstd` command-line tool.
//!
//! `history.db` rows are insert-only except the derived `raw.summary` column, which the importer
//! recomputes from `raw.json`; a daily differential therefore restores summaries as of the week's
//! full for older rows, and the next import refills any that changed.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Manifest format version.
pub const FORMAT: u32 = 1;
/// Manifest file name inside every archive directory.
pub const MANIFEST: &str = "MANIFEST.json";

/// Archive kinds, which are also the subdirectories of the archive root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Live database full plus history differential against the week's full.
    Daily,
    /// Full copies of both databases.
    Weekly,
    /// A weekly full recompressed for long retention.
    Monthly,
}

impl Kind {
    /// Subdirectory name under the archive root.
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Daily => "daily",
            Kind::Weekly => "weekly",
            Kind::Monthly => "monthly",
        }
    }
}

/// What a file in an archive holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Full copy of the live database (`svanbot10.db`).
    LiveFull,
    /// Full copy of `history.db`.
    HistoryFull,
    /// Rows of `history.db` added since the base weekly full.
    HistoryDelta,
    /// `git bundle --all` of the repository (stored as is). Archives written before #772 carry one; none is written now.
    RepoBundle,
}

/// One file of an archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// File name inside the archive directory.
    pub name: String,
    /// What the file holds.
    pub role: Role,
    /// Stored (compressed) size.
    pub bytes: u64,
    /// SHA-256 of the stored file.
    pub sha256: String,
    /// Decompressed size.
    pub raw_bytes: u64,
    /// SHA-256 of the decompressed content.
    pub raw_sha256: String,
    /// Row count per table of the database this file restores to (for a delta: the full
    /// database's counts at archive time, which a restore must reproduce).
    #[serde(default)]
    pub rows: BTreeMap<String, i64>,
    /// Highest `id` per `id`-keyed table (fulls only; a daily's delta starts above these).
    #[serde(default)]
    pub watermarks: BTreeMap<String, i64>,
}

/// The sealed description of one archive directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Manifest format version ([`FORMAT`]).
    pub format: u32,
    /// Archive kind.
    pub kind: Kind,
    /// Relative path under the root, e.g. `weekly/2026-W38`.
    pub name: String,
    /// Creation time, RFC 3339 UTC.
    pub created_at: String,
    /// Workspace version that wrote the archive.
    pub app_version: String,
    /// Repository commit at archive time, when known.
    pub git_commit: Option<String>,
    /// Weekly archive this one depends on (dailies) or was made from (monthlies).
    pub base: Option<String>,
    /// Files in the archive.
    pub files: Vec<Entry>,
    /// How to restore it.
    pub restore: String,
}

/// Inputs to an archive run.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The live database (`artifacts/svanbot10.db`).
    pub live: PathBuf,
    /// The history database (`artifacts/history.db`).
    pub history: PathBuf,
    /// Repository whose head commit is recorded in manifests, if any.
    pub repo: Option<PathBuf>,
    /// Workspace version recorded in manifests.
    pub app_version: String,
}

/// How many archives of each kind are kept.
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// Daily differentials kept.
    pub daily: usize,
    /// Weekly fulls kept (a weekly a kept daily depends on is always kept).
    pub weekly: usize,
    /// Monthly archives kept.
    pub monthly: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Retention { daily: 14, weekly: 8, monthly: 12 }
    }
}

/// zstd level for daily and weekly files (fast, multi-threaded).
const LEVEL_FAST: u32 = 6;
/// zstd level for monthly files.
const LEVEL_MONTHLY: u32 = 19;

/// Run whatever is due at `now`: the week's full if missing (it stands in for that day's daily),
/// otherwise today's daily; then the month's archive once the month has a weekly full; then prune.
/// Returns the archive names written and removed.
pub fn run(root: &Path, src: &Sources, now: chrono::DateTime<chrono::Utc>, keep: Retention) -> Result<(Vec<String>, Vec<String>)> {
    std::fs::create_dir_all(root).with_context(|| format!("archive root {}", root.display()))?;
    let mut made = Vec::new();
    let week = format!("weekly/{}", now.format("%G-W%V"));
    let day = format!("daily/{}", now.format("%Y-%m-%d"));
    if !root.join(&week).join(MANIFEST).exists() {
        made.push(make_weekly(root, &week, src, now)?.name);
    } else if !root.join(&day).join(MANIFEST).exists() {
        made.push(make_daily(root, &day, &week, src, now)?.name);
    }
    let month = format!("monthly/{}", now.format("%Y-%m"));
    if !root.join(&month).join(MANIFEST).exists() {
        let prefix = now.format("%Y-%m").to_string();
        let first_weekly = list(root, Kind::Weekly)
            .into_iter()
            .filter_map(|n| load_manifest(&root.join(&n)).ok())
            .filter(|m| m.created_at.starts_with(&prefix))
            .min_by(|a, b| a.created_at.cmp(&b.created_at));
        if let Some(w) = first_weekly {
            made.push(make_monthly(root, &month, &w.name, now)?.name);
        }
    }
    let removed = prune(root, keep)?;
    Ok((made, removed))
}

/// Bytes an archive run may need on the archive disk at peak: uncompressed staging copies of
/// both databases plus 1 GiB of headroom.
pub fn space_needed(src: &Sources) -> u64 {
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    size(&src.live) + size(&src.history) + (1 << 30)
}

/// Write the weekly full `name` (e.g. `weekly/2026-W38`).
pub fn make_weekly(root: &Path, name: &str, src: &Sources, now: chrono::DateTime<chrono::Utc>) -> Result<Manifest> {
    let tmp = stage_dir(root, name)?;
    let mut files = Vec::new();
    for (db, file, role) in [(&src.live, "svanbot10.db", Role::LiveFull), (&src.history, "history.db", Role::HistoryFull)] {
        let copy = tmp.join(file);
        vacuum_into(db, &copy)?;
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        crate::integrity::check_connection(&conn).map_err(|e| anyhow::anyhow!("{file} copy failed its check: {e}"))?;
        let (rows, watermarks) = (row_counts(&conn, "main")?, id_watermarks(&conn, "main")?);
        drop(conn);
        files.push(compress_entry(&copy, role, LEVEL_FAST, rows, watermarks)?);
    }
    // Data only (#772): the code is not archived. A restore installs the recorded `git_commit` from git and
    // lays the data on it, so the archive carries the commit id and nothing that rebuilds from it.
    let manifest = Manifest {
        format: FORMAT,
        kind: Kind::Weekly,
        name: name.to_string(),
        created_at: now.to_rfc3339(),
        app_version: src.app_version.clone(),
        git_commit: src.repo.as_deref().and_then(git_head),
        base: None,
        files,
        restore: format!("archive restore {name} --to <dir>  (writes svanbot10.db and history.db; install git_commit for the code)"),
    };
    finish(root, &tmp, manifest)
}

/// Write the daily `name` (e.g. `daily/2026-09-16`) against the weekly full `base`.
pub fn make_daily(root: &Path, name: &str, base: &str, src: &Sources, now: chrono::DateTime<chrono::Utc>) -> Result<Manifest> {
    let base_manifest = load_manifest(&root.join(base)).with_context(|| format!("base {base}"))?;
    let base_marks = base_manifest
        .files
        .iter()
        .find(|e| e.role == Role::HistoryFull)
        .map(|e| e.watermarks.clone())
        .context("base has no history full")?;
    let tmp = stage_dir(root, name)?;
    let live = tmp.join("svanbot10.db");
    vacuum_into(&src.live, &live)?;
    let conn = Connection::open_with_flags(&live, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    crate::integrity::check_connection(&conn).map_err(|e| anyhow::anyhow!("live copy failed its check: {e}"))?;
    let rows = row_counts(&conn, "main")?;
    drop(conn);
    let mut files = vec![compress_entry(&live, Role::LiveFull, LEVEL_FAST, rows, BTreeMap::new())?];
    let delta = tmp.join("history-delta.db");
    let rows = write_delta(&src.history, &delta, &base_marks)?;
    files.push(compress_entry(&delta, Role::HistoryDelta, LEVEL_FAST, rows, BTreeMap::new())?);
    let manifest = Manifest {
        format: FORMAT,
        kind: Kind::Daily,
        name: name.to_string(),
        created_at: now.to_rfc3339(),
        app_version: src.app_version.clone(),
        git_commit: src.repo.as_deref().and_then(git_head),
        base: Some(base.to_string()),
        files,
        restore: format!("archive restore {name} --to <dir>  (needs {base}; writes svanbot10.db and history.db)"),
    };
    finish(root, &tmp, manifest)
}

/// Write the monthly `name` (e.g. `monthly/2026-09`) by recompressing the weekly full `weekly`.
pub fn make_monthly(root: &Path, name: &str, weekly: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Manifest> {
    let base = load_manifest(&root.join(weekly))?;
    let tmp = stage_dir(root, name)?;
    let mut files = Vec::new();
    for e in &base.files {
        let from = root.join(weekly).join(&e.name);
        let to = tmp.join(&e.name);
        let mut entry = e.clone();
        if e.role == Role::RepoBundle {
            std::fs::copy(&from, &to)?;
        } else {
            recompress(&from, &to, LEVEL_MONTHLY)?;
            let (raw_sha, raw_bytes) = zstd_content_sha(&to)?;
            if raw_sha != e.raw_sha256 || raw_bytes != e.raw_bytes {
                bail!("{} changed while recompressing", e.name);
            }
        }
        entry.bytes = std::fs::metadata(&to)?.len();
        entry.sha256 = crate::integrity::file_sha256(&to)?;
        files.push(entry);
    }
    let manifest = Manifest {
        kind: Kind::Monthly,
        name: name.to_string(),
        created_at: now.to_rfc3339(),
        base: Some(weekly.to_string()),
        restore: format!("archive restore {name} --to <dir>  (writes svanbot10.db and history.db; install git_commit for the code)"),
        files,
        ..base
    };
    finish(root, &tmp, manifest)
}

mod verify;
pub use verify::verify;

/// Restore the archive `name` under `root` into `out`, verifying hashes, structure and row
/// counts. A daily is rebuilt from its weekly base plus its differential. Returns written paths.
pub fn restore(root: &Path, name: &str, out: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join(name);
    let problems = verify(&dir, false);
    if !problems.is_empty() {
        bail!("{name} failed verification: {}", problems.join("; "));
    }
    let manifest = load_manifest(&dir)?;
    std::fs::create_dir_all(out)?;
    let mut written = Vec::new();
    for e in &manifest.files {
        let from = crate::paths::confined(&dir, &e.name)?; // both ends: a seal proves rot, not intent (#603)
        match e.role {
            Role::RepoBundle => {
                let to = crate::paths::confined(out, &e.name)?;
                std::fs::copy(&from, &to)?;
                written.push(to);
            }
            Role::LiveFull | Role::HistoryFull => {
                let to = crate::paths::confined(out, e.name.trim_end_matches(".zst"))?;
                restore_db(&from, &to, e)?;
                written.push(to);
            }
            Role::HistoryDelta => {
                let base_name = manifest.base.as_deref().context("daily without a base")?;
                let base_dir = root.join(base_name);
                let base = load_manifest(&base_dir)?;
                let full = base.files.iter().find(|f| f.role == Role::HistoryFull).context("base has no history full")?;
                let problems = verify(&base_dir, false);
                if !problems.is_empty() {
                    bail!("base {base_name} failed verification: {}", problems.join("; "));
                }
                let to = out.join("history.db");
                restore_db(&base_dir.join(&full.name), &to, full)?;
                let delta = out.join(".history-delta.db");
                decompress_checked(&from, &delta, e)?;
                apply_delta(&to, &delta, &full.watermarks)?;
                // Best-effort: the restore is what was asked for and it is written and checked; this
                // is the temporary the rows came out of, and failing the restore over it would be wrong.
                let _ = std::fs::remove_file(&delta);
                let conn = Connection::open(&to)?;
                crate::integrity::check_connection(&conn).map_err(|err| anyhow::anyhow!("restored history.db failed its check: {err}"))?;
                let counts = row_counts(&conn, "main")?;
                if counts != e.rows {
                    bail!("restored history.db rows {counts:?} differ from the archive's {:?}", e.rows);
                }
                written.push(to);
            }
        }
    }
    // The restored files are what the operator starts from next: durable before this returns.
    for path in &written {
        sv10_rt::sync_file(path)?;
    }
    sv10_rt::sync_dir(&out.join("."))?;
    Ok(written)
}

/// Delete archives beyond `keep`, oldest first; a weekly that a kept daily or monthly names as
/// its base is kept. Returns the removed names.
pub fn prune(root: &Path, keep: Retention) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for (kind, n) in [(Kind::Daily, keep.daily), (Kind::Monthly, keep.monthly)] {
        let names = list(root, kind);
        for old in names.iter().take(names.len().saturating_sub(n)) {
            std::fs::remove_dir_all(root.join(old))?;
            removed.push(old.clone());
        }
    }
    let needed: Vec<String> = [Kind::Daily, Kind::Monthly]
        .into_iter()
        .flat_map(|k| list(root, k))
        .filter_map(|n| load_manifest(&root.join(n)).ok()?.base)
        .collect();
    let weeklies = list(root, Kind::Weekly);
    for old in weeklies.iter().take(weeklies.len().saturating_sub(keep.weekly)) {
        if !needed.contains(old) {
            std::fs::remove_dir_all(root.join(old))?;
            removed.push(old.clone());
        }
    }
    Ok(removed)
}

/// Archive names of `kind` with a manifest, oldest first (`weekly/2026-W38`, …).
pub fn list(root: &Path, kind: Kind) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(kind.dir()))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join(MANIFEST).exists())
        .map(|e| format!("{}/{}", kind.dir(), e.file_name().to_string_lossy()))
        .collect();
    names.sort();
    names
}

/// Read and check the sealed manifest of the archive at `dir`.
pub fn load_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let seal = std::fs::read_to_string(dir.join(format!("{MANIFEST}.sha256"))).context("manifest seal missing")?;
    if crate::integrity::file_sha256(&path)? != seal.trim() {
        bail!("manifest seal mismatch");
    }
    let m: Manifest = serde_json::from_str(&text)?;
    if m.format != FORMAT {
        bail!("manifest format {} (this build reads {FORMAT})", m.format);
    }
    Ok(m)
}

/// Staging directory for `name`, with the `.tmp-*` a run that was interrupted left under any kind
/// cleared first: an archive staged into one of those would be written and verified on top of it.
fn stage_dir(root: &Path, name: &str) -> Result<PathBuf> {
    let final_dir = root.join(name);
    if final_dir.join(MANIFEST).exists() {
        bail!("{name} already exists");
    }
    let parent = final_dir.parent().context("archive name without a kind")?;
    for kind in [Kind::Daily, Kind::Weekly, Kind::Monthly] {
        let entries = std::fs::read_dir(root.join(kind.dir())).into_iter().flatten().filter_map(|e| e.ok());
        for e in entries.filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-")) {
            sv10_rt::remove_stale_dir(&e.path()).with_context(|| format!("stale staging {}", e.path().display()))?;
        }
    }
    let leaf = final_dir.file_name().context("archive name without a leaf")?.to_string_lossy();
    let tmp = parent.join(format!(".tmp-{leaf}"));
    std::fs::create_dir_all(&tmp)?;
    Ok(tmp)
}

/// Make every file of `dir` durable and drop it from the page cache, so the check that follows reads
/// the disk: the archive disk dropped writes that had verified from memory (2026-09-27, 0307/0308).
fn sync_tree(dir: &Path) -> Result<usize> {
    let mut n = 0;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() {
            sv10_rt::sync_file(&path)?;
            sv10_rt::evict_cache(&path)?;
            n += 1;
        }
    }
    sv10_rt::sync_dir(&dir.join("."))?;
    Ok(n)
}

/// Seal the manifest, make everything durable, verify it end to end from the disk, then rename the
/// staging dir into place.
fn finish(root: &Path, tmp: &Path, manifest: Manifest) -> Result<Manifest> {
    let path = tmp.join(MANIFEST);
    std::fs::write(&path, serde_json::to_string_pretty(&manifest)?)?;
    std::fs::write(tmp.join(format!("{MANIFEST}.sha256")), crate::integrity::file_sha256(&path)?)?;
    sync_tree(tmp)?;
    let problems = verify(tmp, true);
    if !problems.is_empty() {
        let left = sv10_rt::remove_stale_dir(tmp).err().map(|e| format!(" (its staging dir was left behind: {e})"));
        bail!("{} failed verification after writing: {}{}", manifest.name, problems.join("; "), left.unwrap_or_default());
    }
    let final_dir = root.join(&manifest.name);
    // An older copy under the same name is replaced; failing to remove it makes the rename fail below.
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(tmp, &final_dir)?;
    sv10_rt::sync_dir(&final_dir)?;
    Ok(manifest)
}

fn vacuum_into(db: &Path, to: &Path) -> Result<()> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).with_context(|| format!("open {}", db.display()))?;
    conn.execute("VACUUM INTO ?1", [to.to_string_lossy()])?;
    Ok(())
}

/// Compress `raw` next to itself as `<name>.zst`, remove `raw`, and describe the result.
fn compress_entry(raw: &Path, role: Role, level: u32, rows: BTreeMap<String, i64>, watermarks: BTreeMap<String, i64>) -> Result<Entry> {
    let raw_sha256 = crate::integrity::file_sha256(raw)?;
    let raw_bytes = std::fs::metadata(raw)?.len();
    let mut zst = raw.as_os_str().to_owned();
    zst.push(".zst");
    let zst = PathBuf::from(zst);
    zstd(&[&format!("-{level}"), "-T0", "--long=27", "-q", "-f"], Some(raw), Some(&zst))?;
    std::fs::remove_file(raw)?;
    Ok(Entry {
        name: zst.file_name().context("no file name")?.to_string_lossy().to_string(),
        role,
        bytes: std::fs::metadata(&zst)?.len(),
        sha256: crate::integrity::file_sha256(&zst)?,
        raw_bytes,
        raw_sha256,
        rows,
        watermarks,
    })
}

fn git_head(repo: &Path) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(["rev-parse", "HEAD"]).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A table name as a SQL identifier. These names are read out of an *archive* — an external file
/// under `/backup-disk` — and interpolated straight into DDL and DML, so a name containing a quote would
/// close the identifier and the rest would run as SQL. SQLite's own rule is to double the quote
/// inside a quoted identifier (0248).
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn tables(conn: &Connection, schema: &str) -> Result<Vec<String>> {
    let mut st =
        conn.prepare(&format!("SELECT name FROM {schema}.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"))?;
    let names = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// Whether `table` has an `INTEGER PRIMARY KEY` column named `id` (an append-ordered rowid alias).
fn has_id_key(conn: &Connection, schema: &str, table: &str) -> Result<bool> {
    let mut st = conn.prepare(&format!("PRAGMA {schema}.table_info({})", ident(table)))?;
    let cols =
        st.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(5)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(cols.iter().any(|(name, ty, pk)| name == "id" && ty.eq_ignore_ascii_case("INTEGER") && *pk == 1))
}

fn row_counts(conn: &Connection, schema: &str) -> Result<BTreeMap<String, i64>> {
    let mut out = BTreeMap::new();
    for t in tables(conn, schema)? {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {schema}.{}", ident(&t)), [], |r| r.get(0))?;
        out.insert(t, n);
    }
    Ok(out)
}

fn id_watermarks(conn: &Connection, schema: &str) -> Result<BTreeMap<String, i64>> {
    let mut out = BTreeMap::new();
    for t in tables(conn, schema)? {
        if has_id_key(conn, schema, &t)? {
            let m: i64 = conn.query_row(&format!("SELECT COALESCE(MAX(id), 0) FROM {schema}.{}", ident(&t)), [], |r| r.get(0))?;
            out.insert(t, m);
        }
    }
    Ok(out)
}

/// Write the rows of `history` above `marks` (and every row of tables without a watermark) into
/// a new database at `to`, in one read snapshot. Returns the source's full row counts at that
/// snapshot, which a restore must reproduce.
fn write_delta(history: &Path, to: &Path, marks: &BTreeMap<String, i64>) -> Result<BTreeMap<String, i64>> {
    sv10_rt::remove_stale_file(to).with_context(|| format!("clearing the delta at {}", to.display()))?;
    let conn =
        Connection::open_with_flags(to, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_URI)?;
    let uri = format!("file:{}?mode=ro", history.to_string_lossy().replace('?', "%3f").replace('#', "%23"));
    conn.execute("ATTACH DATABASE ?1 AS src", [uri])?;
    conn.execute_batch("BEGIN")?;
    let counts = row_counts(&conn, "src")?;
    for t in tables(&conn, "src")? {
        match marks.get(&t) {
            Some(mark) => {
                conn.execute(&format!("CREATE TABLE main.{} AS SELECT * FROM src.{} WHERE id > {mark}", ident(&t), ident(&t)), [])?
            }
            None => conn.execute(&format!("CREATE TABLE main.{} AS SELECT * FROM src.{}", ident(&t), ident(&t)), [])?,
        };
    }
    conn.execute_batch("COMMIT")?;
    conn.execute("DETACH DATABASE src", [])?;
    Ok(counts)
}

/// The column names of `schema`.`table`, in the order the table stores them.
fn columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut st = conn.prepare(&format!("PRAGMA {schema}.table_info({})", ident(table)))?;
    let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<Vec<String>, _>>()?;
    Ok(names)
}

/// Merge a differential into a restored full: id-keyed tables gain the new rows, other tables are
/// replaced by the delta's copy.
///
/// The columns are **named on both sides**. `SELECT *` maps by position, so a column added or
/// reordered between the full backup and the differential would put values in the wrong columns
/// with no error at all when the count happens to match — a restore that silently misplaces `net`,
/// `digest` or `scale` corrupts every record the review and learning stack reads. A column the
/// differential does not have is an error, not a silent skip (0249).
fn apply_delta(full: &Path, delta: &Path, marks: &BTreeMap<String, i64>) -> Result<()> {
    let conn = Connection::open(full)?;
    conn.execute("ATTACH DATABASE ?1 AS d", [delta.to_string_lossy()])?;
    conn.execute_batch("BEGIN")?;
    for t in tables(&conn, "d")? {
        let cols = columns(&conn, "d", &t)?;
        anyhow::ensure!(!cols.is_empty(), "differential table {} has no columns", t);
        let list = cols.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ");
        let wanted = match columns(&conn, "main", &t) {
            Ok(target) => {
                // The full may carry columns the differential predates; map by name and take the
                // delta's value for the ones it has, leaving the rest as they are.
                let known: Vec<&String> = cols.iter().filter(|c| target.contains(c)).collect();
                anyhow::ensure!(
                    known.len() == cols.len(),
                    "differential {t} has columns the restored full does not: {:?}",
                    cols.iter().filter(|c| !target.contains(c)).collect::<Vec<_>>()
                );
                known.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ")
            }
            Err(_) => list.clone(),
        };
        if marks.contains_key(&t) {
            conn.execute(&format!("INSERT OR IGNORE INTO main.{} ({wanted}) SELECT {wanted} FROM d.{}", ident(&t), ident(&t)), [])?;
        } else {
            conn.execute(&format!("DELETE FROM main.{}", ident(&t)), [])?;
            conn.execute(&format!("INSERT INTO main.{} ({wanted}) SELECT {wanted} FROM d.{}", ident(&t), ident(&t)), [])?;
        }
    }
    conn.execute_batch("COMMIT")?;
    conn.execute("DETACH DATABASE d", [])?;
    Ok(())
}

fn restore_db(from: &Path, to: &Path, e: &Entry) -> Result<()> {
    decompress_checked(from, to, e)?;
    let conn = Connection::open_with_flags(to, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    crate::integrity::check_connection(&conn).map_err(|err| anyhow::anyhow!("restored {} failed its check: {err}", to.display()))?;
    let counts = row_counts(&conn, "main")?;
    if counts != e.rows {
        bail!("restored {} rows {counts:?} differ from the archive's {:?}", to.display(), e.rows);
    }
    Ok(())
}

fn decompress_checked(from: &Path, to: &Path, e: &Entry) -> Result<()> {
    zstd(&["-d", "--long=27", "-q", "-f"], Some(from), Some(to))?;
    if crate::integrity::file_sha256(to)? != e.raw_sha256 {
        let left = sv10_rt::remove_stale_file(to).err().map(|err| format!("; it is still at {} ({err})", to.display()));
        bail!("{} decompressed to different content{}", e.name, left.unwrap_or_default());
    }
    Ok(())
}

fn zstd(args: &[&str], input: Option<&Path>, output: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new("zstd");
    cmd.args(args).stdout(Stdio::null()).stderr(Stdio::piped());
    if let Some(i) = input {
        cmd.arg(i);
    }
    if let Some(o) = output {
        cmd.arg("-o").arg(o);
    }
    let out = cmd.output().context("running zstd (install the zstd package)")?;
    if !out.status.success() {
        bail!("zstd {args:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Recompress a zstd file at `level` without an uncompressed copy on disk.
fn recompress(from: &Path, to: &Path, level: u32) -> Result<()> {
    let mut dec = Command::new("zstd").args(["-dc", "--long=27", "-q"]).arg(from).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let pipe = dec.stdout.take().context("zstd stdout")?;
    let enc = Command::new("zstd")
        .args([&format!("-{level}"), "-T0", "--long=27", "-q", "-f", "-o"])
        .arg(to)
        .stdin(Stdio::from(pipe))
        .stderr(Stdio::null())
        .status()?;
    let dec_status = dec.wait()?;
    if !enc.success() || !dec_status.success() {
        bail!("recompressing {} failed", from.display());
    }
    Ok(())
}

/// SHA-256 and length of a zstd file's decompressed content, streamed.
fn zstd_content_sha(path: &Path) -> Result<(String, u64)> {
    use sv10_digest::Sha256;
    let mut child = Command::new("zstd").args(["-dc", "--long=27", "-q"]).arg(path).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut out = child.stdout.take().context("zstd stdout")?;
    let (mut h, mut n, mut buf) = (Sha256::new(), 0u64, vec![0u8; 1 << 20]);
    loop {
        let k = out.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
    if !child.wait()?.success() {
        bail!("{} is not a valid zstd file", path.display());
    }
    Ok((sv10_digest::hex(h.finalize()), n))
}

#[cfg(test)]
mod tests;
