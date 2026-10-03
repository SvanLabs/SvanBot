//! The activity log: best-effort appends, never fatal.
use super::*;
use anyhow::Result;
use rusqlite::params;

impl Store {
    /// Append to the activity log (a failed write is logged, never fatal).
    pub fn log_event(&self, bot: &str, level: &str, message: &str) {
        if let Err(e) = self.write_lock().execute(
            "INSERT INTO events (ts, bot, level, message) VALUES (?1,?2,?3,?4)",
            params![chrono::Utc::now().to_rfc3339(), bot, level, message],
        ) {
            tracing::warn!("event log write failed ({e}): [{bot}] {level}: {message}");
        }
    }

    /// Highest event id (0 when empty): the watermark the monitor reads errors after.
    pub fn max_event_id(&self) -> Result<i64> {
        Ok(self.read().query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| r.get(0))?)
    }

    /// Error-level events after `id`, oldest first, at most `limit`: (id, bot, message).
    pub fn errors_after(&self, id: i64, limit: usize) -> Result<Vec<(i64, String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT id, bot, message FROM events WHERE id > ?1 AND level = 'error' ORDER BY id LIMIT ?2")?;
        let rows = stmt
            .query_map(params![id, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default())))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
