//! state_hash mismatch incidents: rare, serious, and therefore kept with their evidence
//! instead of as a counter (0265). Capped at 50 rows; a mismatch storm still fits in kilobytes.
use super::*;
use rusqlite::params;

/// One diagnosed `state_hash` mismatch.
pub struct Incident {
    /// Row id, ascending.
    pub id: i64,
    /// RFC 3339 UTC.
    pub ts: String,
    /// Bot that saw it.
    pub bot: String,
    /// Table id, if the tracker knew it.
    pub table: String,
    /// Snapshot's `table_seq` (-1 when absent).
    pub table_seq: i64,
    /// `STALE` (replayed frame) or `DIVERGED` (real divergence).
    pub verdict: String,
    /// One-line field census from `mismatch_report`.
    pub summary: String,
    /// Full canonical bytes the server hashed (the evidence a resync throws away).
    pub canonical: String,
}

/// Rows kept; mismatches are rare enough that the cap is headroom, not policy.
pub const INCIDENTS_KEPT: i64 = 50;

impl Store {
    /// Record a diagnosed mismatch and prune past the cap. Best-effort like the event log:
    /// a failed write is logged, never fatal.
    pub fn record_hash_incident(&self, bot: &str, table: &str, table_seq: i64, verdict: &str, summary: &str, canonical: &str) {
        let write = || -> Result<()> {
            let conn = self.write_lock();
            conn.execute(
                "INSERT INTO hash_incidents (ts, bot, table_id, table_seq, verdict, summary, canonical)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![chrono::Utc::now().to_rfc3339(), bot, table, table_seq, verdict, summary, canonical],
            )?;
            conn.execute(
                "DELETE FROM hash_incidents WHERE id NOT IN
                 (SELECT id FROM hash_incidents ORDER BY id DESC LIMIT ?1)",
                params![INCIDENTS_KEPT],
            )?;
            Ok(())
        };
        if let Err(e) = write() {
            tracing::warn!("hash incident write failed ({e})");
        }
    }

    /// Newest incidents first.
    pub fn hash_incidents(&self, limit: usize) -> Result<Vec<Incident>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT id, ts, bot, table_id, table_seq, verdict, summary, canonical
             FROM hash_incidents ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(Incident {
                id: r.get(0)?,
                ts: r.get(1)?,
                bot: r.get(2)?,
                table: r.get(3)?,
                table_seq: r.get(4)?,
                verdict: r.get(5)?,
                summary: r.get(6)?,
                canonical: r.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(anyhow::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incidents_keep_the_newest_with_their_evidence() {
        let dir = std::env::temp_dir().join(format!("sv10-store-incidents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for i in 0..(INCIDENTS_KEPT + 5) {
            store.record_hash_incident(
                "A",
                "t",
                i,
                if i % 2 == 0 { "STALE" } else { "DIVERGED" },
                "pot=number",
                &format!("{{\"seq\":{i}}}"),
            );
        }
        let kept = store.hash_incidents(1000).unwrap();
        assert_eq!(kept.len() as i64, INCIDENTS_KEPT);
        assert_eq!(kept[0].table_seq, INCIDENTS_KEPT + 4, "newest first");
        assert_eq!(kept.last().unwrap().table_seq, 5, "the oldest five pruned");
        assert_eq!(kept[0].canonical, format!("{{\"seq\":{}}}", INCIDENTS_KEPT + 4));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
