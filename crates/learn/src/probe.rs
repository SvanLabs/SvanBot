//! The Probe (`GLOSSARY.md`): a live experiment on the canary that compares the champion with
//! one challenger (`docs/research/2026-11/04-learner-experiments.md`, section 5).
//!
//! - A **hypothesis** is named by its pair of parameter digests, so it survives a refit.
//! - Each **orbit** plays one arm, drawn by a coin fixed by the probe, the table and the orbit.
//!   The coin's chance is logged with the orbit, and the estimate weights by it.
//! - An orbit's result has the luck of our own draws taken out ([`corrected`]): AIVAT's term for
//!   a player whose strategy is known, which has mean zero however wrong the values are.
//! - The **verdict** comes from a confidence sequence ([`Sequence`]) on the challenger's edge,
//!   so it holds at whatever moment it is read. A probe ends supported, harmful, at odds with
//!   its sim estimate, or out of orbits. A **guard** only checks for harm, over a set number of
//!   hands, before the rest of the fleet adopts a promotion.
//!
//! This module is the arithmetic. `runtime` latches the arm, stores the orbits and acts on the
//! verdict.

use svanbot_core::rng::Rng;
use svanbot_strategy::params::Digest;

/// What a probe is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Compare the challenger with the champion until a verdict or the budget.
    Probe,
    /// The harm guard of a promotion: clear after this many hands unless the challenger, the
    /// promoted set, is clearly losing to the champion it replaced.
    Guard { hands: u64 },
}

/// One hypothesis: this challenger against this champion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hypothesis {
    pub champion: Digest,
    pub challenger: Digest,
    pub kind: Kind,
    /// The loss, in bb/100, past which the challenger is harmful.
    pub harm_margin: f64,
    /// What the paired sim said the challenger's edge is, in bb/100, when it said anything.
    pub sim_estimate: Option<f64>,
    /// The orbits a probe may take before it ends inconclusive.
    pub max_orbits: u64,
}

impl Hypothesis {
    /// The probe's id: its pair of digests, in order.
    pub fn id(&self) -> u64 {
        Rng::stream(self.champion.value(), self.challenger.value()).next_u64()
    }
}

/// One of the two parameter sets in a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    Champion,
    Challenger,
}

/// The chance an orbit plays the challenger.
pub const CHALLENGER: f64 = 0.5;

/// The arm an orbit plays, and the chance it had of being the challenger: a coin fixed by the
/// probe's id, the table and the orbit's number at it.
pub fn arm(id: u64, table: u64, orbit: u64) -> (Arm, f64) {
    let coin = Rng::stream(Rng::stream(id, table).next_u64(), orbit).next_u64();
    let arm = if coin >> 63 == 1 {
        Arm::Challenger
    } else {
        Arm::Champion
    };
    (arm, CHALLENGER)
}

/// One of our decisions as it was made: the chance of each action, what each was valued at, and
/// the one drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Decided {
    pub strategy: Vec<f64>,
    pub values: Vec<f64>,
    pub taken: usize,
}

/// `result` with the luck of our own draws taken out: for each decision, the value of the action
/// drawn less the strategy's mean value. The values may be wrong; the mean of the correction is
/// still zero, because the strategy is known exactly.
pub fn corrected(result: f64, decided: &[Decided]) -> f64 {
    decided.iter().fold(result, |result, decision| {
        let mean: f64 = decision
            .strategy
            .iter()
            .zip(&decision.values)
            .map(|(chance, value)| chance * value)
            .sum();
        let drawn = decision.values.get(decision.taken).copied().unwrap_or(mean);
        result - (drawn - mean)
    })
}

/// A confidence sequence on the mean of a stream: an interval that holds at every look at once,
/// so it can be read after each observation and acted on (the asymptotic sequence of
/// Waudby-Smith, Arbour, Sinha, Kennedy and Ramdas, 2021: a normal mixture over the mean, as in
/// the mSPRT, with the spread estimated as it goes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sequence {
    count: f64,
    sum: f64,
    squares: f64,
    /// The mixing variance: it sets the count at which the interval is tightest.
    mixing: f64,
}

