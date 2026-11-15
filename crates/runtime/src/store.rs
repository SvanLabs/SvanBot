//! The store: one SQLite file per bot, so each bot's reads are its own by construction.
//!
//! It runs in WAL mode, so a reader never blocks the bot's writes. Its tables are `STRICT` and
//! append-only: triggers refuse every `UPDATE` and `DELETE`, so a stored hand or incident stays as
//! it was written. The schema is v11's own and takes no v10 migration (`user_version` 1).

use std::fmt;
use std::path::Path;

use rusqlite::{Connection, params};

use crate::record::Hand;
use crate::session::Incident;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS hands (
    hand_id     TEXT PRIMARY KEY,
    table_id    TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('live', 'history')),
    recorded_ms INTEGER NOT NULL,
    frames      TEXT NOT NULL CHECK (json_valid(frames))
) STRICT;
CREATE TRIGGER IF NOT EXISTS hands_no_update BEFORE UPDATE ON hands
    BEGIN SELECT RAISE(ABORT, 'hands are append-only'); END;
CREATE TRIGGER IF NOT EXISTS hands_no_delete BEFORE DELETE ON hands
    BEGIN SELECT RAISE(ABORT, 'hands are append-only'); END;
CREATE TABLE IF NOT EXISTS incidents (
    id          INTEGER PRIMARY KEY,
    recorded_ms INTEGER NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('shared_table', 'shared_hand')),
    -- The table for a shared table, the hand for a shared hand.
    at_id       TEXT NOT NULL,
    stablemate  TEXT NOT NULL
) STRICT;
CREATE TRIGGER IF NOT EXISTS incidents_no_update BEFORE UPDATE ON incidents
    BEGIN SELECT RAISE(ABORT, 'incidents are append-only'); END;
CREATE TRIGGER IF NOT EXISTS incidents_no_delete BEFORE DELETE ON incidents
    BEGIN SELECT RAISE(ABORT, 'incidents are append-only'); END;
PRAGMA user_version = 1;
";

/// Where a stored hand came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Recorded at the table.
    Live,
    /// Downloaded from the venue's hand history.
    History,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::History => "history",
        }
    }
}

/// A store failure, with SQLite's reason.
#[derive(Debug)]
pub struct StoreError(String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

/// One bot's store.
#[derive(Debug)]
pub struct Store {
    db: Connection,
}

impl Store {
    /// Opens the store at `path`, creating it and its schema if it is new.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Connection::open(path)?;
        let mode: String = db.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if mode != "wal" {
            return Err(StoreError(format!("WAL refused: the journal is {mode}")));
        }
        // WAL with NORMAL sync keeps every committed write but the last few across a power cut.
        db.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;")?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    /// The journal mode SQLite reports: `wal` once open.
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
    }

    /// Keeps a hand. Returns false if it is kept already: the first copy stands.
    pub fn append_hand(
        &self,
        hand: &Hand,
        source: Source,
        recorded_ms: i64,
    ) -> Result<bool, StoreError> {
        // Each frame is a JSON object as the venue sent it, so the list is a JSON array.
        let frames = format!("[{}]", hand.frames.join(","));
        let added = self.db.execute(
            "INSERT INTO hands (hand_id, table_id, source, recorded_ms, frames)
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (hand_id) DO NOTHING",
            params![
                hand.hand_id,
                hand.table_id,
                source.name(),
                recorded_ms,
                frames
            ],
        )?;
        Ok(added == 1)
    }

    /// Every kept hand, in the order it was kept. Each frame comes back as compact JSON.
    pub fn hands(&self) -> Result<Vec<Hand>, StoreError> {
        let mut query = self
            .db
            .prepare("SELECT hand_id, table_id, frames FROM hands ORDER BY rowid")?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (hand_id, table_id, frames) = row?;
            let frames: Vec<serde_json::Value> =
                serde_json::from_str(&frames).map_err(|e| StoreError(e.to_string()))?;
            Ok(Hand {
                hand_id,
                table_id,
                frames: frames.iter().map(ToString::to_string).collect(),
            })
        })
        .collect()
    }

    /// Keeps an incident. Every one is kept, repeats included.
    pub fn append_incident(&self, incident: &Incident, recorded_ms: i64) -> Result<(), StoreError> {
        let (kind, at_id, stablemate) = match incident {
            Incident::SharedTable {
                table_id,
                stablemate,
            } => ("shared_table", table_id, stablemate),
            Incident::SharedHand {
                hand_id,
                stablemate,
            } => ("shared_hand", hand_id, stablemate),
        };
        self.db.execute(
            "INSERT INTO incidents (recorded_ms, kind, at_id, stablemate) VALUES (?1, ?2, ?3, ?4)",
            params![recorded_ms, kind, at_id, stablemate],
        )?;
        Ok(())
    }

    /// Every kept incident, oldest first.
    pub fn incidents(&self) -> Result<Vec<Incident>, StoreError> {
        let mut query = self
            .db
            .prepare("SELECT kind, at_id, stablemate FROM incidents ORDER BY id")?;
        let rows = query.query_map([], |row| {
            let kind: String = row.get(0)?;
            let at_id: String = row.get(1)?;
            let stablemate: String = row.get(2)?;
            Ok(match kind.as_str() {
                "shared_table" => Incident::SharedTable {
                    table_id: at_id,
                    stablemate,
                },
                _ => Incident::SharedHand {
                    hand_id: at_id,
                    stablemate,
                },
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
