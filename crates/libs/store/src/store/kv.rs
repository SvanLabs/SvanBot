//! Key-value state: models, params, status flags. Keys are `&str` constants owned by
//! the calling domain (see `sv10_bot`); this seam only stores them.
use super::*;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

impl Store {
    /// Store a value; an identical value is not rewritten (no page change, no SSD write).
    pub fn put_kv(&self, key: &str, value: &str) -> Result<()> {
        self.write_lock().execute(
            "INSERT INTO kv (key, value, updated) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated
             WHERE kv.value IS NOT excluded.value",
            params![key, value, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
    /// Atomically replace a related set of durable frontiers (for example champion params and
    /// lineage). Either every key advances or none does.
    pub fn put_kv_batch(&self, entries: &[(&str, &str)]) -> Result<()> {
        let conn = self.write_lock();
        let tx = conn.unchecked_transaction()?;
        let updated = chrono::Utc::now().to_rfc3339();
        for (key, value) in entries {
            tx.execute(
                "INSERT INTO kv (key, value, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated
                 WHERE kv.value IS NOT excluded.value",
                params![key, value, updated],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    /// When `key` was last written (RFC 3339 UTC).
    pub fn kv_updated(&self, key: &str) -> Result<Option<String>> {
        Ok(self.read().query_row("SELECT updated FROM kv WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }
    /// Value stored under `key`.
    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        Ok(self.read().query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }
    /// Every (key, value) whose key starts with `prefix` (taken literally, not as a pattern), by key.
    pub fn kv_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT key, value FROM kv WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")?;
        let rows = stmt.query_map(params![prefix], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_see_committed_writes_and_never_block_them() {
        let dir = std::env::temp_dir().join(format!("sv10-store-readers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        store.put_kv("k", "1").unwrap();
        assert_eq!(store.get_kv("k").unwrap().as_deref(), Some("1"));
        // Every reader busy: a write still goes through, and the next read sees it.
        let held: Vec<_> = store.readers.iter().map(|r| r.lock()).collect();
        store.put_kv("k", "2").unwrap();
        drop(held);
        assert_eq!(store.get_kv("k").unwrap().as_deref(), Some("2"));
        // An open read transaction does not stop the writer (WAL).
        let reader = store.read();
        reader.execute_batch("BEGIN; SELECT COUNT(*) FROM kv;").unwrap();
        store.put_kv("k", "3").unwrap();
        reader.execute_batch("COMMIT;").unwrap();
        drop(reader);
        assert_eq!(store.get_kv("k").unwrap().as_deref(), Some("3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn promotion_frontier_batch_rolls_back_on_late_write_failure() {
        let dir = std::env::temp_dir().join(format!("sv10-store-promotion-frontier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        store.put_kv_batch(&[("params", "old-params"), ("lineage", "old-lineage")]).unwrap();
        store
            .conn
            .lock()
            .execute_batch(
                "CREATE TRIGGER fail_lineage BEFORE UPDATE ON kv
                 WHEN NEW.key = 'lineage' BEGIN SELECT RAISE(ABORT, 'injected late frontier failure'); END;",
            )
            .unwrap();
        assert!(store.put_kv_batch(&[("params", "new-params"), ("lineage", "new-lineage")]).is_err());
        assert_eq!(store.get_kv("params").unwrap().as_deref(), Some("old-params"));
        assert_eq!(store.get_kv("lineage").unwrap().as_deref(), Some("old-lineage"));
        store.conn.lock().execute_batch("DROP TRIGGER fail_lineage;").unwrap();
        store.put_kv_batch(&[("params", "new-params"), ("lineage", "new-lineage")]).unwrap();
        assert_eq!(store.get_kv("params").unwrap().as_deref(), Some("new-params"));
        assert_eq!(store.get_kv("lineage").unwrap().as_deref(), Some("new-lineage"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
