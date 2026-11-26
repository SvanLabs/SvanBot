//! The experiment ledger (`experiments.json`, V11-129): the bot's hypothesis, the live evidence
//! that decides it, and the outcome, in one record.
//!
//! The learner writes a waiting experiment (the bot's own belief from its own search log). The
//! canary claims it when it takes up the matching proposal, and the guard's verdict ends it: a
//! clear keeps the challenger, harm or an undecided budget leaves the bot exactly as it was. A
//! bot has one experiment at a time and the fleet at most [`ACTIVE`]; a challenger already asked
//! about is never asked again.

use std::collections::BTreeSet;
use std::fs;

use serde_json::{Map, Value, json};
use svanbot_learn::experiment::{Claim, Experiment, Metric, Outcome, StopRule};
use svanbot_learn::paired::Verdict as Sim;
use svanbot_strategy::params::Digest;

use super::{Fleet, params_from, params_json};

const EXPERIMENTS: &str = "experiments.json";

/// The file's three parts: the waiting experiment, each bot's record, and the retired ones.
type Ledger = (Option<Value>, Map<String, Value>, Vec<Value>);

/// The most experiments live across the fleet at once: one a bot.
pub const ACTIVE: usize = 5;

/// Where an experiment stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentStatus {
    /// Written by the learner, waiting for a bot to claim it.
    Waiting,
    /// A bot is playing it; its live hands are counting.
    Testing,
    /// The guard cleared: the challenger stays.
    Cleared,
    /// The guard found harm: the bot is left as it was.
    Harmful,
    /// The budget was spent without a verdict: the bot is left as it was.
    Undecided,
}

impl ExperimentStatus {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Testing => "testing",
            Self::Cleared => "cleared",
            Self::Harmful => "harmful",
            Self::Undecided => "undecided",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        [
            Self::Waiting,
            Self::Testing,
            Self::Cleared,
            Self::Harmful,
            Self::Undecided,
        ]
        .into_iter()
        .find(|status| status.name() == name)
    }

    /// Still owns its place: waiting for a bot or running.
    pub const fn active(self) -> bool {
        matches!(self, Self::Waiting | Self::Testing)
    }

    /// The status a probe verdict leaves.
    pub fn from_outcome(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Testing => Self::Testing,
            Outcome::Cleared => Self::Cleared,
            Outcome::Harmful => Self::Harmful,
            Outcome::Undecided => Self::Undecided,
        }
    }
}

/// The live evidence read into the ledger when the guard ends or runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    /// The guard's verdict, as the probe named it.
    pub verdict: String,
    /// The live edge over the reference, in bb/100, and its 95% half-width.
    pub edge: f64,
    pub radius: f64,
    /// The hands the guard has counted.
    pub hands: u64,
}

/// One experiment, as the ledger holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ExperimentRecord {
    pub experiment: Experiment,
    /// The bot that claimed it; `None` while it waits.
    pub bot: Option<String>,
    pub status: ExperimentStatus,
    pub probe: Option<u64>,
    pub evidence: Option<Evidence>,
    pub opened_ms: i64,
    pub decided_ms: Option<i64>,
    pub note: Option<String>,
}

impl Fleet {
    /// The waiting experiment, if the learner has written one.
    pub fn waiting_experiment(&self) -> Result<Option<ExperimentRecord>, String> {
        self.experiments()
            .map(|(waiting, _, _)| {
                waiting.map(|value| {
                    record_from(&value).map_err(|error| format!("{EXPERIMENTS}: {error}"))
                })
            })?
            .transpose()
    }

    /// `bot`'s experiment, or `None` while it has none.
    pub fn experiment_for(&self, bot: &str) -> Result<Option<ExperimentRecord>, String> {
        let (_, bots, _) = self.experiments()?;
        bots.get(bot)
            .map(|value| {
                record_from(value).map_err(|error| format!("{EXPERIMENTS}: {bot}: {error}"))
            })
            .transpose()
    }

