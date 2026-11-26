//! Each bot's search progress (`searches.json`): the log a run continues after a restart, keyed
//! by the champion it measured, and the candidate its search found for its own probe and
//! promotion (#1222). Parameter sets and sim numbers only, never a read of an opponent
//! (`docs/adr/0004-reads-are-per-bot.md`).
//!
//! A champion change invalidates a bot's log and its candidate: the next read under the new
//! champion starts empty, and the next write replaces the old entry rather than merging it.

use serde_json::{Map, Value, json};
use svanbot_learn::paired::Verdict;
use svanbot_learn::search::Entry;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::{Digest, Params};
use svanbot_strategy::style;

use super::search::{entry_from, entry_json};
use super::{Fleet, SEARCH_KEPT, params_from, params_json};

const SEARCHES: &str = "searches.json";

/// One bot's search: what it has screened and confirmed, what it last did, and the candidate it
/// found, all against `champion`.
#[derive(Clone, Debug, PartialEq)]
pub struct BotSearch {
    pub champion: Digest,
    pub entries: Vec<Entry>,
    pub last: Option<(i64, String)>,
    pub candidate: Option<Candidate>,
}

impl BotSearch {
    /// A bot's search under `champion` with nothing in it yet.
    pub fn empty(champion: Digest) -> Self {
        Self {
            champion,
            entries: Vec::new(),
            last: None,
            candidate: None,
        }
    }
}

/// A challenger a bot's search put above its champion on a fresh confirmation: the candidate
/// its own probe and promotion take up. It names the kind and style it was simulated with, so a
/// changed assignment makes it stale (ADR 0005).
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub challenger: Params,
    pub kind: PolicyKind,
    pub style: &'static str,
    pub verdict: Verdict,
}

impl Fleet {
    /// `bot`'s search under `champion`. A log under another champion is stale: it is not read,
    /// so a champion change starts the search afresh.
    pub fn bot_search(&self, bot: &str, champion: &Params) -> Result<BotSearch, String> {
        let digest = champion.digest();
        match self.searches()?.get(bot) {
            Some(value) => bot_search_from(value, digest)
                .map_err(|error| format!("{SEARCHES}: {bot}: {error}")),
            None => Ok(BotSearch::empty(digest)),
        }
    }

    /// Adds `entries` to `bot`'s log, notes what the run did, and stores `candidate` when the
    /// search found one. A log under another champion is replaced: a stale round does not carry
    /// into the new champion's search, and a stale candidate is dropped.
    pub fn record_bot_search(
        &self,
        bot: &str,
        champion: &Params,
        entries: &[Entry],
        note: &str,
        at_ms: i64,
        candidate: Option<&Candidate>,
    ) -> Result<(), String> {
        let _lock = self.lock_fleet_file(SEARCHES)?;
        let digest = champion.digest();
        let mut bots = self.searches()?;
        let mut search = match bots.get(bot) {
            Some(value) => bot_search_from(value, digest)
                .map_err(|error| format!("{SEARCHES}: {bot}: {error}"))?,
            None => BotSearch::empty(digest),
        };
        search.entries.extend_from_slice(entries);
        let over = search.entries.len().saturating_sub(SEARCH_KEPT);
        search.entries.drain(..over);
        search.last = Some((at_ms, note.to_owned()));
        if let Some(candidate) = candidate {
            search.candidate = Some(candidate.clone());
        }
        bots.insert(bot.into(), bot_search_json(&search));
        self.write(SEARCHES, &json!({ "bots": bots }).to_string())
    }

    fn searches(&self) -> Result<Map<String, Value>, String> {
        self.bots_in(SEARCHES)
    }
}

fn bot_search_from(value: &Value, champion: Digest) -> Result<BotSearch, String> {
    let stored = value["champion"]
        .as_str()
        .and_then(|text| text.parse::<Digest>().ok())
        .ok_or("a bot's search lacks its champion")?;
    if stored != champion {
        return Ok(BotSearch::empty(champion));
    }
    let entries = value["entries"]
        .as_array()
        .ok_or("a bot's search lacks entries")?
        .iter()
        .map(entry_from)
        .collect::<Result<_, _>>()?;
    let last = match (value["last_ms"].as_i64(), value["last"].as_str()) {
        (Some(at), Some(note)) => Some((at, note.to_owned())),
        _ => None,
    };
    let candidate = match &value["candidate"] {
        Value::Null => None,
        found => Some(candidate_from(found)?),
    };
    Ok(BotSearch {
        champion,
        entries,
        last,
        candidate,
    })
}

fn bot_search_json(search: &BotSearch) -> Value {
    json!({
        "champion": search.champion.to_string(),
        "entries": search.entries.iter().map(entry_json).collect::<Vec<_>>(),
        "last_ms": search.last.as_ref().map(|(at, _)| *at),
        "last": search.last.as_ref().map(|(_, note)| note),
        "candidate": search.candidate.as_ref().map(candidate_json),
    })
}

fn candidate_json(candidate: &Candidate) -> Value {
    json!({
        "challenger": params_json(&candidate.challenger),
        "kind": candidate.kind.name(),
        "style": candidate.style,
        "deals": candidate.verdict.deals,
        "mbb_per_100": candidate.verdict.mbb_per_100.to_string(),
        "ci95": candidate.verdict.ci95.to_string(),
    })
}

fn candidate_from(value: &Value) -> Result<Candidate, String> {
    let kind = value["kind"]
        .as_str()
        .and_then(PolicyKind::named)
        .ok_or("a search candidate's kind is unknown")?;
    let style = value["style"]
        .as_str()
        .and_then(style::named)
        .ok_or("a search candidate's style is unknown")?;
    let number = |key: &str| {
        value[key]
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .ok_or(format!("a search candidate lacks {key}"))
    };
    Ok(Candidate {
        challenger: params_from(&value["challenger"])
            .map_err(|error| format!("a search candidate's challenger: {error}"))?,
        kind,
        style: style.name,
        verdict: Verdict {
            deals: value["deals"]
                .as_u64()
                .ok_or("a search candidate lacks deals")? as usize,
            mbb_per_100: number("mbb_per_100")?,
            ci95: number("ci95")?,
        },
    })
}