impl Sequence {
    /// A sequence that is tightest around `tuned_for` observations, at the 5% level.
    pub fn new(tuned_for: f64) -> Self {
        let log = -2.0 * 0.05f64.ln();
        Self {
            count: 0.0,
            sum: 0.0,
            squares: 0.0,
            mixing: (log + (log + 1.0).ln()) / tuned_for.max(1.0),
        }
    }

    pub fn observe(&mut self, value: f64) {
        self.count += 1.0;
        self.sum += value;
        self.squares += value * value;
    }

    pub fn count(&self) -> f64 {
        self.count
    }

    pub fn mean(&self) -> f64 {
        self.sum / self.count.max(1.0)
    }

    /// The half-width of the interval at level `alpha`, infinite before two observations.
    pub fn radius(&self, alpha: f64) -> f64 {
        let n = self.count;
        if n < 2.0 {
            return f64::INFINITY;
        }
        let spread = ((self.squares - self.sum * self.sum / n) / (n - 1.0))
            .max(0.0)
            .sqrt();
        let scaled = n * self.mixing + 1.0;
        spread * (2.0 * scaled / (n * n * self.mixing) * (scaled.sqrt() / alpha).ln()).sqrt()
    }
}

/// A probe's conclusion (`GLOSSARY.md`). It holds at the moment it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No conclusion yet.
    Running,
    /// The challenger wins: the whole interval is above 0.
    Supported,
    /// The challenger loses: the whole interval is below the harm margin. A guard rolls back.
    Harmful,
    /// The interval leaves out what the paired sim estimated.
    SimGap,
    /// The orbits ran out with the arms not told apart.
    Inconclusive,
    /// A guard's hands are played with no harm found: the fleet may adopt.
    Cleared,
}

/// Orbits before a probe says anything: the spread is not yet known.
pub const MIN_ORBITS: f64 = 30.0;

/// A running probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    hypothesis: Hypothesis,
    /// The challenger's edge, one estimate an orbit, in bb/100.
    edge: Sequence,
    hands: u64,
}

impl Probe {
    pub fn new(hypothesis: Hypothesis) -> Self {
        let tuned_for = match hypothesis.kind {
            Kind::Probe => 2_000.0,
            Kind::Guard { hands } => hands as f64 / 12.0,
        };
        Self {
            hypothesis,
            edge: Sequence::new(tuned_for),
            hands: 0,
        }
    }

    /// Takes one orbit: the arm it played, the chance that arm had of being the challenger, the
    /// big blinds it won with the luck taken out, and its hands. The orbit's rate is weighted by
    /// one over its arm's chance, so the mean over orbits is the challenger's edge whatever the
    /// chance was.
    pub fn observe(&mut self, arm: Arm, chance: f64, won: f64, hands: u32) {
        if hands == 0 {
            return;
        }
        let rate = won / f64::from(hands) * 100.0;
        self.edge.observe(match arm {
            Arm::Challenger => rate / chance,
            Arm::Champion => -rate / (1.0 - chance),
        });
        self.hands += u64::from(hands);
    }

    /// The challenger's edge so far, in bb/100, and the half-width of its 95% interval.
    pub fn estimate(&self) -> (f64, f64) {
        (self.edge.mean(), self.edge.radius(0.05))
    }

    pub fn verdict(&self) -> Verdict {
        let orbits = self.edge.count();
        let (mean, radius) = self.estimate();
        let known = orbits >= MIN_ORBITS;
        if known && mean + radius < -self.hypothesis.harm_margin {
            return Verdict::Harmful;
        }
        if let Kind::Guard { hands } = self.hypothesis.kind {
            return if self.hands >= hands {
                Verdict::Cleared
            } else {
                Verdict::Running
            };
        }
        if known && mean - radius > 0.0 {
            return Verdict::Supported;
        }
        if known
            && self
                .hypothesis
                .sim_estimate
                .is_some_and(|sim| (sim - mean).abs() > radius)
        {
            return Verdict::SimGap;
        }
        if orbits >= self.hypothesis.max_orbits as f64 {
            return Verdict::Inconclusive;
        }
        Verdict::Running
    }
}
