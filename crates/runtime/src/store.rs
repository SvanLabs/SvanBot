//! The store: one SQLite file per bot, so each bot's reads are its own by construction
//! (`docs/adr/0004-reads-are-per-bot.md`). A store keeps its bot's name and refuses to open as
//! any other bot.
//!
//! It runs in WAL mode, so a reader never blocks the bot's writes. Its tables are `STRICT` and
//! append-only: triggers refuse every `UPDATE` and `DELETE`, so a stored hand or incident stays as
//! it was written. The one exception is `decisions`, the hand trace: it is capped, so it takes no
//! `DELETE` trigger (`store/trace.rs`). The schema is v11's own and takes no v10 migration
//! (`user_version` 1).

use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use svanbot_learn::probe::Arm;

use crate::ledger::Played;
use crate::record::Hand;
use crate::session::Incident;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
CREATE TRIGGER IF NOT EXISTS meta_no_update BEFORE UPDATE ON meta
    BEGIN SELECT RAISE(ABORT, 'meta is append-only'); END;
CREATE TRIGGER IF NOT EXISTS meta_no_delete BEFORE DELETE ON meta
    BEGIN SELECT RAISE(ABORT, 'meta is append-only'); END;
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
CREATE TABLE IF NOT EXISTS downloads (
    id          INTEGER PRIMARY KEY,
    recorded_ms INTEGER NOT NULL,
    -- The hand history offset read up to.
    offset      INTEGER NOT NULL CHECK (offset >= 0)
) STRICT;
CREATE TRIGGER IF NOT EXISTS downloads_no_update BEFORE UPDATE ON downloads
    BEGIN SELECT RAISE(ABORT, 'downloads are append-only'); END;
CREATE TRIGGER IF NOT EXISTS downloads_no_delete BEFORE DELETE ON downloads
    BEGIN SELECT RAISE(ABORT, 'downloads are append-only'); END;
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
CREATE TABLE IF NOT EXISTS played (
    hand_id     TEXT PRIMARY KEY,
    table_id    TEXT NOT NULL,
    recorded_ms INTEGER NOT NULL,
    -- The parameter set the hand was played by.
    digest      TEXT NOT NULL,
    -- The chips won, and the chips of that which were the luck of our own draws.
    net         INTEGER NOT NULL,
    luck        REAL NOT NULL,
    big_blind   INTEGER NOT NULL CHECK (big_blind > 0),
    -- The probe the hand counted in, as 16 hex digits, its orbit there and the arm played.
    probe       TEXT,
    orbit       INTEGER,
    arm         TEXT CHECK (arm IN ('champion', 'challenger'))
) STRICT;
CREATE TRIGGER IF NOT EXISTS played_no_update BEFORE UPDATE ON played
    BEGIN SELECT RAISE(ABORT, 'played hands are append-only'); END;
CREATE TRIGGER IF NOT EXISTS played_no_delete BEFORE DELETE ON played
    BEGIN SELECT RAISE(ABORT, 'played hands are append-only'); END;
CREATE TABLE IF NOT EXISTS probes (
    id          INTEGER PRIMARY KEY,
    recorded_ms INTEGER NOT NULL,
    probe       TEXT NOT NULL,
    -- 'started', or the verdict the probe ended on.
    event       TEXT NOT NULL,
    detail      TEXT NOT NULL CHECK (json_valid(detail))
) STRICT;
CREATE TRIGGER IF NOT EXISTS probes_no_update BEFORE UPDATE ON probes
    BEGIN SELECT RAISE(ABORT, 'probes are append-only'); END;
CREATE TRIGGER IF NOT EXISTS probes_no_delete BEFORE DELETE ON probes
    BEGIN SELECT RAISE(ABORT, 'probes are append-only'); END;
CREATE TABLE IF NOT EXISTS decisions (
    hand_id TEXT NOT NULL,
    -- Where in the hand this decision came, counting from 0 across the whole hand.
    seq     INTEGER NOT NULL CHECK (seq >= 0),
    street  TEXT NOT NULL CHECK (street IN ('preflop', 'flop', 'turn', 'river')),
    -- 'resolved' or 'priced': which of the policy's two ways decided.
    branch  TEXT NOT NULL CHECK (branch IN ('resolved', 'priced')),
    -- The trace itself as JSON: the cards, the values, the mix, the price, the ranges.
    trace   TEXT NOT NULL CHECK (json_valid(trace)),
    PRIMARY KEY (hand_id, seq)
) STRICT;
CREATE TRIGGER IF NOT EXISTS decisions_no_update BEFORE UPDATE ON decisions
    BEGIN SELECT RAISE(ABORT, 'decisions are append-only'); END;
