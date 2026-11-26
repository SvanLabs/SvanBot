//! The frozen reference (V11-78): the fleet champion at Phase 2's cutover, held in
//! `reference.json` with its digest and cutover time.
//!
//! - [`Fleet::freeze`] records the champion in play, once. From then [`Fleet::champion`] reads
//!   this file, and the fleet-wide search, proposals and promotions are stopped.
//! - [`Fleet::rebase`] is the operator's deliberate move to a new parameter set: it keeps the
//!   old reference in the file's history, with its own cutover time.
//! - A bot's own slot lives in `champions.json` and never touches this file
//!   (`docs/adr/0006-the-reference-is-frozen.md`).

use std::fs;
use std::io::ErrorKind;

use serde_json::{Value, json};
use svanbot_strategy::params::{Digest, Params};

use super::{Fleet, params_from, params_json};

const REFERENCE: &str = "reference.json";

/// The fleet's reference: the parameter set every bot without a slot plays by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reference {
    pub params: Params,
    /// When the reference was frozen, or last rebased, in ms since the epoch.
    pub cutover_ms: i64,
}

impl Reference {
    /// The content hash that names the parameter set.
    pub fn digest(&self) -> Digest {
        self.params.digest()
    }
}

/// A reference as the file writes it.
fn point_json(reference: &Reference) -> Value {
    let mut value = params_json(&reference.params);
    value["cutover_ms"] = json!(reference.cutover_ms);
    value
}

/// A reference read back, refused when its numbers do not hash to its digest.
fn point_from(value: &Value) -> Result<Reference, String> {
    Ok(Reference {
        params: params_from(value)?,
        cutover_ms: value["cutover_ms"]
            .as_i64()
            .ok_or("the reference has no cutover time")?,
    })
}

impl Fleet {
    /// The frozen reference, or `None` before the freeze.
    pub fn reference(&self) -> Result<Option<Reference>, String> {
        Ok(self.reference_document()?.map(|(reference, _)| reference))
    }

    /// The references this one replaced, newest first.
    pub fn rebases(&self) -> Result<Vec<Reference>, String> {
        Ok(self
            .reference_document()?
            .map_or_else(Vec::new, |(_, rebases)| rebases))
    }

    /// Freezes the fleet champion in play at `cutover_ms`. One freeze only: a reference already
    /// frozen is moved by [`Fleet::rebase`], on purpose.
    pub fn freeze(&self, cutover_ms: i64) -> Result<Reference, String> {
        let _lock = self.lock_fleet_file(REFERENCE)?;
        if let Some(held) = self.reference()? {
            return Err(format!(
                "{REFERENCE}: the reference is already frozen at {} under {}",
                held.cutover_ms,
                held.digest()
            ));
        }
        let reference = Reference {
            params: self.champion()?,
            cutover_ms,
        };
        self.write_reference(&reference, &[])?;
        // A fleet-wide proposal waiting at cutover is not taken up.
        match fs::remove_file(self.dir.join("proposal.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("fleet: {error}")),
        }
        Ok(reference)
    }

    /// Moves the frozen reference to `params` at `cutover_ms`, keeping the reference it replaces
    /// in the history. A rebase before the freeze is refused.
    pub fn rebase(&self, params: &Params, cutover_ms: i64) -> Result<Reference, String> {
        let _lock = self.lock_fleet_file(REFERENCE)?;
        let Some((held, mut rebases)) = self.reference_document()? else {
            return Err(format!("{REFERENCE}: the reference is not frozen"));
        };
        rebases.insert(0, held);
        let reference = Reference {
            params: *params,
            cutover_ms,
        };
        self.write_reference(&reference, &rebases)?;
        Ok(reference)
    }

    /// Refuses a fleet-wide write once the reference is frozen. The per-bot slots do not come
    /// through here: only the operator's rebase moves the reference.
    pub(super) fn refuse_when_frozen(&self, what: &str) -> Result<(), String> {
        if self.reference()?.is_some() {
            return Err(format!("{REFERENCE}: the reference is frozen: {what}"));
        }
        Ok(())
    }

    fn write_reference(&self, reference: &Reference, rebases: &[Reference]) -> Result<(), String> {
        let rebases: Vec<Value> = rebases.iter().map(point_json).collect();
        self.write(
            REFERENCE,
            &json!({ "reference": point_json(reference), "rebases": rebases }).to_string(),
        )
    }

    /// The reference and its history, or `None` before the freeze.
    fn reference_document(&self) -> Result<Option<(Reference, Vec<Reference>)>, String> {
        let text = match fs::read_to_string(self.dir.join(REFERENCE)) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{REFERENCE}: {error}")),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{REFERENCE}: {error}"))?;
        let reference =
            point_from(&value["reference"]).map_err(|error| format!("{REFERENCE}: {error}"))?;
        let rebases = value["rebases"]
            .as_array()
            .ok_or_else(|| format!("{REFERENCE}: expected a rebases list"))?
            .iter()
            .map(point_from)
            .collect::<Result<_, _>>()
            .map_err(|error| format!("{REFERENCE}: {error}"))?;
        Ok(Some((reference, rebases)))
    }
}
