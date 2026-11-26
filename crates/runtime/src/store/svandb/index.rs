//! The in-memory index SvanDB rebuilds from its committed log, and the one function that applies
//! a record to it. Open and every append run the same `apply`, so a reopened store answers
//! exactly as the running one did.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};
use svanbot_learn::reference::{Side, Standing};

use super::super::hands::quality;
use super::super::played::{arm_name, arm_named};
use super::super::trace::{TRACED_HANDS, trace_json};
use super::super::{Source, StoreError};
use crate::ledger::Played;
use crate::record::Hand;
use crate::session::Incident;

/// One hand as the log keeps it: the frames as received, the provenance mark, the export when a
/// downloaded copy arrived later, and the decision traces.
#[derive(Clone, Debug)]
pub(super) struct HandEntry {
    pub(super) hand_id: String,
    pub(super) table_id: String,
    pub(super) source: Source,
    pub(super) recorded_ms: i64,
    pub(super) frames: Vec<String>,
    /// `None` on a hand kept before the provenance mark existed: unknown, not clean.
    pub(super) quality: Option<String>,
    pub(super) export: Option<String>,
    pub(super) traces: Vec<TraceRow>,
}

#[derive(Clone, Debug)]
pub(super) struct TraceRow {
    pub(super) street: String,
    pub(super) branch: String,
    pub(super) json: String,
}

#[derive(Clone, Debug)]
pub(super) struct PlayedEntry {
    pub(super) played: Played,
    pub(super) recorded_ms: i64,
}

