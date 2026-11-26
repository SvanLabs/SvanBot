//! Closed seasons are fixed once ended. Retain completion only after checking coverage and scope.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{DownloadError, Summary, agent_id, export_path, fetch_export};
use crate::rest::Rest;
use crate::ship::window::started_ms;
use crate::store::Store;

const STATS: &str = "/season/stats";
const CATALOG: &str = "/season/list";

/// Download the active season and every closed season in the bot's own statistics. Report each
/// attempt, including a skipped season already verified, and keep going when one export fails.
pub fn download_all(
    rest: &mut Rest,
    store: &Store,
    mut now_ms: impl FnMut() -> i64,
    mut report: impl FnMut(Option<u64>, Result<Option<Summary>, DownloadError>),
) -> Result<(), DownloadError> {
    store.wait_for_download_writer()?;
    let agent = agent_id(rest)?;
    let base = export_path(&agent);
    let stats = played(&rest.get(STATS)?)?;
    let catalog = rest.get(CATALOG)?;
    let listed = catalog
        .as_array()
        .ok_or_else(|| history("the venue season list is not an array"))?;
    let current = fetch_export(rest, store, &base, None, &mut now_ms).map(Some);
    let mut failures = usize::from(current.is_err());
    report(None, current);
    for (number, hands) in stats.into_iter().rev() {
        let result = season(listed, number, hands).and_then(|season| {
            if store.verified_season(season.id)? >= season.hands {
                return Ok(None);
            }
            let path = format!("{base}&season_id={}", season.id);
            let summary = fetch_export(rest, store, &path, Some(&season), &mut now_ms)?;
            store.record_verified_season(season.id, summary.offset, now_ms())?;
            Ok(Some(summary))
        });
        failures += usize::from(result.is_err());
        report(Some(number), result);
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(history(format!(
            "{failures} season export(s) could not be verified"
        )))
    }
}

pub(super) struct ClosedSeason<'a> {
    id: &'a str,
    pub(super) hands: u64,
    start: i64,
    end: i64,
}

impl ClosedSeason<'_> {
    pub(super) fn includes(&self, hand: &Value) -> bool {
        hand["started_at"]
            .as_str()
            .and_then(started_ms)
            .is_some_and(|start| (self.start..self.end).contains(&start))
    }
}

fn played(stats: &Value) -> Result<BTreeMap<u64, u64>, DownloadError> {
    let seasons = stats["seasons"]
        .as_array()
        .ok_or_else(|| history("the bot's season statistics have no seasons array"))?;
    let mut played = BTreeMap::new();
    for row in seasons {
        let number = row["season_number"]
            .as_u64()
            .ok_or_else(|| history("a played season is missing its number"))?;
        let hands = row["hands_played"]
            .as_u64()
            .ok_or_else(|| history(format!("season {number} is missing its hand count")))?;
        if hands > 0 && played.insert(number, hands).is_some() {
            return Err(history(format!("season {number} is listed more than once")));
        }
    }
    Ok(played)
}

fn season(listed: &[Value], number: u64, hands: u64) -> Result<ClosedSeason<'_>, DownloadError> {
    let error = || history(format!("season {number} has no usable closed-season entry"));
    let row = listed
        .iter()
        .find(|row| row["season_number"].as_u64() == Some(number))
        .ok_or_else(error)?;
    let Some(id) = row["season_id"].as_str().filter(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) else {
        return Err(error());
    };
    let start = row["start_date"]
        .as_str()
        .and_then(started_ms)
        .ok_or_else(error)?;
    let end = row["end_date"]
        .as_str()
        .and_then(started_ms)
        .ok_or_else(error)?;
    if row["status"] != "ended" || end <= start {
        return Err(error());
    }
    Ok(ClosedSeason {
        id,
        hands,
        start,
        end,
    })
}

pub(super) fn history(error: impl Into<String>) -> DownloadError {
    DownloadError::History(error.into())
}
