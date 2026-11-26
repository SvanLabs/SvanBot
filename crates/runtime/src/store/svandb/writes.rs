//! SvanDB's writes: each keeps the record the SQLite store keeps, in one appended line.

use std::path::Path;

use serde_json::json;

use super::super::{Source, Store, StoreError};
use super::Svandb;
use super::index::{export_record, hand_record, played_record};
use crate::ledger::Played;
use crate::record::Hand;
use crate::session::Incident;

impl Svandb {
    /// Records that the hand history has been read up to `offset`.
    pub fn record_download(&self, offset: u64, recorded_ms: i64) -> Result<(), StoreError> {
        self.append(json!({
            "kind": "download",
            "recorded_ms": recorded_ms,
            "offset": offset,
        }))
    }

    /// Keeps an incident. Every one is kept, repeats included.
    pub fn append_incident(&self, incident: &Incident, recorded_ms: i64) -> Result<(), StoreError> {
        let (at, at_id, stablemate) = match incident {
            Incident::SharedTable {
                table_id,
                stablemate,
            } => ("shared_table", table_id, stablemate),
            Incident::SharedHand {
                hand_id,
                stablemate,
            } => ("shared_hand", hand_id, stablemate),
        };
        self.append(json!({
            "kind": "incident",
            "recorded_ms": recorded_ms,
            "at": at,
            "at_id": at_id,
            "stablemate": stablemate,
        }))
    }

    /// Every kept incident, oldest first.
    pub fn incidents(&self) -> Result<Vec<Incident>, StoreError> {
        Ok(self.index.borrow().incidents.clone())
    }

    /// Records that `probe` started, or the verdict it ended on, with `detail` as JSON.
    pub fn append_probe(
        &self,
        probe: u64,
        event: &str,
        detail: &str,
        recorded_ms: i64,
    ) -> Result<(), StoreError> {
        self.append(json!({
            "kind": "probe",
            "recorded_ms": recorded_ms,
            "probe": format!("{probe:016x}"),
            "event": event,
            "detail": detail,
        }))
    }

    /// The probe that started last, with the detail it started with, unless it has ended.
    pub fn open_probe(&self) -> Result<Option<(u64, String)>, StoreError> {
        let index = self.index.borrow();
        Ok(index
            .probes
            .last()
            .filter(|probe| probe.event == "started")
            .map(|probe| (probe.probe, probe.detail.clone())))
    }

    /// Keeps a hand. Returns false if it is kept already: the first copy stands.
    pub fn append_hand(
        &self,
        hand: &Hand,
        source: Source,
        recorded_ms: i64,
    ) -> Result<bool, StoreError> {
        let record = hand_record(hand, source, recorded_ms)?;
        let existed = self.index.borrow().by_id.contains_key(&hand.hand_id);
        if !existed {
            self.append(record)?;
            return Ok(true);
        }
        // A downloaded copy of a kept hand is kept beside it when the two disagree and the
        // download is one whole hand: the fit validates one against the other.
        let export = if source == Source::History {
            let index = self.index.borrow();
            export_record(&index, hand)?
        } else {
            None
        };
        if let Some(export) = export {
            self.append(export)?;
        }
        Ok(false)
    }

    /// Append a small group of exported hands with one commit.
    // The mirror of the `Store` surface; the fleet adopts it only if the gate says so.
    #[allow(dead_code)]
    pub(crate) fn append_history(&self, hands: &[(Hand, i64)]) -> Result<u32, StoreError> {
        if hands.is_empty() {
            return Ok(0);
        }
        let mut records = Vec::with_capacity(hands.len());
        for (hand, recorded_ms) in hands {
            records.push(hand_record(hand, Source::History, *recorded_ms)?);
        }
        let before = self.index.borrow().hands.len();
        self.append(json!({ "kind": "history", "hands": records }))?;
        let after = self.index.borrow().hands.len();
        Ok(u32::try_from(after - before).unwrap_or(u32::MAX))
    }

