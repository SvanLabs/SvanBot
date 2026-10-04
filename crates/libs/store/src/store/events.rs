//! The activity log: best-effort appends, never fatal.
use super::*;
use anyhow::Result;
use rusqlite::params;

/// Days an ordinary event is kept. About 7,000 land per day, and the dashboard reads the newest.
pub const EVENTS_INFO_DAYS: i64 = 7;

/// Days a warn or error event is kept: the timeline reads these by time range, they are about 1.5%
/// of the log, and a count cap would have dropped them with the noise (#778).
pub const EVENTS_ALERT_DAYS: i64 = 90;

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

    /// Drop events past their retention: ordinary ones after `info_days`, warn and error after
    /// `alert_days`. Returns the rows removed. The cut is a text comparison on the RFC 3339 UTC stamp
    /// the log writes, so it needs no index at this size.
    pub fn prune_events(&self, info_days: i64, alert_days: i64) -> Result<usize> {
        let cut = |days: i64| (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        Ok(self.write_lock().execute(
            "DELETE FROM events WHERE ts < CASE WHEN level IN ('warn', 'error') THEN ?2 ELSE ?1 END",
            params![cut(info_days), cut(alert_days)],
        )?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_bounded_by_age_and_the_alerts_outlive_the_noise() {
        let dir = std::env::temp_dir().join(format!("sv10-store-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        let ago = |days: i64| (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let insert = |days: i64, level: Option<&str>, message: &str| {
            store
                .write_lock()
                .execute("INSERT INTO events (ts, bot, level, message) VALUES (?1,'A',?2,?3)", params![ago(days), level, message])
                .unwrap();
        };
        // A growing log: a month of ordinary lines, one warn at 30 days and one at 100, an error at 8
        // days, a row with no level, and today's lines written through the normal path.
        for day in 0..30 {
            insert(day, Some("info"), "ordinary");
        }
        insert(30, Some("warn"), "kept: a warn inside 90 days");
        insert(100, Some("warn"), "dropped: a warn past 90 days");
        insert(8, Some("error"), "kept: an error inside 90 days");
        insert(8, None, "dropped: no level reads as ordinary");
        store.log_event("A", "info", "today");
        let before: i64 = store.read().query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(before, 35);

        // Info lines at 7 to 29 days (23; the 7-day row was written before the cut), the unlabelled row
        // and the 100-day warn go: 25 rows.
        assert_eq!(store.prune_events(EVENTS_INFO_DAYS, EVENTS_ALERT_DAYS).unwrap(), 25);
        let kept: Vec<String> = store
            .read()
            .prepare("SELECT message FROM events ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(kept.iter().filter(|m| *m == "ordinary").count(), 7, "days 0 to 6");
        assert!(kept.iter().any(|m| m.starts_with("kept: a warn")) && kept.iter().any(|m| m.starts_with("kept: an error")));
        assert!(kept.iter().all(|m| !m.starts_with("dropped")), "{kept:?}");
        assert_eq!(store.prune_events(EVENTS_INFO_DAYS, EVENTS_ALERT_DAYS).unwrap(), 0, "a second pass removes nothing");
        // What the timeline and the monitor read is untouched.
        assert_eq!(store.timeline_events(&ago(60), &ago(-1)).unwrap().len(), 2);
        assert_eq!(store.errors_after(0, 10).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