    /// Writes `experiment` as waiting, unless one already waits or [`ACTIVE`] are live. Returns
    /// whether it was written.
    pub fn open_experiment(&self, experiment: &Experiment, now_ms: i64) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(EXPERIMENTS)?;
        let (waiting, bots, history) = self.experiments()?;
        if waiting.is_some() || active(&bots)? >= ACTIVE {
            return Ok(false);
        }
        let record = ExperimentRecord {
            experiment: experiment.clone(),
            bot: None,
            status: ExperimentStatus::Waiting,
            probe: None,
            evidence: None,
            opened_ms: now_ms,
            decided_ms: None,
            note: None,
        };
        self.write_experiments(Some(&record_json(&record)), &bots, &history)?;
        Ok(true)
    }

    /// Claims the waiting experiment for `bot` when its challenger is `challenger`. False when
    /// nothing waits for that challenger, or the bot already has a live experiment.
    pub fn claim_experiment(
        &self,
        bot: &str,
        challenger: &Digest,
        now_ms: i64,
    ) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(EXPERIMENTS)?;
        let (Some(waiting), mut bots, history) = self.experiments()? else {
            return Ok(false);
        };
        let mut record =
            record_from(&waiting).map_err(|error| format!("{EXPERIMENTS}: {error}"))?;
        if record.experiment.claim.challenger().digest() != *challenger
            || !record.status.active()
            || bots
                .get(bot)
                .is_some_and(|held| status_of(held).is_ok_and(ExperimentStatus::active))
        {
            return Ok(false);
        }
        record.bot = Some(bot.into());
        record.status = ExperimentStatus::Testing;
        record.note = Some(format!("claimed by {bot} at {now_ms}"));
        bots.insert(bot.into(), record_json(&record));
        self.write_experiments(None, &bots, &history)?;
        Ok(true)
    }

    /// Records the guard's reading for `bot`'s running experiment: the evidence, the status its
    /// verdict names, and, for a terminal verdict, when it was decided. Returns its note.
    pub fn record_experiment(
        &self,
        bot: &str,
        probe: Option<u64>,
        outcome: Outcome,
        evidence: Evidence,
        now_ms: i64,
    ) -> Result<Option<String>, String> {
        let _lock = self.lock_fleet_file(EXPERIMENTS)?;
        let (waiting, mut bots, history) = self.experiments()?;
        let Some(value) = bots.get(bot) else {
            return Ok(None);
        };
        let mut record =
            record_from(value).map_err(|error| format!("{EXPERIMENTS}: {bot}: {error}"))?;
        if record.status != ExperimentStatus::Testing {
            return Ok(None);
        }
        record.probe = probe;
        record.evidence = Some(evidence.clone());
        record.status = ExperimentStatus::from_outcome(outcome);
        if !record.status.active() {
            record.decided_ms = Some(now_ms);
        }
        record.note = Some(format!(
            "{}: {} {:+.1} bb/100 over {} hands",
            record.status.name(),
            evidence.verdict,
            evidence.edge,
            evidence.hands
        ));
        bots.insert(bot.into(), record_json(&record));
        self.write_experiments(waiting.as_ref(), &bots, &history)?;
        Ok(Some(format!(
            "{bot}'s experiment {}: {} {:+.1} bb/100, 95% ±{:.1} over {} hands",
            record.status.name(),
            evidence.verdict,
            evidence.edge,
            evidence.radius,
            evidence.hands
        )))
    }

    /// How many experiments are live: the waiting one and every bot's testing one.
    pub fn active_experiments(&self) -> Result<usize, String> {
        let (waiting, bots, _) = self.experiments()?;
        Ok(usize::from(waiting.is_some()) + active(&bots)?)
    }

    /// Every challenger already asked about, so the learner never asks the same question twice.
    pub fn tried_challengers(&self) -> Result<BTreeSet<Digest>, String> {
        let (waiting, bots, history) = self.experiments()?;
        let mut tried = BTreeSet::new();
        let mut held = Vec::new();
        if let Some(waiting) = waiting
            && let Ok(record) = record_from(&waiting)
        {
            held.push(record);
        }
        for value in bots.values().chain(history.iter()) {
            if let Ok(record) = record_from(value) {
                held.push(record);
            }
        }
        for record in held {
            tried.insert(record.experiment.claim.challenger().digest());
        }
        Ok(tried)
    }

    /// Retires a waiting experiment the fleet cannot run (a stale champion, a dropped
    /// proposal): it ends undecided in the history, so the learner may ask the next question.
    pub fn retire_waiting(&self, note: &str, now_ms: i64) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(EXPERIMENTS)?;
        let (Some(waiting), bots, mut history) = self.experiments()? else {
            return Ok(false);
        };
        let mut record =
            record_from(&waiting).map_err(|error| format!("{EXPERIMENTS}: {error}"))?;
        record.status = ExperimentStatus::Undecided;
        record.decided_ms = Some(now_ms);
        record.note = Some(note.to_owned());
        history.push(record_json(&record));
        self.write_experiments(None, &bots, &history)?;
        Ok(true)
    }

    fn experiments(&self) -> Result<Ledger, String> {
        let text = match fs::read_to_string(self.dir.join(EXPERIMENTS)) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((None, Map::new(), Vec::new()));
            }
            Err(error) => return Err(format!("{EXPERIMENTS}: {error}")),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{EXPERIMENTS}: {error}"))?;
        let waiting = match &value["waiting"] {
            Value::Null => None,
            held => Some(held.clone()),
        };
        let bots = match &value["bots"] {
            Value::Object(bots) => bots.clone(),
            Value::Null => Map::new(),
            _ => return Err(format!("{EXPERIMENTS}: expected a bots object")),
        };
        let history = match &value["history"] {
            Value::Array(history) => history.clone(),
            Value::Null => Vec::new(),
            _ => return Err(format!("{EXPERIMENTS}: expected a history list")),
        };
        Ok((waiting, bots, history))
    }

    fn write_experiments(
        &self,
        waiting: Option<&Value>,
        bots: &Map<String, Value>,
        history: &[Value],
    ) -> Result<(), String> {
        let text = json!({
            "waiting": waiting.cloned().unwrap_or(Value::Null),
            "bots": Value::Object(bots.clone()),
            "history": history,
        })
        .to_string();
        self.write(EXPERIMENTS, &text)
    }
}

