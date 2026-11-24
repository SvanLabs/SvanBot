//! The unattended search, as the fleet document carries it (V11-49): the champion it searches
//! round, what the canary has pending, what the last run did, and the newest steps first.

use std::path::Path;

use serde_json::{Value, json};
use svanbot_learn::search::{Entry, Stage};
use svanbot_runtime::fleet::{Fleet, params_json};

/// Steps the page shows, newest first.
const SHOWN: usize = 12;

/// The search in `fleet_dir`. A file that cannot be read is said in `error`, beside what could be.
pub(super) fn search(fleet_dir: &Path) -> Value {
    let fleet = Fleet::in_dir(fleet_dir);
    let mut errors = Vec::new();
    let mut read = |found: Result<Option<String>, String>| {
        found.unwrap_or_else(|why| {
            errors.push(why);
            None
        })
    };
    let digest = |proposal: Option<svanbot_runtime::fleet::Proposal>| {
        proposal.map(|proposal| proposal.challenger.digest().to_string())
    };
    let probe = read(fleet.probe().map(digest));
    let waiting = read(fleet.waiting().map(digest));
    let champion = fleet.champion().map_err(|why| errors.push(why)).ok();
    let log = fleet.search_log().unwrap_or_else(|why| {
        errors.push(why);
        Default::default()
    });
    let simmed = champion.map_or(0, |champion| {
        log.entries
            .iter()
            .filter(|entry| {
                matches!(entry, Entry::Simmed { step, .. } if step.champion == champion.digest())
            })
            .count()
    });
    json!({
        "champion": champion.as_ref().map(params_json),
        "probe": probe,
        "waiting": waiting,
        "last": log.last.map(|(at_ms, note)| json!({ "at_ms": at_ms, "note": note })),
        "simmed": simmed,
        "steps": log.entries.iter().rev().take(SHOWN).map(step_json).collect::<Vec<_>>(),
        "error": (!errors.is_empty()).then(|| errors.join("; ")),
    })
}

fn step_json(entry: &Entry) -> Value {
    match *entry {
        Entry::Simmed { step, verdict } => json!({
            "champion": step.champion.to_string(),
            "challenger": params_json(&step.challenger),
            "stage": match step.stage {
                Stage::First => "first",
                Stage::Confirmation => "confirmation",
            },
            "deals": verdict.deals,
            "mbb_per_100": verdict.mbb_per_100,
            "ci95": verdict.ci95,
        }),
        Entry::Proposed {
            champion,
            challenger,
        } => json!({
            "champion": champion.to_string(),
            "challenger": params_json(&challenger),
            "stage": "proposed",
        }),
    }
}
