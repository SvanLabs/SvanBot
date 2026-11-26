//! The standing reference arm (`GLOSSARY.md`): every bot plays the frozen reference policy on
//! one orbit in ten, so each bot has its own live control from the same tables in the same hours.
//!
//! - The arm is a coin fixed by the bot's seed and the orbit's number, so a restart draws the
//!   same arm again: an orbit is never re-rolled.
//! - The bot's policy is the challenger and the reference is the champion. The chance the arm
//!   played had of being drawn is kept with the hand (1/10 for the reference, 9/10 for the
//!   policy), and [`Estimate`] feeds it to the probe's existing propensity-weighted estimate.
//! - A probe that is running owns every hand it plays: the standing arm waits, so a hand is
//!   never counted on both arms.
//!
//! This module is the arithmetic. `runtime` decides which arm a hand played and stores it.

use svanbot_core::rng::Rng;
use svanbot_strategy::params::Digest;

use crate::probe::{Arm, Hypothesis, Kind, Probe, Verdict};

/// The share of a bot's orbits that plays the reference policy.
pub const SHARE: f64 = 0.1;

/// The sides of the standing reference arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The bot's own policy.
    Policy,
    /// The frozen reference policy.
    Reference,
}

impl Side {
    /// The side's name in the store.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Reference => "reference",
        }
    }

    /// The side called `name`, or `None` when no side is.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "policy" => Some(Self::Policy),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// The arm an orbit plays and the chance it had of being drawn: a ten-sided die fixed by the
/// bot's seed and the orbit's number.
pub fn assign(seed: u64, orbit: u64) -> (Side, f64) {
    if Rng::stream(seed, orbit).below(10) == 0 {
        (Side::Reference, SHARE)
    } else {
        (Side::Policy, 1.0 - SHARE)
    }
}

/// A bot's seed: a hash of its name, so a restart and a new process draw the same arms. The name
/// is the bot's identity (one store per bot, ADR 0004), and no opponent or hand enters it.
pub fn bot_seed(bot: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325 ^ 0x7265_6600_0000_0000;
    for byte in bot.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// One orbit's standing arm, as the hand it played records it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Standing {
    /// The orbit's number: the unit the arm is drawn for, and the unit the estimate weighs.
    pub orbit: u64,
    pub side: Side,
    /// The chance the side played had of being drawn: 1/10 for the reference, 9/10 for the
    /// policy. The estimate turns it back into the chance the challenger had.
    pub chance: f64,
    /// The digest of the reference policy the orbit was measured against.
    pub reference: Digest,
}

impl Standing {
    /// The arm and the chance it had of being the challenger, as [`Probe::observe`] reads them.
    pub fn challenger(&self) -> (Arm, f64) {
        match self.side {
            Side::Policy => (Arm::Challenger, self.chance),
            Side::Reference => (Arm::Champion, 1.0 - self.chance),
        }
    }
}

/// The standing arm's estimate of the bot's policy against the reference: the probe's
/// propensity-weighted sequence, fed one observation per orbit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    probe: Probe,
}

impl Estimate {
    /// An estimate of `policy` against `reference`, with no end to its orbits.
    pub fn new(reference: Digest, policy: Digest) -> Self {
        Self::with_hypothesis(Hypothesis {
            champion: reference,
            challenger: policy,
            kind: Kind::Probe,
            harm_margin: 0.0,
            sim_estimate: None,
            max_orbits: u64::MAX,
        })
    }

    /// The harm guard of a promotion: clear after `hands` hands of the standing arm, unless the
    /// policy is clearly worse than the reference by more than `harm_margin` bb/100.
    pub fn guard(reference: Digest, policy: Digest, hands: u64, harm_margin: f64) -> Self {
        Self::with_hypothesis(Hypothesis {
            champion: reference,
            challenger: policy,
            kind: Kind::Guard { hands },
            harm_margin,
            sim_estimate: None,
            max_orbits: u64::MAX,
        })
    }

    fn with_hypothesis(hypothesis: Hypothesis) -> Self {
        Self {
            probe: Probe::new(hypothesis),
        }
    }

    /// Takes one standing orbit: the arm it played, the chance that arm had of being drawn, the
    /// big blinds it won with the luck taken out, and its hands.
    pub fn observe(&mut self, standing: &Standing, won: f64, hands: u32) {
        let (arm, chance) = standing.challenger();
        self.probe.observe(arm, chance, won, hands);
    }

    /// The policy's edge over the reference so far, in bb/100, and the half-width of its 95%
    /// interval.
    pub fn estimate(&self) -> (f64, f64) {
        self.probe.estimate()
    }

    /// The estimate's verdict as of the last finished orbit.
    pub fn verdict(&self) -> Verdict {
        self.probe.verdict()
    }
}
