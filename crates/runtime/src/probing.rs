//! A bot's ledger kept in its store, and, on the canary, the probe from proposal to promotion
//! (`GLOSSARY.md`):
//!
//! - Every bot keeps what each counted hand won ([`Store::append_played`]).
//! - The **canary**, with no probe running, takes up the fleet's waiting proposal between hands.
//!   It stores the start, plays the arms ([`Ledger`]), and acts on the verdict. A harm guard
//!   that clears promotes its challenger: the canary writes the fleet's champion. Any other
//!   ending leaves the champion as it was, which is the rollback.
//! - Every other bot adopts the fleet's champion between hands, and so does the canary while
//!   no probe runs.
//!
//! A restarted canary takes its probe up from the store: the start's detail and the orbits its
//! played hands add up to.
//!
//! Nothing here stops play: a failure of the store or of a fleet file is a note to the operator.

use serde_json::{Value, json};
use svanbot_learn::probe::{Decided, Verdict};
use svanbot_strategy::params::Params;

use crate::fleet::{Fleet, Proposal, params_from, params_json};
use crate::ledger::{Ledger, Played};
use crate::record::Hand;
use crate::store::Store;

/// What a finished hand came to: the hand as the ledger counted it, if it could be, and notes
/// for the operator.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    pub played: Option<Played>,
    pub notes: Vec<String>,
}

/// One bot's ledger, its store and the fleet's files.
#[derive(Debug)]
pub struct Probing<'a> {
    ledger: Ledger,
    store: &'a Store,
    fleet: Fleet,
    canary: bool,
    /// The running probe's id and proposal.
    running: Option<(u64, Proposal)>,
}

