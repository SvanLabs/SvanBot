//! The ledger: what each of a bot's hands won, and, on the canary, the probe that runs on them
//! (`GLOSSARY.md`; the arithmetic is `learn::probe`).
//!
//! - A hand **wins** what our stack gained since the hand before it at the same table. The first
//!   hand at a table has no hand before it, so it is not counted, and nor is a hand whose result
//!   never came. Leaving a hand out does not depend on how it went, so it biases nothing.
//! - The **luck** of our own draws is set beside the result: the value of each action the
//!   re-solve drew, less the mean value of the strategy it drew from.
//! - A **probe** plays one arm for an orbit of [`ORBIT`] counted hands, then draws the next
//!   orbit's arm by a coin fixed by the probe and the orbit's number. A change of table ends the
//!   orbit early. The ledger says which parameter set the hand in play is played by.
//!
//! The ledger does no I/O. Its caller stores each [`Played`] hand, starts a probe from a
//! proposal, and acts on the verdict.

use svanbot_core::rules::Chips;
use svanbot_learn::probe::{self, Arm, Decided, Hypothesis, Probe, Verdict};
use svanbot_strategy::params::{Digest, Params};

use crate::record::Hand;
use crate::wire::{Inbound, parse};

/// The counted hands of one orbit: once round a six-seat table, so the positions balance.
pub const ORBIT: u32 = 6;

/// One hand of ours, as counted.
#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub hand_id: String,
    pub table_id: String,
    /// The parameter set the hand was played by.
    pub digest: Digest,
    /// The chips the hand won, negative for a loss.
    pub net: i64,
    /// The chips of `net` that were the luck of our own draws.
    pub luck: f64,
    pub big_blind: Chips,
    /// The probe the hand counted in, the orbit's number in it, and the arm it played.
    pub orbit: Option<(u64, u64, Arm)>,
}

/// A bot's ledger.
#[derive(Clone, Debug)]
pub struct Ledger {
    champion: Params,
    running: Option<Running>,
    table: String,
    /// Our stack after the last hand at this table, when that hand's result came.
    stack: Option<Chips>,
    /// The re-solve's draws in the hand in play.
    decided: Vec<Decided>,
}

/// A probe in progress.
#[derive(Clone, Debug)]
struct Running {
    id: u64,
    challenger: Params,
    probe: Probe,
    /// The number and the arm of the orbit in play.
    orbit: u64,
    arm: Arm,
    /// The big blinds won in the orbit so far, with the luck taken out, and its hands.
    won: f64,
    hands: u32,
}

impl Running {
    /// Ends the orbit in play, hands it to the probe, and draws the next orbit's arm.
    fn close(&mut self) {
        self.probe
            .observe(self.arm, probe::CHALLENGER, self.won, self.hands);
        // An orbit cut before its first counted hand keeps its number and its coin.
        if self.hands > 0 {
            self.orbit += 1;
            self.arm = probe::arm(self.id, 0, self.orbit).0;
        }
        (self.won, self.hands) = (0.0, 0);
    }
}

impl Ledger {
    /// A ledger for a bot that plays by `champion`, with no probe running.
    pub fn new(champion: Params) -> Self {
        Self {
            champion,
            running: None,
            table: String::new(),
            stack: None,
            decided: Vec::new(),
        }
    }

    /// Starts a probe of `challenger`, whose next orbit is number `orbit`: 0 for a new probe,
    /// and the count of stored orbits for one taken up after a restart ([`Ledger::replay`]).
    pub fn start(&mut self, hypothesis: Hypothesis, challenger: Params, orbit: u64) {
        let id = hypothesis.id();
        self.running = Some(Running {
            id,
            challenger,
            probe: Probe::new(hypothesis),
            orbit,
            arm: probe::arm(id, 0, orbit).0,
            won: 0.0,
            hands: 0,
        });
    }

    /// Hands the running probe an orbit played before a restart: its arm, the big blinds it won
    /// with the luck taken out, and its hands.
    pub fn replay(&mut self, arm: Arm, won: f64, hands: u32) {
        if let Some(running) = &mut self.running {
            running.probe.observe(arm, probe::CHALLENGER, won, hands);
        }
    }

    /// The champion's parameter set.
    pub fn champion(&self) -> &Params {
        &self.champion
    }

    /// The parameter set the hand in play is played by: the champion's, or the challenger's in
    /// an orbit that drew it.
    pub fn params(&self) -> &Params {
        match &self.running {
            Some(running) if running.arm == Arm::Challenger => &running.challenger,
            _ => &self.champion,
        }
    }

    /// Notes one draw of the re-solve in the hand in play.
    pub fn decided(&mut self, drawn: Decided) {
        self.decided.push(drawn);
    }

    /// Takes a finished hand, and returns it as counted, or `None` when it cannot be.
    pub fn on_hand(&mut self, hand: &Hand) -> Option<Played> {
        let decided = std::mem::take(&mut self.decided);
        let before = if hand.table_id == self.table {
            self.stack
        } else {
            None
        };
        let result = result_of(hand);
        self.table.clone_from(&hand.table_id);
        self.stack = result.map(|(stack, _)| stack);
        let (Some(before), Some((after, big_blind))) = (before, result) else {
            // The stack is not known across this hand, so the orbit cannot go on through it.
            if let Some(running) = &mut self.running {
                running.close();
            }
            return None;
        };
        let net = after as i64 - before as i64;
        let luck = net as f64 - probe::corrected(net as f64, &decided);
        let digest = self.params().digest();
        let orbit = self.running.as_mut().map(|running| {
            let at = (running.id, running.orbit, running.arm);
            running.won += (net as f64 - luck) / big_blind.max(1) as f64;
            running.hands += 1;
            if running.hands == ORBIT {
                running.close();
            }
            at
        });
        Some(Played {
            hand_id: hand.hand_id.clone(),
            table_id: hand.table_id.clone(),
            digest,
            net,
            luck,
            big_blind,
            orbit,
        })
    }

    /// The running probe's verdict as of the last finished orbit, or `None` with no probe.
    pub fn verdict(&self) -> Option<Verdict> {
        self.running.as_ref().map(|running| running.probe.verdict())
    }

    /// The running probe's estimate of the challenger's edge, in bb/100, and the half-width of
    /// its 95% interval.
    pub fn estimate(&self) -> Option<(f64, f64)> {
        self.running
            .as_ref()
            .map(|running| running.probe.estimate())
    }

    /// Ends the running probe. With `promote`, its challenger becomes the champion.
    pub fn finish(&mut self, promote: bool) {
        if let Some(running) = self.running.take()
            && promote
        {
            self.champion = running.challenger;
        }
    }

    /// Makes `champion` the parameter set this bot plays by: a promotion adopted from the fleet.
    pub fn adopt(&mut self, champion: Params) {
        self.champion = champion;
    }
}

/// Our stack after `hand` and its big blind, when the hand holds its start and its result.
fn result_of(hand: &Hand) -> Option<(Chips, Chips)> {
    let mut seat = None;
    let mut stack = None;
    for text in &hand.frames {
        match parse(text) {
            Ok(Inbound::HandStart {
                seat: at,
                big_blind,
                ..
            }) => seat = Some((at, big_blind)),
            Ok(Inbound::HandResult { final_stacks, .. }) => {
                let (at, _) = seat?;
                stack = final_stacks
                    .iter()
                    .find(|&&(theirs, _)| theirs == at)
                    .map(|&(_, chips)| chips);
            }
            _ => {}
        }
    }
    Some((stack?, seat?.1))
}
