//! The round-robin's own record: `round-robin.json` in the fleet directory, never `arena.json`.
//!
//! A record is one bounded pass: the settings it played, how many of the plan's rows it played and
//! left, and a matrix row for each bot. The matrix is standing: a row's `bouts` are its evidence so
//! far, the newest [`svanbot_learn::round_robin::KEEP_BOUTS`] of them, with this pass's appended.
//! It holds the bot's archive (its previous champions with the numbers needed to play them again)
//! and the regression rule's verdict. The next pass reads the archive from it, and the dashboard
//! board reads the matrix.
//!
//! The file is written whole. A reader sees the previous pass or this one, never a half-written
//! record, and the file is the round-robin's alone: it is not the release Arena.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use svanbot_learn::archive::Regression;
use svanbot_learn::round_robin::{Reading, Table};
use svanbot_strategy::params::{Digest, Params};

use crate::fleet::{params_from, params_json};
use crate::health::write_atomically;

/// The record's file in the fleet directory.
pub const FILE: &str = "round-robin.json";

/// The archived champions one bot keeps: the archive's own bound.
pub const KEEP: usize = svanbot_learn::archive::KEEP;

/// One archived champion of a bot, with the numbers to play it again.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivedChampion {
    pub params: Params,
}

impl ArchivedChampion {
    pub fn digest(&self) -> Digest {
        self.params.digest()
    }
}

/// One row of a bot's matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct RowRecord {
    pub row: String,
    pub table: Table,
    /// The archived champion this row measured, for an archive row.
    pub archive: Option<Digest>,
    /// The paired bouts in mbb/100; for an archive row, the bot minus its champion.
    pub bouts: Vec<f64>,
    pub mean: f64,
    /// The sequence's half-width, or `None` before two bouts gave it one.
    pub radius: Option<f64>,
}

impl RowRecord {
    /// The row's reading as the round-robin recorded it.
    pub fn reading(&self) -> Reading {
        Reading {
            mean: self.mean,
            radius: self.radius.unwrap_or(f64::INFINITY),
            bouts: self.bouts.len(),
        }
    }
}

/// One bot's line in the matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct BotRecord {
    pub bot: String,
    /// The policy the pass played, as `Policy::name` renders it. A change starts the matrix
    /// afresh, as a champion change does.
    pub policy: String,
    /// The champion the pass played, kept with its numbers for the next pass's archive.
    pub champion: Params,
    /// The bot's previous champions, newest first.
    pub archive: Vec<ArchivedChampion>,
    pub rows: Vec<RowRecord>,
    pub regression: Regression,
}

/// One bounded pass of the round-robin.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// When the pass was written, in Unix milliseconds.
    pub recorded_ms: i64,
    pub seed: u64,
    pub bouts: u64,
    pub iterations: Option<u64>,
    pub budget: usize,
    pub played: usize,
    pub skipped: usize,
    pub bots: Vec<BotRecord>,
}

impl Record {
    /// The record `dir` holds, or `None` before the first pass.
    pub fn load(dir: &Path) -> Result<Option<Self>, String> {
        let path = path(dir);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        Self::from_text(&text)
            .map(Some)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Writes the record whole, beside its final name and then renamed over it.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        write_atomically(&path(dir), self.as_text().as_bytes())
            .map_err(|error| format!("{}: {error}", path(dir).display()))
    }

    /// One bot's line, by name.
    pub fn bot(&self, bot: &str) -> Option<&BotRecord> {
        self.bots.iter().find(|line| line.bot == bot)
    }

    pub fn as_text(&self) -> String {
        self.as_value().to_string()
    }

    pub fn as_value(&self) -> Value {
        json!({
            "recorded_ms": self.recorded_ms,
            "seed": self.seed,
            "bouts": self.bouts,
            "iterations": self.iterations,
            "budget": self.budget,
            "played": self.played,
            "skipped": self.skipped,
            "bots": self.bots.iter().map(bot_value).collect::<Vec<_>>(),
        })
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let bots = value["bots"]
            .as_array()
            .ok_or("the round-robin record lacks bots")?
            .iter()
            .map(bot_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            recorded_ms: value["recorded_ms"]
                .as_i64()
                .ok_or("the round-robin record lacks its recorded_ms")?,
            seed: value["seed"]
                .as_u64()
                .ok_or("the round-robin record lacks its seed")?,
            bouts: value["bouts"]
                .as_u64()
                .ok_or("the round-robin record lacks its bouts")?,
            iterations: match &value["iterations"] {
                Value::Null => None,
                held => Some(
                    held.as_u64()
                        .ok_or("the round-robin record's iterations are not a count")?,
                ),
            },
            budget: value["budget"]
                .as_u64()
                .ok_or("the round-robin record lacks its budget")? as usize,
            played: value["played"]
                .as_u64()
                .ok_or("the round-robin record lacks its played count")?
                as usize,
            skipped: value["skipped"]
                .as_u64()
                .ok_or("the round-robin record lacks its skipped count")?
                as usize,
            bots,
        })
    }
}

