//! What the fleet's bots share about parameter sets, through two files in one directory beside
//! their stores:
//!
//! - `champion.json`: the parameter set every bot plays by. The canary writes it when a
//!   promotion's harm guard clears, and each bot reads it between hands. With no file, the
//!   champion is [`Params::DEFAULT`].
//! - `proposal.json`: a challenger the learner puts forward, with how to try it. The canary takes
//!   it up, and removes it, when no probe is running.
//!
//! A parameter set is numbers the owner tuned, not a read of an opponent, so sharing it crosses no
//! bot's reads (`docs/adr/0004-reads-are-per-bot.md`). Each number is written as the text that
//! reads back to the same bits, beside the digest those bits must hash to.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use svanbot_learn::probe::{Hypothesis, Kind};
use svanbot_strategy::params::Params;

use crate::health::write_atomically;

/// Where the fleet's files live, derived from a bot's store: the `fleet` directory beside it.
pub fn beside_store(store: impl AsRef<Path>) -> PathBuf {
    match store.as_ref().parent() {
        Some(dir) => dir.join("fleet"),
        None => PathBuf::from("fleet"),
    }
}

/// A parameter set as JSON: its digest, and each number as text.
pub fn params_json(params: &Params) -> Value {
    let numbers: Map<String, Value> = params
        .named()
        .into_iter()
        .map(|(name, value)| (name.to_owned(), Value::String(value.to_string())))
        .collect();
    json!({ "digest": params.digest().to_string(), "numbers": numbers })
}

/// The parameter set in `value`, refused when a number is missing or the numbers do not hash to
/// the digest written beside them.
pub fn params_from(value: &Value) -> Result<Params, String> {
    let mut params = Params::DEFAULT;
    for (name, _) in Params::DEFAULT.named() {
        let number = value["numbers"][name]
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .ok_or_else(|| format!("the parameter set lacks {name}"))?;
        params = params.with(name, number).unwrap_or(params);
    }
    if value["digest"].as_str() != Some(&params.digest().to_string()) {
        return Err("the parameter set does not hash to its digest".into());
    }
    Ok(params)
}

/// A challenger put forward, and how the canary is to try it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Proposal {
    pub challenger: Params,
    /// A probe, whose verdict is evidence only, or the harm guard of a promotion.
    pub kind: Kind,
    /// The loss, in bb/100, past which the challenger is harmful.
    pub harm_margin: f64,
    /// The challenger's edge by the paired sim, in bb/100, when one ran.
    pub sim_estimate: Option<f64>,
    pub max_orbits: u64,
}

impl Proposal {
    /// The hypothesis this proposal is against `champion`.
    pub fn hypothesis(&self, champion: &Params) -> Hypothesis {
        Hypothesis {
            champion: champion.digest(),
            challenger: self.challenger.digest(),
            kind: self.kind,
            harm_margin: self.harm_margin,
            sim_estimate: self.sim_estimate,
            max_orbits: self.max_orbits,
        }
    }

    pub fn to_json(&self) -> String {
        let guard_hands = match self.kind {
            Kind::Probe => Value::Null,
            Kind::Guard { hands } => json!(hands),
        };
        json!({
            "challenger": params_json(&self.challenger),
            "guard_hands": guard_hands,
            "harm_margin": self.harm_margin.to_string(),
            "sim_estimate": self.sim_estimate.map(|estimate| estimate.to_string()),
            "max_orbits": self.max_orbits,
        })
        .to_string()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let number = |key: &str| {
            value[key]
                .as_str()
                .and_then(|text| text.parse::<f64>().ok())
        };
        Ok(Self {
            challenger: params_from(&value["challenger"])?,
            kind: match value["guard_hands"].as_u64() {
                Some(hands) => Kind::Guard { hands },
                None => Kind::Probe,
            },
            harm_margin: number("harm_margin").ok_or("the proposal lacks harm_margin")?,
            sim_estimate: number("sim_estimate"),
            max_orbits: value["max_orbits"]
                .as_u64()
                .ok_or("the proposal lacks max_orbits")?,
        })
    }
}

/// The fleet's files in one directory.
#[derive(Clone, Debug)]
pub struct Fleet {
    dir: PathBuf,
}

impl Fleet {
    /// The fleet's files under `dir`, whether or not `dir` exists yet.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The champion: the last promoted parameter set, or the default before any promotion.
    pub fn champion(&self) -> Result<Params, String> {
        match fs::read_to_string(self.dir.join("champion.json")) {
            Ok(text) => {
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                params_from(&value)
            }
            Err(_) => Ok(Params::DEFAULT),
        }
    }

    /// Makes `champion` the parameter set the fleet plays by.
    pub fn promote(&self, champion: &Params) -> Result<(), String> {
        self.write("champion.json", &params_json(champion).to_string())
    }

    /// Puts `proposal` forward, in place of any that still waits.
    pub fn propose(&self, proposal: &Proposal) -> Result<(), String> {
        self.write("proposal.json", &proposal.to_json())
    }

    /// The waiting proposal, removed so it is taken once. A proposal that cannot be read is
    /// removed too, and reported.
    pub fn take_proposal(&self) -> Result<Option<Proposal>, String> {
        let path = self.dir.join("proposal.json");
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(None);
        };
        fs::remove_file(&path).map_err(|e| format!("fleet: {e}"))?;
        Proposal::from_json(&text).map(Some)
    }

    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("fleet: {e}"))?;
        write_atomically(&self.dir.join(name), text.as_bytes()).map_err(|e| format!("fleet: {e}"))
    }
}
