//! The `played` table: what each counted hand won, with the probe orbit or standing reference
//! arm it counted in.

use rusqlite::{Connection, params};

use svanbot_learn::probe::Arm;
use svanbot_learn::reference::Side;
use svanbot_strategy::params::Digest;

use super::{Store, StoreError};
use crate::ledger::Played;

/// One stored standing orbit as the estimate reads it: its number, the side it played, the
/// chance that side was drawn, the big blinds it won with the luck taken out, and its hands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReferenceOrbit {
    pub orbit: u64,
    pub side: Side,
    pub chance: f64,
    pub won: f64,
    pub hands: u32,
}

impl Store {
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
        let (reference, reference_orbit, reference_side, reference_chance) = match played.reference
        {
            Some(standing) => (
                Some(standing.reference.to_string()),
                Some(i64::try_from(standing.orbit).map_err(|e| StoreError(e.to_string()))?),
                Some(standing.side.name()),
                Some(standing.chance),
            ),
            None => (None, None, None, None),
        };
        let added = self.db.execute(
            "INSERT INTO played
                (hand_id, table_id, recorded_ms, digest, net, luck, big_blind, probe, orbit, arm,
                 reference, reference_orbit, reference_side, reference_chance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (hand_id) DO NOTHING",
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
                arm,
                reference,
                reference_orbit,
                reference_side,
                reference_chance
            ],
        )?;
        Ok(added == 1)
    }

    /// Every hand's winnings, in the order they were kept.
    pub fn played(&self) -> Result<Vec<Played>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT hand_id, table_id, digest, net, luck, big_blind, probe, orbit, arm,
                    reference, reference_orbit, reference_side, reference_chance
             FROM played ORDER BY rowid",
        )?;
        let rows = query.query_map([], |row| {
            let digest: String = row.get(2)?;
            let probe: Option<String> = row.get(6)?;
            let orbit: Option<i64> = row.get(7)?;
            let arm: Option<String> = row.get(8)?;
            let reference: Option<String> = row.get(9)?;
            let reference_orbit: Option<i64> = row.get(10)?;
            let reference_side: Option<String> = row.get(11)?;
            let reference_chance: Option<f64> = row.get(12)?;
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
                reference: match (reference, reference_orbit, reference_side, reference_chance) {
                    (Some(reference), Some(orbit), Some(side), Some(chance)) => {
                        Some(svanbot_learn::reference::Standing {
                            orbit: u64::try_from(orbit).unwrap_or(0),
                            side: Side::named(&side).ok_or(rusqlite::Error::InvalidQuery)?,
                            chance,
                            reference: reference
                                .parse()
                                .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        })
                    }
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

    /// The number of the last standing orbit a hand counted in, or `None` while no standing hand
    /// is kept: the next orbit after a restart.
    pub fn last_reference_orbit(&self) -> Result<Option<u64>, StoreError> {
        let last: Option<i64> = self.db.query_row(
            "SELECT MAX(reference_orbit) FROM played WHERE reference IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(last.and_then(|orbit| u64::try_from(orbit).ok()))
    }

    /// The standing reference arm's orbits kept at or after `since_ms`, against `reference`,
    /// oldest first: each one's number, side, chance of being drawn, luck-corrected big blinds
    /// and hands. A bot reads its own store, and its own arms, here.
    pub fn reference_orbits(
        &self,
        reference: &Digest,
        since_ms: i64,
    ) -> Result<Vec<ReferenceOrbit>, StoreError> {
        let mut query = self.db.prepare(
            "SELECT reference_orbit, reference_side, reference_chance,
                    SUM((net - luck) / big_blind), COUNT(*)
             FROM played
             WHERE reference = ?1 AND reference_orbit IS NOT NULL AND recorded_ms >= ?2
             GROUP BY reference_orbit, reference_side, reference_chance
             ORDER BY reference_orbit",
        )?;
        let rows = query.query_map(params![reference.to_string(), since_ms], |row| {
            let orbit: i64 = row.get(0)?;
            let side: String = row.get(1)?;
            Ok(ReferenceOrbit {
                orbit: u64::try_from(orbit).unwrap_or(0),
                side: Side::named(&side).ok_or(rusqlite::Error::InvalidQuery)?,
                chance: row.get(2)?,
                won: row.get(3)?,
                hands: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Adds the standing reference arm's columns to a store made before it. New stores have them
/// from the schema; a live store is never rebuilt, so the columns are added in place.
pub(super) fn migrate(db: &Connection) -> Result<(), StoreError> {
    let mut query = db.prepare("PRAGMA table_info(played)")?;
    let held: Vec<String> = query
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()?;
    for (name, kind) in [
        ("reference", "TEXT"),
        ("reference_orbit", "INTEGER"),
        (
            "reference_side",
            "TEXT CHECK (reference_side IN ('policy', 'reference'))",
        ),
        ("reference_chance", "REAL"),
    ] {
        if !held.iter().any(|column| column == name) {
            db.execute_batch(&format!("ALTER TABLE played ADD COLUMN {name} {kind}"))?;
        }
    }
    Ok(())
}

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
