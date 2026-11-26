//! SvanDB's reads: the same answers, in the same order, as the SQLite store's.

use std::collections::BTreeMap;

use serde_json::Value;
use svanbot_learn::probe::Arm;
use svanbot_learn::reference::Side;
use svanbot_strategy::params::Digest;
use svanbot_strategy::policy::Trace;

use super::super::trace::trace_from;
use super::super::{ExportLine, FitHand, ReferenceOrbit, Source, StoreError, TracedHand};
use super::Svandb;
use super::index::{big_blind, corrected, normalized, parsed, time, visible_trace_rows};
use crate::record::Hand;

impl Svandb {
    /// The hand history offset the last download reached, 0 before the first.
    pub fn download_offset(&self) -> Result<u64, StoreError> {
        Ok(self
            .index
            .borrow()
            .downloads
            .iter()
            .map(|download| download.offset)
            .max()
            .unwrap_or(0))
    }

    /// One hand's provenance text, or `None` when the hand is not kept or was kept before the
    /// mark existed.
    pub fn hand_quality(&self, hand_id: &str) -> Result<Option<String>, StoreError> {
        let index = self.index.borrow();
        Ok(index
            .by_id
            .get(hand_id)
            .and_then(|at| index.hands[*at].quality.clone()))
    }

    /// Every kept hand, in the order it was kept.
    pub fn hands(&self) -> Result<Vec<Hand>, StoreError> {
        let index = self.index.borrow();
        index
            .hands
            .iter()
            .map(|entry| {
                Ok(Hand {
                    hand_id: entry.hand_id.clone(),
                    table_id: entry.table_id.clone(),
                    frames: normalized(&entry.frames)?,
                    traced: Vec::new(),
                })
            })
            .collect()
    }

    /// Every kept hand in play order, using the venue's time on a downloaded hand, and the export
    /// that arrived beside it: the same order and prefer-live rule as SQLite's `hands_for_fit`.
    pub fn hands_for_fit(&self) -> Result<Vec<FitHand>, StoreError> {
        let index = self.index.borrow();
        let mut rows: Vec<(i64, usize, FitHand)> = Vec::with_capacity(index.hands.len());
        for (at, entry) in index.hands.iter().enumerate() {
            let frames = parsed(&entry.frames)?;
            let export = match &entry.export {
                Some(frame) => Some(
                    serde_json::from_str::<Value>(frame)
                        .map_err(|error| StoreError(error.to_string()))?,
                ),
                None => None,
            };
            let actions = frames
                .iter()
                .filter(|frame| frame["type"].as_str() == Some("player_action"))
                .count();
            let has = |name: &str| {
                frames
                    .iter()
                    .any(|frame| frame["type"].as_str() == Some(name))
            };
            let prefer_live = entry.source == Source::Live
                && has("hand_start")
                && has("hand_result")
                && actions
                    >= export
                        .as_ref()
                        .and_then(|frame| frame["all_actions"].as_array().map(Vec::len))
                        .unwrap_or(0);
            let order = [
                export.as_ref().and_then(|frame| time(frame, "ended_at")),
                frames.first().and_then(|frame| time(frame, "ended_at")),
                export.as_ref().and_then(|frame| time(frame, "started_at")),
                frames.first().and_then(|frame| time(frame, "started_at")),
            ]
            .into_iter()
            .flatten()
            .next()
            .unwrap_or(entry.recorded_ms);
            rows.push((
                order,
                at,
                FitHand {
                    kept: Hand {
                        hand_id: entry.hand_id.clone(),
                        table_id: entry.table_id.clone(),
                        frames: normalized(&entry.frames)?,
                        traced: Vec::new(),
                    },
                    export: entry.export.as_ref().map(|frame| Hand {
                        hand_id: entry.hand_id.clone(),
                        table_id: entry.table_id.clone(),
                        frames: vec![frame.clone()],
                        traced: Vec::new(),
                    }),
                    prefer_live,
                    kept_is_export: entry.source == Source::History,
                },
            ));
        }
        rows.sort_by_key(|(order, at, _)| (*order, *at));
        Ok(rows.into_iter().map(|(_, _, fit)| fit).collect())
    }

    /// Every hand's winnings, in the order they were kept.
    pub fn played(&self) -> Result<Vec<crate::ledger::Played>, StoreError> {
        Ok(self
            .index
            .borrow()
            .played
            .iter()
            .map(|entry| entry.played.clone())
            .collect())
    }

    /// The orbits of `probe`, oldest first: each one's number, its arm, the big blinds it won
    /// with the luck taken out, and its hands.
    pub fn orbits(&self, probe: u64) -> Result<Vec<(u64, Arm, f64, u32)>, StoreError> {
        let index = self.index.borrow();
        let mut orbits: BTreeMap<u64, (Arm, f64, u32)> = BTreeMap::new();
        for entry in &index.played {
            let Some((found, orbit, arm)) = entry.played.orbit else {
                continue;
            };
            if found != probe {
                continue;
            }
            let won = corrected(entry.played.big_blind, entry.played.net, entry.played.luck);
            let held = orbits.entry(orbit).or_insert((arm, 0.0, 0));
            *held = (held.0, held.1 + won, held.2 + 1);
        }
        Ok(orbits
            .into_iter()
            .map(|(orbit, (arm, won, hands))| (orbit, arm, won, hands))
            .collect())
    }