/// Where the record lives in fleet directory `dir`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn bot_value(bot: &BotRecord) -> Value {
    json!({
        "bot": bot.bot,
        "policy": bot.policy,
        "champion": params_json(&bot.champion),
        "archive": bot.archive.iter().map(|held| json!({
            "champion": params_json(&held.params),
        })).collect::<Vec<_>>(),
        "rows": bot.rows.iter().map(row_value).collect::<Vec<_>>(),
        "regression": regression_name(bot.regression),
    })
}

fn bot_from(value: &Value) -> Result<BotRecord, String> {
    Ok(BotRecord {
        bot: value["bot"]
            .as_str()
            .ok_or("a round-robin bot has no name")?
            .to_owned(),
        policy: value["policy"]
            .as_str()
            .ok_or("a round-robin bot has no policy")?
            .to_owned(),
        champion: params_from(&value["champion"])?,
        archive: value["archive"]
            .as_array()
            .ok_or("a round-robin bot has no archive")?
            .iter()
            .map(|held| {
                Ok(ArchivedChampion {
                    params: params_from(&held["champion"])?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        rows: value["rows"]
            .as_array()
            .ok_or("a round-robin bot has no rows")?
            .iter()
            .map(row_from)
            .collect::<Result<Vec<_>, String>>()?,
        regression: regression_from(
            value["regression"]
                .as_str()
                .ok_or("a round-robin bot has no regression verdict")?,
        )?,
    })
}

fn row_value(row: &RowRecord) -> Value {
    json!({
        "row": row.row,
        "table": row.table.name(),
        "archive": row.archive.map(|champion| champion.to_string()),
        "bouts": row.bouts,
        "mean": row.mean,
        "radius": row.radius,
    })
}

fn row_from(value: &Value) -> Result<RowRecord, String> {
    Ok(RowRecord {
        row: value["row"]
            .as_str()
            .ok_or("a round-robin row has no name")?
            .to_owned(),
        table: table_from(
            value["table"]
                .as_str()
                .ok_or("a round-robin row has no table")?,
        )?,
        archive: match &value["archive"] {
            Value::Null => None,
            held => Some(
                held.as_str()
                    .ok_or("a round-robin row's archive is not a digest")?
                    .parse()
                    .map_err(|_| "a round-robin row's archive is not a digest".to_owned())?,
            ),
        },
        bouts: value["bouts"]
            .as_array()
            .ok_or("a round-robin row has no bouts")?
            .iter()
            .map(|bout| bout.as_f64().ok_or("a round-robin bout is not a number"))
            .collect::<Result<_, _>>()?,
        mean: value["mean"]
            .as_f64()
            .ok_or("a round-robin row has no mean")?,
        radius: match &value["radius"] {
            Value::Null => None,
            held => Some(
                held.as_f64()
                    .ok_or("a round-robin row's radius is not a number")?,
            ),
        },
    })
}

fn table_from(name: &str) -> Result<Table, String> {
    [Table::HeadsUp, Table::SixMax]
        .into_iter()
        .find(|table| table.name() == name)
        .ok_or_else(|| format!("no round-robin table is called {name}"))
}

/// The regression verdict as the record writes it.
pub fn regression_name(verdict: Regression) -> String {
    match verdict {
        Regression::Admitted => "admitted".into(),
        Regression::Unresolved => "unresolved".into(),
        Regression::Refused { champion } => format!("refused {champion}"),
    }
}

fn regression_from(name: &str) -> Result<Regression, String> {
    match name {
        "admitted" => Ok(Regression::Admitted),
        "unresolved" => Ok(Regression::Unresolved),
        _ => {
            let champion = name
                .strip_prefix("refused ")
                .ok_or_else(|| format!("no round-robin verdict is called {name}"))?;
            Ok(Regression::Refused {
                champion: champion
                    .parse()
                    .map_err(|_| format!("a refused champion is not a digest: {champion}"))?,
            })
        }
    }
}