impl<'a> Probing<'a> {
    /// Opens the bot's ledger on the fleet's champion at `now_ms`, with notes for the operator.
    /// The canary also takes up the probe its store left open.
    pub fn open(store: &'a Store, fleet: Fleet, canary: bool, now_ms: i64) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let champion = fleet.champion().unwrap_or_else(|why| {
            notes.push(format!(
                "the champion file is refused ({why}): playing the default"
            ));
            Params::DEFAULT
        });
        let mut probing = Self {
            ledger: Ledger::new(champion),
            store,
            fleet,
            canary,
            running: None,
        };
        if canary {
            match probing.resume(now_ms) {
                Ok(Some(note)) => notes.push(note),
                Ok(None) => {}
                Err(why) => notes.push(format!("the open probe is not taken up: {why}")),
            }
        }
        (probing, notes)
    }

    /// The parameter set the hand in play is played by.
    pub fn params(&self) -> Params {
        *self.ledger.params()
    }

    /// The champion's parameter set.
    pub fn champion(&self) -> Params {
        *self.ledger.champion()
    }

    /// Notes one draw of the re-solve in the hand in play.
    pub fn decided(&mut self, drawn: Decided) {
        self.ledger.decided(drawn);
    }

    /// The running probe in one line, for the status.
    pub fn describe(&self) -> Option<String> {
        let (_, proposal) = self.running?;
        let (edge, radius) = self.ledger.estimate()?;
        Some(format!(
            "{} against {}: {edge:+.1} bb/100, 95% ±{radius:.1}",
            proposal.challenger.digest(),
            self.ledger.champion().digest()
        ))
    }

    /// Takes a finished hand at `now_ms`: keeps what it won, then does what falls between hands.
    pub fn on_hand(&mut self, hand: &Hand, now_ms: i64) -> Kept {
        let mut notes = Vec::new();
        let played = self.ledger.on_hand(hand);
        if let Some(played) = &played
            && let Err(error) = self.store.append_played(played, now_ms)
        {
            notes.push(format!("NOT KEPT: {error}"));
        }
        let done = if self.canary {
            self.between_hands(now_ms)
        } else {
            self.adopt()
        };
        match done {
            Ok(note) => notes.extend(note),
            Err(why) => notes.push(why),
        }
        Kept { played, notes }
    }

    /// Adopts the fleet's champion, when it is not the one this bot plays by already.
    fn adopt(&mut self) -> Result<Option<String>, String> {
        let champion = self.fleet.champion()?;
        if champion == *self.ledger.champion() {
            return Ok(None);
        }
        self.ledger.adopt(champion);
        Ok(Some(format!("adopted champion {}", champion.digest())))
    }

    /// The canary ends a probe that has its verdict, or takes up a waiting proposal.
    fn between_hands(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let store = |error| format!("probe: {error}");
        match (self.ledger.verdict(), self.running) {
            (Some(Verdict::Running), _) => Ok(None),
            (Some(verdict), Some((id, proposal))) => {
                let (edge, radius) = self.ledger.estimate().unwrap_or((0.0, f64::INFINITY));
                let name = verdict_name(verdict);
                let detail = json!({ "edge": edge.to_string(), "radius": radius.to_string() });
                self.store
                    .append_probe(id, name, &detail.to_string(), now_ms)
                    .map_err(store)?;
                let promote = verdict == Verdict::Cleared;
                if promote {
                    self.fleet.promote(&proposal.challenger)?;
                }
                self.ledger.finish(promote);
                self.running = None;
                Ok(Some(format!(
                    "probe {id:016x} of {} ended {name}: {edge:+.1} bb/100, 95% ±{radius:.1}{}",
                    proposal.challenger.digest(),
                    if promote { ", promoted" } else { "" },
                )))
            }
            _ => {
                // With no probe running the canary follows the champion file too, so a
                // promotion the operator takes back by hand reaches it.
                if let Some(note) = self.adopt()? {
                    return Ok(Some(note));
                }
                let Some(proposal) = self.fleet.take_proposal()? else {
                    return Ok(None);
                };
                let champion = *self.ledger.champion();
                if proposal.challenger == champion {
                    return Ok(Some("the proposal is the champion: dropped".into()));
                }
                let id = proposal.hypothesis(&champion).id();
                let detail = json!({
                    "champion": params_json(&champion),
                    "proposal": serde_json::from_str::<Value>(&proposal.to_json())
                        .unwrap_or(Value::Null),
                });
                self.store
                    .append_probe(id, "started", &detail.to_string(), now_ms)
                    .map_err(store)?;
                self.ledger
                    .start(proposal.hypothesis(&champion), proposal.challenger, 0);
                self.running = Some((id, proposal));
                Ok(Some(format!(
                    "probe {id:016x} started: {} against champion {}",
                    proposal.challenger.digest(),
                    champion.digest()
                )))
            }
        }
    }

    /// Takes up the probe the store left open, if it is against today's champion. One against
    /// an older champion is ended as superseded.
    fn resume(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let store = |error| format!("probe: {error}");
        let Some((id, detail)) = self.store.open_probe().map_err(store)? else {
            return Ok(None);
        };
        let detail: Value = serde_json::from_str(&detail).map_err(|e| e.to_string())?;
        let proposal = Proposal::from_json(&detail["proposal"].to_string())?;
        let champion = *self.ledger.champion();
        if params_from(&detail["champion"])? != champion {
            self.store
                .append_probe(id, "superseded", "{}", now_ms)
                .map_err(store)?;
            return Ok(Some(format!(
                "probe {id:016x} superseded: the champion changed"
            )));
        }
        let orbits = self.store.orbits(id).map_err(store)?;
        let next = orbits.last().map_or(0, |&(number, ..)| number + 1);
        self.ledger
            .start(proposal.hypothesis(&champion), proposal.challenger, next);
        for &(_, arm, won, hands) in &orbits {
            self.ledger.replay(arm, won, hands);
        }
        self.running = Some((id, proposal));
        Ok(Some(format!(
            "probe {id:016x} taken up after {} orbits",
            orbits.len()
        )))
    }
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Running => "running",
        Verdict::Supported => "supported",
        Verdict::Harmful => "harmful",
        Verdict::SimGap => "sim-gap",
        Verdict::Inconclusive => "inconclusive",
        Verdict::Cleared => "cleared",
    }
}
