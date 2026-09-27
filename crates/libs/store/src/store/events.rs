//! The activity log: best-effort appends, never fatal.
use super::*;
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
}
