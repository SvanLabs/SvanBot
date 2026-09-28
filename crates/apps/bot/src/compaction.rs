//! Background compaction of the cold JSON columns (0229).
//!
//! New rows are packed as they are written (`sv10_store::packed`); this loop packs the rows written
//! before, in both databases: 500 rows per column per round, each round one short transaction, so
//! the importer and the bots never wait long. It trains each column's dictionary first (from the
//! column's own newest rows). When nothing is left it returns `history.db`'s freed pages to the disk
//! with a VACUUM (nothing plays from that file, and every table there has an INTEGER PRIMARY KEY, so
//! no rowid moves); the main database's freed pages are reused by new rows (`archive compact
//! --vacuum-main` returns them with the fleet stopped). Progress is in kv `compaction.status` and
//! the fleet log.

use crate::history::HistoryDb;
use crate::live::Shared;
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use sv10_store::store::Store;

/// Where the loop reports progress.
pub const STATUS_KEY: &str = "compaction.status";
/// Rows per column per round.
pub const BATCH: usize = 500;
/// Freed space that makes a VACUUM worth its rewrite: 128 MB and a fifth of the file.
const VACUUM_MIN_FREE: u64 = 128 << 20;

/// Resume points of one pass over both databases' columns (`None` once a column is done).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursors {
    /// Main database columns (decision details, replay records, audit records).
    pub main: [Option<i64>; 3],
    /// `history.db` columns (export JSON, export summaries, corpus summaries).
    pub history: [Option<i64>; 3],
}

impl Cursors {
    /// A pass from the first row.
    pub fn start() -> Cursors {
        Cursors { main: [Some(0); 3], history: [Some(0); 3] }
    }

    /// Whether every column is done.
    pub fn done(&self) -> bool {
        self.main.iter().chain(&self.history).all(|c| c.is_none())
    }
}

/// One round: a batch of every unfinished column. Returns rows packed.
pub fn step(store: &Store, history: Option<&HistoryDb>, cursors: &mut Cursors, batch: usize) -> Result<usize> {
    let mut packed = store.compact(&mut cursors.main, batch)?;
    match history {
        Some(db) => packed += db.compact(&mut cursors.history, batch)?,
        None => cursors.history = [None; 3],
    }
    Ok(packed)
}

/// Whether `free` bytes in a file of `size` bytes make a VACUUM worthwhile.
pub fn worth_vacuum(free: u64, size: u64) -> bool {
    free >= VACUUM_MIN_FREE && free.saturating_mul(5) >= size
}

/// Text rows left per column, both databases.
fn remaining(store: &Store, history: Option<&HistoryDb>) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    let main = store.text_rows();
    let hist = history.map(|h| h.text_rows()).transpose();
    for (db, rows) in [("main", main.ok()), ("history", hist.ok().flatten())] {
        for (family, n) in rows.unwrap_or_default() {
            out.insert(format!("{db}:{family}"), json!(n));
        }
    }
    out
}