/// The live experiments among `bots`.
fn active(bots: &Map<String, Value>) -> Result<usize, String> {
    let mut live = 0;
    for value in bots.values() {
        if status_of(value).is_ok_and(ExperimentStatus::active) {
            live += 1;
        }
    }
    Ok(live)
}

fn status_of(value: &Value) -> Result<ExperimentStatus, String> {
    value["status"]
        .as_str()
        .and_then(ExperimentStatus::named)
        .ok_or_else(|| "the experiment's status is unknown".to_owned())
}

fn record_json(record: &ExperimentRecord) -> Value {
    json!({
        "experiment": experiment_json(&record.experiment),
        "bot": record.bot,
        "status": record.status.name(),
        "probe": record.probe.map(|probe| format!("{probe:016x}")),
        "evidence": record.evidence.as_ref().map(|evidence| json!({
            "verdict": evidence.verdict,
            "edge": evidence.edge.to_string(),
            "radius": evidence.radius.to_string(),
            "hands": evidence.hands,
        })),
        "opened_ms": record.opened_ms,
        "decided_ms": record.decided_ms,
        "note": record.note,
    })
}

fn record_from(value: &Value) -> Result<ExperimentRecord, String> {
    Ok(ExperimentRecord {
        experiment: experiment_from(&value["experiment"])?,
        bot: value["bot"].as_str().map(str::to_owned),
        status: status_of(value)?,
        probe: value["probe"]
            .as_str()
            .and_then(|text| u64::from_str_radix(text, 16).ok()),
        evidence: match &value["evidence"] {
            Value::Null => None,
            held => Some(Evidence {
                verdict: held["verdict"]
                    .as_str()
                    .ok_or("the evidence has no verdict")?
                    .to_owned(),
                edge: number(held, "edge")?,
                radius: number(held, "radius")?,
                hands: held["hands"].as_u64().ok_or("the evidence has no hands")?,
            }),
        },
        opened_ms: value["opened_ms"]
            .as_i64()
            .ok_or("the experiment has no open time")?,
        decided_ms: value["decided_ms"].as_i64(),
        note: value["note"].as_str().map(str::to_owned),
    })
}

fn experiment_json(experiment: &Experiment) -> Value {
    let Claim::ChallengerKeepsMargin {
        champion,
        challenger,
        sim,
    } = &experiment.claim;
    let stop = match experiment.stop {
        StopRule::Guard { margin_bb } => json!({
            "kind": "guard",
            "margin_bb": margin_bb.to_string(),
        }),
    };
    json!({
        "claim": {
            "kind": "challenger_keeps_margin",
            "champion": champion.to_string(),
            "challenger": params_json(challenger),
            "sim": {
                "deals": sim.deals,
                "mbb_per_100": sim.mbb_per_100.to_string(),
                "ci95": sim.ci95.to_string(),
            },
        },
        "metric": experiment.metric.name(),
        "budget_hands": experiment.budget.hands,
        "stop": stop,
    })
}

fn experiment_from(value: &Value) -> Result<Experiment, String> {
    if value["claim"]["kind"].as_str() != Some("challenger_keeps_margin") {
        return Err("the experiment's claim is unknown".into());
    }
    if value["metric"].as_str() != Some(Metric::LiveEdge.name()) {
        return Err("the experiment's metric is unknown".into());
    }
    let stop = match value["stop"]["kind"].as_str() {
        Some("guard") => StopRule::Guard {
            margin_bb: number(&value["stop"], "margin_bb")?,
        },
        _ => return Err("the experiment's stop rule is unknown".into()),
    };
    Ok(Experiment {
        claim: Claim::ChallengerKeepsMargin {
            champion: value["claim"]["champion"]
                .as_str()
                .and_then(|text| text.parse().ok())
                .ok_or("the claim's champion is not a digest")?,
            challenger: params_from(&value["claim"]["challenger"])
                .map_err(|error| format!("the claim's challenger: {error}"))?,
            sim: Sim {
                deals: usize::try_from(
                    value["claim"]["sim"]["deals"]
                        .as_u64()
                        .ok_or("the claim's sim has no deals")?,
                )
                .unwrap_or(usize::MAX),
                mbb_per_100: number(&value["claim"]["sim"], "mbb_per_100")?,
                ci95: number(&value["claim"]["sim"], "ci95")?,
            },
        },
        metric: Metric::LiveEdge,
        budget: svanbot_learn::experiment::Budget {
            hands: value["budget_hands"]
                .as_u64()
                .ok_or("the experiment has no budget")?,
        },
        stop,
    })
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("the experiment lacks {key}"))
}