-- No `no_delete` trigger here, and here only. The trace is the one record that is capped: past
-- TRACED_HANDS hands the oldest fall off, and a table that cannot delete cannot cap. The cap and
-- only the cap: it lives in `store::trace::trim`, which the write runs after the newest hand, and
-- nothing else may delete a decision. Nothing may rewrite one.
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

/// What a store holds, in one read: the dashboard's starting point for a bot that has just
/// started, before the process has counted anything itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub hands: u64,
    pub incidents: u64,
    /// When the newest hand was written, in Unix milliseconds.
    pub last_hand_ms: Option<i64>,
}

/// One bot's store.
#[derive(Debug)]
pub struct Store {
    db: Connection,
}

impl Store {
    /// Opens `bot`'s store at `path`, creating it and its schema if it is new. A store made for
    /// another bot is refused: two bots on one store would pool their reads.
    pub fn open(path: &Path, bot: &str) -> Result<Self, StoreError> {
        let db = Connection::open(path)?;
        let mode: String = db.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if mode != "wal" {
            return Err(StoreError(format!("WAL refused: the journal is {mode}")));
        }
        // WAL with NORMAL sync keeps every committed write but the last few across a power cut.
        db.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;")?;
        db.execute_batch(SCHEMA)?;
        db.execute(
            "INSERT INTO meta (key, value) VALUES ('bot', ?1) ON CONFLICT (key) DO NOTHING",
            [bot],
        )?;
        let owner: String =
            db.query_row("SELECT value FROM meta WHERE key = 'bot'", [], |row| {
                row.get(0)
            })?;
        if owner != bot {
            return Err(StoreError(format!(
                "this store is {owner}'s; {bot} may not open it"
            )));
        }
        Ok(Self { db })
    }

    /// Opens `path` to read only, with no bot named and none checked.
    ///
    /// This is the dashboard's handle, and it is not `open`: `open` refuses a bot that is not the
    /// store's owner, because two bots sharing one store would pool their reads (ADR 0004). A
    /// reader pools nothing — it takes one look at a hand the bot's own process wrote, and the
    /// store runs in WAL, so the bot's writes are never blocked. Nothing can be written through it.
    pub fn open_reader(path: &Path) -> Result<Self, StoreError> {
        let db = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.execute_batch("PRAGMA busy_timeout = 5000;")?;
        Ok(Self { db })
    }

