//! What the ship bar's canary stage reads of a store (V11-22): when the bot first took a release,
//! what it won on each hand since, and the three fields of each export hand it needs.
//!
//! A release is marked in `meta`, which is append-only, under `release <version>`: the first
//! mark stands, so a bot restarted on the same release keeps the hour it took it.

use rusqlite::params;
use rusqlite::types::Value;

use super::{Store, StoreError};

/// One export hand as the ship bar reads it: when it started, as the venue wrote it, and what it
/// won, in chips, at what big blind. A field the item does not carry, or carries as something
/// else, is `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportLine {
    pub started_at: Option<String>,
    pub profit: Option<f64>,
    pub big_blind: Option<f64>,
}

impl Store {
    /// Marks that the bot took `version` at `recorded_ms`, if it has not before.
    pub fn mark_release(&self, version: &str, recorded_ms: i64) -> Result<(), StoreError> {
        self.db.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING",
            params![format!("release {version}"), recorded_ms.to_string()],
        )?;
        Ok(())
    }

    /// When the bot first took `version`, if it has.
    pub fn released(&self, version: &str) -> Result<Option<i64>, StoreError> {
        let mut query = self.db.prepare("SELECT value FROM meta WHERE key = ?1")?;
        let mut rows = query.query([format!("release {version}")])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let value: String = row.get(0)?;
        value
            .parse()
            .map(Some)
            .map_err(|_| StoreError(format!("release {version} is marked at {value:?}")))
    }

    /// What each hand kept at or after `since_ms` won, in big blinds, with when it was kept, in
    /// the order they were kept.
    pub fn results_since(&self, since_ms: i64) -> Result<Vec<(i64, f64)>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT recorded_ms, net, big_blind FROM played WHERE recorded_ms >= ?1 ORDER BY rowid",
        )?;
        let rows = query.query_map([since_ms], |row| {
            let (at, net, big_blind): (i64, i64, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
            Ok((at, net as f64 / big_blind as f64))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The luck-corrected result of each hand kept at or after `since_ms`, in big blinds.
    pub fn corrected_since(&self, since_ms: i64) -> Result<Vec<f64>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT (net - luck) / big_blind FROM played
             WHERE recorded_ms >= ?1 ORDER BY rowid",
        )?;
        let rows = query.query_map([since_ms], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every export hand's start and result. The item is read in SQLite, so a season's export is
    /// never held whole in memory.
    pub fn export_lines(&self) -> Result<Vec<ExportLine>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT json_extract(frames, '$[0].started_at'),
                    json_extract(frames, '$[0].profit'),
                    json_extract(frames, '$[0].big_blind')
             FROM hands WHERE source = 'history' ORDER BY rowid",
        )?;
        let rows = query.query_map([], |row| {
            Ok(ExportLine {
                started_at: match row.get(0)? {
                    Value::Text(text) => Some(text),
                    _ => None,
                },
                profit: number(row.get(1)?),
                big_blind: number(row.get(2)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn number(value: Value) -> Option<f64> {
    match value {
        Value::Integer(whole) => Some(whole as f64),
        Value::Real(real) => Some(real),
        _ => None,
    }
}