    /// The number of the last standing orbit a hand counted in, or `None` while no standing hand
    /// is kept.
    pub fn last_reference_orbit(&self) -> Result<Option<u64>, StoreError> {
        Ok(self
            .index
            .borrow()
            .played
            .iter()
            .filter_map(|entry| {
                entry
                    .played
                    .reference
                    .as_ref()
                    .map(|standing| standing.orbit)
            })
            .max())
    }

    /// The standing reference arm's orbits kept at or after `since_ms`, against `reference`,
    /// oldest first.
    pub fn reference_orbits(
        &self,
        reference: &Digest,
        since_ms: i64,
    ) -> Result<Vec<ReferenceOrbit>, StoreError> {
        let index = self.index.borrow();
        let mut orbits: BTreeMap<(u64, u8, u64), ReferenceOrbit> = BTreeMap::new();
        for entry in &index.played {
            if entry.recorded_ms < since_ms {
                continue;
            }
            let Some(standing) = &entry.played.reference else {
                continue;
            };
            if standing.reference != *reference {
                continue;
            }
            let side = match standing.side {
                Side::Policy => 0,
                Side::Reference => 1,
            };
            let key = (standing.orbit, side, standing.chance.to_bits());
            let won = corrected(entry.played.big_blind, entry.played.net, entry.played.luck);
            let held = orbits.entry(key).or_insert(ReferenceOrbit {
                orbit: standing.orbit,
                side: standing.side,
                chance: standing.chance,
                won: 0.0,
                hands: 0,
            });
            held.won += won;
            held.hands += 1;
        }
        let mut out: Vec<ReferenceOrbit> = orbits.into_values().collect();
        out.sort_by_key(|orbit| orbit.orbit);
        Ok(out)
    }

    /// When the bot first took `version`, if it has.
    pub fn released(&self, version: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.index.borrow().releases.get(version).copied())
    }

    /// What each hand kept at or after `since_ms` won, in big blinds, with when it was kept.
    pub fn results_since(&self, since_ms: i64) -> Result<Vec<(i64, f64)>, StoreError> {
        Ok(self
            .index
            .borrow()
            .played
            .iter()
            .filter(|entry| entry.recorded_ms >= since_ms)
            .map(|entry| {
                (
                    entry.recorded_ms,
                    entry.played.net as f64 / big_blind(entry.played.big_blind),
                )
            })
            .collect())
    }

    /// The luck-corrected result of each policy hand kept at or after `since_ms`, in big blinds.
    /// A hand on the standing reference arm is left out, as SQLite leaves it out.
    pub fn corrected_since(&self, since_ms: i64) -> Result<Vec<f64>, StoreError> {
        Ok(self
            .index
            .borrow()
            .played
            .iter()
            .filter(|entry| entry.recorded_ms >= since_ms)
            .filter(|entry| {
                entry
                    .played
                    .reference
                    .as_ref()
                    .is_none_or(|standing| standing.side != Side::Reference)
            })
            .map(|entry| corrected(entry.played.big_blind, entry.played.net, entry.played.luck))
            .collect())
    }

    /// Every export hand's start and result, in the order it was kept.
    pub fn export_lines(&self) -> Result<Vec<ExportLine>, StoreError> {
        let index = self.index.borrow();
        let mut lines = Vec::new();
        for entry in &index.hands {
            if entry.source != Source::History {
                continue;
            }
            let frames = parsed(&entry.frames)?;
            let first = frames.first();
            lines.push(ExportLine {
                started_at: first
                    .and_then(|frame| frame["started_at"].as_str())
                    .map(str::to_owned),
                profit: first.and_then(|frame| frame["profit"].as_f64()),
                big_blind: first.and_then(|frame| frame["big_blind"].as_f64()),
            });
        }
        Ok(lines)
    }

    /// The trace of each decision in `hand_id`, oldest first. `None` when the hand carries no
    /// trace, or its trace has fallen past the newest [`super::super::TRACED_HANDS`] hands.
    pub fn traces(&self, hand_id: &str) -> Result<Option<Vec<Trace>>, StoreError> {
        let index = self.index.borrow();
        let Some(at) = index.by_id.get(hand_id).copied() else {
            return Ok(None);
        };
        if !visible_trace_rows(&index)
            .into_iter()
            .any(|visible| index.hands[visible].hand_id == hand_id)
        {
            return Ok(None);
        }
        let mut traces = Vec::new();
        for row in &index.hands[at].traces {
            traces.push(trace_from(&row.street, &row.branch, &row.json)?);
        }
        Ok((!traces.is_empty()).then_some(traces))
    }

    /// Whether `hand_id` is in this log at all, traced or not.
    pub fn holds(&self, hand_id: &str) -> Result<bool, StoreError> {
        Ok(self.index.borrow().by_id.contains_key(hand_id))
    }

    /// The newest hands whose decisions are kept, newest first.
    pub fn traced_hands(&self) -> Result<Vec<TracedHand>, StoreError> {
        let index = self.index.borrow();
        Ok(visible_trace_rows(&index)
            .into_iter()
            .map(|at| {
                let entry = &index.hands[at];
                TracedHand {
                    hand_id: entry.hand_id.clone(),
                    table_id: entry.table_id.clone(),
                    recorded_ms: entry.recorded_ms,
                    decisions: entry.traces.len(),
                    quality: entry.quality.clone(),
                }
            })
            .collect())
    }
}
