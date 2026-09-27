//! Bit-exact replay records of big decisions and their response networks.
use super::*;
use anyhow::Result;
use rusqlite::params;

/// One stored replay record (see `sv10_bot::replay`).
#[derive(Clone, Debug)]
pub struct ReplayRow {
    /// Row id.
    pub id: i64,
    /// Decision time, RFC 3339.
    pub ts: String,
    /// Bot that decided.
    pub bot: String,
    /// Hand id ('' when unknown).
    pub hand_id: String,
    /// `ReplayRecord` JSON.
    pub record: String,
}

impl Store {
    /// Store the full inputs of a big decision (`record` JSON) for bit-exact replay; the response
    /// network it used is stored once per content digest.
    pub fn insert_replay(&self, bot: &str, hand_id: &str, record: &str, net: Option<(&str, &str)>) -> Result<()> {
        // Compressed before the write lock, which then covers only the write (0322); only a due
        // dictionary reload packs under it.
        let cached = self.codec.pack_cached(crate::packed::REPLAY_RECORD, record);
        let conn = self.write_lock();
        let packed = cached.unwrap_or_else(|| self.codec.pack(&conn, crate::packed::REPLAY_RECORD, record));
        let tx = conn.unchecked_transaction()?;
        if let Some((digest, json)) = net {
            tx.execute("INSERT OR IGNORE INTO replay_nets (digest, json) VALUES (?1, ?2)", params![digest, json])?;
        }
        tx.execute(
            "INSERT INTO replays (ts, bot, hand_id, net_digest, record) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chrono::Utc::now().to_rfc3339(), bot, hand_id, net.map(|n| n.0), packed],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Replay records, newest first; `id` restricts to one row.
    pub fn replays(&self, limit: usize, id: Option<i64>) -> Result<Vec<ReplayRow>> {
        let conn = self.read();
        let mut st = conn.prepare(
            "SELECT id, ts, bot, COALESCE(hand_id, ''), record FROM replays WHERE ?1 IS NULL OR id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st
            .query_map(params![id, limit as i64], |r| {
                Ok(ReplayRow { id: r.get(0)?, ts: r.get(1)?, bot: r.get(2)?, hand_id: r.get(3)?, record: self.codec.text(r.get_ref(4)?)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Stored replay records and the newest one's time.
    pub fn replay_stats(&self) -> Result<(i64, Option<String>)> {
        Ok(self.read().query_row("SELECT COUNT(*), MAX(ts) FROM replays", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }
    /// The response network stored under `digest`.
    pub fn replay_net(&self, digest: &str) -> Result<Option<String>> {
        let conn = self.read();
        let mut st = conn.prepare("SELECT json FROM replay_nets WHERE digest = ?1")?;
        let mut rows = st.query_map([digest], |r| r.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }
    /// Delete replay records older than `days` and networks no remaining record uses; returns rows removed.
    pub fn prune_replays(&self, days: i64) -> Result<usize> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let conn = self.write_lock();
        let n = conn.execute("DELETE FROM replays WHERE ts < ?1", [cutoff])?;
        conn.execute(
            "DELETE FROM replay_nets WHERE digest NOT IN (SELECT net_digest FROM replays WHERE net_digest IS NOT NULL)
               AND digest NOT IN (SELECT net_digest FROM audit_queue WHERE net_digest IS NOT NULL)",
            [],
        )?;
        Ok(n)
    }
}
