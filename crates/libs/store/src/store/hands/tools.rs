//! Reads the command-line tools make against stored hands: the monitor's rowid window (0317) and
//! the offline check that matches server exports back to our rows by hand id (0151). The API's own
//! reads do not need either, which is why they live away from them.

use super::super::*;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

/// (rowid, bot, hand id, net chips, summary json, settled pot, comma-separated winner names).
pub type MonitorRow = (i64, String, String, Option<i64>, String, i64, String);

impl Store {
    /// Every hand stored after `rowid` with its result and id, oldest first:
    /// (rowid, bot, hand_id, net, summary, pot, winners).
    ///
    /// Treatment-arm hands are included, unlike [`Store::ordinary_results_after`]: the monitor's
    /// window is a results read where the chips are the answer (0361), and the only part of it that
    /// needs champion play — the head-to-head ledger — is the ordinary read.
    pub fn results_after(&self, rowid: i64) -> Result<Vec<MonitorRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT rowid, bot, hand_id, net, COALESCE(summary, ''), COALESCE(pot, 0), COALESCE(winners, '') FROM hands
             WHERE rowid > ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map(params![rowid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// (bot, newest `ended_at`) of every bot with a stored hand; `ended_at` may be empty.
    pub fn last_hand_per_bot(&self) -> Result<Vec<(String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT bot, COALESCE(MAX(ended_at), '') FROM hands GROUP BY bot")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Which of `hand_ids` this store has a hand for, by id alone — a renamed bot's older hands the
    /// server later labels with its new name still count as recorded (0151).
    pub fn recorded_hand_ids(&self, hand_ids: &[String]) -> Result<HashSet<String>> {
        let conn = self.read();
        let mut out = HashSet::new();
        // Batches keep the statement under SQLite's variable limit; the placeholders carry no data.
        for chunk in hand_ids.chunks(500) {
            let list = vec!["?"; chunk.len()].join(",");
            let mut stmt = conn.prepare(&format!("SELECT DISTINCT hand_id FROM hands WHERE hand_id IN ({list})"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |r| r.get::<_, String>(0))?;
            for id in rows {
                out.insert(id?);
            }
        }
        Ok(out)
    }
}
