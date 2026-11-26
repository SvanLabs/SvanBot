//! The fleet's requested policy for each bot.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;

use serde_json::{Map, Value, json};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::style::{self, Style};

use super::{ArenaRun, Fleet};

const POLICY_RUNS: &str = "policy-runs.json";
const PLAYS: &str = "plays.json";

#[derive(Clone, Copy, Debug)]
pub struct Assignment {
    pub kind: PolicyKind,
    pub style: &'static Style,
}

impl PartialEq for Assignment {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.style.name == other.style.name
    }
}

fn parse(bot: &str, entry: &Value) -> Result<(Assignment, i64), String> {
    let field = |name| {
        entry[name]
            .as_str()
            .ok_or_else(|| format!("{PLAYS}: {bot} has no {name}"))
    };
    let kind = field("kind")?;
    let style = field("style")?;
    let since = entry["since_ms"]
        .as_i64()
        .ok_or_else(|| format!("{PLAYS}: {bot} has no valid since_ms"))?;
    let kind = PolicyKind::named(kind)
        .ok_or_else(|| format!("{PLAYS}: {bot} names unknown policy kind {kind:?}"))?;
    let style = style::named(style)
        .ok_or_else(|| format!("{PLAYS}: {bot} names unknown style {style:?}"))?;
    Ok((Assignment { kind, style }, since))
}

impl Fleet {
    /// The policy Arena's history, separate from the runs the ship bar reads.
    pub fn policy_arena_runs(&self) -> Result<Vec<ArenaRun>, String> {
        self.runs_in(POLICY_RUNS)
    }

    /// Record a policy Arena run without updating the release Arena.
    pub fn record_policy_arena(&self, run: &ArenaRun) -> Result<(), String> {
        self.record_in(POLICY_RUNS, run)
    }

    pub fn assignment(&self, bot: &str) -> Result<Option<Assignment>, String> {
        self.plays()?
            .get(bot)
            .map(|entry| parse(bot, entry).map(|(assignment, _)| assignment))
            .transpose()
    }

    pub fn assignments(&self) -> Result<Vec<(String, Assignment, i64)>, String> {
        let mut rows = self
            .plays()?
            .into_iter()
            .map(|(bot, entry)| {
                parse(&bot, &entry).map(|(assignment, since)| (bot, assignment, since))
            })
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        Ok(rows)
    }

    pub fn assign(&self, bot: &str, assignment: Assignment, since_ms: i64) -> Result<(), String> {
        let _lock = self.lock_fleet_file(PLAYS)?;
        let mut bots = self.plays()?;
        bots.insert(
            bot.into(),
            json!({
                "kind": assignment.kind.name(),
                "style": assignment.style.name,
                "since_ms": since_ms,
            }),
        );
        self.write(PLAYS, &json!({ "bots": bots }).to_string())
    }

    pub fn revert(&self, bot: &str) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(PLAYS)?;
        let mut bots = self.plays()?;
        self.remove_play(bot, &mut bots)
    }

    /// Revert only the assignment whose own hands the guard measured.
    pub fn revert_if(
        &self,
        bot: &str,
        assignment: Assignment,
        since_ms: i64,
    ) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(PLAYS)?;
        let mut bots = self.plays()?;
        let Some(entry) = bots.get(bot) else {
            return Ok(false);
        };
        if parse(bot, entry)? != (assignment, since_ms) {
            return Ok(false);
        }
        self.remove_play(bot, &mut bots)
    }

    fn remove_play(&self, bot: &str, bots: &mut Map<String, Value>) -> Result<bool, String> {
        if bots.remove(bot).is_none() {
            return Ok(false);
        }
        if bots.is_empty() {
            match fs::remove_file(self.dir.join(PLAYS)) {
                Ok(()) => (),
                Err(error) if error.kind() == ErrorKind::NotFound => (),
                Err(error) => return Err(format!("{PLAYS}: {error}")),
            }
        } else {
            self.write(PLAYS, &json!({ "bots": bots }).to_string())?;
        }
        Ok(true)
    }

    fn plays(&self) -> Result<Map<String, Value>, String> {
        self.bots_in(PLAYS)
    }

    // Writers share a separate lock so replacing or removing plays.json cannot move the lock.
    pub(super) fn lock_fleet_file(&self, file: &str) -> Result<File, String> {
        let error = |error| format!("{file}: {error}");
        fs::create_dir_all(&self.dir).map_err(error)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(file).with_extension("lock"))
            .map_err(error)?;
        lock.lock().map_err(error)?;
        Ok(lock)
    }
}