    /// A download's writer may wait for live hands to finish committing; SvanDB writes one line
    /// at a time and has no writer to wait for.
    // The mirror of the `Store` surface; the fleet adopts it only if the gate says so.
    #[allow(dead_code)]
    pub(crate) fn wait_for_download_writer(&self) -> Result<(), StoreError> {
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn verified_season(&self, season_id: &str) -> Result<u64, StoreError> {
        Ok(self
            .index
            .borrow()
            .seasons
            .iter()
            .filter(|season| season.season_id == season_id)
            .map(|season| season.hands)
            .max()
            .unwrap_or(0))
    }

    #[allow(dead_code)]
    pub(crate) fn record_verified_season(
        &self,
        season_id: &str,
        hands: u64,
        recorded_ms: i64,
    ) -> Result<(), StoreError> {
        let known = self
            .index
            .borrow()
            .seasons
            .iter()
            .any(|season| season.season_id == season_id && season.hands == hands);
        if known {
            return Ok(());
        }
        self.append(json!({
            "kind": "season",
            "season_id": season_id,
            "hands": hands,
            "recorded_ms": recorded_ms,
        }))
    }

    /// Keeps what a hand won. Returns false if it is kept already: the first copy stands.
    pub fn append_played(&self, played: &Played, recorded_ms: i64) -> Result<bool, StoreError> {
        if self.index.borrow().played_ids.contains(&played.hand_id) {
            return Ok(false);
        }
        self.append(played_record(played, recorded_ms))?;
        Ok(true)
    }

    /// Marks that the bot took `version` at `recorded_ms`, if it has not before.
    pub fn mark_release(&self, version: &str, recorded_ms: i64) -> Result<(), StoreError> {
        if self.index.borrow().releases.contains_key(version) {
            return Ok(());
        }
        self.append(json!({
            "kind": "release",
            "version": version,
            "recorded_ms": recorded_ms,
        }))
    }

    /// Copies a SQLite store into a new SvanDB log at `path` (V11-128's migration gate). Every
    /// read surface the two share is the same afterwards: the same hands (with their provenance
    /// and traces), played hands with their kept times, incidents, probe events, the download
    /// offset, release marks and verified seasons.
    pub fn import(store: &Store, path: &Path) -> Result<Self, StoreError> {
        if path.exists() || super::manifest::manifest_path(path).exists() {
            return Err(StoreError(format!(
                "{} already exists; an import writes a new log",
                path.display()
            )));
        }
        let db = Self::create(path, &store.bot()?)?;
        let offset = store.download_offset()?;
        if offset > 0 {
            // The public read keeps only the highest offset, so that is the one migrated.
            db.append(json!({
                "kind": "download",
                "recorded_ms": 0,
                "offset": offset,
            }))?;
        }
        for incident in store.incidents()? {
            db.append_incident(&incident, 0)?;
        }
        for (recorded_ms, probe, event, detail) in store.probe_events()? {
            db.append(json!({
                "kind": "probe",
                "recorded_ms": recorded_ms,
                "probe": format!("{probe:016x}"),
                "event": event,
                "detail": detail,
            }))?;
        }
        for row in store.hands_for_migration()? {
            let mut hand = row.hand;
            hand.traced = store.traces(&hand.hand_id)?.unwrap_or_default();
            let mut record = hand_record(&hand, row.source, row.recorded_ms)?;
            // The provenance mark is kept as the store held it: a row written before the mark
            // existed reads as `None`, and keeps reading as `None` here.
            record["quality"] = json!(row.quality);
            record["export"] = json!(row.export);
            db.append(record)?;
        }
        for (played, recorded_ms) in store.played_with_ms()? {
            db.append(played_record(&played, recorded_ms))?;
        }
        for (version, recorded_ms) in store.release_marks()? {
            db.append(json!({
                "kind": "release",
                "version": version,
                "recorded_ms": recorded_ms,
            }))?;
        }
        for (season_id, hands, recorded_ms) in store.seasons_verified()? {
            db.append(json!({
                "kind": "season",
                "season_id": season_id,
                "hands": hands,
                "recorded_ms": recorded_ms,
            }))?;
        }
        Ok(db)
    }
}
