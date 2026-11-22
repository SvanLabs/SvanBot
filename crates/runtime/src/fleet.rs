//! What the fleet's bots share about parameter sets, through two files in one directory beside
//! their stores:
//!
//! - `champion.json`: the parameter set every bot plays by. The canary writes it when a
//!   promotion's harm guard clears, and each bot reads it between hands. With no file, the
//!   champion is [`Params::DEFAULT`].
//! - `proposal.json`: a challenger the learner puts forward, with how to try it. The canary takes
//!   it up, and removes it, when no probe is running.
//! - `arena.json`: the Arena's last runs. The newest is the scoreboard the dashboard shows and
//!   the release gate reads; [`ARENA_HISTORY`] of them are kept, so a trend across commits is a
//!   read and not a re-run.
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

/// Runs kept in `arena.json` for each league: the newest, and the ones before it, for the trend.
pub const ARENA_HISTORY: usize = 12;

/// Which Arena league a run was played in (V11-29). The two measure different things: heads up
/// is the re-solve after the flop, 6-max the multiway path. They are kept side by side and
/// never pooled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum League {
    HeadsUp,
    SixMax,
}

impl League {
    /// The league's name in `arena.json` and on the scoreboard.
    pub const fn name(self) -> &'static str {
        match self {
            Self::HeadsUp => "heads-up",
            Self::SixMax => "6-max",
        }
    }

    /// The league of a stored run. A run stored before 6-max has no league, and was heads up.
    fn from_json(value: &Value) -> Result<Self, String> {
        match value.as_str() {
            None if value.is_null() => Ok(Self::HeadsUp),
            Some("heads-up") => Ok(Self::HeadsUp),
            Some("6-max") => Ok(Self::SixMax),
            _ => Err(format!("no Arena league is called {value}")),
        }
    }
}

/// One line of a recorded run: how the policy did against one opponent.
#[derive(Clone, Debug, PartialEq)]
pub struct ArenaRow {
    pub opponent: String,
    pub deals: usize,
    pub mbb_per_100: f64,
    pub ci95: f64,
}

/// A recorded run of the Arena: the commit, the champion and the seed it was played from, and its
/// scoreboard. The champion is what makes the run's numbers mean anything: a promotion changes
/// how the bots play without touching the commit, so a scoreboard keyed on the commit alone
/// could stand for a policy the fleet no longer plays.
#[derive(Clone, Debug, PartialEq)]
pub struct ArenaRun {
    pub commit: String,
    /// The digest of the champion the run played, as `Params::digest` renders it.
    pub champion: String,
    pub seed: u64,
    pub league: League,
    pub rows: Vec<ArenaRow>,
}

impl ArenaRun {
    /// The commit line of a run played from a checkout. There is no commit to play it again from,
    /// so two such runs are the same run only if every number agrees.
    pub const LOCAL_COMMIT: &'static str = "local";

    /// The run as the page reads it: every number a JSON number, the way of the fleet document.
    pub fn as_value(&self) -> Value {
        json!({
            "commit": self.commit,
            "champion": self.champion,
            "seed": self.seed,
            "league": self.league.name(),
            "rows": self
                .rows
                .iter()
                .map(|row| json!({
                    "opponent": row.opponent,
                    "deals": row.deals,
                    "mbb_per_100": row.mbb_per_100,
                    "ci95": row.ci95,
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// The run as the JSON text it is written in, here and in `svanbot arena --out`: one format
    /// for a run, so the scoreboard and the file it hands an operator cannot disagree.
    pub fn as_text(&self) -> String {
        self.as_value().to_string()
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let number = |value: &Value, key: &str| -> Result<f64, String> {
            value[key]
                .as_f64()
                .ok_or_else(|| format!("the arena run lacks {key}"))
        };
        let rows = value["rows"]
            .as_array()
            .ok_or("the arena run lacks rows")?
            .iter()
            .map(|row| {
                Ok(ArenaRow {
                    opponent: row["opponent"]
                        .as_str()
                        .ok_or("an arena row lacks its opponent")?
                        .to_owned(),
                    deals: row["deals"].as_u64().ok_or("an arena row lacks deals")? as usize,
                    mbb_per_100: number(row, "mbb_per_100")?,
                    ci95: number(row, "ci95")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            commit: value["commit"]
                .as_str()
                .ok_or("the arena run lacks its commit")?
                .to_owned(),
            champion: value["champion"]
                .as_str()
                .ok_or("the arena run lacks its champion")?
                .to_owned(),
            seed: value["seed"]
                .as_u64()
                .ok_or("the arena run lacks its seed")?,
            league: League::from_json(&value["league"])?,
            rows,
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

    /// The Arena's recorded runs, newest first. No file yet is no run yet.
    pub fn arena_runs(&self) -> Result<Vec<ArenaRun>, String> {
        let Ok(text) = fs::read_to_string(self.dir.join("arena.json")) else {
            return Ok(Vec::new());
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        value["runs"]
            .as_array()
            .ok_or("the arena file lacks runs")?
            .iter()
            .map(ArenaRun::from_json)
            .collect()
    }

    /// Records `run` as the latest, keeping the runs before it up to [`ARENA_HISTORY`] in each
    /// league. A run of the same league, commit, champion and seed replaces its old line rather
    /// than repeating it; the same commit under another champion is a different run, and keeps
    /// its own line.
    pub fn record_arena(&self, run: &ArenaRun) -> Result<(), String> {
        let mut runs = self.arena_runs()?;
        // A run with a commit is played again from it, so commit, champion and seed name it. A
        // local run has no commit to be played from, so it is the same run only if it measured
        // the same: two runs of changed code must not replace one another.
        let same = |held: &ArenaRun| {
            held.league == run.league
                && held.commit == run.commit
                && held.champion == run.champion
                && held.seed == run.seed
                && (run.commit != ArenaRun::LOCAL_COMMIT || held.rows == run.rows)
        };
        runs.retain(|held| !same(held));
        runs.insert(0, run.clone());
        // Newest first, so the first ARENA_HISTORY of a league are the ones it keeps.
        let mut kept = [0usize; 2];
        runs.retain(|held| {
            let count = &mut kept[held.league as usize];
            *count += 1;
            *count <= ARENA_HISTORY
        });
        let runs: Vec<String> = runs.iter().map(ArenaRun::as_text).collect();
        self.write("arena.json", &format!("{{\"runs\":[{}]}}", runs.join(",")))
    }

    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("fleet: {e}"))?;
        write_atomically(&self.dir.join(name), text.as_bytes()).map_err(|e| format!("fleet: {e}"))
    }
}