#[derive(Clone, Debug)]
pub(super) struct ProbeEvent {
    pub(super) probe: u64,
    pub(super) event: String,
    pub(super) detail: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Download {
    pub(super) offset: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Season {
    pub(super) season_id: String,
    pub(super) hands: u64,
}

/// The rebuilt index: every read answers from it.
#[derive(Debug, Default)]
pub(super) struct Index {
    pub(super) hands: Vec<HandEntry>,
    pub(super) by_id: HashMap<String, usize>,
    pub(super) played: Vec<PlayedEntry>,
    pub(super) played_ids: HashSet<String>,
    pub(super) incidents: Vec<Incident>,
    pub(super) probes: Vec<ProbeEvent>,
    pub(super) downloads: Vec<Download>,
    pub(super) releases: BTreeMap<String, i64>,
    pub(super) seasons: Vec<Season>,
}

/// Applies one committed record to the index. The same function reads the log at open and takes
/// each append.
pub(super) fn apply(index: &mut Index, record: &Value) -> Result<(), String> {
    match record["kind"]
        .as_str()
        .ok_or("a SvanDB record has no kind")?
    {
        "hand" => {
            apply_hand(index, record)?;
            Ok(())
        }
        "history" => {
            let hands = record["hands"]
                .as_array()
                .ok_or("a history record has no hands")?;
            for hand in hands {
                apply_hand(index, hand)?;
            }
            Ok(())
        }
        "export" => {
            let hand_id = record["hand_id"]
                .as_str()
                .ok_or("an export record has no hand")?;
            let frame = record["frame"]
                .as_str()
                .ok_or("an export record has no frame")?;
            if let Some(at) = index.by_id.get(hand_id) {
                index.hands[*at].export = Some(frame.to_owned());
            }
            Ok(())
        }
        "played" => apply_played(index, record),
        "incident" => apply_incident(index, record),
        "download" => {
            index.downloads.push(Download {
                offset: record["offset"]
                    .as_u64()
                    .ok_or("a download has no offset")?,
            });
            Ok(())
        }
        "probe" => {
            index.probes.push(ProbeEvent {
                probe: record["probe"]
                    .as_str()
                    .and_then(|text| u64::from_str_radix(text, 16).ok())
                    .ok_or("a probe has no id")?,
                event: record["event"]
                    .as_str()
                    .ok_or("a probe has no event")?
                    .to_owned(),
                detail: record["detail"]
                    .as_str()
                    .ok_or("a probe has no detail")?
                    .to_owned(),
            });
            Ok(())
        }
        "release" => {
            let version = record["version"]
                .as_str()
                .ok_or("a release has no version")?;
            let recorded_ms = record["recorded_ms"]
                .as_i64()
                .ok_or("a release has no time")?;
            index
                .releases
                .entry(version.to_owned())
                .or_insert(recorded_ms);
            Ok(())
        }
        "season" => {
            let season_id = record["season_id"].as_str().ok_or("a season has no id")?;
            let hands = record["hands"].as_u64().ok_or("a season has no count")?;
            if !index
                .seasons
                .iter()
                .any(|season| season.season_id == season_id && season.hands == hands)
            {
                index.seasons.push(Season {
                    season_id: season_id.to_owned(),
                    hands,
                });
            }
            Ok(())
        }
        other => Err(format!("no SvanDB record is called {other}")),
    }
}

/// One hand record, found or not. A duplicate follows the SQLite rule: the first copy stands,
/// and a differing downloaded copy arrives as an export beside it.
fn apply_hand(index: &mut Index, record: &Value) -> Result<bool, String> {
    let hand_id = record["hand_id"]
        .as_str()
        .ok_or("a hand record has no hand id")?
        .to_owned();
    let frames: Vec<String> = record["frames"]
        .as_array()
        .ok_or("a hand record has no frames")?
        .iter()
        .map(|frame| {
            frame
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "a hand record's frame is not text".to_owned())
        })
        .collect::<Result<_, _>>()?;
    let source = record["source"]
        .as_str()
        .and_then(Source::named)
        .ok_or("a hand record's source is unknown")?;
    let recorded_ms = record["recorded_ms"]
        .as_i64()
        .ok_or("a hand record has no time")?;
    if let Some(&at) = index.by_id.get(&hand_id) {
        if source == Source::History {
            let stored = canonical(&index.hands[at].frames);
            let text = canonical(&frames);
            let one_object = frames.len() == 1
                && serde_json::from_str::<Value>(&frames[0])
                    .map(|frame| frame.is_object())
                    .unwrap_or(false);
            if stored != text && one_object {
                index.hands[at].export = Some(frames[0].clone());
            }
        }
        return Ok(false);
    }
    let traces = record["traces"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    Ok(TraceRow {
                        street: row["street"]
                            .as_str()
                            .ok_or("a trace has no street")?
                            .to_owned(),
                        branch: row["branch"]
                            .as_str()
                            .ok_or("a trace has no branch")?
                            .to_owned(),
                        json: row["trace"]
                            .as_str()
                            .ok_or("a trace has no record")?
                            .to_owned(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let quality = match &record["quality"] {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        _ => Some(quality(&frames)),
    };
    let export = record["export"].as_str().map(str::to_owned);
    index.by_id.insert(hand_id.clone(), index.hands.len());
    index.hands.push(HandEntry {
        hand_id,
        table_id: record["table_id"]
            .as_str()
            .ok_or("a hand record has no table")?
            .to_owned(),
        source,
        recorded_ms,
        frames,
        quality,
        export,
        traces,
    });
    Ok(true)
}

fn apply_played(index: &mut Index, record: &Value) -> Result<(), String> {
    let hand_id = record["hand_id"]
        .as_str()
        .ok_or("a played record has no hand")?;
    if !index.played_ids.insert(hand_id.to_owned()) {
        return Ok(());
    }
    let digest = record["digest"]
        .as_str()
        .ok_or("a played record has no digest")?
        .parse()
        .map_err(|_| "a played record's digest is not a digest".to_owned())?;
    let orbit = match (
        record["probe"].as_str(),
        record["orbit"].as_u64(),
        record["arm"].as_str(),
    ) {
        (Some(probe), Some(orbit), Some(arm)) => Some((
            u64::from_str_radix(probe, 16).unwrap_or(0),
            orbit,
            arm_named(arm),
        )),
        _ => None,
    };
    let reference = match (
        record["reference"].as_str(),
        record["reference_orbit"].as_u64(),
        record["reference_side"].as_str(),
        record["reference_chance"].as_f64(),
    ) {
        (Some(reference), Some(orbit), Some(side), Some(chance)) => Some(Standing {
            orbit,
            side: Side::named(side).ok_or("a played record's side is unknown")?,
            chance,
            reference: reference
                .parse()
                .map_err(|_| "a played record's reference is not a digest".to_owned())?,
        }),
        _ => None,
    };
    index.played.push(PlayedEntry {
        played: Played {
            hand_id: hand_id.to_owned(),
            table_id: record["table_id"]
                .as_str()
                .ok_or("a played record has no table")?
                .to_owned(),
            digest,
            net: record["net"].as_i64().ok_or("a played record has no net")?,
            luck: record["luck"]
                .as_f64()
                .ok_or("a played record has no luck")?,
            big_blind: record["big_blind"]
                .as_u64()
                .ok_or("a played record has no big blind")?,
            orbit,
            reference,
        },
        recorded_ms: record["recorded_ms"]
            .as_i64()
            .ok_or("a played record has no time")?,
    });
    Ok(())
}

fn apply_incident(index: &mut Index, record: &Value) -> Result<(), String> {
    let at = record["at"].as_str().ok_or("an incident has no kind")?;
    let at_id = record["at_id"]
        .as_str()
        .ok_or("an incident has no place")?
        .to_owned();
    let stablemate = record["stablemate"]
        .as_str()
        .ok_or("an incident has no stablemate")?
        .to_owned();
    index.incidents.push(match at {
        "shared_table" => Incident::SharedTable {
            table_id: at_id,
            stablemate,
        },
        "shared_hand" => Incident::SharedHand {
            hand_id: at_id,
            stablemate,
        },
        other => return Err(format!("no incident is called {other}")),
    });
    Ok(())
}

/// The traced hands the cap keeps, newest first: [`TRACED_HANDS`] of them, with insertion order
/// breaking a tie the way SQLite's `rowid DESC` does.
pub(super) fn visible_trace_rows(index: &Index) -> Vec<usize> {
    let mut held: Vec<(i64, usize)> = index
        .hands
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.traces.is_empty())
        .map(|(at, entry)| (entry.recorded_ms, at))
        .collect();
    held.sort_by_key(|(time, at)| (std::cmp::Reverse(*time), std::cmp::Reverse(*at)));
    held.into_iter()
        .take(TRACED_HANDS)
        .map(|(_, at)| at)
        .collect()
}

/// The record a hand is kept as, with its traces in the same write.
pub(super) fn hand_record(
    hand: &Hand,
    source: Source,
    recorded_ms: i64,
) -> Result<Value, StoreError> {
    // The store's own check is `json_valid` on the joined array; refuse the same frames here.
    let _: Value = serde_json::from_str(&canonical(&hand.frames))
        .map_err(|error| StoreError(format!("a hand's frames are not JSON: {error}")))?;
    let traces: Vec<Value> = hand
        .traced
        .iter()
        .map(|trace| {
            json!({
                "street": trace.street.name(),
                "branch": trace.branch.name(),
                "trace": trace_json(trace),
            })
        })
        .collect();
    Ok(json!({
        "kind": "hand",
        "hand_id": hand.hand_id,
        "table_id": hand.table_id,
        "source": source.name(),
        "recorded_ms": recorded_ms,
        "frames": hand.frames,
        "quality": Some(quality(&hand.frames)),
        "traces": traces,
    }))
}

/// The export record a differing downloaded copy is kept as, when there is one to keep.
pub(super) fn export_record(index: &Index, hand: &Hand) -> Result<Option<Value>, StoreError> {
    let Some(at) = index.by_id.get(&hand.hand_id) else {
        return Ok(None);
    };
    let stored = canonical(&index.hands[*at].frames);
    let text = canonical(&hand.frames);
    let one_object = hand.frames.len() == 1
        && serde_json::from_str::<Value>(&hand.frames[0])
            .map(|frame| frame.is_object())
            .unwrap_or(false);
    Ok((stored != text && one_object).then(|| {
        json!({
            "kind": "export",
            "hand_id": hand.hand_id,
            "frame": hand.frames[0],
        })
    }))
}

pub(super) fn played_record(played: &Played, recorded_ms: i64) -> Value {
    let (probe, orbit, arm) = match played.orbit {
        Some((probe, orbit, arm)) => (
            Some(format!("{probe:016x}")),
            Some(orbit),
            Some(arm_name(arm)),
        ),
        None => (None, None, None),
    };
    let (reference, reference_orbit, reference_side, reference_chance) = match played.reference {
        Some(standing) => (
            Some(standing.reference.to_string()),
            Some(standing.orbit),
            Some(standing.side.name()),
            Some(standing.chance),
        ),
        None => (None, None, None, None),
    };
    json!({
        "kind": "played",
        "hand_id": played.hand_id,
        "table_id": played.table_id,
        "recorded_ms": recorded_ms,
        "digest": played.digest.to_string(),
        "net": played.net,
        "luck": played.luck,
        "big_blind": played.big_blind,
        "probe": probe,
        "orbit": orbit,
        "arm": arm,
        "reference": reference,
        "reference_orbit": reference_orbit,
        "reference_side": reference_side,
        "reference_chance": reference_chance,
    })
}

/// The frames as the record keeps them: one JSON array.
pub(super) fn canonical(frames: &[String]) -> String {
    format!("[{}]", frames.join(","))
}

/// The frames parsed, as a list.
pub(super) fn parsed(frames: &[String]) -> Result<Vec<Value>, StoreError> {
    serde_json::from_str(&canonical(frames)).map_err(|error| StoreError(error.to_string()))
}

/// The frames in the compact shape SQLite's reader gives them back.
pub(super) fn normalized(frames: &[String]) -> Result<Vec<String>, StoreError> {
    Ok(parsed(frames)?.iter().map(ToString::to_string).collect())
}

/// A JSON time as milliseconds, through the ship bar's RFC 3339 reader.
pub(super) fn time(value: &Value, field: &str) -> Option<i64> {
    value[field]
        .as_str()
        .and_then(crate::ship::window::started_ms)
}

/// The big blind an operation divides by: SQLite kept it as `i64`, saturating.
pub(super) fn big_blind(big_blind: u64) -> f64 {
    i64::try_from(big_blind).unwrap_or(i64::MAX) as f64
}

/// One hand's luck-corrected win in big blinds.
pub(super) fn corrected(big_blind: u64, net: i64, luck: f64) -> f64 {
    (net as f64 - luck) / self::big_blind(big_blind)
}
