//! Evidence recorded by a paired policy margin check.

use std::fs;
use std::io::ErrorKind;

use serde_json::{Value, json};
use svanbot_strategy::{kind::PolicyKind, params::Digest};

use super::Fleet;

const CHECKS: &str = "checks.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub kind: PolicyKind,
    pub style: String,
    pub reference: Digest,
    pub seed: u64,
    pub iterations: Option<u64>,
    pub bouts: u64,
    pub admitted: bool,
    pub heads_up: String,
    pub six_max: String,
    pub checked_ms: i64,
}

impl Check {
    fn from_json(value: &Value) -> Result<Self, String> {
        let missing = |key: &str| format!("{CHECKS}: invalid {key}");
        let text = |key: &str| value[key].as_str().ok_or_else(|| missing(key));
        let number = |key: &str| value[key].as_u64().ok_or_else(|| missing(key));
        let kind = text("kind")?;
        Ok(Self {
            kind: PolicyKind::named(kind)
                .ok_or_else(|| format!("{CHECKS}: unknown kind {kind:?}"))?,
            style: text("style")?.into(),
            reference: text("reference")?
                .parse()
                .map_err(|_| missing("reference"))?,
            seed: number("seed")?,
            iterations: match value.get("iterations") {
                None | Some(Value::Null) => None,
                Some(_) => Some(number("iterations")?),
            },
            bouts: number("bouts")?,
            admitted: value["admitted"]
                .as_bool()
                .ok_or_else(|| missing("admitted"))?,
            heads_up: text("heads_up")?.into(),
            six_max: text("six_max")?.into(),
            checked_ms: value["checked_ms"]
                .as_i64()
                .ok_or_else(|| missing("checked_ms"))?,
        })
    }

    fn as_json(&self) -> Value {
        json!({
            "kind": self.kind.name(),
            "style": self.style,
            "reference": self.reference.to_string(),
            "seed": self.seed,
            "iterations": self.iterations,
            "bouts": self.bouts,
            "admitted": self.admitted,
            "heads_up": self.heads_up,
            "six_max": self.six_max,
            "checked_ms": self.checked_ms,
        })
    }
}

impl Fleet {
    /// The latest check per kind and style, newest first.
    pub fn checks(&self) -> Result<Vec<Check>, String> {
        let text = match fs::read_to_string(self.dir.join(CHECKS)) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("{CHECKS}: {error}")),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{CHECKS}: {error}"))?;
        value["checks"]
            .as_array()
            .ok_or_else(|| format!("{CHECKS}: expected a checks list"))?
            .iter()
            .map(Check::from_json)
            .collect()
    }

    pub fn record_check(&self, check: &Check) -> Result<(), String> {
        let _lock = self.lock_fleet_file(CHECKS)?;
        let mut checks = self.checks()?;
        let same = |held: &Check| held.kind == check.kind && held.style == check.style;
        if checks
            .iter()
            .any(|held| same(held) && held.checked_ms > check.checked_ms)
        {
            return Ok(());
        }
        checks.retain(|held| !same(held));
        checks.insert(0, check.clone());
        checks.sort_by_key(|check| std::cmp::Reverse(check.checked_ms));
        let checks: Vec<_> = checks.iter().map(Check::as_json).collect();
        self.write(CHECKS, &json!({ "checks": checks }).to_string())
    }
}
