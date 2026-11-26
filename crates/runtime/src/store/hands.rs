use rusqlite::{Transaction, params};

use super::{Source, Store, StoreError, trace};
use crate::record::Hand;

pub(super) const SEASON_DOWNLOAD_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS verified_season_exports (
    season_id   TEXT NOT NULL,
    hands       INTEGER NOT NULL CHECK (hands >= 0),
    recorded_ms INTEGER NOT NULL,
    PRIMARY KEY (season_id, hands)
) STRICT;
CREATE TRIGGER IF NOT EXISTS verified_season_exports_no_update BEFORE UPDATE ON verified_season_exports
    BEGIN SELECT RAISE(ABORT, 'verified season exports are append-only'); END;
CREATE TRIGGER IF NOT EXISTS verified_season_exports_no_delete BEFORE DELETE ON verified_season_exports
    BEGIN SELECT RAISE(ABORT, 'verified season exports are append-only'); END;
";

/// A hand and its distinct export, kept apart until the fit validates either copy.
pub struct FitHand {
    pub kept: Hand,
    pub export: Option<Hand>,
    pub prefer_live: bool,
    pub kept_is_export: bool,
}

impl Store {
    pub(crate) fn owner(&self) -> Result<String, StoreError> {
        Ok(self
            .db
            .query_row("SELECT value FROM meta WHERE key = 'bot'", [], |row| {
                row.get(0)
            })?)
    }

    /// Keeps a hand. Returns false if it is kept already: the first copy stands.
    pub fn append_hand(
        &self,
        hand: &Hand,
        source: Source,
        recorded_ms: i64,
    ) -> Result<bool, StoreError> {
        let tx = self.db.unchecked_transaction()?;
        let added = keep_hand(&tx, hand, source, recorded_ms)?;
        tx.commit()?;
        Ok(added)
    }

    /// Append a small group of exported hands with one commit.
    pub(crate) fn append_history(&self, hands: &[(Hand, i64)]) -> Result<u32, StoreError> {
        if hands.is_empty() {
            return Ok(0);
        }
        let tx = self.db.unchecked_transaction()?;
        let mut added = 0;
        for (hand, recorded_ms) in hands {
            added += u32::from(keep_hand(&tx, hand, Source::History, *recorded_ms)?);
        }
        tx.commit()?;
        Ok(added)
    }

    /// A download's writer may wait for live hands to finish committing.
    pub(crate) fn wait_for_download_writer(&self) -> Result<(), StoreError> {
        self.db.busy_timeout(std::time::Duration::from_secs(30))?;
        Ok(())
    }

    pub(crate) fn verified_season(&self, season_id: &str) -> Result<u64, StoreError> {
        let hands: i64 = self.db.query_row(
            "SELECT COALESCE(MAX(hands), 0) FROM verified_season_exports WHERE season_id = ?1",
            [season_id],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(hands).unwrap_or(0))
    }

    pub(crate) fn record_verified_season(
        &self,
        season_id: &str,
        hands: u64,
        recorded_ms: i64,
    ) -> Result<(), StoreError> {
        let hands = i64::try_from(hands).map_err(|e| StoreError(e.to_string()))?;
        self.db.execute(
            "INSERT INTO verified_season_exports (season_id, hands, recorded_ms)
             VALUES (?1, ?2, ?3) ON CONFLICT (season_id, hands) DO NOTHING",
            params![season_id, hands, recorded_ms],
        )?;
        Ok(())
    }

    /// Every kept hand, in the order it was kept. Each frame comes back as compact JSON.
    pub fn hands(&self) -> Result<Vec<Hand>, StoreError> {
        self.read_hands("SELECT hand_id, table_id, frames FROM hands ORDER BY rowid")
    }

    /// Every kept hand in play order, using the venue's end time on a downloaded hand.
    pub fn hands_for_fit(&self) -> Result<Vec<FitHand>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT hands.hand_id, table_id, hands.frames, hand_exports.frame,
                    source = 'history',
                    source = 'live'
                      AND EXISTS (SELECT 1 FROM json_each(hands.frames)
                                  WHERE json_extract(value, '$.type') = 'hand_start')
                      AND EXISTS (SELECT 1 FROM json_each(hands.frames)
                                  WHERE json_extract(value, '$.type') = 'hand_result')
                      AND (SELECT COUNT(*) FROM json_each(hands.frames)
                           WHERE json_extract(value, '$.type') = 'player_action') >=
                          COALESCE(json_array_length(hand_exports.frame, '$.all_actions'), 0)
             FROM hands LEFT JOIN hand_exports USING (hand_id)
             ORDER BY COALESCE(
                 unixepoch(json_extract(hand_exports.frame, '$.ended_at'), 'subsec') * 1000,
                 unixepoch(json_extract(hands.frames, '$[0].ended_at'), 'subsec') * 1000,
                 unixepoch(json_extract(hand_exports.frame, '$.started_at'), 'subsec') * 1000,
                 unixepoch(json_extract(hands.frames, '$[0].started_at'), 'subsec') * 1000,
                 recorded_ms
             ), hands.rowid",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, bool>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (hand_id, table_id, frames, export, kept_is_export, prefer_live) = row?;
            let export = export.map(|frame| Hand {
                hand_id: hand_id.clone(),
                table_id: table_id.clone(),
                frames: vec![frame],
                traced: vec![],
            });
            let kept = hand(hand_id, table_id, &frames)?;
            Ok(FitHand {
                kept,
                export,
                prefer_live,
                kept_is_export,
            })
        })
        .collect()
    }

    fn read_hands(&self, sql: &str) -> Result<Vec<Hand>, StoreError> {
        let mut query = self.db.prepare(sql)?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (hand_id, table_id, frames) = row?;
            hand(hand_id, table_id, &frames)
        })
        .collect()
    }
}

fn keep_hand(
    tx: &Transaction<'_>,
    hand: &Hand,
    source: Source,
    recorded_ms: i64,
) -> Result<bool, StoreError> {
    // The hand and its traces are one write. A trace has no meaning without its hand.
    let frames = format!("[{}]", hand.frames.join(","));
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
        trace::keep(tx, hand)?;
    } else if source == Source::History {
        tx.execute(
            "INSERT INTO hand_exports (hand_id, frame)
             SELECT hand_id, json_extract(?2, '$[0]') FROM hands
             WHERE hand_id = ?1 AND frames <> ?2
               AND json_array_length(?2) = 1 AND json_type(?2, '$[0]') = 'object'
             ON CONFLICT (hand_id) DO NOTHING",
            params![hand.hand_id, frames],
        )?;
    }
    Ok(added == 1)
}

fn hand(hand_id: String, table_id: String, frames: &str) -> Result<Hand, StoreError> {
    let frames: Vec<serde_json::Value> =
        serde_json::from_str(frames).map_err(|error| StoreError(error.to_string()))?;
    Ok(Hand {
        hand_id,
        table_id,
        frames: frames.iter().map(ToString::to_string).collect(),
        traced: Vec::new(),
    })
}