    /// The journal mode SQLite reports: `wal` once open.
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
    }

    /// How many hands and incidents the store holds, and when the newest hand was written.
    pub fn totals(&self) -> Result<Totals, StoreError> {
        let (hands, incidents, last_hand_ms) = self.db.query_row(
            "SELECT (SELECT COUNT(*) FROM hands),
                    (SELECT COUNT(*) FROM incidents),
                    (SELECT MAX(recorded_ms) FROM hands)",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )?;
        Ok(Totals {
            hands: u64::try_from(hands).map_err(|error| StoreError(error.to_string()))?,
            incidents: u64::try_from(incidents).map_err(|error| StoreError(error.to_string()))?,
            last_hand_ms,
        })
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
        // The hand and its traces are one write: a trace has no meaning without its hand, and the
        // recorder drops an open hand when the connection breaks, so a decision written at
        // decision time can outlive a hand the store never kept.
        let tx = self.db.unchecked_transaction()?;
        let added = tx.execute(
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
        if added == 1 {
            trace::keep(&tx, hand)?;
        }
        tx.commit()?;
        Ok(added == 1)
    }

    /// The hand history offset the last download reached, 0 before the first.
    pub fn download_offset(&self) -> Result<u64, StoreError> {
        let offset: i64 = self.db.query_row(
            "SELECT COALESCE(MAX(offset), 0) FROM downloads",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(offset).unwrap_or(0))
    }

    /// Records that the hand history has been read up to `offset`.
    pub fn record_download(&self, offset: u64, recorded_ms: i64) -> Result<(), StoreError> {
        let offset = i64::try_from(offset).map_err(|e| StoreError(e.to_string()))?;
        self.db.execute(
            "INSERT INTO downloads (recorded_ms, offset) VALUES (?1, ?2)",
            params![recorded_ms, offset],
        )?;
        Ok(())
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
                traced: Vec::new(),
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

    /// Keeps what a hand won. Returns false if it is kept already: the first copy stands.
    pub fn append_played(&self, played: &Played, recorded_ms: i64) -> Result<bool, StoreError> {
        let (probe, orbit, arm) = match played.orbit {
            Some((probe, orbit, arm)) => (
                Some(format!("{probe:016x}")),
                Some(i64::try_from(orbit).map_err(|e| StoreError(e.to_string()))?),
                Some(arm_name(arm)),
            ),
            None => (None, None, None),
        };
        let added = self.db.execute(
            "INSERT INTO played
                (hand_id, table_id, recorded_ms, digest, net, luck, big_blind, probe, orbit, arm)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT (hand_id) DO NOTHING",
            params![
                played.hand_id,
                played.table_id,
                recorded_ms,
                played.digest.to_string(),
                played.net,
                played.luck,
                // SQLite keeps integers as i64; a big blind always fits.
                i64::try_from(played.big_blind).unwrap_or(i64::MAX),
                probe,
                orbit,
                arm
            ],
        )?;
        Ok(added == 1)
    }

    /// Every hand's winnings, in the order they were kept.
    pub fn played(&self) -> Result<Vec<Played>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT hand_id, table_id, digest, net, luck, big_blind, probe, orbit, arm
             FROM played ORDER BY rowid",
        )?;
        let rows = query.query_map([], |row| {
            let digest: String = row.get(2)?;
            let probe: Option<String> = row.get(6)?;
            let orbit: Option<i64> = row.get(7)?;
            let arm: Option<String> = row.get(8)?;
            Ok(Played {
                hand_id: row.get(0)?,
                table_id: row.get(1)?,
                digest: digest.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                net: row.get(3)?,
                luck: row.get(4)?,
                big_blind: u64::try_from(row.get::<_, i64>(5)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                orbit: match (probe, orbit, arm) {
                    (Some(probe), Some(orbit), Some(arm)) => Some((
                        u64::from_str_radix(&probe, 16).unwrap_or(0),
                        u64::try_from(orbit).unwrap_or(0),
                        arm_named(&arm),
                    )),
                    _ => None,
                },
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The orbits of `probe`, oldest first: each one's number, its arm, the big blinds it won
    /// with the luck taken out, and its hands.
    pub fn orbits(&self, probe: u64) -> Result<Vec<(u64, Arm, f64, u32)>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT orbit, arm, SUM((net - luck) / big_blind), COUNT(*)
             FROM played WHERE probe = ?1 GROUP BY orbit ORDER BY orbit",
        )?;
        let rows = query.query_map([format!("{probe:016x}")], |row| {
            let orbit: i64 = row.get(0)?;
            let arm: String = row.get(1)?;
            Ok((
                u64::try_from(orbit).unwrap_or(0),
                arm_named(&arm),
                row.get(2)?,
                row.get(3)?,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records that `probe` started, or the verdict it ended on, with `detail` as JSON.
    pub fn append_probe(
        &self,
        probe: u64,
        event: &str,
        detail: &str,
        recorded_ms: i64,
    ) -> Result<(), StoreError> {
        self.db.execute(
            "INSERT INTO probes (recorded_ms, probe, event, detail) VALUES (?1, ?2, ?3, ?4)",
            params![recorded_ms, format!("{probe:016x}"), event, detail],
        )?;
        Ok(())
    }

    /// The probe that started last, with the detail it started with, unless it has ended.
    pub fn open_probe(&self) -> Result<Option<(u64, String)>, StoreError> {
        let last = self
            .db
            .query_row(
                "SELECT probe, event, detail FROM probes ORDER BY id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(last
            .filter(|(_, event, _)| event == "started")
            .map(|(probe, _, detail)| (u64::from_str_radix(&probe, 16).unwrap_or(0), detail)))
    }
}

mod trace;
mod window;

pub use trace::{TRACED_HANDS, TracedHand};
pub use window::ExportLine;

fn arm_name(arm: Arm) -> &'static str {
    match arm {
        Arm::Champion => "champion",
        Arm::Challenger => "challenger",
    }
}

fn arm_named(name: &str) -> Arm {
    match name {
        "challenger" => Arm::Challenger,
        _ => Arm::Champion,
    }
}
