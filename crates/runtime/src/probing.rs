//! A bot's ledger kept in its store, and, on the canary, the probe from proposal to promotion
//! (`GLOSSARY.md`):
//!
//! - Every bot keeps what each counted hand won ([`Store::append_played`]), and plays the
//!   **standing reference arm** on one orbit in ten: its own live control, measured against the
//!   reference digest frozen when it opened. A running probe owns its hands, so the standing arm
//!   waits.
//! - The **canary**, with no probe running, takes up the fleet's waiting proposal between hands.
//!   It stores the start, plays the arms ([`Ledger`]), and acts on the verdict. A harm guard
//!   that clears promotes its challenger: the canary writes the fleet's champion. Any other
//!   ending leaves the champion as it was, which is the rollback.
//! - Every other bot adopts its own champion slot between hands, or the fleet's champion while
//!   it has no slot; so does the canary while no probe runs.
//!
//! A restarted canary takes its probe up from the store: the start's detail and the orbits its
//! played hands add up to. Once the fleet's reference is frozen (Phase 2's cutover), the
//! fleet-wide probe is retired instead: one in flight ends without promoting, and one the store
//! left open is not taken up again (`docs/adr/0006-the-reference-is-frozen.md`).
//!
//! Nothing here stops play: a failure of the store or of a fleet file is a note to the operator.

use serde_json::{Value, json};
use svanbot_learn::calibration::Point;
use svanbot_learn::probe::{Decided, Verdict};
use svanbot_learn::reference::{self, Estimate, Side, Standing};
use svanbot_strategy::params::Params;

use crate::fleet::{Fleet, Proposal, params_from, params_json};
use crate::ledger::{Ledger, Played};
use crate::record::Hand;
use crate::store::Store;

mod promotion;

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
    bot: String,
    /// The reference policy every standing orbit is measured against: the fleet's champion, the
    /// frozen reference once it is frozen.
    reference: Params,
    canary: bool,
    /// Whether a per-bot promotion may start its live guard. Off by default: the rollout switch
    /// is the operator's, and the evidence sources must be present first.
    live_promotions: bool,
    /// The running probe's id and proposal.
    running: Option<(u64, Proposal)>,
}

impl<'a> Probing<'a> {
    /// Opens the bot's ledger on the champion its slot names, or the fleet champion while it has
    /// no slot, at `now_ms`, with notes for the operator. The canary also takes up the probe its
    /// store left open. Per-bot promotions stay in their waiting state: this is the disabled
    /// production path.
    pub fn open(store: &'a Store, fleet: Fleet, canary: bool, now_ms: i64) -> (Self, Vec<String>) {
        Self::open_with_live(store, fleet, canary, now_ms, false)
    }

