//! The search (V11-49, `GLOSSARY.md`): the learner's unattended hunt for a challenger round the
//! champion, one paired sim at a time.
//!
//! - Each number of the champion is moved a step down and a step up, in rings of shrinking steps
//!   ([`RINGS`]): the wide ring first, the finer ones once it is spent.
//! - A challenger's first sim must put the whole 95% interval above the champion. Then a
//!   **confirmation** plays it again on fresh deals, and only a challenger both put above is
//!   proposed. A search tries many challengers, and one lucky sim among them must not become a
//!   promotion: a second, independent sim has to agree.
//! - Each sim's seed is named by the champion and the challenger, so the same step is the same
//!   sim on any machine, and a step the log holds is never run again.
//! - The log is keyed by the champion: a promotion starts the search afresh round the new one.
//!
//! What the search proposes still goes through the canary's harm guard before the fleet adopts
//! it. The search only decides what to sim next; running a sim and keeping the log are the
//! caller's.

use svanbot_core::rng::Rng;
use svanbot_strategy::params::{Digest, Params};

use crate::paired::Verdict;

/// The steps each number is moved by, a ring at a time: each pair a step down and its inverse up.
pub const RINGS: [[f64; 2]; 3] = [[0.8, 1.25], [0.9, 1.0 / 0.9], [0.95, 1.0 / 0.95]];

/// Which of a challenger's two sims a step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The sim that finds the challenger above the champion, or not.
    First,
    /// The sim on fresh deals that must agree before the challenger is proposed.
    Confirmation,
}

/// A paired sim the search asks for: `challenger` against the champion named `champion`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub champion: Digest,
    pub challenger: Params,
    pub stage: Stage,
    pub seed: u64,
}

/// What the log holds of one step: the sim it ran and what it found, or that the challenger was
/// proposed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry {
    Simmed {
        step: Step,
        verdict: Verdict,
    },
    Proposed {
        champion: Digest,
        challenger: Params,
    },
}

/// What the search does next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Next {
    /// Run this sim and log what it found.
    Sim(Step),
    /// Both sims put `challenger` above the champion: propose it, and log that it was.
    Propose {
        challenger: Params,
        first: Verdict,
        confirmation: Verdict,
    },
    /// Every ring round this champion is spent.
    Exhausted,
}

/// What the search does next round `champion`, given everything the log holds.
pub fn next(champion: &Params, log: &[Entry]) -> Next {
    let digest = champion.digest();
    for ring in RINGS {
        for (name, value) in champion.named() {
            for step in ring {
                let Some(challenger) = champion.with(name, value * step) else {
                    continue;
                };
                if challenger == *champion || !challenger.in_range() {
                    continue;
                }
                if let Some(found) = judge(digest, challenger, log) {
                    return found;
                }
            }
        }
    }
    Next::Exhausted
}

/// What is left to do for `challenger` against the champion named `champion`, or `None` when
/// it is done with: proposed already, or not above in one of its sims.
fn judge(champion: Digest, challenger: Params, log: &[Entry]) -> Option<Next> {
    let mut found = [None, None];
    for entry in log {
        match *entry {
            Entry::Proposed {
                champion: against,
                challenger: proposed,
            } if against == champion && proposed == challenger => return None,
            Entry::Simmed { step, verdict }
                if step.champion == champion && step.challenger == challenger =>
            {
                found[step.stage as usize].get_or_insert(verdict);
            }
            _ => {}
        }
    }
    let ask = |stage| {
        Some(Next::Sim(Step {
            champion,
            challenger,
            stage,
            seed: seed(champion, challenger.digest(), stage),
        }))
    };
    match found {
        [None, _] => ask(Stage::First),
        [Some(first), _] if !first.above_zero() => None,
        [Some(_), None] => ask(Stage::Confirmation),
        [Some(first), Some(confirmation)] if confirmation.above_zero() => Some(Next::Propose {
            challenger,
            first,
            confirmation,
        }),
        [Some(_), Some(_)] => None,
    }
}

/// The seed of a step: a stream of the champion and the challenger, one index a stage, so the
/// confirmation plays deals the first sim did not.
fn seed(champion: Digest, challenger: Digest, stage: Stage) -> u64 {
    let named = champion.value() ^ challenger.value().rotate_left(32);
    Rng::stream(named, stage as u64).next_u64()
}
