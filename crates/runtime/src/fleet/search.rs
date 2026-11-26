//! What the unattended search keeps in the fleet's directory (V11-49):
//!
//! - `probe.json`: the canary's running probe, mirrored from the proposal it took up, so the
//!   learner can tell that evidence is pending without reading a bot's store.
//! - `search.json`: the search log, every step simmed and every challenger proposed, keyed by the
//!   champion it was measured against, with the last thing the learner did. Parameter sets and
//!   sim numbers only, never a read of an opponent (`docs/adr/0004-reads-are-per-bot.md`).

use std::fs;
use std::io::ErrorKind;

use serde_json::{Value, json};
use svanbot_learn::paired::Verdict;
use svanbot_learn::search::{Entry, Stage, Step};
use svanbot_strategy::params::Digest;

use super::{Fleet, Proposal, params_from, params_json};

/// The entries the search log keeps, newest last: a champion's whole search is a handful, so this
/// holds the searches round many champions.
pub const SEARCH_KEPT: usize = 200;

/// The search log, and the last thing the learner did with when.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchLog {
    pub entries: Vec<Entry>,
    /// When the learner last ran, in ms since the epoch, and what it did.
    pub last: Option<(i64, String)>,
}

impl Fleet {
    /// Puts `proposal` forward only if the canary has nothing else: no proposal waits and no probe
    /// runs. Returns whether it was put forward. A proposal that waits is never replaced, so the
    /// learner cannot overwrite an experiment the operator or an earlier search set up.
    pub fn propose_if_free(&self, proposal: &Proposal) -> Result<bool, String> {
        if self.probe()?.is_some() {
            return Ok(false);
        }
        fs::create_dir_all(&self.dir).map_err(|e| format!("fleet: {e}"))?;
        // Written whole beside it, then linked into place: a link to a name that exists fails,
        // so of two writers only one is put forward, and the canary never reads half a file.
        let staged = self
            .dir
            .join(format!("proposal.{}.new", std::process::id()));
        fs::write(&staged, proposal.to_json()).map_err(|e| format!("fleet: {e}"))?;
        let linked = fs::hard_link(&staged, self.dir.join("proposal.json"));
        let _ = fs::remove_file(&staged);
        match linked {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(format!("fleet: {error}")),
        }
    }

    /// The proposal waiting for the canary, read and left in place.
    pub fn waiting(&self) -> Result<Option<Proposal>, String> {
        match fs::read_to_string(self.dir.join("proposal.json")) {
            Ok(text) => Proposal::from_json(&text).map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("fleet: {error}")),
        }
    }

    /// The probe the canary is running, as the proposal it took up, or `None` when none runs.
    pub fn probe(&self) -> Result<Option<Proposal>, String> {
        match fs::read_to_string(self.dir.join("probe.json")) {
            Ok(text) => Proposal::from_json(&text).map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("fleet: {error}")),
        }
    }

    /// The canary's probe is over: its mirror goes.
    pub fn end_probe(&self) -> Result<(), String> {
        match fs::remove_file(self.dir.join("probe.json")) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("fleet: {error}")),
        }
    }

    /// Mirrors `proposal` as the canary's running probe: a probe taken up again after a restart.
    pub fn mirror_probe(&self, proposal: &Proposal) -> Result<(), String> {
        self.write("probe.json", &proposal.to_json())
    }

    /// The search log. No file yet is an empty log.
    pub fn search_log(&self) -> Result<SearchLog, String> {
        let text = match fs::read_to_string(self.dir.join("search.json")) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(SearchLog::default()),
            Err(error) => return Err(format!("fleet: {error}")),
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let entries = value["entries"]
            .as_array()
            .ok_or("the search log lacks entries")?
            .iter()
            .map(entry_from)
            .collect::<Result<_, _>>()?;
        let last = match (value["last_ms"].as_i64(), value["last"].as_str()) {
            (Some(at), Some(note)) => Some((at, note.to_owned())),
            _ => None,
        };
        Ok(SearchLog { entries, last })
    }

    /// Adds `entries` to the search log and notes what the learner did at `at_ms`, keeping the
    /// newest [`SEARCH_KEPT`].
    pub fn record_search(&self, entries: &[Entry], note: &str, at_ms: i64) -> Result<(), String> {
        let mut log = self.search_log()?;
        log.entries.extend_from_slice(entries);
        let over = log.entries.len().saturating_sub(SEARCH_KEPT);
        log.entries.drain(..over);
        let text = json!({
            "entries": log.entries.iter().map(entry_json).collect::<Vec<_>>(),
            "last_ms": at_ms,
            "last": note,
        });
        self.write("search.json", &text.to_string())
    }
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::First => "first",
        Stage::Confirmation => "confirmation",
    }
}

/// An entry as the log writes it. Each number is the text that reads back to the same bits, and
/// a seed is text, since a reader in a browser holds a number only to 53 bits.
pub(super) fn entry_json(entry: &Entry) -> Value {
    match *entry {
        Entry::Simmed { step, verdict } => json!({
            "champion": step.champion.to_string(),
            "challenger": params_json(&step.challenger),
            "stage": stage_name(step.stage),
            "seed": step.seed.to_string(),
            "deals": verdict.deals,
            "mbb_per_100": verdict.mbb_per_100.to_string(),
            "ci95": verdict.ci95.to_string(),
        }),
        Entry::Proposed {
            champion,
            challenger,
        } => json!({
            "champion": champion.to_string(),
            "challenger": params_json(&challenger),
            "proposed": true,
        }),
    }
}

pub(super) fn entry_from(value: &Value) -> Result<Entry, String> {
    let champion = value["champion"]
        .as_str()
        .and_then(|text| text.parse::<Digest>().ok())
        .ok_or("a search entry's champion is not a digest")?;
    let challenger = params_from(&value["challenger"])?;
    if value["proposed"].as_bool() == Some(true) {
        return Ok(Entry::Proposed {
            champion,
            challenger,
        });
    }
    let number = |key: &str| {
        value[key]
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .ok_or(format!("a search entry lacks {key}"))
    };
    let stage = match value["stage"].as_str() {
        Some("first") => Stage::First,
        Some("confirmation") => Stage::Confirmation,
        _ => return Err("a search entry's stage is neither first nor confirmation".into()),
    };
    Ok(Entry::Simmed {
        step: Step {
            champion,
            challenger,
            stage,
            seed: value["seed"]
                .as_str()
                .and_then(|text| text.parse().ok())
                .ok_or("a search entry's seed is not a number")?,
        },
        verdict: Verdict {
            deals: value["deals"]
                .as_u64()
                .ok_or("a search entry lacks deals")? as usize,
            mbb_per_100: number("mbb_per_100")?,
            ci95: number("ci95")?,
        },
    })
}