    /// [`Probing::open`], with the per-bot promotion guard allowed or held back. An explicit
    /// seam so the live path is off unless the operator or a test turns it on.
    pub fn open_with_live(
        store: &'a Store,
        fleet: Fleet,
        canary: bool,
        now_ms: i64,
        live_promotions: bool,
    ) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let bot = store.bot().unwrap_or_else(|why| {
            notes.push(format!(
                "the store does not name its bot ({why}): playing the fleet champion"
            ));
            String::new()
        });
        // The reference is the fleet's champion — the frozen reference after the cutover — not
        // this bot's slot: a slot moves with its own search, while every bot's control is held
        // to the same reference.
        let fleet_champion = fleet.champion();
        let reference = match &fleet_champion {
            Ok(champion) => *champion,
            Err(why) => {
                notes.push(format!(
                    "the champion file is refused ({why}): playing the default"
                ));
                Params::DEFAULT
            }
        };
        let champion = match fleet.champion_slot(&bot) {
            Ok(Some(slot)) => slot,
            Ok(None) => reference,
            Err(why) => {
                // A slot that cannot be read is not one to play by. The fleet champion still is,
                // and the default is the last resort, as it is for a refused champion.json.
                notes.push(format!(
                    "the champion slot is refused ({why}): playing the fleet champion"
                ));
                reference
            }
        };
        let next = match store.last_reference_orbit() {
            Ok(last) => last.map_or(0, |orbit| orbit + 1),
            Err(why) => {
                notes.push(format!(
                    "the standing arm's last orbit is not read ({why}): starting at 0"
                ));
                0
            }
        };
        let mut ledger = Ledger::new(champion);
        ledger.stand(reference, reference::bot_seed(&bot), next);
        let mut probing = Self {
            ledger,
            store,
            fleet,
            bot,
            reference,
            canary,
            live_promotions,
            running: None,
        };
        if canary {
            match probing.fleet.reference() {
                // A frozen reference retires the fleet-wide probe: it is not taken up again.
                Ok(Some(_)) => match probing.abandon(now_ms) {
                    Ok(Some(note)) => notes.push(note),
                    Ok(None) => {}
                    Err(why) => notes.push(format!("the open probe is not taken up: {why}")),
                },
                Ok(None) => match probing.resume(now_ms) {
                    Ok(Some(note)) => notes.push(note),
                    Ok(None) => {}
                    Err(why) => notes.push(format!("the open probe is not taken up: {why}")),
                },
                Err(why) => notes.push(format!("the frozen reference is refused: {why}")),
            }
        }
        match probing.open_promotion(now_ms) {
            Ok(Some(note)) => notes.push(note),
            Ok(None) => {}
            Err(why) => notes.push(format!("the open promotion is not taken up: {why}")),
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

    /// Whether the hand in play is the standing reference arm's reference side.
    pub fn on_reference(&self) -> bool {
        self.ledger.on_reference()
    }

    /// The standing orbit in play: its number, the side it plays and the chance it was drawn.
    pub fn standing(&self) -> Option<(u64, Side, f64)> {
        self.ledger.standing()
    }

    /// The harm guard read from this bot's own standing orbits kept at or after `since_ms`: the
    /// policy's edge over the reference, weighted by the chance each arm was drawn, with the
    /// verdict as of the last finished orbit.
    pub fn standing_guard(
        &self,
        since_ms: i64,
        hands: u64,
        harm_margin: f64,
    ) -> Result<(Verdict, f64, f64), String> {
        let reference = self.reference.digest();
        let policy = self.ledger.champion().digest();
        let orbits = self
            .store
            .reference_orbits(&reference, since_ms)
            .map_err(|error| error.to_string())?;
        let mut estimate = Estimate::guard(reference, policy, hands, harm_margin);
        for orbit in orbits {
            estimate.observe(
                &Standing {
                    orbit: orbit.orbit,
                    side: orbit.side,
                    chance: orbit.chance,
                    reference,
                },
                orbit.won,
                orbit.hands,
            );
        }
        let (edge, radius) = estimate.estimate();
        Ok((estimate.verdict(), edge, radius))
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
        let done = match self.promotion_step(now_ms) {
            Ok(Some(note)) => Ok(Some(note)),
            Ok(None) if self.canary => self.between_hands(now_ms),
            Ok(None) => self.adopt(),
            Err(why) => Err(why),
        };
        match done {
            Ok(note) => notes.extend(note),
            Err(why) => notes.push(why),
        }
        Kept { played, notes }
    }

    /// Adopts this bot's champion, when it is not the one this bot plays by already: its own
    /// slot's, or the fleet's while it has no slot.
    fn adopt(&mut self) -> Result<Option<String>, String> {
        let champion = self.fleet.champion_for(&self.bot)?;
        if champion == *self.ledger.champion() {
            return Ok(None);
        }
        self.ledger.adopt(champion);
        Ok(Some(format!("adopted champion {}", champion.digest())))
    }

    /// The canary ends a probe that has its verdict, or takes up a waiting proposal.
    fn between_hands(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        if self.fleet.reference()?.is_some() {
            return self.after_freeze(now_ms);
        }
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
                self.fleet.end_probe()?;
                let mut note = format!(
                    "probe {id:016x} of {} ended {name}: {edge:+.1} bb/100, 95% ±{radius:.1}{}",
                    proposal.challenger.digest(),
                    if promote { ", promoted" } else { "" },
                );
                if let Some(sim) = proposal.sim_estimate {
                    let point = Point {
                        sim,
                        live: edge,
                        radius,
                    };
                    if point.is_valid()
                        && let Err(error) = self.fleet.record_calibration(id, &point, name, now_ms)
                    {
                        note.push_str(&format!(". Calibration was not recorded: {error}"));
                    }
                }
                Ok(Some(note))
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
                    self.fleet.end_probe()?;
                    return Ok(Some("the proposal is the champion: dropped".into()));
                }
                if proposal
                    .against
                    .is_some_and(|against| against != champion.digest())
                {
                    // Its sims measured it against a champion since replaced: the evidence that
                    // put it forward no longer speaks to the champion it would face.
                    self.fleet.end_probe()?;
                    return Ok(Some(format!(
                        "the proposal of {} was measured against another champion: dropped",
                        proposal.challenger.digest()
                    )));
                }
                let id = proposal.hypothesis(&champion).id();
                let detail = json!({
                    "champion": params_json(&champion),
                    "proposal": serde_json::from_str::<Value>(&proposal.to_json())
                        .unwrap_or(Value::Null),
                });
                if let Err(error) =
                    self.store
                        .append_probe(id, "started", &detail.to_string(), now_ms)
                {
                    // Not started: the mirror taking it up wrote must not hold the learner back.
                    let _ = self.fleet.end_probe();
                    return Err(store(error));
                }
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

    /// A frozen reference stops the fleet-wide probe: one in flight ends without promoting, and
    /// the canary then plays its own slot, or the frozen reference.
    fn after_freeze(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let note = match self.running.take() {
            Some((id, proposal)) => {
                let (edge, radius) = self.ledger.estimate().unwrap_or((0.0, f64::INFINITY));
                let detail = json!({
                    "edge": edge.to_string(),
                    "radius": radius.to_string(),
                    "reason": "the reference is frozen",
                });
                self.store
                    .append_probe(id, "frozen", &detail.to_string(), now_ms)
                    .map_err(|error| format!("probe: {error}"))?;
                self.ledger.finish(false);
                self.fleet.end_probe()?;
                Some(format!(
                    "probe {id:016x} of {} ended: the reference is frozen, so it was not promoted",
                    proposal.challenger.digest()
                ))
            }
            None => None,
        };
        let adopted = self.adopt()?;
        Ok(note.or(adopted))
    }

    /// Closes a probe the store left open when the reference froze: no arm is replayed, and no
    /// promotion may come of it.
    fn abandon(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let Some((id, _)) = self
            .store
            .open_probe()
            .map_err(|error| format!("probe: {error}"))?
        else {
            self.fleet.end_probe()?;
            return Ok(None);
        };
        self.store
            .append_probe(id, "frozen", "{}", now_ms)
            .map_err(|error| format!("probe: {error}"))?;
        self.fleet.end_probe()?;
        Ok(Some(format!(
            "probe {id:016x} ended: the reference is frozen, so it was not taken up"
        )))
    }

    /// Takes up the probe the store left open, if it is against today's champion. One against
    /// an older champion is ended as superseded.
    fn resume(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let store = |error| format!("probe: {error}");
        let Some((id, detail)) = self.store.open_probe().map_err(store)? else {
            // The store is the record: with no probe open, a mirror is one left behind, and the
            // learner would wait on a probe nobody runs.
            self.fleet.end_probe()?;
            return Ok(None);
        };
        let detail: Value = serde_json::from_str(&detail).map_err(|e| e.to_string())?;
        let proposal = Proposal::from_json(&detail["proposal"].to_string())?;
        let champion = *self.ledger.champion();
        if params_from(&detail["champion"])? != champion {
            self.store
                .append_probe(id, "superseded", "{}", now_ms)
                .map_err(store)?;
            self.fleet.end_probe()?;
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
        self.fleet.mirror_probe(&proposal)?;
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
