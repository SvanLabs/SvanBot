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
use svanbot_strategy::style::Style;

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

/// What one bot's search does next: screen the challengers round its champion with short runs,
/// then spend a longer confirmation on the promising ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotNext {
    /// Run the short screening sim on the challenger, on the round's common deals.
    Screen { challenger: Params, seed: u64 },
    /// Run the longer confirmation on the challenger, on fresh deals.
    Confirm { challenger: Params, seed: u64 },
    /// A confirmation put the challenger above the champion: it is the bot's candidate for its
    /// own probe and promotion (#1222).
    Candidate {
        challenger: Params,
        screening: Verdict,
        confirmation: Verdict,
    },
    /// Every challenger is screened out or failed its confirmation, or every promising one is
    /// confirmed without a verdict: the incumbent holds.
    Held,
}

/// What `bot`'s search does next round `champion`, given its log and the style it plays.
///
/// Every challenger gets a short screening sim on the same deals, so the ranking is fair. The
/// promising ones, those with a screening mean above the champion, get a longer confirmation on
/// fresh deals; only a confirmation whose whole 95% interval is above 0 is a candidate, so no
/// screening deal is used as promotion evidence. A style keeps the search inside the numbers it
/// bounds and never moves a number it pins.
pub fn next_bot(champion: &Params, style: &Style, log: &[Entry]) -> BotNext {
    let digest = champion.digest();
    let challengers = bot_challengers(champion, style);
    for challenger in &challengers {
        if screening(digest, *challenger, log).is_none() {
            return BotNext::Screen {
                challenger: *challenger,
                seed: screen_seed(digest),
            };
        }
    }
    let mut promising: Vec<(Params, Verdict)> = challengers
        .into_iter()
        .filter_map(|challenger| {
            let screening = screening(digest, challenger, log)?;
            (screening.mbb_per_100 > 0.0).then_some((challenger, screening))
        })
        .collect();
    if promising.is_empty() {
        return BotNext::Held;
    }
    promising.sort_by(|(_, left), (_, right)| right.mbb_per_100.total_cmp(&left.mbb_per_100));
    promising.truncate(promising.len().div_ceil(2));
    for (challenger, _) in &promising {
        if confirmation(digest, *challenger, log).is_none() {
            return BotNext::Confirm {
                challenger: *challenger,
                seed: seed(digest, challenger.digest(), Stage::Confirmation),
            };
        }
    }
    let best = promising
        .into_iter()
        .filter_map(|(challenger, screening)| {
            let confirmation = confirmation(digest, challenger, log)?;
            confirmation
                .above_zero()
                .then_some((challenger, screening, confirmation))
        })
        .max_by(|(_, _, left), (_, _, right)| {
            (left.mbb_per_100 - left.ci95).total_cmp(&(right.mbb_per_100 - right.ci95))
        });
    match best {
        Some((challenger, screening, confirmation)) => BotNext::Candidate {
            challenger,
            screening,
            confirmation,
        },
        None => BotNext::Held,
    }
}

/// Every one-step challenger round `champion` that the style lets the search move, in the order
/// the rings try them.
fn bot_challengers(champion: &Params, style: &Style) -> Vec<Params> {
    let mut found = Vec::new();
    for ring in RINGS {
        for (name, value) in champion.named() {
            for step in ring {
                let Some(challenger) = champion.with(name, value * step) else {
                    continue;
                };
                if challenger == *champion
                    || !challenger.in_range()
                    || !style.allows(name, value * step)
                    || found.contains(&challenger)
                {
                    continue;
                }
                found.push(challenger);
            }
        }
    }
    found
}

/// The screening verdict already logged for `challenger` against `champion`, if it ran.
fn screening(champion: Digest, challenger: Params, log: &[Entry]) -> Option<Verdict> {
    verdict_at(champion, challenger, Stage::First, log)
}

/// The confirmation verdict already logged for `challenger` against `champion`, if it ran.
fn confirmation(champion: Digest, challenger: Params, log: &[Entry]) -> Option<Verdict> {
    verdict_at(champion, challenger, Stage::Confirmation, log)
}

/// The verdict already logged for `challenger` against `champion` at `stage`, if it ran.
fn verdict_at(
    champion: Digest,
    challenger: Params,
    stage: Stage,
    log: &[Entry],
) -> Option<Verdict> {
    log.iter().find_map(|entry| match *entry {
        Entry::Simmed { step, verdict }
            if step.champion == champion
                && step.challenger == challenger
                && step.stage == stage =>
        {
            Some(verdict)
        }
        _ => None,
    })
}

/// The common screening seed of a champion's round: every challenger meets the same deals, and
/// the confirmation's own stream is different.
fn screen_seed(champion: Digest) -> u64 {
    const SCREEN: u64 = 0x5eed_5c7e_0000_0076;
    Rng::stream(champion.value() ^ SCREEN, 0).next_u64()
}