/// Start the loop (the fleet head or the all-in-one fleet: one process packs).
pub fn spawn(shared: &Arc<Shared>) {
    let shared = shared.clone();
    tokio::spawn(async move {
        // After startup's own work (integrity checks, model load, first tables).
        tokio::time::sleep(Duration::from_secs(90)).await;
        let path = shared.config.artifacts.join("history.db");
        let history: Option<Arc<HistoryDb>> = match HistoryDb::open(&path) {
            Ok(db) => Some(Arc::new(db)),
            Err(e) => {
                tracing::warn!("compaction: history.db not opened ({e}); packing the main database only");
                None
            }
        };
        let mut total = 0usize;
        let mut reported = false;
        loop {
            let mut cursors = Cursors::start();
            let mut pass = 0usize;
            while !cursors.done() {
                let (s, h) = (shared.clone(), history.clone());
                let round = crate::jobs::blocking("compaction", move || {
                    let mut c = cursors;
                    step(&s.store, h.as_deref(), &mut c, BATCH).map(|n| (n, c))
                })
                .await;
                match round {
                    Some(Ok((n, c))) => {
                        cursors = c;
                        pass += n;
                        tokio::time::sleep(Duration::from_millis(if n > 0 { 250 } else { 0 })).await;
                    }
                    Some(Err(e)) => {
                        tracing::warn!("compaction round failed (retrying in 10 minutes): {e}");
                        break;
                    }
                    None => break,
                }
            }
            total += pass;
            let (s, h) = (shared.clone(), history.clone());
            let report = pass > 0 || !reported;
            let p = path.clone();
            crate::jobs::blocking("compaction status", move || finish_pass(&s, h.as_deref(), &p, pass, total, report)).await;
            reported = true;
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
}

/// After a pass: return freed pages when worth it, and report.
fn finish_pass(shared: &Shared, history: Option<&HistoryDb>, path: &std::path::Path, pass: usize, total: usize, report: bool) {
    let mut vacuumed = serde_json::Map::new();
    if let Some(db) = history {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        // Free pages *and* the space left inside pages by rows that shrank in place: the free-page count
        // alone missed 780 MB of a 971 MB file (2026-09-27).
        if let Ok(free) = db.reclaimable_bytes()
            && worth_vacuum(free, size)
        {
            match db.vacuum(path) {
                Ok((before, after)) => {
                    shared.log("fleet", "info", format!("history.db compacted: {} MB -> {} MB", before >> 20, after >> 20));
                    vacuumed.insert("history".into(), json!([before, after]));
                }
                Err(e) => tracing::warn!("history.db VACUUM failed: {e}"),
            }
        }
    }
    if !report && vacuumed.is_empty() {
        return;
    }
    if pass > 0 {
        shared.log("fleet", "info", format!("compaction: packed {pass} stored rows ({total} since start)"));
    }
    let status = json!({
        "updated": chrono::Utc::now().to_rfc3339(),
        "packed_this_pass": pass,
        "packed_since_start": total,
        "text_rows_left": remaining(&shared.store, history),
        "free_bytes": {"main": shared.store.free_bytes().ok(), "history": history.and_then(|h| h.free_bytes().ok())},
        "vacuumed": vacuumed,
    });
    if let Err(e) = shared.store.put_kv(STATUS_KEY, &status.to_string()) {
        tracing::warn!("compaction status not stored: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacuum_only_when_the_free_space_is_large_and_a_real_share() {
        assert!(!worth_vacuum(100 << 20, 200 << 20), "under 128 MB");
        assert!(!worth_vacuum(200 << 20, 2_000 << 20), "a tenth of the file");
        assert!(worth_vacuum(700 << 20, 1_000 << 20));
    }

    /// 2026-09-27: history.db was 971 MB of pages holding 182 MB of data. Rows shrunk in place (packing,
    /// summaries rewritten) leave their space unused *inside* pages, which the free-page count never
    /// sees, so the VACUUM that would return 780 MB never ran. Reclaimable space counts both.
    #[test]
    fn space_left_inside_pages_counts_as_reclaimable() {
        let dir = std::env::temp_dir().join(format!("sv10-compaction-slack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        let history = HistoryDb::open(&path).unwrap();
        // Incompressible padding (packing would shrink a repeated character to nothing), so each row
        // really takes its pages before it shrinks.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut noise = || {
            (0..3_000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    char::from(b'a' + (seed % 26) as u8)
                })
                .collect::<String>()
        };
        let page: Vec<serde_json::Value> =
            (0..400).map(|i| json!({"hand_id": format!("h{i}"), "table_id": "t", "hand_number": i, "profit": 1, "pad": noise()})).collect();
        history.insert_page("A", &page).unwrap();
        // Every row shrinks in place, as packing does: the pages stay, mostly empty.
        history.shrink_rows_for_test().unwrap();
        let (free, reclaimable) = (history.free_bytes().unwrap(), history.reclaimable_bytes().unwrap());
        assert!(reclaimable > free + 500_000, "the slack inside pages is counted: free {free}, reclaimable {reclaimable}");
        history.vacuum(&path).unwrap();
        let left = history.reclaimable_bytes().unwrap();
        assert!(left < reclaimable / 4, "and a VACUUM returns it: {reclaimable} reclaimable before, {left} after");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pass_packs_both_databases_and_ends() {
        let dir = std::env::temp_dir().join(format!("sv10-compaction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        let history = HistoryDb::open(&dir.join("history.db")).unwrap();
        let page: Vec<serde_json::Value> = (0..250)
            .map(|i| json!({"hand_id": format!("h{i}"), "table_id": "t", "hand_number": i, "profit": i - 100, "players": ["a", "b"]}))
            .collect();
        history.insert_page("A", &page).unwrap();
        for i in 0..30 {
            store
                .insert_decision("A", &format!("h{i}"), "flop", "call", None, Some(0.4), 100, 20, 2.0, &json!({"opponents": 1}).to_string())
                .unwrap();
        }
        let mut cursors = Cursors::start();
        let mut packed = 0;
        let mut rounds = 0;
        while !cursors.done() {
            packed += step(&store, Some(&history), &mut cursors, 100).unwrap();
            rounds += 1;
            assert!(rounds < 50, "a pass ends");
        }
        // The exports were packed at insert without a dictionary; the pass trains one (250 rows)
        // and repacks them against it. Too few decisions for a dictionary: they stay as written.
        assert_eq!(packed, 250);
        assert_eq!(history.profit("A", "h7"), Some(-93));
        assert!(history.text_rows().unwrap().iter().all(|(_, n)| *n == 0));
    }
}
